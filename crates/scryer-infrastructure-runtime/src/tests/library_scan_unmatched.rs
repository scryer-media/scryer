use super::*;

#[tokio::test]
async fn library_scan_unmatched_items_round_trip_and_preserve_created_at() {
    let db = std::env::temp_dir().join(format!(
        "scryer_scan_unmatched_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let library_scan_unmatched = library_scan_unmatched_store(&services);

    let created_at = "2026-04-07T00:00:00Z".to_string();
    let updated_at = "2026-04-07T00:00:00Z".to_string();
    let item = LibraryScanUnmatchedItem {
        id: "library_scan_unmatched:test".to_string(),
        library_id: scryer_domain::default_library_id_for_facet(&MediaFacet::Movie),
        facet: MediaFacet::Movie,
        status: PendingImportStatus::Pending,
        title_id: None,
        scan_session_id: "session-1".to_string(),
        scan_root: "/library".to_string(),
        item_path: "/library/Unknown.Movie.2020.mkv".to_string(),
        display_name: "Unknown.Movie.2020".to_string(),
        query: "Unknown Movie".to_string(),
        year_hint: Some(2020),
        reason_code: "no_metadata_search_results".to_string(),
        error_message: None,
        search_attempts: vec![LibraryScanUnmatchedSearchAttempt {
            query: "Unknown Movie".to_string(),
            result_count: 0,
            top_results: Vec::new(),
        }],
        size_bytes: Some(1_234_567_890),
        created_at: created_at.clone(),
        updated_at: updated_at.clone(),
    };

    library_scan_unmatched
        .upsert_library_scan_unmatched_item(&item)
        .await
        .expect("insert unmatched item");

    let count = library_scan_unmatched
        .count_library_scan_unmatched_items(
            Some(MediaFacet::Movie),
            Some("/library"),
            Some(PendingImportStatus::Pending),
        )
        .await
        .expect("count unmatched items after insert");
    assert_eq!(count, 1);

    let listed = library_scan_unmatched
        .list_library_scan_unmatched_items(
            Some(MediaFacet::Movie),
            Some("/library"),
            Some(PendingImportStatus::Pending),
            10,
            0,
        )
        .await
        .expect("list unmatched items after insert");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].search_attempts.len(), 1);
    assert_eq!(listed[0].search_attempts[0].query, "Unknown Movie");
    assert_eq!(listed[0].created_at, created_at);
    assert_eq!(listed[0].size_bytes, Some(1_234_567_890));

    // The rescan reports no size (as a folder-shaped or unreadable candidate
    // would); the upsert must keep the size the earlier scan recorded.
    let updated = LibraryScanUnmatchedItem {
        scan_session_id: "session-2".to_string(),
        reason_code: "no_acceptable_metadata_match".to_string(),
        search_attempts: vec![LibraryScanUnmatchedSearchAttempt {
            query: "Unknown Movie 2020".to_string(),
            result_count: 2,
            top_results: vec![
                "Known Movie (2019)".to_string(),
                "Known Movie 2 (2020)".to_string(),
            ],
        }],
        size_bytes: None,
        created_at: "2026-04-08T00:00:00Z".to_string(),
        updated_at: "2026-04-08T01:00:00Z".to_string(),
        ..item.clone()
    };

    library_scan_unmatched
        .upsert_library_scan_unmatched_item(&updated)
        .await
        .expect("update unmatched item");

    let listed_after_update = library_scan_unmatched
        .list_library_scan_unmatched_items(
            Some(MediaFacet::Movie),
            Some("/library"),
            Some(PendingImportStatus::Pending),
            10,
            0,
        )
        .await
        .expect("list unmatched items after update");
    assert_eq!(listed_after_update.len(), 1);
    assert_eq!(listed_after_update[0].scan_session_id, "session-2");
    assert_eq!(
        listed_after_update[0].reason_code,
        "no_acceptable_metadata_match"
    );
    assert_eq!(listed_after_update[0].created_at, item.created_at);
    assert_eq!(listed_after_update[0].updated_at, updated.updated_at);
    assert_eq!(listed_after_update[0].search_attempts[0].result_count, 2);
    assert_eq!(
        listed_after_update[0].size_bytes,
        Some(1_234_567_890),
        "a sizeless rescan must not erase a previously recorded size"
    );

    library_scan_unmatched
        .delete_library_scan_unmatched_item(&item.library_id, MediaFacet::Movie, &item.item_path)
        .await
        .expect("delete unmatched item");

    let count_after_delete = library_scan_unmatched
        .count_library_scan_unmatched_items(
            Some(MediaFacet::Movie),
            Some("/library"),
            Some(PendingImportStatus::Pending),
        )
        .await
        .expect("count unmatched items after delete");
    assert_eq!(count_after_delete, 0);

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn library_scan_unmatched_upsert_heals_legacy_id_on_library_path_conflict() {
    let db = std::env::temp_dir().join(format!(
        "scryer_scan_unmatched_legacy_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let library_scan_unmatched = library_scan_unmatched_store(&services);

    fn unmatched_id(input: &str) -> String {
        let hash = aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, input.as_bytes());
        let hex = hash
            .as_ref()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        format!("library_scan_unmatched:{}", &hex[..24])
    }

    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Series);
    let item_path = "/library/Harbor Pals/Harbor.Pals.S01E01.720p.WEB-DL.AV1.AAC2.0-NTb";
    let created_at = "2026-04-07T00:00:00Z".to_string();
    let legacy_id = unmatched_id(format!("series:{item_path}").as_str());
    let current_id = unmatched_id(format!("series:{library_id}:{item_path}").as_str());

    let legacy_item = LibraryScanUnmatchedItem {
        id: legacy_id.clone(),
        library_id: library_id.clone(),
        facet: MediaFacet::Series,
        status: PendingImportStatus::Pending,
        title_id: Some("title-harbor-pals".to_string()),
        scan_session_id: "legacy-session".to_string(),
        scan_root: "/library/Harbor Pals".to_string(),
        item_path: item_path.to_string(),
        display_name: "4f8e2c7a91b6d3e0".to_string(),
        query: "Harbor Pals".to_string(),
        year_hint: None,
        reason_code: "legacy_row".to_string(),
        error_message: None,
        search_attempts: Vec::new(),
        size_bytes: None,
        created_at: created_at.clone(),
        updated_at: created_at.clone(),
    };

    library_scan_unmatched
        .upsert_library_scan_unmatched_item(&legacy_item)
        .await
        .expect("insert legacy unmatched item");

    let refreshed_item = LibraryScanUnmatchedItem {
        id: current_id.clone(),
        library_id: library_id.clone(),
        facet: MediaFacet::Series,
        status: PendingImportStatus::Pending,
        title_id: Some("title-harbor-pals".to_string()),
        scan_session_id: "current-session".to_string(),
        scan_root: "/library/Harbor Pals".to_string(),
        item_path: item_path.to_string(),
        display_name: "4f8e2c7a91b6d3e0".to_string(),
        query: "Harbor Pals".to_string(),
        year_hint: None,
        reason_code: "scan_refresh".to_string(),
        error_message: None,
        search_attempts: vec![LibraryScanUnmatchedSearchAttempt {
            query: "Harbor Pals".to_string(),
            result_count: 1,
            top_results: vec!["Harbor Pals".to_string()],
        }],
        size_bytes: None,
        created_at: "2026-04-08T00:00:00Z".to_string(),
        updated_at: "2026-04-08T01:00:00Z".to_string(),
    };

    library_scan_unmatched
        .upsert_library_scan_unmatched_item(&refreshed_item)
        .await
        .expect("upsert current unmatched item over legacy row");

    let count = library_scan_unmatched
        .count_library_scan_unmatched_items(
            Some(MediaFacet::Series),
            Some("/library/Harbor Pals"),
            Some(PendingImportStatus::Pending),
        )
        .await
        .expect("count unmatched items after heal");
    assert_eq!(count, 1);

    let healed = library_scan_unmatched
        .get_library_scan_unmatched_item(&current_id)
        .await
        .expect("load healed unmatched item")
        .expect("healed unmatched item should exist");
    assert_eq!(healed.id, current_id);
    assert_eq!(healed.scan_session_id, "current-session");
    assert_eq!(healed.reason_code, "scan_refresh");
    assert_eq!(healed.created_at, created_at);

    let legacy_lookup = library_scan_unmatched
        .get_library_scan_unmatched_item(&legacy_id)
        .await
        .expect("load legacy unmatched item after heal");
    assert!(legacy_lookup.is_none());

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn library_scan_unmatched_upsert_preserves_ignored_status_for_scan_refresh() {
    let db = std::env::temp_dir().join(format!(
        "scryer_scan_unmatched_status_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let library_scan_unmatched = library_scan_unmatched_store(&services);

    let ignored_item = LibraryScanUnmatchedItem {
        id: "library_scan_unmatched:ignored".to_string(),
        library_id: scryer_domain::default_library_id_for_facet(&MediaFacet::Movie),
        facet: MediaFacet::Movie,
        status: PendingImportStatus::Ignored,
        title_id: None,
        scan_session_id: "session-1".to_string(),
        scan_root: "/library".to_string(),
        item_path: "/library/Unknown.Movie.2020.mkv".to_string(),
        display_name: "Unknown.Movie.2020".to_string(),
        query: "Unknown Movie".to_string(),
        year_hint: Some(2020),
        reason_code: "no_metadata_search_results".to_string(),
        error_message: None,
        search_attempts: vec![],
        size_bytes: None,
        created_at: "2026-04-07T00:00:00Z".to_string(),
        updated_at: "2026-04-07T00:00:00Z".to_string(),
    };

    library_scan_unmatched
        .upsert_library_scan_unmatched_item(&ignored_item)
        .await
        .expect("seed ignored item");

    let scan_refresh = LibraryScanUnmatchedItem {
        status: PendingImportStatus::Pending,
        scan_session_id: "session-2".to_string(),
        updated_at: "2026-04-08T00:00:00Z".to_string(),
        ..ignored_item.clone()
    };

    library_scan_unmatched
        .upsert_library_scan_unmatched_item(&scan_refresh)
        .await
        .expect("refresh ignored item from scan");

    let pending_count = library_scan_unmatched
        .count_library_scan_unmatched_items(
            Some(MediaFacet::Movie),
            Some("/library"),
            Some(PendingImportStatus::Pending),
        )
        .await
        .expect("count pending items");
    let ignored_count = library_scan_unmatched
        .count_library_scan_unmatched_items(
            Some(MediaFacet::Movie),
            Some("/library"),
            Some(PendingImportStatus::Ignored),
        )
        .await
        .expect("count ignored items");
    assert_eq!(pending_count, 0);
    assert_eq!(ignored_count, 1);

    let stored = library_scan_unmatched
        .get_library_scan_unmatched_item(&ignored_item.id)
        .await
        .expect("load stored item")
        .expect("item should still exist");
    assert_eq!(stored.status, PendingImportStatus::Ignored);
    assert_eq!(stored.scan_session_id, "session-2");

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn library_scan_unmatched_items_delete_for_title_keeps_other_rows() {
    let db = std::env::temp_dir().join(format!(
        "scryer_scan_unmatched_title_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let library_scan_unmatched = library_scan_unmatched_store(&services);

    let row = |id: &str, title_id: Option<&str>| LibraryScanUnmatchedItem {
        id: format!("library_scan_unmatched:{id}"),
        library_id: scryer_domain::default_library_id_for_facet(&MediaFacet::Series),
        facet: MediaFacet::Series,
        status: PendingImportStatus::Pending,
        title_id: title_id.map(str::to_string),
        scan_session_id: "session-1".to_string(),
        scan_root: "/library/Show".to_string(),
        item_path: format!("/library/Show/{id}.avi"),
        display_name: id.to_string(),
        query: id.to_string(),
        year_hint: None,
        reason_code: "episode_identity_missing".to_string(),
        error_message: None,
        search_attempts: Vec::new(),
        size_bytes: None,
        created_at: "2026-04-07T00:00:00Z".to_string(),
        updated_at: "2026-04-07T00:00:00Z".to_string(),
    };
    for item in [
        row("first", Some("title-gone")),
        row("second", Some("title-gone")),
        row("kept", Some("title-kept")),
        row("unbound", None),
    ] {
        library_scan_unmatched
            .upsert_library_scan_unmatched_item(&item)
            .await
            .expect("insert unmatched item");
    }

    let removed = library_scan_unmatched
        .delete_for_title("title-gone")
        .await
        .expect("delete rows for title");
    assert_eq!(removed, 2);

    let mut remaining = library_scan_unmatched
        .list_library_scan_unmatched_items(Some(MediaFacet::Series), None, None, 10, 0)
        .await
        .expect("list remaining rows")
        .into_iter()
        .map(|item| item.id)
        .collect::<Vec<_>>();
    remaining.sort();
    assert_eq!(
        remaining,
        vec![
            "library_scan_unmatched:kept".to_string(),
            "library_scan_unmatched:unbound".to_string(),
        ]
    );
    let _ = std::fs::remove_file(db);
}

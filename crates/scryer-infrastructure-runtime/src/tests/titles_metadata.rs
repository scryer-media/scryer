use super::*;
use crate::queries::sql_runtime::{SqlArg, SqlRuntime, StoreDatastore};

#[tokio::test]
async fn nzbget_client_is_sendable() {
    let client = NzbgetDownloadClient::new(
        "http://127.0.0.1:6789".to_string(),
        Some("user".into()),
        Some("pass".into()),
        "SCORE".to_string(),
    );
    // We only validate that it can be built and is callable in type system.
    let _ = client.endpoint();
}

async fn insert_test_library(services: &SqliteServices, id: &str, facet: MediaFacet) {
    sqlx::query(
        "INSERT INTO libraries
            (id, facet, name, slug, is_default, created_at, updated_at)
         VALUES (?, ?, ?, ?, 0, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)",
    )
    .bind(id)
    .bind(facet.as_str())
    .bind(id)
    .bind(id)
    .execute(services.pool())
    .await
    .expect("test library should insert");
}

#[tokio::test]
async fn title_quality_profile_reference_count_matches_resolver_normalization() {
    let (services, db) = temp_services("scryer_title_quality_profile_references").await;
    let catalog = title_store(&services);

    for (id, tag) in [
        ("title-profile-exact", "scryer:quality-profile:1080p"),
        ("title-profile-whitespace", "scryer:quality-profile: 1080p "),
        (
            "title-profile-control-whitespace",
            "scryer:quality-profile:\t1080p\n",
        ),
        (
            "title-profile-unicode-whitespace",
            "scryer:quality-profile:\u{2003}1080p\u{2003}",
        ),
        ("title-profile-value-case", "scryer:quality-profile:1080P"),
        ("title-profile-wrong-case", "SCRYER:quality-profile:1080p"),
    ] {
        let mut title = make_test_title(id, None);
        title.tags = vec![tag.to_string()];
        TitleRepository::create(&catalog, title)
            .await
            .expect("title should insert");
    }

    assert_eq!(
        TitleRepository::count_by_quality_profile_id(&catalog, "1080p")
            .await
            .expect("profile references should count"),
        5,
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn hydrated_original_language_can_be_set_cleared_or_left_unchanged() {
    let (services, db) = temp_services("scryer_title_original_language_update").await;
    let catalog = title_store(&services);
    let title = make_test_title("title-original-language-update", None);
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");

    let set = TitleRepository::update_title_hydrated_metadata(
        &catalog,
        &title.id,
        TitleMetadataUpdate {
            language: MetadataFieldUpdate::Set("jpn".to_string()),
            ..Default::default()
        },
    )
    .await
    .expect("original language should set");
    assert_eq!(set.language.as_deref(), Some("jpn"));

    let unchanged = TitleRepository::update_title_hydrated_metadata(
        &catalog,
        &title.id,
        TitleMetadataUpdate {
            overview: Some("partial update".to_string()),
            ..Default::default()
        },
    )
    .await
    .expect("partial metadata should update");
    assert_eq!(unchanged.language.as_deref(), Some("jpn"));

    let cleared = TitleRepository::update_title_hydrated_metadata(
        &catalog,
        &title.id,
        TitleMetadataUpdate {
            language: MetadataFieldUpdate::Clear,
            ..Default::default()
        },
    )
    .await
    .expect("missing authoritative language should clear stale metadata");
    assert_eq!(cleared.language, None);

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_create_requires_an_existing_library_for_the_title_facet() {
    let (services, db) = temp_services("scryer_title_library_ownership_validation").await;
    let catalog = title_store(&services);

    let mut missing_library = make_test_title("title-missing-library", None);
    missing_library.library_id = "missing-library".to_string();
    let missing_err = TitleRepository::create(&catalog, missing_library)
        .await
        .expect_err("missing library should reject title creation");
    assert!(matches!(
        missing_err,
        scryer_application::AppError::Validation(message)
            if message.contains("missing-library") && message.contains("movie")
    ));

    let mut wrong_facet = make_test_title("title-wrong-library-facet", None);
    wrong_facet.facet = MediaFacet::Series;
    let facet_err = TitleRepository::create(&catalog, wrong_facet)
        .await
        .expect_err("library from another facet should reject title creation");
    assert!(matches!(
        facet_err,
        scryer_application::AppError::Validation(message)
            if message.contains("series")
    ));

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_queries_prefer_local_cached_poster_url() {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_poster_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = title_store(&services);
    let title_images = title_image_store(&services);

    let title = make_test_title("title-1", Some("https://artworks.thetvdb.com/poster.jpg"));
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");

    let before_cache = TitleRepository::get_by_id(&catalog, &title.id)
        .await
        .expect("title lookup should succeed")
        .expect("title should exist");
    assert_eq!(
        before_cache.poster_url.as_deref(),
        Some("https://artworks.thetvdb.com/poster.jpg")
    );

    let variant_bytes = vec![7, 8, 9];
    let variant_digest = format!("blake3:{}", blake3::hash(&variant_bytes).to_hex());
    let expected_local_url = format!(
        "/images/titles/title-1/poster/w250?v={}",
        variant_digest
            .split_once(':')
            .map(|(_, digest)| digest)
            .unwrap_or(&variant_digest)
            .chars()
            .take(16)
            .collect::<String>()
    );
    title_images
        .upsert_title_image_source_result(
            &title.id,
            TitleImageSourceResult {
                kind: TitleImageKind::Poster,
                requested_source_url: "https://artworks.thetvdb.com/poster.jpg".to_string(),
                source_url: "https://artworks.thetvdb.com/poster.jpg".to_string(),
                source_etag: Some("\"etag-1\"".to_string()),
                source_last_modified: None,
                source_format: "jpeg".to_string(),
                source_width: 1000,
                source_height: 1500,
                variants: vec![TitleImageVariantRecord {
                    variant_key: "w250".to_string(),
                    format: "avif".to_string(),
                    width: 250,
                    height: 375,
                    bytes: variant_bytes,
                    digest: variant_digest,
                }],
            },
            None,
        )
        .await
        .expect("title image should insert");

    let after_cache = TitleRepository::get_by_id(&catalog, &title.id)
        .await
        .expect("title lookup should succeed")
        .expect("title should exist");
    assert_eq!(
        after_cache.poster_url.as_deref(),
        Some(expected_local_url.as_str())
    );
    assert_eq!(
        after_cache.poster_source_url.as_deref(),
        Some("https://artworks.thetvdb.com/poster.jpg")
    );

    let listed = TitleRepository::list(&catalog, None, None)
        .await
        .expect("title list should succeed");
    assert_eq!(listed.len(), 1);
    assert_eq!(
        listed[0].poster_url.as_deref(),
        Some(expected_local_url.as_str())
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn hydrated_title_metadata_with_extra_external_ids_completes_on_single_connection_sqlite() {
    let (services, db) =
        single_connection_services("scryer_title_hydration_single_connection").await;
    let catalog = title_store(&services);

    let mut title = make_test_title("title-hydration-extra-ids", None);
    title.facet = MediaFacet::Anime;
    title.library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Anime);
    title.external_ids = vec![
        ExternalId {
            source: "tvdb".to_string(),
            value: "12345".to_string(),
        },
        ExternalId {
            source: "mal".to_string(),
            value: "old-mal".to_string(),
        },
    ];
    // The hydration payload's `extra_tags` are the anime metadata trio, which
    // all live in the reserved `scryer:` namespace; a stale one is replaced by
    // its successor. `season 1: opener` is the counter-case: an ordinary user
    // label that happens to contain a colon shares no namespace with anything
    // and must survive hydration untouched.
    title.tags = vec![
        "scryer:mal-score:old".to_string(),
        "season 1: opener".to_string(),
        "keep".to_string(),
    ];

    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");

    let update = TitleMetadataUpdate {
        metadata_language: Some("eng".to_string()),
        metadata_fetched_at: Some(Utc::now().to_rfc3339()),
        extra_external_ids: vec![
            ExternalId {
                source: "mal".to_string(),
                value: "834".to_string(),
            },
            ExternalId {
                source: "anilist".to_string(),
                value: "269".to_string(),
            },
        ],
        extra_tags: vec!["scryer:mal-score:9.1".to_string()],
        ..TitleMetadataUpdate::default()
    };

    let updated = timeout(
        Duration::from_secs(1),
        TitleRepository::update_title_hydrated_metadata(&catalog, &title.id, update),
    )
    .await
    .expect("hydrated metadata update should not self-deadlock on single-connection sqlite")
    .expect("hydrated metadata update should succeed");

    assert!(
        updated
            .external_ids
            .iter()
            .any(|external_id| { external_id.source == "mal" && external_id.value == "834" })
    );
    assert!(
        updated
            .external_ids
            .iter()
            .any(|external_id| { external_id.source == "anilist" && external_id.value == "269" })
    );
    assert_eq!(
        updated
            .external_ids
            .iter()
            .filter(|external_id| external_id.source == "mal")
            .map(|external_id| external_id.value.as_str())
            .collect::<Vec<_>>(),
        vec!["834"]
    );
    assert!(!updated.tags.iter().any(|tag| tag == "scryer:mal-score:old"));
    assert!(updated.tags.iter().any(|tag| tag == "scryer:mal-score:9.1"));
    assert!(updated.tags.iter().any(|tag| tag == "keep"));
    assert!(
        updated.tags.iter().any(|tag| tag == "season 1: opener"),
        "a user label is not a namespace: hydration must not evict it, {:?}",
        updated.tags
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn hydrated_title_metadata_preserves_retry_until_fetch_marker_sqlite() {
    let (services, db) = temp_services("scryer_title_hydration_retry_preserve").await;
    let catalog = title_store(&services);

    let mut title = make_test_title("title-hydration-retry-preserve", None);
    title.external_ids = vec![ExternalId {
        source: "tvdb".to_string(),
        value: "12345".to_string(),
    }];
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");

    sqlx::query(
        "UPDATE titles
         SET metadata_hydration_next_attempt_at = ?,
             metadata_hydration_attempt_count = ?
         WHERE id = ?",
    )
    .bind("2026-01-01T00:00:00Z")
    .bind(7_i64)
    .bind(&title.id)
    .execute(services.pool())
    .await
    .expect("retry state should update");

    TitleRepository::update_title_hydrated_metadata(
        &catalog,
        &title.id,
        TitleMetadataUpdate {
            metadata_language: Some("eng".to_string()),
            ..Default::default()
        },
    )
    .await
    .expect("partial metadata update should succeed");

    let retry_state: (Option<String>, i64) = sqlx::query_as(
        "SELECT metadata_hydration_next_attempt_at, metadata_hydration_attempt_count
         FROM titles
         WHERE id = ?",
    )
    .bind(&title.id)
    .fetch_one(services.pool())
    .await
    .expect("retry state should load");
    assert_eq!(retry_state.0.as_deref(), Some("2026-01-01T00:00:00Z"));
    assert_eq!(retry_state.1, 7);

    TitleRepository::update_title_hydrated_metadata(
        &catalog,
        &title.id,
        TitleMetadataUpdate {
            metadata_fetched_at: Some("2026-02-01T00:00:00Z".to_string()),
            ..Default::default()
        },
    )
    .await
    .expect("fetched metadata update should succeed");

    let cleared_retry_state: (Option<String>, i64) = sqlx::query_as(
        "SELECT metadata_hydration_next_attempt_at, metadata_hydration_attempt_count
         FROM titles
         WHERE id = ?",
    )
    .bind(&title.id)
    .fetch_one(services.pool())
    .await
    .expect("cleared retry state should load");
    assert_eq!(cleared_retry_state.0, None);
    assert_eq!(cleared_retry_state.1, 0);

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn replace_title_match_state_completes_on_single_connection_sqlite() {
    let (services, db) =
        single_connection_services("scryer_replace_match_state_single_connection").await;
    let catalog = title_store(&services);

    let mut title = make_test_title("title-replace-match-state", None);
    title.facet = MediaFacet::Anime;
    title.library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Anime);
    title.external_ids = vec![ExternalId {
        source: "tvdb".to_string(),
        value: "12345".to_string(),
    }];
    title.year = Some(2024);
    title.overview = Some("overview before clear".to_string());
    title.popularity = Some(42.0);
    title.poster_url = Some("https://artworks.thetvdb.com/rematch-poster.jpg".to_string());

    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");
    title_image_store(&services)
        .upsert_title_image_source_result(
            &title.id,
            test_title_image_source_result(
                TitleImageKind::Poster,
                "https://artworks.thetvdb.com/rematch-poster.jpg",
                "w250",
                250,
                375,
                "rematch-poster-bytes",
            ),
            None,
        )
        .await
        .expect("title image should insert");
    sqlx::query(
        "INSERT INTO media_files (
            id, title_id, file_path, size_bytes, scan_status, created_at
         ) VALUES (?, ?, ?, 1234, 'complete', ?)",
    )
    .bind("media-replace-match-state")
    .bind(&title.id)
    .bind("/anime/replace-match-state.mkv")
    .bind(Utc::now().to_rfc3339())
    .execute(services.pool())
    .await
    .expect("media file should insert");

    let updated = timeout(
        Duration::from_secs(1),
        TitleRepository::replace_match_state(
            &catalog,
            &title.id,
            vec![ExternalId {
                source: "tvdb".to_string(),
                value: "99999".to_string(),
            }],
            vec!["score:9.1".to_string()],
        ),
    )
    .await
    .expect("replace match state should not self-deadlock on single-connection sqlite")
    .expect("replace match state should succeed");

    assert_eq!(updated.year, None);
    assert_eq!(updated.overview, None);
    assert_eq!(updated.popularity, None);
    assert_eq!(updated.poster_url, None);
    assert!(
        updated
            .external_ids
            .iter()
            .any(|external_id| { external_id.source == "tvdb" && external_id.value == "99999" })
    );
    assert!(updated.tags.iter().any(|tag| tag == "score:9.1"));
    let remaining_image_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM title_images WHERE title_id = ?")
            .bind(&title.id)
            .fetch_one(services.pool())
            .await
            .expect("title image count should load");
    let remaining_media_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM media_files WHERE title_id = ?")
            .bind(&title.id)
            .fetch_one(services.pool())
            .await
            .expect("media file count should load");
    assert_eq!(remaining_image_count, 0);
    assert_eq!(remaining_media_count, 1);

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_update_metadata_keeps_validation_and_not_found_errors() {
    let (services, db) = temp_services("scryer_title_update_metadata_errors").await;
    let catalog = title_store(&services);

    let title = make_test_title("title-update-metadata-errors", None);
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");

    let empty_err = TitleRepository::update_metadata(&catalog, &title.id, None, None, None, None)
        .await
        .expect_err("empty update should fail validation");
    assert!(matches!(
        empty_err,
        scryer_application::AppError::Validation(message)
            if message.contains("at least one title field")
    ));

    let blank_name_err = TitleRepository::update_metadata(
        &catalog,
        &title.id,
        Some("   ".to_string()),
        None,
        None,
        None,
    )
    .await
    .expect_err("blank title name should fail validation");
    assert!(matches!(
        blank_name_err,
        scryer_application::AppError::Validation(message)
            if message.contains("title name cannot be empty")
    ));

    let missing_err = TitleRepository::update_metadata(
        &catalog,
        "missing-title",
        Some("Renamed".to_string()),
        None,
        None,
        None,
    )
    .await
    .expect_err("missing title update should fail not found");
    assert!(matches!(
        missing_err,
        scryer_application::AppError::NotFound(message) if message.contains("missing-title")
    ));

    let renamed = TitleRepository::update_metadata(
        &catalog,
        &title.id,
        Some(" Renamed Title ".to_string()),
        None,
        None,
        None,
    )
    .await
    .expect("valid metadata update should succeed");
    assert_eq!(renamed.name, "Renamed Title");

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_writes_generate_and_refresh_catalog_sort_key_sqlite() {
    let (services, db) = temp_services("scryer_title_catalog_sort_key").await;
    let catalog = title_store(&services);

    let mut title = make_test_title("title-catalog-sort-key", None);
    title.name = "The Meridian".to_string();
    title.metadata_language = Some("eng".to_string());
    let created = TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");
    let expected_created_key = scryer_domain::title_catalog_sort_key("The Meridian", Some("eng"));
    assert_eq!(created.catalog_sort_key, expected_created_key);
    assert_eq!(
        stored_catalog_sort_key(&services, &title.id).await,
        expected_created_key
    );

    let renamed = TitleRepository::update_metadata(
        &catalog,
        &title.id,
        Some("An Education".to_string()),
        None,
        None,
        None,
    )
    .await
    .expect("title name update should succeed");
    let expected_renamed_key = scryer_domain::title_catalog_sort_key("An Education", Some("eng"));
    assert_eq!(renamed.catalog_sort_key, expected_renamed_key);
    assert_eq!(
        stored_catalog_sort_key(&services, &title.id).await,
        expected_renamed_key
    );

    let hydrated = TitleRepository::update_title_hydrated_metadata(
        &catalog,
        &title.id,
        TitleMetadataUpdate {
            name: Some("鋼の錬金術師".to_string()),
            year: None,
            overview: None,
            poster_url: None,
            background_url: None,
            sort_title: None,
            slug: None,
            imdb_id: None,
            runtime_minutes: None,
            popularity: None,
            canonical_tags: vec![],
            content_status: None,
            language: MetadataFieldUpdate::Unchanged,
            first_aired: None,
            network: None,
            studio: None,
            country: None,
            aliases: vec![],
            tagged_aliases: vec![],
            metadata_language: Some("jpn".to_string()),
            metadata_fetched_at: Some(Utc::now().to_rfc3339()),
            digital_release_date: None,
            ratings: None,
            credits: None,
            extra_external_ids: vec![],
            extra_tags: vec![],
        },
    )
    .await
    .expect("hydrated metadata update should succeed");
    let expected_hydrated_key = scryer_domain::title_catalog_sort_key("鋼の錬金術師", Some("jpn"));
    assert_eq!(hydrated.catalog_sort_key, expected_hydrated_key);
    assert_eq!(
        stored_catalog_sort_key(&services, &title.id).await,
        expected_hydrated_key
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_catalog_scryer_rating_sort_uses_grouped_rating_summary() {
    let (services, db) = temp_services("scryer_title_catalog_scryer_rating_sort").await;
    let catalog = title_store(&services);
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);

    for title_id in [
        "title-rating-high",
        "title-rating-low",
        "title-rating-missing",
    ] {
        let mut title = make_test_title(title_id, None);
        title.name = title_id.to_string();
        TitleRepository::create(&catalog, title)
            .await
            .expect("title should insert");
    }

    for (title_id, rating) in [("title-rating-high", 9.2), ("title-rating-low", 6.5)] {
        TitleRepository::update_title_hydrated_metadata(
            &catalog,
            title_id,
            TitleMetadataUpdate {
                metadata_language: Some("eng".to_string()),
                metadata_fetched_at: Some(Utc::now().to_rfc3339()),
                ratings: Some(TitleRatingSummary {
                    rating: Some(rating),
                    ..TitleRatingSummary::default()
                }),
                ..TitleMetadataUpdate::default()
            },
        )
        .await
        .expect("rating should persist");
    }

    let page = TitleRepository::list_for_libraries_catalog(
        &catalog,
        Some(MediaFacet::Movie),
        &[library_id],
        None,
        TitleCatalogFilter::default(),
        TitleCatalogSort {
            key: TitleCatalogSortKey::RatingScryer,
            direction: SortDirection::Desc,
        },
        10,
        0,
        false,
        true,
    )
    .await
    .expect("catalog rating sort should succeed");

    let ids = page
        .items
        .iter()
        .map(|title| title.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        ids,
        vec![
            "title-rating-high",
            "title-rating-low",
            "title-rating-missing"
        ]
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn hydrated_title_metadata_persists_external_ratings() {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_external_ratings_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = title_store(&services);

    let title = make_test_title("title-ratings", None);
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");

    TitleRepository::update_title_hydrated_metadata(
        &catalog,
        &title.id,
        TitleMetadataUpdate {
            metadata_language: Some("eng".to_string()),
            metadata_fetched_at: Some(Utc::now().to_rfc3339()),
            ratings: Some(TitleRatingSummary {
                rating: Some(82.5),
                rating_sources: vec!["imdb".to_string(), "rottentomatoes".to_string()],
                external_ratings: vec![
                    TitleExternalRating {
                        source: "imdb".to_string(),
                        value: Some(8.2),
                        score: Some(82.0),
                        normalized: 8.2,
                        votes: Some(123_456),
                        url: "https://imdb.test/title/tt001".to_string(),
                    },
                    TitleExternalRating {
                        source: "rottentomatoes".to_string(),
                        value: None,
                        score: Some(94.0),
                        normalized: 9.4,
                        votes: None,
                        url: "https://rt.test/m/test".to_string(),
                    },
                ],
            }),
            ..Default::default()
        },
    )
    .await
    .expect("hydrated metadata should persist ratings");

    let ratings = TitleRepository::get_title_ratings(&catalog, &title.id)
        .await
        .expect("ratings should load");
    assert_eq!(ratings.rating, Some(82.5));
    assert_eq!(
        ratings.rating_sources,
        vec!["imdb".to_string(), "rottentomatoes".to_string()]
    );
    assert_eq!(ratings.external_ratings.len(), 2);
    assert_eq!(ratings.external_ratings[0].source, "imdb");
    assert_eq!(ratings.external_ratings[0].value, Some(8.2));
    assert_eq!(ratings.external_ratings[0].votes, Some(123_456));
    assert_eq!(ratings.external_ratings[1].source, "rottentomatoes");
    assert_eq!(ratings.external_ratings[1].score, Some(94.0));

    TitleRepository::update_title_hydrated_metadata(
        &catalog,
        &title.id,
        TitleMetadataUpdate {
            metadata_fetched_at: Some(Utc::now().to_rfc3339()),
            ratings: None,
            ..Default::default()
        },
    )
    .await
    .expect("metadata update without ratings should preserve rows");

    let preserved = TitleRepository::get_title_ratings(&catalog, &title.id)
        .await
        .expect("preserved ratings should load");
    assert_eq!(preserved, ratings);

    TitleRepository::update_title_hydrated_metadata(
        &catalog,
        &title.id,
        TitleMetadataUpdate {
            metadata_fetched_at: Some(Utc::now().to_rfc3339()),
            ratings: Some(TitleRatingSummary::default()),
            ..Default::default()
        },
    )
    .await
    .expect("empty ratings update should clear rows");

    let cleared = TitleRepository::get_title_ratings(&catalog, &title.id)
        .await
        .expect("cleared ratings should load");
    assert_eq!(cleared, TitleRatingSummary::default());

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn identical_metadata_identity_is_isolated_by_library_title() {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_metadata_library_isolation_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = title_store(&services);

    for id in ["movie-library-a", "movie-library-b"] {
        insert_test_library(&services, id, MediaFacet::Movie).await;
    }

    let external_ids = vec![ExternalId {
        source: "tvdb".to_string(),
        value: "123456".to_string(),
    }];
    let mut title_a = make_test_title("title-library-a", None);
    title_a.library_id = "movie-library-a".to_string();
    title_a.facet = MediaFacet::Movie;
    title_a.slug = Some("library-a-title".to_string());
    title_a.external_ids = external_ids.clone();
    let mut title_b = make_test_title("title-library-b", None);
    title_b.library_id = "movie-library-b".to_string();
    title_b.facet = MediaFacet::Movie;
    title_b.slug = Some("library-b-title".to_string());
    title_b.external_ids = external_ids;
    TitleRepository::create(&catalog, title_a.clone())
        .await
        .expect("first library title should insert");
    TitleRepository::create(&catalog, title_b.clone())
        .await
        .expect("second library title should insert");

    let tag_a = scryer_domain::CanonicalMediaTag {
        key: "canonical:genre:library-a".to_string(),
        category: "genre".to_string(),
        name: "Library A".to_string(),
        confidence: Some(0.9),
        sources: vec!["test".to_string()],
        source_tag_keys: vec!["library-a".to_string()],
        is_adult: false,
        is_spoiler: false,
    };
    let mut tag_b = tag_a.clone();
    tag_b.key = "canonical:genre:library-b".to_string();
    tag_b.name = "Library B".to_string();
    tag_b.source_tag_keys = vec!["library-b".to_string()];

    let hydrate_a = TitleRepository::update_title_hydrated_metadata(
        &catalog,
        &title_a.id,
        TitleMetadataUpdate {
            metadata_fetched_at: Some(Utc::now().to_rfc3339()),
            canonical_tags: vec![tag_a.clone()],
            ratings: Some(TitleRatingSummary {
                rating: Some(9.1),
                ..Default::default()
            }),
            ..Default::default()
        },
    );
    let hydrate_b = TitleRepository::update_title_hydrated_metadata(
        &catalog,
        &title_b.id,
        TitleMetadataUpdate {
            metadata_fetched_at: Some(Utc::now().to_rfc3339()),
            canonical_tags: vec![tag_b.clone()],
            ratings: Some(TitleRatingSummary {
                rating: Some(6.4),
                ..Default::default()
            }),
            ..Default::default()
        },
    );
    let (hydrated_a, hydrated_b) = tokio::join!(hydrate_a, hydrate_b);
    hydrated_a.expect("first title metadata should persist concurrently");
    hydrated_b.expect("second title metadata should persist concurrently");

    let reloaded_a = TitleRepository::get_by_id(&catalog, &title_a.id)
        .await
        .expect("first title lookup should succeed")
        .expect("first title should exist");
    let reloaded_b = TitleRepository::get_by_id(&catalog, &title_b.id)
        .await
        .expect("second title lookup should succeed")
        .expect("second title should exist");
    assert_eq!(reloaded_a.canonical_tags, vec![tag_a.clone()]);
    assert_eq!(reloaded_b.canonical_tags, vec![tag_b.clone()]);

    let page_a = TitleRepository::list_for_libraries_catalog(
        &catalog,
        Some(MediaFacet::Movie),
        std::slice::from_ref(&title_a.library_id),
        None,
        TitleCatalogFilter::default(),
        TitleCatalogSort::default(),
        10,
        0,
        true,
        false,
    )
    .await
    .expect("first library catalog should load owner tags");
    assert_eq!(page_a.items.len(), 1);
    assert_eq!(page_a.items[0].canonical_tags, vec![tag_a.clone()]);

    let slug_a = TitleRepository::get_by_facet_libraries_and_slug(
        &catalog,
        MediaFacet::Movie,
        std::slice::from_ref(&title_a.library_id),
        "library-a-title",
    )
    .await
    .expect("first library slug lookup should succeed")
    .expect("first library slug should resolve");
    assert_eq!(slug_a.canonical_tags, vec![tag_a.clone()]);

    let searched = TitleRepository::list(
        &catalog,
        Some(MediaFacet::Movie),
        Some("Poster Test".to_string()),
    )
    .await
    .expect("presentation search should load owner tags");
    assert_eq!(searched.len(), 2);
    for title in searched {
        let expected = if title.id == title_a.id {
            &tag_a
        } else {
            &tag_b
        };
        assert_eq!(title.canonical_tags, vec![expected.clone()]);
    }

    let lookup_matches = TitleRepository::list_by_external_id_lookups(
        &catalog,
        &[TitleExternalIdLookup {
            lookup_index: 0,
            source: "tvdb".to_string(),
            external_id: "123456".to_string(),
        }],
    )
    .await
    .expect("external-id lookup should load owner tags");
    assert_eq!(lookup_matches.len(), 2);
    for matched in lookup_matches {
        let expected = if matched.title.id == title_a.id {
            &tag_a
        } else {
            &tag_b
        };
        assert_eq!(matched.title.canonical_tags, vec![expected.clone()]);
    }
    assert_eq!(
        TitleRepository::get_title_ratings(&catalog, &title_a.id)
            .await
            .expect("first title ratings should load")
            .rating,
        Some(9.1)
    );
    assert_eq!(
        TitleRepository::get_title_ratings(&catalog, &title_b.id)
            .await
            .expect("second title ratings should load")
            .rating,
        Some(6.4)
    );

    let rematched_a = TitleRepository::replace_match_state(
        &catalog,
        &title_a.id,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "654321".to_string(),
        }],
        vec!["rematched".to_string()],
    )
    .await
    .expect("first title should rematch independently");
    assert!(rematched_a.canonical_tags.is_empty());
    assert_eq!(
        TitleRepository::get_title_ratings(&catalog, &title_a.id)
            .await
            .expect("rematched title ratings should load"),
        TitleRatingSummary::default()
    );
    let after_rematch_b = TitleRepository::get_by_id(&catalog, &title_b.id)
        .await
        .expect("second title lookup after rematch should succeed")
        .expect("second title should survive first title rematch");
    assert_eq!(after_rematch_b.canonical_tags, vec![tag_b.clone()]);
    assert_eq!(
        TitleRepository::get_title_ratings(&catalog, &title_b.id)
            .await
            .expect("second title ratings should survive first title rematch")
            .rating,
        Some(6.4)
    );

    let mut updated_tag_a = tag_a;
    updated_tag_a.key = "canonical:genre:library-a-updated".to_string();
    updated_tag_a.name = "Library A Updated".to_string();
    updated_tag_a.source_tag_keys = vec!["library-a-updated".to_string()];
    TitleRepository::update_title_hydrated_metadata(
        &catalog,
        &title_a.id,
        TitleMetadataUpdate {
            metadata_fetched_at: Some(Utc::now().to_rfc3339()),
            canonical_tags: vec![updated_tag_a.clone()],
            ratings: Some(TitleRatingSummary {
                rating: Some(8.2),
                ..Default::default()
            }),
            ..Default::default()
        },
    )
    .await
    .expect("first title metadata should rehydrate independently");
    let unchanged_b = TitleRepository::get_by_id(&catalog, &title_b.id)
        .await
        .expect("second title lookup after first rehydrate should succeed")
        .expect("second title should remain after first rehydrate");
    assert_eq!(unchanged_b.canonical_tags, vec![tag_b]);
    assert_eq!(
        TitleRepository::get_title_ratings(&catalog, &title_b.id)
            .await
            .expect("second title ratings should remain after first rehydrate")
            .rating,
        Some(6.4)
    );

    let now = Utc::now().to_rfc3339();
    for (id, title_id, path) in [
        ("file-library-a", &title_a.id, "/library-a/movie.mkv"),
        ("file-library-b", &title_b.id, "/library-b/movie.mkv"),
    ] {
        sqlx::query(
            "INSERT INTO media_files (
                id, title_id, file_path, size_bytes, scan_status, created_at
             ) VALUES (?, ?, ?, 100, 'complete', ?)",
        )
        .bind(id)
        .bind(title_id)
        .bind(path)
        .bind(&now)
        .execute(&services.pool)
        .await
        .expect("library-owned media file should insert");
    }

    TitleRepository::delete(&catalog, &title_a.id)
        .await
        .expect("first library title should delete");
    let deleted_owned_rows: i64 = sqlx::query_scalar(
        "SELECT
            (SELECT COUNT(*) FROM title_metadata_tags WHERE title_id = ?)
          + (SELECT COUNT(*) FROM title_metadata_rating_summaries WHERE title_id = ?)
          + (SELECT COUNT(*) FROM media_files WHERE title_id = ?)",
    )
    .bind(&title_a.id)
    .bind(&title_a.id)
    .bind(&title_a.id)
    .fetch_one(&services.pool)
    .await
    .expect("deleted title-owned row count should load");
    assert_eq!(deleted_owned_rows, 0);
    let remaining_b_files: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM media_files WHERE title_id = ?")
            .bind(&title_b.id)
            .fetch_one(&services.pool)
            .await
            .expect("second title media file count should load");
    assert_eq!(remaining_b_files, 1);
    let remaining_b = TitleRepository::get_by_id(&catalog, &title_b.id)
        .await
        .expect("second title lookup after first delete should succeed")
        .expect("second title should survive first delete");
    assert_eq!(remaining_b.canonical_tags, unchanged_b.canonical_tags);

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn full_empty_title_tags_clear_while_partial_updates_preserve() {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_empty_canonical_tags_preserve_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = title_store(&services);

    let mut title = make_test_title("title-canonical-tags-preserve", None);
    title.facet = MediaFacet::Series;
    title.library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Series);
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");

    let tag = scryer_domain::CanonicalMediaTag {
        key: "canonical:genre:test-preserve".to_string(),
        category: "genre".to_string(),
        name: "Test Preserve".to_string(),
        confidence: Some(0.9),
        sources: vec!["test".to_string()],
        source_tag_keys: vec!["test-preserve".to_string()],
        is_adult: false,
        is_spoiler: false,
    };
    TitleRepository::update_title_hydrated_metadata(
        &catalog,
        &title.id,
        TitleMetadataUpdate {
            metadata_language: Some("en".to_string()),
            metadata_fetched_at: Some(Utc::now().to_rfc3339()),
            canonical_tags: vec![tag.clone()],
            ..Default::default()
        },
    )
    .await
    .expect("canonical tags should persist");

    let partial = TitleRepository::update_title_hydrated_metadata(
        &catalog,
        &title.id,
        TitleMetadataUpdate {
            overview: Some("partial manual update".to_string()),
            ..Default::default()
        },
    )
    .await
    .expect("partial title metadata should preserve existing tags");
    assert_eq!(partial.canonical_tags, vec![tag.clone()]);

    let updated = TitleRepository::update_title_hydrated_metadata(
        &catalog,
        &title.id,
        TitleMetadataUpdate {
            metadata_language: Some("en".to_string()),
            metadata_fetched_at: Some(Utc::now().to_rfc3339()),
            canonical_tags: vec![],
            ..Default::default()
        },
    )
    .await
    .expect("full metadata with empty canonical tags should clear existing tags");
    assert!(updated.canonical_tags.is_empty());

    let reloaded = TitleRepository::get_by_id(&catalog, &title.id)
        .await
        .expect("title lookup should succeed")
        .expect("title should exist");
    assert!(reloaded.canonical_tags.is_empty());

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_queries_change_local_version_when_cached_poster_changes() {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_poster_version_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = title_store(&services);
    let title_images = title_image_store(&services);

    let title = make_test_title("title-2", Some("https://artworks.thetvdb.com/poster-a.jpg"));
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");

    for (source_url, sha) in [
        (
            "https://artworks.thetvdb.com/poster-a.jpg",
            "11111111111111111111111111111111",
        ),
        (
            "https://artworks.thetvdb.com/poster-b.jpg",
            "22222222222222222222222222222222",
        ),
    ] {
        sqlx::query("UPDATE titles SET poster_url = ? WHERE id = ?")
            .bind(source_url)
            .bind(&title.id)
            .execute(&services.pool)
            .await
            .expect("source urls should update");
        title_images
            .upsert_title_image_source_result(
                &title.id,
                test_title_image_source_result_with_variants(
                    TitleImageKind::Poster,
                    source_url,
                    vec![test_title_image_variant_record("w250", 250, 375, sha)],
                ),
                None,
            )
            .await
            .expect("title image should upsert");
    }

    let updated = TitleRepository::get_by_id(&catalog, &title.id)
        .await
        .expect("title lookup should succeed")
        .expect("title should exist");
    let expected_poster_url = format!(
        "/images/titles/title-2/poster/w250?v={}",
        test_title_image_version("22222222222222222222222222222222")
    );
    assert_eq!(
        updated.poster_url.as_deref(),
        Some(expected_poster_url.as_str())
    );

    let _ = std::fs::remove_file(db);
}

async fn stored_catalog_sort_key(services: &SqliteServices, title_id: &str) -> String {
    sqlx::query_scalar("SELECT catalog_sort_key FROM titles WHERE id = ?")
        .bind(title_id)
        .fetch_one(services.pool())
        .await
        .expect("stored catalog sort key should load")
}

#[tokio::test]
async fn title_lookup_by_external_id_preserves_source_image_url() {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_external_id_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = title_store(&services);
    let title_images = title_image_store(&services);

    let mut title = make_test_title(
        "title-external-id",
        Some("https://artworks.thetvdb.com/poster-external.jpg"),
    );
    title.external_ids = vec![ExternalId {
        source: "TVDB".to_string(),
        value: "123456".to_string(),
    }];
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");
    title_images
        .upsert_title_image_source_result(
            &title.id,
            test_title_image_source_result_with_variants(
                TitleImageKind::Poster,
                "https://artworks.thetvdb.com/poster-external.jpg",
                vec![test_title_image_variant_record(
                    "w250",
                    250,
                    375,
                    "ffffffffffffffffffffffffffffffff",
                )],
            ),
            None,
        )
        .await
        .expect("title image should insert");

    let found = catalog
        .find_by_external_id("tvdb", "123456")
        .await
        .expect("lookup should succeed")
        .expect("title should exist");
    assert_eq!(
        found.poster_source_url.as_deref(),
        Some("https://artworks.thetvdb.com/poster-external.jpg")
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn create_title_marks_supported_movie_identities_for_background_hydration() {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_hydration_seed_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = title_store(&services);

    for source in ["smg", "tmdb", "imdb", "tvdb", "wikidata"] {
        let mut title = make_test_title(&format!("title-{source}"), None);
        title.external_ids = vec![ExternalId {
            source: source.to_string(),
            value: format!("{source}-id"),
        }];
        TitleRepository::create(&catalog, title)
            .await
            .expect("identity-backed title should insert");
    }

    let markers: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT id, metadata_hydration_next_attempt_at
         FROM titles
         WHERE id LIKE 'title-%'
         ORDER BY id",
    )
    .fetch_all(&services.pool)
    .await
    .expect("load hydration markers");

    let markers = markers
        .into_iter()
        .collect::<std::collections::HashMap<_, _>>();
    for source in ["smg", "tmdb", "imdb", "tvdb"] {
        assert!(
            markers[&format!("title-{source}")].is_some(),
            "{source}-backed movie should be queued for background hydration"
        );
    }
    assert_eq!(markers["title-wikidata"], None);

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn list_titles_due_for_hydration_excludes_active_facets_in_due_order() {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_hydration_excluded_facets_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = title_store(&services);

    let mut anime_title = make_test_title("anime-due", None);
    anime_title.facet = MediaFacet::Anime;
    anime_title.library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Anime);
    anime_title.external_ids = vec![ExternalId {
        source: "tvdb".to_string(),
        value: "301".to_string(),
    }];
    TitleRepository::create(&catalog, anime_title)
        .await
        .expect("anime title should insert");

    let mut movie_title = make_test_title("movie-due", None);
    movie_title.facet = MediaFacet::Movie;
    movie_title.external_ids = vec![ExternalId {
        source: "tvdb".to_string(),
        value: "101".to_string(),
    }];
    TitleRepository::create(&catalog, movie_title)
        .await
        .expect("movie title should insert");

    let mut series_title = make_test_title("series-due", None);
    series_title.facet = MediaFacet::Series;
    series_title.library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Series);
    series_title.external_ids = vec![ExternalId {
        source: "tvdb".to_string(),
        value: "201".to_string(),
    }];
    TitleRepository::create(&catalog, series_title)
        .await
        .expect("series title should insert");

    sqlx::query(
        "UPDATE titles
         SET metadata_fetched_at = '2025-12-31T00:00:00Z',
             metadata_hydration_next_attempt_at = ?,
             metadata_hydration_attempt_count = 0
         WHERE id IN (?, ?, ?)",
    )
    .bind("2026-01-01T00:00:00Z")
    .bind("anime-due")
    .bind("movie-due")
    .bind("series-due")
    .execute(&services.pool)
    .await
    .expect("normalize due timestamps");

    let due_titles =
        TitleRepository::list_titles_due_for_hydration(&catalog, 10, &[MediaFacet::Series])
            .await
            .expect("load due titles excluding active series facet");

    let due_ids = due_titles
        .into_iter()
        .map(|pending| pending.title.id)
        .collect::<Vec<_>>();
    assert_eq!(
        due_ids,
        vec!["anime-due".to_string(), "movie-due".to_string()]
    );

    TitleRepository::clear_title_metadata_hydration_retry_state(&catalog, "anime-due")
        .await
        .expect("successful hydration should clear the retry schedule");
    let remaining =
        TitleRepository::list_titles_due_for_hydration(&catalog, 10, &[MediaFacet::Series])
            .await
            .expect("load due titles after clearing successful hydration");
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].title.id, "movie-due");

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_queries_find_by_external_id() {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_external_id_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = title_store(&services);
    let title_images = title_image_store(&services);

    let mut title = make_test_title(
        "title-external-id",
        Some("https://artworks.thetvdb.com/poster-external.jpg"),
    );
    title.external_ids = vec![ExternalId {
        source: "TVDB".to_string(),
        value: "123456".to_string(),
    }];
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");
    title_images
        .upsert_title_image_source_result(
            &title.id,
            test_title_image_source_result_with_variants(
                TitleImageKind::Poster,
                "https://artworks.thetvdb.com/poster-external.jpg",
                vec![test_title_image_variant_record(
                    "w250",
                    250,
                    375,
                    "ffffffffffffffffffffffffffffffff",
                )],
            ),
            None,
        )
        .await
        .expect("title image should insert");

    let found = catalog
        .find_by_external_id("tvdb", "123456")
        .await
        .expect("lookup should succeed")
        .expect("title should exist");

    assert_eq!(found.id, title.id);
    let expected_poster_url = format!(
        "/images/titles/title-external-id/poster/w250?v={}",
        test_title_image_version("ffffffffffffffffffffffffffffffff")
    );
    assert_eq!(
        found.poster_url.as_deref(),
        Some(expected_poster_url.as_str())
    );
    assert_eq!(
        found.poster_source_url.as_deref(),
        Some("https://artworks.thetvdb.com/poster-external.jpg")
    );

    let uppercase_source = catalog
        .find_by_external_id("TVDB", "123456")
        .await
        .expect("uppercase source lookup should succeed")
        .expect("title should exist for uppercase source");
    assert_eq!(uppercase_source.id, title.id);

    let padded_source = catalog
        .find_by_external_id(" tvdb ", "123456")
        .await
        .expect("padded source lookup should succeed");
    assert!(padded_source.is_none());

    let padded_value = catalog
        .find_by_external_id("tvdb", " 123456 ")
        .await
        .expect("padded value lookup should succeed");
    assert!(padded_value.is_none());

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_queries_list_existing_external_ids_in_library_and_facet_is_scoped() {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_existing_external_ids_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = title_store(&services);
    let movie_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    insert_test_library(&services, "other-movie-library", MediaFacet::Movie).await;

    let mut same_library_movie = make_test_title(
        "same-library-movie",
        Some("https://artworks.thetvdb.com/same.jpg"),
    );
    same_library_movie.external_ids = vec![ExternalId {
        source: "TVDB".to_string(),
        value: "333333".to_string(),
    }];
    TitleRepository::create(&catalog, same_library_movie)
        .await
        .expect("same-library movie should insert");

    let mut same_library_movie_upper = make_test_title(
        "same-library-movie-upper",
        Some("https://artworks.thetvdb.com/upper.jpg"),
    );
    same_library_movie_upper.external_ids = vec![ExternalId {
        source: "TVDB".to_string(),
        value: "555555".to_string(),
    }];
    TitleRepository::create(&catalog, same_library_movie_upper)
        .await
        .expect("same-library uppercase-source movie should insert");

    let mut other_library_movie = make_test_title(
        "other-library-movie",
        Some("https://artworks.thetvdb.com/other.jpg"),
    );
    other_library_movie.library_id = "other-movie-library".to_string();
    other_library_movie.external_ids = vec![ExternalId {
        source: "tvdb".to_string(),
        value: "123456".to_string(),
    }];
    TitleRepository::create(&catalog, other_library_movie)
        .await
        .expect("other-library movie should insert");

    let mut different_facet_title = make_test_title(
        "same-library-series",
        Some("https://artworks.thetvdb.com/series.jpg"),
    );
    different_facet_title.facet = MediaFacet::Series;
    different_facet_title.library_id =
        scryer_domain::default_library_id_for_facet(&MediaFacet::Series);
    different_facet_title.external_ids = vec![ExternalId {
        source: "tvdb".to_string(),
        value: "444444".to_string(),
    }];
    TitleRepository::create(&catalog, different_facet_title)
        .await
        .expect("different-facet title should insert");

    let values = vec![
        "123456".to_string(),
        "333333".to_string(),
        "333333".to_string(),
        "444444".to_string(),
        "555555".to_string(),
        "999999".to_string(),
    ];
    let existing = catalog
        .list_existing_external_ids_in_library_and_facet(
            &movie_library_id,
            MediaFacet::Movie,
            "TVDB",
            &values,
        )
        .await
        .expect("scoped existing-id lookup should succeed");

    assert_eq!(
        existing,
        std::collections::BTreeSet::from(["333333".to_string(), "555555".to_string()])
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_queries_list_by_external_ids_preserve_request_order_for_unique_first_matches() {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_external_id_batch_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = title_store(&services);

    let mut first = make_test_title("title-a", Some("https://artworks.thetvdb.com/a.jpg"));
    first.external_ids = vec![ExternalId {
        source: "tvdb".to_string(),
        value: "123456".to_string(),
    }];
    TitleRepository::create(&catalog, first.clone())
        .await
        .expect("first title should insert");

    let mut second = make_test_title("title-b", Some("https://artworks.thetvdb.com/b.jpg"));
    second.external_ids = vec![ExternalId {
        source: "tvdb".to_string(),
        value: "345678".to_string(),
    }];
    TitleRepository::create(&catalog, second.clone())
        .await
        .expect("second title should insert");

    let values = vec![
        "345678".to_string(),
        "123456".to_string(),
        "123456".to_string(),
        "000000".to_string(),
    ];
    let matches = catalog
        .list_by_external_ids("tvdb", &values)
        .await
        .expect("batch lookup should succeed");

    assert_eq!(matches.len(), 2);
    assert_eq!(matches[0].id, second.id);
    assert_eq!(matches[1].id, first.id);

    let padded_values = vec![" 345678 ".to_string()];
    let padded_value_matches = catalog
        .list_by_external_ids("tvdb", &padded_values)
        .await
        .expect("padded value batch lookup should succeed");
    assert!(padded_value_matches.is_empty());

    let exact_values = vec!["345678".to_string()];
    let padded_source_matches = catalog
        .list_by_external_ids(" tvdb ", &exact_values)
        .await
        .expect("padded source batch lookup should succeed");
    assert!(padded_source_matches.is_empty());

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_queries_list_by_external_id_lookups_return_all_matching_titles() {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_external_id_lookup_matches_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = title_store(&services);

    for id in ["library-a", "library-b"] {
        insert_test_library(&services, id, MediaFacet::Movie).await;
    }

    let mut first = make_test_title("title-shared-a", Some("https://artworks.thetvdb.com/a.jpg"));
    first.library_id = "library-a".to_string();
    first.external_ids = vec![ExternalId {
        source: "tvdb".to_string(),
        value: "123456".to_string(),
    }];
    TitleRepository::create(&catalog, first.clone())
        .await
        .expect("first title should insert");

    let mut second = make_test_title("title-shared-b", Some("https://artworks.thetvdb.com/b.jpg"));
    second.library_id = "library-b".to_string();
    second.external_ids = vec![ExternalId {
        source: "tvdb_series".to_string(),
        value: "123456".to_string(),
    }];
    TitleRepository::create(&catalog, second.clone())
        .await
        .expect("second title should insert");

    let lookups = vec![
        TitleExternalIdLookup {
            lookup_index: 7,
            source: "tvdb".to_string(),
            external_id: "123456".to_string(),
        },
        TitleExternalIdLookup {
            lookup_index: 9,
            source: "tvdb_series".to_string(),
            external_id: "123456".to_string(),
        },
    ];
    let matches = catalog
        .list_by_external_id_lookups(&lookups)
        .await
        .expect("lookup should succeed");

    assert_eq!(matches.len(), 2);
    assert_eq!(matches[0].lookup_index, 7);
    assert_eq!(matches[0].title.id, first.id);
    assert_eq!(matches[1].lookup_index, 9);
    assert_eq!(matches[1].title.id, second.id);

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_queries_with_empty_library_allowlist_return_no_results() {
    let (services, db) = temp_services("scryer_title_empty_library_allowlist").await;
    let catalog = title_store(&services);

    let mut title = make_test_title("title-empty-library-allowlist", None);
    title.name = "Alpha Allowlist".to_string();
    TitleRepository::create(&catalog, title)
        .await
        .expect("title should insert");

    let empty_library_ids = Vec::<String>::new();

    let listed = TitleRepository::list_for_libraries(&catalog, None, &empty_library_ids, None)
        .await
        .expect("plain library listing should succeed");
    assert!(listed.is_empty());

    let searched = TitleRepository::list_for_libraries_without_external_ids(
        &catalog,
        None,
        &empty_library_ids,
        Some("alpha".to_string()),
    )
    .await
    .expect("ranked library listing should succeed");
    assert!(searched.is_empty());

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_queries_get_by_facet_and_slug_trim_input_and_reject_duplicates() {
    let (services, db) = temp_services("scryer_title_slug_lookup").await;
    let catalog = title_store(&services);

    for id in ["library-a", "library-b"] {
        insert_test_library(&services, id, MediaFacet::Movie).await;
    }

    let mut first = make_test_title("title-slug-primary", None);
    first.facet = MediaFacet::Movie;
    first.library_id = "library-a".to_string();
    first.slug = Some("earth-defenders".to_string());
    TitleRepository::create(&catalog, first.clone())
        .await
        .expect("first title should insert");

    let found =
        TitleRepository::get_by_facet_and_slug(&catalog, MediaFacet::Movie, " earth-defenders ")
            .await
            .expect("trimmed slug lookup should succeed")
            .expect("trimmed slug lookup should find a title");
    assert_eq!(found.id, first.id);

    let mut duplicate = make_test_title("title-slug-duplicate", None);
    duplicate.facet = MediaFacet::Movie;
    duplicate.library_id = "library-b".to_string();
    duplicate.slug = Some("earth-defenders".to_string());
    TitleRepository::create(&catalog, duplicate)
        .await
        .expect("duplicate title should insert in a different library");

    let err =
        TitleRepository::get_by_facet_and_slug(&catalog, MediaFacet::Movie, "earth-defenders")
            .await
            .expect_err("duplicate slug lookup should fail validation");
    assert!(matches!(
        err,
        scryer_application::AppError::Validation(message)
            if message.contains("earth-defenders") && message.contains("multiple titles")
    ));

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_queries_get_by_facet_libraries_and_slug_trim_input_and_reject_duplicates() {
    let (services, db) = temp_services("scryer_title_library_slug_lookup").await;
    let catalog = title_store(&services);

    for id in ["library-a", "library-b"] {
        insert_test_library(&services, id, MediaFacet::Movie).await;
    }

    let mut first = make_test_title("title-library-slug-a", None);
    first.facet = MediaFacet::Movie;
    first.library_id = "library-a".to_string();
    first.slug = Some("planet-heroes".to_string());
    TitleRepository::create(&catalog, first)
        .await
        .expect("first title should insert");

    let mut second = make_test_title("title-library-slug-b", None);
    second.facet = MediaFacet::Movie;
    second.library_id = "library-b".to_string();
    second.slug = Some("planet-heroes".to_string());
    TitleRepository::create(&catalog, second.clone())
        .await
        .expect("second title should insert");

    let library_b = vec!["library-b".to_string()];
    let found = TitleRepository::get_by_facet_libraries_and_slug(
        &catalog,
        MediaFacet::Movie,
        &library_b,
        " planet-heroes ",
    )
    .await
    .expect("trimmed library slug lookup should succeed")
    .expect("trimmed library slug lookup should find a title");
    assert_eq!(found.id, second.id);

    let libraries = vec!["library-a".to_string(), "library-b".to_string()];
    let err = TitleRepository::get_by_facet_libraries_and_slug(
        &catalog,
        MediaFacet::Movie,
        &libraries,
        "planet-heroes",
    )
    .await
    .expect_err("duplicate library slug lookup should fail validation");
    assert!(matches!(
        err,
        scryer_application::AppError::Validation(message)
            if message.contains("planet-heroes") && message.contains("multiple titles")
    ));

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_query_modes_keep_spellfix_search_scoped_to_presentation_sqlite() {
    let (services, db) = temp_services("scryer_title_query_mode_search_scope").await;
    let catalog = title_store(&services);

    let mut title = make_test_title("title-query-mode-search-scope", None);
    title.name = "Canonical Search Name".to_string();
    title.aliases = vec!["Hidden Search Alias".to_string()];
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");

    let presentation_hits =
        TitleRepository::list(&catalog, None, Some("hidden search alias".to_string()))
            .await
            .expect("presentation search should load");
    assert_eq!(
        presentation_hits.first().map(|title| title.id.as_str()),
        Some(title.id.as_str())
    );

    let matching_alias_hits =
        TitleRepository::list_for_matching(&catalog, None, Some("hidden search alias".to_string()))
            .await
            .expect("matching search should load");
    assert!(
        !matching_alias_hits
            .iter()
            .any(|candidate| candidate.id == title.id)
    );

    let matching_name_hits =
        TitleRepository::list_for_matching(&catalog, None, Some("canonical search".to_string()))
            .await
            .expect("matching name search should load");
    assert!(
        matching_name_hits
            .iter()
            .any(|candidate| candidate.id == title.id)
    );

    let padded_matching_name_hits =
        TitleRepository::list_for_matching(&catalog, None, Some(" canonical search ".to_string()))
            .await
            .expect("padded matching name search should load");
    assert!(
        !padded_matching_name_hits
            .iter()
            .any(|candidate| candidate.id == title.id)
    );

    let library_ids = vec![title.library_id.clone()];
    let library_alias_hits = TitleRepository::list_for_libraries(
        &catalog,
        None,
        &library_ids,
        Some("hidden search alias".to_string()),
    )
    .await
    .expect("library search should load");
    assert!(
        !library_alias_hits
            .iter()
            .any(|candidate| candidate.id == title.id)
    );

    let library_padded_name_hits = TitleRepository::list_for_libraries(
        &catalog,
        None,
        &library_ids,
        Some(" canonical search ".to_string()),
    )
    .await
    .expect("padded library search should load");
    assert!(
        !library_padded_name_hits
            .iter()
            .any(|candidate| candidate.id == title.id)
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_create_or_get_existing_reuses_external_ids_not_slug_only() {
    let (services, db) = temp_services("scryer_title_create_or_get_existing_parity").await;
    let catalog = title_store(&services);

    let mut existing = make_test_title("title-existing-external-id", None);
    existing.slug = Some("shared-slug".to_string());
    existing.external_ids = vec![ExternalId {
        source: "tvdb".to_string(),
        value: "12345".to_string(),
    }];
    TitleRepository::create(&catalog, existing.clone())
        .await
        .expect("existing title should insert");

    let mut same_slug = make_test_title("title-same-slug-new-external-id", None);
    same_slug.slug = Some("shared-slug".to_string());
    same_slug.external_ids = vec![ExternalId {
        source: "tvdb".to_string(),
        value: "67890".to_string(),
    }];
    let same_slug_outcome = TitleRepository::create_or_get_existing(&catalog, same_slug.clone())
        .await
        .expect("same-slug title should create");
    assert!(!same_slug_outcome.reused_existing);
    assert_eq!(same_slug_outcome.title.id, same_slug.id);

    let mut same_external_id = make_test_title("title-same-external-id", None);
    same_external_id.slug = Some("different-slug".to_string());
    same_external_id.external_ids = existing.external_ids.clone();
    let same_external_id_outcome =
        TitleRepository::create_or_get_existing(&catalog, same_external_id)
            .await
            .expect("same external id title should reuse");
    assert!(same_external_id_outcome.reused_existing);
    assert_eq!(same_external_id_outcome.title.id, existing.id);

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_list_for_matching_keeps_source_image_urls() {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_list_for_matching_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = title_store(&services);
    let title_images = title_image_store(&services);

    let title = make_test_title(
        "title-list-matching",
        Some("https://artworks.thetvdb.com/poster.jpg"),
    );
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");
    title_images
        .upsert_title_image_source_result(
            &title.id,
            test_title_image_source_result_with_variants(
                TitleImageKind::Poster,
                "https://artworks.thetvdb.com/poster.jpg",
                vec![test_title_image_variant_record(
                    "w250",
                    250,
                    375,
                    "cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd",
                )],
            ),
            None,
        )
        .await
        .expect("title image should insert");

    let titles = TitleRepository::list_for_matching(&catalog, None, None)
        .await
        .expect("matching list should succeed");
    let listed = titles
        .into_iter()
        .find(|candidate| candidate.id == title.id)
        .expect("title should be listed");

    assert_eq!(
        listed.poster_url.as_deref(),
        Some("https://artworks.thetvdb.com/poster.jpg")
    );
    assert!(listed.poster_source_url.is_none());

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn media_file_source_signature_refresh_preserves_scan_status() {
    let db = std::env::temp_dir().join(format!(
        "scryer_media_file_signature_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = title_store(&services);
    let media_files = media_file_store(&services);

    let title = make_test_title("title-media-file", None);
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");

    let file_id = media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: "/library/Movie.Title.2024.mkv".to_string(),
            size_bytes: 4_096,
            ..Default::default()
        })
        .await
        .expect("media file should insert");

    sqlx::query("UPDATE media_files SET scan_status = 'scanned' WHERE id = ?")
        .bind(&file_id)
        .execute(&services.pool)
        .await
        .expect("scan status should update");

    media_files
        .update_media_file_source_signature(
            &file_id,
            4_096,
            Some("unix_mtime_nsec_v1".to_string()),
            Some("1:2".to_string()),
        )
        .await
        .expect("source signature should refresh");

    let media_file = media_files
        .get_media_file_by_id(&file_id)
        .await
        .expect("lookup should succeed")
        .expect("media file should exist");

    assert_eq!(media_file.scan_status, "scanned");
    assert_eq!(
        media_file.source_signature_scheme.as_deref(),
        Some("unix_mtime_nsec_v1")
    );
    assert_eq!(media_file.source_signature_value.as_deref(), Some("1:2"));

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn media_file_aggregates_ignore_additional_files_but_listing_includes_them() {
    let (services, db) = temp_services("scryer_media_file_primary_aggregates").await;
    let catalog = title_store(&services);
    let shows = show_store(&services);
    let media_files = media_file_store(&services);

    let movie_title = make_test_title("title-primary-aggregate-movie", None);
    TitleRepository::create(&catalog, movie_title.clone())
        .await
        .expect("movie title should insert");

    let movie_primary_id = media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: movie_title.id.clone(),
            file_path: "/library/Movie.Primary.2160p.mkv".to_string(),
            size_bytes: 8_192,
            quality_label: Some("2160p".to_string()),
            ..Default::default()
        })
        .await
        .expect("movie primary media file should insert");
    let movie_additional_id = media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: movie_title.id.clone(),
            file_path: "/library/Movie.Additional.720p.mkv".to_string(),
            size_bytes: 4_096,
            role: MediaFileRole::Additional,
            quality_label: Some("720p".to_string()),
            ..Default::default()
        })
        .await
        .expect("movie additional media file should insert");

    let movie_listing = media_files
        .list_media_files_for_title(&movie_title.id)
        .await
        .expect("movie media files should list");
    assert_eq!(movie_listing.len(), 2);
    assert!(
        movie_listing
            .iter()
            .any(|file| { file.id == movie_primary_id && file.role == MediaFileRole::Primary })
    );
    assert!(
        movie_listing.iter().any(|file| {
            file.id == movie_additional_id && file.role == MediaFileRole::Additional
        })
    );

    let quality_summaries = media_files
        .list_title_quality_summaries(std::slice::from_ref(&movie_title.id))
        .await
        .expect("title quality summaries should list");
    assert_eq!(quality_summaries.len(), 1);
    assert_eq!(quality_summaries[0].title_id, movie_title.id);
    assert_eq!(quality_summaries[0].quality_tier, "2160P");

    let mut series_title = make_test_title("title-primary-aggregate-series", None);
    series_title.facet = MediaFacet::Series;
    series_title.library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Series);
    TitleRepository::create(&catalog, series_title.clone())
        .await
        .expect("series title should insert");

    let collection = Collection {
        id: "primary-aggregate-season-1".to_string(),
        title_id: series_title.id.clone(),
        collection_type: CollectionType::Season,
        collection_index: "1".to_string(),
        label: Some("Season 1".to_string()),
        ordered_path: None,
        narrative_order: Some("1".to_string()),
        first_episode_number: Some("1".to_string()),
        last_episode_number: Some("2".to_string()),
        monitored: true,
        created_at: Utc::now(),
    };
    ShowRepository::create_collection(&shows, collection.clone())
        .await
        .expect("collection should insert");

    let episode_one = Episode {
        id: "primary-aggregate-s01e01".to_string(),
        title_id: series_title.id.clone(),
        collection_id: Some(collection.id.clone()),
        episode_type: scryer_domain::EpisodeType::Standard,
        episode_number: Some("1".to_string()),
        season_number: Some("1".to_string()),
        episode_label: Some("S01E01".to_string()),
        title: Some("Episode 1".to_string()),
        air_date: Some("2026-01-01".to_string()),
        duration_seconds: Some(1_800),
        has_multi_audio: false,
        has_subtitle: false,
        is_filler: false,
        is_recap: false,
        absolute_number: None,
        overview: None,
        tvdb_id: None,
        image_url: None,
        monitored: true,
        created_at: Utc::now(),
    };
    let episode_two = Episode {
        id: "primary-aggregate-s01e02".to_string(),
        episode_number: Some("2".to_string()),
        episode_label: Some("S01E02".to_string()),
        title: Some("Episode 2".to_string()),
        air_date: Some("2026-01-02".to_string()),
        ..episode_one.clone()
    };
    ShowRepository::create_episode(&shows, episode_one.clone())
        .await
        .expect("episode one should insert");
    ShowRepository::create_episode(&shows, episode_two.clone())
        .await
        .expect("episode two should insert");

    let episode_one_primary_id = media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: series_title.id.clone(),
            file_path: "/library/Series.S01E01.Primary.1080p.mkv".to_string(),
            size_bytes: 2_048,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("episode primary file should insert");
    media_files
        .link_file_to_episode(&episode_one_primary_id, &episode_one.id)
        .await
        .expect("primary file should link to episode one");
    media_files
        .set_media_file_roles_for_episode(
            &series_title.id,
            &episode_one.id,
            &episode_one_primary_id,
            &[],
        )
        .await
        .expect("episode one primary role should set");

    for (file_path, episode_id) in [
        (
            "/library/Series.S01E01.Additional.360p.mkv",
            episode_one.id.as_str(),
        ),
        (
            "/library/Series.S01E02.Additional.360p.mkv",
            episode_two.id.as_str(),
        ),
    ] {
        let additional_id = media_files
            .insert_media_file(&InsertMediaFileInput {
                title_id: series_title.id.clone(),
                file_path: file_path.to_string(),
                size_bytes: 1_024,
                role: MediaFileRole::Additional,
                quality_label: Some("360p".to_string()),
                ..Default::default()
            })
            .await
            .expect("additional episode file should insert");
        media_files
            .link_file_to_episode(&additional_id, episode_id)
            .await
            .expect("additional file should link to episode");
    }

    let cutoff_summaries = media_files
        .list_cutoff_unmet_quality_summaries(&[series_title.id.clone()])
        .await
        .expect("cutoff quality summaries should list");
    assert_eq!(cutoff_summaries.len(), 1);
    assert_eq!(
        cutoff_summaries[0].episode_id.as_deref(),
        Some(episode_one.id.as_str())
    );
    assert_eq!(cutoff_summaries[0].quality_tier, "1080P");

    let progress_summaries = media_files
        .list_title_episode_progress_summaries(&[series_title.id.clone()])
        .await
        .expect("episode progress summaries should list");
    assert_eq!(progress_summaries.len(), 1);
    assert_eq!(progress_summaries[0].title_id, series_title.id);
    assert_eq!(progress_summaries[0].owned_episodes, 1);
    assert_eq!(progress_summaries[0].monitored_episodes, 2);
    assert_eq!(progress_summaries[0].total_episodes, 2);

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_queries_fall_back_to_remote_when_no_local_variant_exists() {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_poster_original_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = title_store(&services);
    let title_images = title_image_store(&services);

    let title = make_test_title(
        "title-3",
        Some("https://artworks.thetvdb.com/poster-original.jpg"),
    );
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");

    title_images
        .upsert_title_image_source_result(
            &title.id,
            test_title_image_source_result_with_variants(
                TitleImageKind::Poster,
                "https://artworks.thetvdb.com/poster-original.jpg",
                Vec::new(),
            ),
            None,
        )
        .await
        .expect("title image should insert");

    let updated = TitleRepository::get_by_id(&catalog, &title.id)
        .await
        .expect("title lookup should succeed")
        .expect("title should exist");
    assert_eq!(
        updated.poster_url.as_deref(),
        Some("https://artworks.thetvdb.com/poster-original.jpg")
    );

    let original = title_images
        .get_title_image_blob(&title.id, TitleImageKind::Poster, "original")
        .await
        .expect("original blob lookup should succeed");
    assert_eq!(original, None);

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn replace_title_image_and_append_event_commits_image_and_event_atomically() {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_image_event_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = title_store(&services);
    let title_images = title_image_store(&services);
    let domain_events = DomainEventStore::new(services.datastore());

    let title = make_test_title(
        "title-image-event",
        Some("https://artworks.thetvdb.com/poster.jpg"),
    );
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");

    let event = NewDomainEvent {
        event_id: Id::new().0,
        occurred_at: Utc::now(),
        actor_kind: scryer_domain::DomainEventActorKind::System,
        actor_user_id: None,
        actor_display_name: "System".to_string(),
        title_id: Some(title.id.clone()),
        facet: Some(title.facet.clone()),
        correlation_id: None,
        causation_id: None,
        schema_version: 1,
        stream: DomainEventStream::Title {
            title_id: title.id.clone(),
        },
        payload: DomainEventPayload::TitleUpdated(TitleUpdatedEventData {
            title: TitleContextSnapshot {
                title_name: title.name.clone(),
                facet: title.facet.clone(),
                external_ids: Default::default(),
                poster_url: title.poster_url.clone(),
                year: title.year,
            },
        }),
    };

    let stored = title_images
        .upsert_title_image_source_result(
            &title.id,
            test_title_image_source_result_with_variants(
                TitleImageKind::Poster,
                "https://artworks.thetvdb.com/poster.jpg",
                vec![test_title_image_variant_record(
                    "w250",
                    250,
                    375,
                    "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
                )],
            ),
            Some(event.clone()),
        )
        .await
        .expect("title image and event should commit");

    assert_eq!(
        stored.expect("event should be stored").event_id,
        event.event_id
    );
    let (storage_type, payload, import_status, delete_reason, download_id): (
        String,
        Vec<u8>,
        Option<String>,
        Option<String>,
        Option<String>,
    ) = sqlx::query_as(
        "SELECT typeof(payload_json), payload_json, import_status,
                media_file_delete_reason, download_id
           FROM domain_events
          WHERE event_id = ?",
    )
    .bind(&event.event_id)
    .fetch_one(services.pool())
    .await
    .expect("stored event payload should be inspectable");
    assert_eq!(storage_type, "blob");
    assert_eq!(
        payload.first().copied(),
        Some(scryer_infrastructure_sql::domain_event_payload::DOMAIN_EVENT_PAYLOAD_FORMAT_V1)
    );
    assert_eq!(
        (import_status, delete_reason, download_id),
        (None, None, None)
    );
    let blob = title_images
        .get_title_image_blob(&title.id, TitleImageKind::Poster, "w250")
        .await
        .expect("blob lookup should succeed")
        .expect("blob should exist");
    assert_eq!(blob.bytes, b"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee".to_vec());

    let events = domain_events
        .list(&DomainEventFilter {
            title_id: Some(title.id.clone()),
            limit: 10,
            ..Default::default()
        })
        .await
        .expect("domain event list should succeed");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_id, event.event_id);
    assert!(matches!(
        events[0].payload,
        DomainEventPayload::TitleUpdated(_)
    ));

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_queries_fall_back_to_original_when_preferred_local_variant_is_missing() {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_poster_incomplete_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = title_store(&services);
    let title_images = title_image_store(&services);

    let title = make_test_title(
        "title-4",
        Some("https://artworks.thetvdb.com/poster-incomplete.jpg"),
    );
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");

    title_images
        .upsert_title_image_source_result(
            &title.id,
            test_title_image_source_result_with_variants(
                TitleImageKind::Poster,
                "https://artworks.thetvdb.com/poster-incomplete.jpg",
                Vec::new(),
            ),
            None,
        )
        .await
        .expect("title image should insert");

    let updated = TitleRepository::get_by_id(&catalog, &title.id)
        .await
        .expect("title lookup should succeed")
        .expect("title should exist");
    assert_eq!(
        updated.poster_url.as_deref(),
        Some("https://artworks.thetvdb.com/poster-incomplete.jpg")
    );

    let pending = title_images
        .list_title_image_refresh_work(10, &[])
        .await
        .expect("list pending poster refresh should succeed");
    assert!(
        pending.iter().any(|task| task.title_id == title.id),
        "incomplete AVIF cache rows should be re-queued for repair"
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn fanart_queries_use_w1280_variant_when_present() {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_fanart_w1280_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = title_store(&services);
    let title_images = title_image_store(&services);

    let title = make_test_title("title-fanart-w1280", None);
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");

    sqlx::query("UPDATE titles SET background_url = ? WHERE id = ?")
        .bind("https://artworks.thetvdb.com/fanart.jpg")
        .bind(&title.id)
        .execute(&services.pool)
        .await
        .expect("source urls should update");

    let fanart_bytes = vec![9_u8, 10, 11];
    let fanart_digest = format!("blake3:{}", blake3::hash(&fanart_bytes).to_hex());
    title_images
        .upsert_title_image_source_result(
            &title.id,
            test_title_image_source_result_with_variants(
                TitleImageKind::Fanart,
                "https://artworks.thetvdb.com/fanart.jpg",
                vec![TitleImageVariantRecord {
                    variant_key: "w1280".to_string(),
                    format: "avif".to_string(),
                    width: 1280,
                    height: 720,
                    bytes: fanart_bytes.clone(),
                    digest: fanart_digest.clone(),
                }],
            ),
            None,
        )
        .await
        .expect("fanart image should insert");

    let updated = TitleRepository::get_by_id(&catalog, &title.id)
        .await
        .expect("title lookup should succeed")
        .expect("title should exist");
    let fanart_version = &fanart_digest["blake3:".len().."blake3:".len() + 16];
    assert_eq!(
        updated.background_url.as_deref(),
        Some(format!("/images/titles/title-fanart-w1280/fanart/w1280?v={fanart_version}").as_str())
    );
    assert_eq!(
        updated.background_source_url.as_deref(),
        Some("https://artworks.thetvdb.com/fanart.jpg")
    );

    let fanart_variant = title_images
        .get_title_image_blob(&title.id, TitleImageKind::Fanart, "w1280")
        .await
        .expect("fanart variant blob lookup should succeed");
    assert_eq!(
        fanart_variant,
        Some(TitleImageBlob {
            content_type: "image/avif".to_string(),
            etag: fanart_digest,
            bytes: fanart_bytes,
        })
    );

    let fanart = title_images
        .get_title_image_blob(&title.id, TitleImageKind::Fanart, "master")
        .await
        .expect("fanart blob lookup should succeed");
    assert_eq!(fanart, None);

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_image_refresh_work_requires_fanart_w1280_variant() {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_fanart_refresh_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = title_store(&services);
    let title_images = title_image_store(&services);

    let title = make_test_title("title-fanart-refresh", None);
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");

    sqlx::query("UPDATE titles SET background_url = ? WHERE id = ?")
        .bind("https://artworks.thetvdb.com/fanart-refresh.jpg")
        .bind(&title.id)
        .execute(&services.pool)
        .await
        .expect("source urls should update");

    title_images
        .upsert_title_image_source_result(
            &title.id,
            test_title_image_source_result_with_variants(
                TitleImageKind::Fanart,
                "https://artworks.thetvdb.com/fanart-refresh.jpg",
                Vec::new(),
            ),
            None,
        )
        .await
        .expect("fanart image should insert");

    let pending_fanart = title_images
        .list_title_image_refresh_work(10, &[])
        .await
        .expect("list pending fanart refresh should succeed");
    assert!(
        pending_fanart.iter().any(|task| task.title_id == title.id),
        "fanart without w1280 should be re-queued for processing"
    );

    title_images
        .upsert_title_image_source_result(
            &title.id,
            test_title_image_source_result_with_variants(
                TitleImageKind::Fanart,
                "https://artworks.thetvdb.com/fanart-refresh.jpg",
                vec![test_title_image_variant_record(
                    "w1280",
                    1280,
                    720,
                    "cccccccccccccccccccccccccccccccc",
                )],
            ),
            None,
        )
        .await
        .expect("fanart image with w1280 should insert");

    let pending_fanart = title_images
        .list_title_image_refresh_work(10, &[])
        .await
        .expect("list pending fanart refresh should succeed");
    assert!(pending_fanart.is_empty());

    let _ = std::fs::remove_file(db);
}

fn sample_title_credits() -> Vec<TitleCredit> {
    vec![
        TitleCredit {
            kind: "actor".to_string(),
            person_id: "person-1".to_string(),
            person_name: "Lead Actor".to_string(),
            person_original_name: "主演".to_string(),
            person_image_url: "https://images.test/person-1.jpg".to_string(),
            person_source: "tmdb".to_string(),
            person_external_id: "tmdb-1".to_string(),
            character_name: "Hero".to_string(),
            language: "eng".to_string(),
            billing_order: 0,
            episode_count: Some(12),
        },
        TitleCredit {
            kind: "voice_actor".to_string(),
            person_id: "person-2".to_string(),
            person_name: "Voice Actor".to_string(),
            person_original_name: "声優".to_string(),
            person_image_url: String::new(),
            person_source: "anilist".to_string(),
            person_external_id: "anilist-2".to_string(),
            character_name: "Hero".to_string(),
            language: "jpn".to_string(),
            billing_order: 1,
            episode_count: None,
        },
        TitleCredit {
            kind: "director".to_string(),
            person_id: "person-3".to_string(),
            person_name: "The Director".to_string(),
            person_original_name: String::new(),
            person_image_url: String::new(),
            person_source: "tmdb".to_string(),
            person_external_id: "tmdb-3".to_string(),
            character_name: String::new(),
            language: String::new(),
            billing_order: 2,
            episode_count: None,
        },
    ]
}

/// SQLite test databases are seeded with the default libraries; a bare PostgreSQL
/// test schema is not, so the shared assertion inserts the row it needs itself.
async fn ensure_default_movie_library(datastore: &StoreDatastore) -> AppResult<()> {
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let existing = SqlRuntime::fetch_optional(
        datastore.read_exec(),
        "SELECT id FROM libraries WHERE id = {}",
        &[SqlArg::Text(library_id.clone())],
    )
    .await?;
    if existing.is_some() {
        return Ok(());
    }

    let now = Utc::now();
    SqlRuntime::run_in_transaction::<(), _>(
        datastore,
        "test_seed_default_movie_library",
        move |tx| {
            let library_id = library_id.clone();
            Box::pin(async move {
                tx.execute(
                    "INSERT INTO libraries (id, facet, name, slug, is_default, created_at, updated_at)
                     VALUES ({}, {}, {}, {}, {}, {}, {})",
                    &[
                        SqlArg::Text(library_id),
                        SqlArg::Text(
                            MediaFacet::Movie.as_str().to_string(),
                        ),
                        SqlArg::Text("Movies".to_string()),
                        SqlArg::Text("movies".to_string()),
                        SqlArg::Bool(true),
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

async fn assert_title_credits_replacement(
    catalog: &TitleStore,
    datastore: &StoreDatastore,
) -> AppResult<()> {
    ensure_default_movie_library(datastore).await?;

    let title = make_test_title("title-credits", None);
    TitleRepository::create(catalog, title.clone()).await?;

    let credits = sample_title_credits();
    TitleRepository::update_title_hydrated_metadata(
        catalog,
        &title.id,
        TitleMetadataUpdate {
            metadata_fetched_at: Some(Utc::now().to_rfc3339()),
            credits: Some(credits.clone()),
            ..Default::default()
        },
    )
    .await?;

    let stored = TitleRepository::get_title_credits(catalog, &title.id).await?;
    assert_eq!(
        stored, credits,
        "every credit kind is cached verbatim in SMG order"
    );

    TitleRepository::update_title_hydrated_metadata(
        catalog,
        &title.id,
        TitleMetadataUpdate {
            metadata_fetched_at: Some(Utc::now().to_rfc3339()),
            credits: None,
            ..Default::default()
        },
    )
    .await?;
    assert_eq!(
        TitleRepository::get_title_credits(catalog, &title.id).await?,
        credits,
        "a metadata update without credits preserves the cache"
    );

    let replacement = vec![TitleCredit {
        kind: "writer".to_string(),
        person_id: "person-9".to_string(),
        person_name: "The Writer".to_string(),
        billing_order: 0,
        ..Default::default()
    }];
    TitleRepository::update_title_hydrated_metadata(
        catalog,
        &title.id,
        TitleMetadataUpdate {
            metadata_fetched_at: Some(Utc::now().to_rfc3339()),
            credits: Some(replacement.clone()),
            ..Default::default()
        },
    )
    .await?;
    assert_eq!(
        TitleRepository::get_title_credits(catalog, &title.id).await?,
        replacement,
        "a new response replaces the whole cached set"
    );

    TitleRepository::update_title_hydrated_metadata(
        catalog,
        &title.id,
        TitleMetadataUpdate {
            metadata_fetched_at: Some(Utc::now().to_rfc3339()),
            credits: Some(Vec::new()),
            ..Default::default()
        },
    )
    .await?;
    assert!(
        TitleRepository::get_title_credits(catalog, &title.id)
            .await?
            .is_empty(),
        "a successful empty response clears the cache"
    );

    Ok(())
}

#[tokio::test]
async fn hydrated_title_metadata_replaces_title_credits() {
    let (services, db) = temp_services("scryer_title_credits").await;
    let catalog = title_store(&services);

    assert_title_credits_replacement(&catalog, &services.datastore())
        .await
        .expect("credit replacement should behave consistently");

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn hydrated_title_metadata_replaces_title_credits_postgres() -> AppResult<()> {
    let Some(raw_url) = std::env::var("SCRYER_TEST_POSTGRES_URL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    else {
        eprintln!("skipping PostgreSQL title credits test; SCRYER_TEST_POSTGRES_URL is not set");
        return Ok(());
    };

    let admin_pool = sqlx::PgPool::connect(&raw_url)
        .await
        .map_err(|error| AppError::Repository(format!("failed to connect to postgres: {error}")))?;
    let schema = format!(
        "scryer_test_{}_{}",
        std::process::id(),
        Id::new().0.replace('-', "_")
    );

    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&admin_pool)
        .await
        .map_err(|error| AppError::Repository(format!("failed to create schema: {error}")))?;

    let result = async {
        let mut url = url::Url::parse(&raw_url)
            .map_err(|error| AppError::Validation(format!("invalid postgres test URL: {error}")))?;
        url.query_pairs_mut()
            .append_pair("options", &format!("-csearch_path={schema}"));
        let services =
            crate::PostgresServices::new_with_mode(url.to_string(), crate::MigrationMode::Apply)
                .await?;
        let datastore = services.datastore();
        let catalog = TitleStore::new(services.datastore());
        let result = assert_title_credits_replacement(&catalog, &datastore).await;
        services.pool().close().await;
        result
    }
    .await;

    let cleanup = sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
        .execute(&admin_pool)
        .await;
    admin_pool.close().await;
    cleanup.map_err(|error| AppError::Repository(format!("failed to drop schema: {error}")))?;
    result
}

#[tokio::test]
async fn failed_transactions_preserve_the_previous_title_credit_cache() {
    let (services, db) = temp_services("scryer_title_credits_rollback").await;
    let catalog = title_store(&services);
    let datastore = services.datastore();

    let title = make_test_title("title-credits-rollback", None);
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");

    let credits = sample_title_credits();
    TitleRepository::update_title_hydrated_metadata(
        &catalog,
        &title.id,
        TitleMetadataUpdate {
            metadata_fetched_at: Some(Utc::now().to_rfc3339()),
            credits: Some(credits.clone()),
            ..Default::default()
        },
    )
    .await
    .expect("hydrated metadata should persist credits");

    let title_id = title.id.clone();
    let rolled_back = SqlRuntime::run_in_transaction::<(), _>(
        &datastore,
        "test_rollback_title_credits",
        move |tx| {
            let title_id = title_id.clone();
            Box::pin(async move {
                crate::media::title_credits::replace_title_credits_tx(tx, &title_id, &[]).await?;
                Err(AppError::Repository("hydration failed".to_string()))
            })
        },
    )
    .await;
    assert!(rolled_back.is_err());

    assert_eq!(
        TitleRepository::get_title_credits(&catalog, &title.id)
            .await
            .expect("credits should load"),
        credits,
        "a rolled-back hydration leaves the prior credit cache intact"
    );

    let _ = std::fs::remove_file(db);
}

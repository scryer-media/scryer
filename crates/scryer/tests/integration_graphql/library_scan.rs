use super::*;

async fn update_library_paths_for_scan(
    ctx: &TestContext,
    movie_path: &str,
    series_path: &str,
    anime_path: &str,
) {
    let update = gql(
        ctx,
        r#"
        mutation UpdateLibraryPaths($input: UpdateLibraryPathsInput!) {
          updateLibraryPaths(input: $input) {
            moviePath
            seriesPath
            animePath
          }
        }
        "#,
        json!({
          "input": {
            "moviePath": movie_path,
            "seriesPath": series_path,
            "animePath": anime_path
          }
        }),
    )
    .await;
    assert_no_errors(&update);
}

#[tokio::test]
async fn graphql_scan_title_library() {
    let ctx = TestContext::new().await;
    let media_root = tempfile::tempdir().expect("media root tempdir");
    let (title, collection) =
        create_series_scan_title(&ctx, media_root.path(), "Scan Show", vec![]).await;
    let episode = create_series_scan_episode(&ctx, &title, &collection, "1", "1", "S01E01").await;

    let season_dir = media_root.path().join(&title.name).join("Season 01");
    std::fs::create_dir_all(&season_dir).expect("create season dir");
    let file_path = season_dir.join("Scan.Show.S01E01.1080p.WEB-DL.mkv");
    std::fs::write(&file_path, b"not-a-real-video").expect("write fake video");

    let body = gql(
        &ctx,
        r#"mutation($titleId: ID!) {
            scanTitleLibrary(titleId: $titleId) {
                scanned
                matched
                imported
                skipped
                unmatched
            }
        }"#,
        json!({ "titleId": title.id.clone() }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["scanTitleLibrary"]["scanned"], 1);
    assert_eq!(body["data"]["scanTitleLibrary"]["matched"], 1);
    assert_eq!(body["data"]["scanTitleLibrary"]["imported"], 1);
    assert_eq!(body["data"]["scanTitleLibrary"]["skipped"], 0);
    assert_eq!(body["data"]["scanTitleLibrary"]["unmatched"], 0);

    let body = gql(
        &ctx,
        r#"query($id: ID!) {
            title(id: $id) {
                mediaFiles {
                    episodeId
                    filePath
                    scanStatus
                }
            }
        }"#,
        json!({ "id": title.id.clone() }),
    )
    .await;
    assert_no_errors(&body);
    let files = body["data"]["title"]["mediaFiles"]
        .as_array()
        .expect("media files array");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0]["episodeId"], episode.id);
    assert_eq!(
        files[0]["filePath"],
        file_path.to_string_lossy().to_string()
    );
    assert_eq!(files[0]["scanStatus"], "scan_failed");

    let persisted_title = ctx
        .titles
        .get_by_id(&title.id)
        .await
        .expect("load title")
        .expect("title exists");
    let expected_folder_path = media_root.path().join(&title.name);
    assert_eq!(
        persisted_title.folder_path.as_deref(),
        Some(expected_folder_path.to_string_lossy().as_ref())
    );
    assert!(
        persisted_title
            .tags
            .iter()
            .all(|tag| tag != "scryer:season-folder:disabled")
    );

    let activity_kinds = activity_kinds_for_title(&ctx, &title.id).await;
    assert!(activity_kinds.iter().any(|kind| kind == "TITLE_UPDATED"));
}

#[tokio::test]
async fn graphql_scan_title_library_removes_stale_media_file_when_file_deleted_on_disk() {
    let ctx = TestContext::new().await;
    let media_root = tempfile::tempdir().expect("media root tempdir");
    let (title, collection) =
        create_series_scan_title(&ctx, media_root.path(), "Stale Scan Show", vec![]).await;
    let episode = create_series_scan_episode(&ctx, &title, &collection, "1", "1", "S01E01").await;

    let season_dir = media_root.path().join(&title.name).join("Season 01");
    std::fs::create_dir_all(&season_dir).expect("create season dir");
    let file_path = season_dir.join("Stale.Scan.Show.S01E01.1080p.WEB-DL.mkv");
    std::fs::write(&file_path, b"not-a-real-video").expect("write fake video");

    let body = gql(
        &ctx,
        r#"mutation($titleId: ID!) {
            scanTitleLibrary(titleId: $titleId) {
                scanned
                matched
                imported
                skipped
                unmatched
            }
        }"#,
        json!({ "titleId": title.id.clone() }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["scanTitleLibrary"]["scanned"], 1);
    assert_eq!(body["data"]["scanTitleLibrary"]["matched"], 1);
    assert_eq!(body["data"]["scanTitleLibrary"]["imported"], 1);

    let body = gql(
        &ctx,
        r#"query($id: ID!) {
            title(id: $id) {
                mediaFiles {
                    episodeId
                    filePath
                }
            }
        }"#,
        json!({ "id": title.id.clone() }),
    )
    .await;
    assert_no_errors(&body);
    let files = body["data"]["title"]["mediaFiles"]
        .as_array()
        .expect("media files array");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0]["episodeId"], episode.id);

    std::fs::remove_file(&file_path).expect("remove scanned file from disk");

    let body = gql(
        &ctx,
        r#"mutation($titleId: ID!) {
            scanTitleLibrary(titleId: $titleId) {
                scanned
                matched
                imported
                skipped
                unmatched
            }
        }"#,
        json!({ "titleId": title.id.clone() }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["scanTitleLibrary"]["scanned"], 0);
    assert_eq!(body["data"]["scanTitleLibrary"]["matched"], 0);
    assert_eq!(body["data"]["scanTitleLibrary"]["imported"], 0);
    assert_eq!(body["data"]["scanTitleLibrary"]["unmatched"], 0);

    let body = gql(
        &ctx,
        r#"query($id: ID!) {
            title(id: $id) {
                mediaFiles {
                    episodeId
                    filePath
                }
            }
        }"#,
        json!({ "id": title.id.clone() }),
    )
    .await;
    assert_no_errors(&body);
    let files = body["data"]["title"]["mediaFiles"]
        .as_array()
        .expect("media files array");
    assert!(
        files.is_empty(),
        "title scan should delete stale media_files rows when the file no longer exists on disk"
    );
}

#[tokio::test]
async fn graphql_scan_title_library_matches_x_episode_numbering_with_title_context() {
    let ctx = TestContext::new().await;
    let media_root = tempfile::tempdir().expect("media root tempdir");
    let (title, collection) =
        create_series_scan_title(&ctx, media_root.path(), "Scan Show", vec![]).await;
    let episode = create_series_scan_episode(&ctx, &title, &collection, "1", "1", "S01E01").await;

    let season_dir = media_root.path().join(&title.name).join("Season 01");
    std::fs::create_dir_all(&season_dir).expect("create season dir");
    let file_path = season_dir.join("Scan Show - 01x01 - Pilot WEBDL-1080p.mkv");
    std::fs::write(&file_path, b"not-a-real-video").expect("write fake video");

    let body = gql(
        &ctx,
        r#"mutation($titleId: ID!) {
            scanTitleLibrary(titleId: $titleId) {
                scanned
                matched
                imported
                skipped
                unmatched
            }
        }"#,
        json!({ "titleId": title.id.clone() }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["scanTitleLibrary"]["scanned"], 1);
    assert_eq!(body["data"]["scanTitleLibrary"]["matched"], 1);
    assert_eq!(body["data"]["scanTitleLibrary"]["imported"], 1);
    assert_eq!(body["data"]["scanTitleLibrary"]["unmatched"], 0);

    let body = gql(
        &ctx,
        r#"query($id: ID!) {
            title(id: $id) {
                mediaFiles {
                    episodeId
                    filePath
                }
            }
        }"#,
        json!({ "id": title.id.clone() }),
    )
    .await;
    assert_no_errors(&body);
    let files = body["data"]["title"]["mediaFiles"]
        .as_array()
        .expect("media files array");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0]["episodeId"], episode.id);
    assert_eq!(
        files[0]["filePath"],
        file_path.to_string_lossy().to_string()
    );
}

#[tokio::test]
async fn graphql_scan_title_library_keeps_standard_episode_titles_with_special_in_name() {
    let ctx = TestContext::new().await;
    let media_root = tempfile::tempdir().expect("media root tempdir");
    let (title, _season_one_collection) =
        create_series_scan_title(&ctx, media_root.path(), "Stoneguard", vec![]).await;

    let season_four = ctx
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: scryer_domain::CollectionType::Season,
            collection_index: "4".to_string(),
            label: Some("Season 4".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: Some("29".to_string()),
            last_episode_number: Some("30".to_string()),
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create season four collection");
    let episode_29 = ctx
        .shows
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(season_four.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("29".to_string()),
            season_number: Some("4".to_string()),
            episode_label: Some("S04E29".to_string()),
            title: Some("The Last Signal Special 1".to_string()),
            air_date: None,
            duration_seconds: Some(1440),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            image_url: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create episode 29");
    let episode_30 = ctx
        .shows
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(season_four.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("30".to_string()),
            season_number: Some("4".to_string()),
            episode_label: Some("S04E30".to_string()),
            title: Some("The Last Signal Special 2".to_string()),
            air_date: None,
            duration_seconds: Some(1440),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            image_url: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create episode 30");

    let season_dir = media_root.path().join(&title.name).join("Season 04");
    std::fs::create_dir_all(&season_dir).expect("create season dir");
    let file_path_29 =
        season_dir.join("Stoneguard.S04E29.The.Last.Signal.Special.1.1080p.WEB-DL.mkv");
    std::fs::write(&file_path_29, b"not-a-real-video").expect("write episode 29");
    let file_path_30 =
        season_dir.join("Stoneguard.S04E30.The.Last.Signal.Special.2.1080p.WEB-DL.mkv");
    std::fs::write(&file_path_30, b"not-a-real-video").expect("write episode 30");

    let body = gql(
        &ctx,
        r#"mutation($titleId: ID!) {
            scanTitleLibrary(titleId: $titleId) {
                scanned
                matched
                imported
                skipped
                unmatched
            }
        }"#,
        json!({ "titleId": title.id.clone() }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["scanTitleLibrary"]["scanned"], 2);
    assert_eq!(body["data"]["scanTitleLibrary"]["matched"], 2);
    assert_eq!(body["data"]["scanTitleLibrary"]["imported"], 2);
    assert_eq!(body["data"]["scanTitleLibrary"]["skipped"], 0);
    assert_eq!(body["data"]["scanTitleLibrary"]["unmatched"], 0);

    let body = gql(
        &ctx,
        r#"query($id: ID!) {
            title(id: $id) {
                mediaFiles {
                    episodeId
                    filePath
                }
            }
        }"#,
        json!({ "id": title.id.clone() }),
    )
    .await;
    assert_no_errors(&body);
    let files = body["data"]["title"]["mediaFiles"]
        .as_array()
        .expect("media files array");
    assert_eq!(files.len(), 2);
    assert!(files.iter().any(|file| {
        file["episodeId"] == episode_29.id
            && file["filePath"] == file_path_29.to_string_lossy().to_string()
    }));
    assert!(files.iter().any(|file| {
        file["episodeId"] == episode_30.id
            && file["filePath"] == file_path_30.to_string_lossy().to_string()
    }));
}

#[tokio::test]
async fn graphql_scan_title_library_matches_numbered_special_episode_on_disk() {
    let ctx = TestContext::new().await;
    let media_root = tempfile::tempdir().expect("media root tempdir");
    let (title, _season_one_collection) =
        create_series_scan_title(&ctx, media_root.path(), "Special Scan Show", vec![]).await;

    let specials_collection = ctx
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: scryer_domain::CollectionType::Season,
            collection_index: "0".to_string(),
            label: Some("Specials".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("1".to_string()),
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create specials collection");
    let special_episode = ctx
        .shows
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(specials_collection.id.clone()),
            episode_type: scryer_domain::EpisodeType::Special,
            episode_number: Some("1".to_string()),
            season_number: Some("0".to_string()),
            episode_label: Some("S00E01".to_string()),
            title: Some("OVA 1".to_string()),
            air_date: None,
            duration_seconds: Some(1440),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            image_url: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create special episode");

    let specials_dir = media_root.path().join(&title.name).join("Specials");
    std::fs::create_dir_all(&specials_dir).expect("create specials dir");
    let file_path = specials_dir.join("Special Scan Show - 01 - OVA 1080p WEB-DL.mkv");
    std::fs::write(&file_path, b"not-a-real-video").expect("write special episode");

    let body = gql(
        &ctx,
        r#"mutation($titleId: ID!) {
            scanTitleLibrary(titleId: $titleId) {
                scanned
                matched
                imported
                skipped
                unmatched
            }
        }"#,
        json!({ "titleId": title.id.clone() }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["scanTitleLibrary"]["scanned"], 1);
    assert_eq!(body["data"]["scanTitleLibrary"]["matched"], 1);
    assert_eq!(body["data"]["scanTitleLibrary"]["imported"], 1);
    assert_eq!(body["data"]["scanTitleLibrary"]["skipped"], 0);
    assert_eq!(body["data"]["scanTitleLibrary"]["unmatched"], 0);

    let body = gql(
        &ctx,
        r#"query($id: ID!) {
            title(id: $id) {
                mediaFiles {
                    episodeId
                    filePath
                }
            }
        }"#,
        json!({ "id": title.id.clone() }),
    )
    .await;
    assert_no_errors(&body);
    let files = body["data"]["title"]["mediaFiles"]
        .as_array()
        .expect("media files array");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0]["episodeId"], special_episode.id);
    assert_eq!(
        files[0]["filePath"],
        file_path.to_string_lossy().to_string()
    );
}

#[tokio::test]
async fn graphql_scan_title_library_matches_daily_episodes_by_air_date() {
    let ctx = TestContext::new().await;
    let media_root = tempfile::tempdir().expect("media root tempdir");
    let (title, collection) =
        create_series_scan_title(&ctx, media_root.path(), "Harbor Report", vec![]).await;
    let episode = Episode {
        id: Id::new().0,
        title_id: title.id.clone(),
        collection_id: Some(collection.id.clone()),
        episode_type: scryer_domain::EpisodeType::Standard,
        episode_number: Some("1".to_string()),
        season_number: Some("1".to_string()),
        episode_label: Some("S01E01".to_string()),
        title: Some("Daily Episode".to_string()),
        air_date: Some("2024-03-15".to_string()),
        duration_seconds: Some(1440),
        has_multi_audio: false,
        has_subtitle: false,
        is_filler: false,
        is_recap: false,
        absolute_number: None,
        overview: None,
        tvdb_id: None,
        image_url: None,
        monitored: true,
        created_at: chrono::Utc::now(),
    };
    let episode = ctx
        .shows
        .create_episode(episode)
        .await
        .expect("create episode");

    let season_dir = media_root.path().join(&title.name).join("Season 01");
    std::fs::create_dir_all(&season_dir).expect("create season dir");
    let file_path = season_dir.join("Harbor.Report.2024.03.15.1080p.WEB-DL.mkv");
    std::fs::write(&file_path, b"not-a-real-video").expect("write fake video");

    let body = gql(
        &ctx,
        r#"mutation($titleId: ID!) {
            scanTitleLibrary(titleId: $titleId) {
                scanned
                matched
                imported
                skipped
                unmatched
            }
        }"#,
        json!({ "titleId": title.id.clone() }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["scanTitleLibrary"]["scanned"], 1);
    assert_eq!(body["data"]["scanTitleLibrary"]["matched"], 1);
    assert_eq!(body["data"]["scanTitleLibrary"]["imported"], 1);
    assert_eq!(body["data"]["scanTitleLibrary"]["skipped"], 0);
    assert_eq!(body["data"]["scanTitleLibrary"]["unmatched"], 0);

    let body = gql(
        &ctx,
        r#"query($id: ID!) {
            title(id: $id) {
                mediaFiles {
                    episodeId
                    filePath
                }
            }
        }"#,
        json!({ "id": title.id.clone() }),
    )
    .await;
    assert_no_errors(&body);
    let files = body["data"]["title"]["mediaFiles"]
        .as_array()
        .expect("media files array");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0]["episodeId"], episode.id);
    assert_eq!(
        files[0]["filePath"],
        file_path.to_string_lossy().to_string()
    );
}

#[tokio::test]
async fn graphql_scan_title_library_disables_season_folders_for_flat_layout() {
    let ctx = TestContext::new().await;
    let media_root = tempfile::tempdir().expect("media root tempdir");
    let (title, collection) =
        create_series_scan_title(&ctx, media_root.path(), "Flat Show", vec![]).await;
    create_series_scan_episode(&ctx, &title, &collection, "1", "1", "S01E01").await;

    let title_dir = media_root.path().join(&title.name);
    std::fs::create_dir_all(&title_dir).expect("create title dir");
    std::fs::write(
        title_dir.join("Flat.Show.S01E01.1080p.WEB-DL.mkv"),
        b"not-a-real-video",
    )
    .expect("write fake video");

    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    ctx.app
        .scan_title_library(&admin, &title.id)
        .await
        .expect("scan title library");

    let persisted_title = ctx
        .titles
        .get_by_id(&title.id)
        .await
        .expect("load title")
        .expect("title exists");
    let expected_folder_path = title_dir.to_string_lossy().to_string();
    assert_eq!(
        persisted_title.folder_path.as_deref(),
        Some(expected_folder_path.as_str())
    );
    assert!(
        persisted_title
            .tags
            .iter()
            .any(|tag| tag == "scryer:season-folder:disabled")
    );
}

#[tokio::test]
async fn graphql_scan_title_library_preserves_existing_layout_when_ambiguous() {
    let ctx = TestContext::new().await;
    let media_root = tempfile::tempdir().expect("media root tempdir");
    let (title, collection) = create_series_scan_title(
        &ctx,
        media_root.path(),
        "Mixed Show",
        vec!["scryer:season-folder:disabled".to_string()],
    )
    .await;
    create_series_scan_episode(&ctx, &title, &collection, "1", "1", "S01E01").await;
    create_series_scan_episode(&ctx, &title, &collection, "1", "2", "S01E02").await;

    let title_dir = media_root.path().join(&title.name);
    let season_dir = title_dir.join("Season 01");
    std::fs::create_dir_all(&season_dir).expect("create season dir");
    std::fs::write(title_dir.join("Mixed.Show.S01E01.1080p.WEB-DL.mkv"), b"one")
        .expect("write flat file");
    std::fs::write(
        season_dir.join("Mixed.Show.S01E02.1080p.WEB-DL.mkv"),
        b"two",
    )
    .expect("write season file");

    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    ctx.app
        .scan_title_library(&admin, &title.id)
        .await
        .expect("scan title library");

    let persisted_title = ctx
        .titles
        .get_by_id(&title.id)
        .await
        .expect("load title")
        .expect("title exists");
    let expected_folder_path = title_dir.to_string_lossy().to_string();
    assert_eq!(
        persisted_title.folder_path.as_deref(),
        Some(expected_folder_path.as_str())
    );
    assert!(
        persisted_title
            .tags
            .iter()
            .any(|tag| tag == "scryer:season-folder:disabled")
    );
    assert_eq!(
        persisted_title
            .tags
            .iter()
            .filter(|tag| tag.starts_with("scryer:season-folder:"))
            .count(),
        1
    );
}

#[tokio::test]
async fn library_series_scan_hydrates_without_creating_wanted_for_unmonitored_titles() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;

    let fixture = json!({
        "data": {
            "metadataBulk": {
                "movies": [],
                "series": [{
                    "tvdb_id": 345678,
                    "name": "Test Show Name",
                    "sort_name": "Test Show Name",
                    "slug": "test-show-name",
                    "status": "Continuing",
                    "year": 2023,
                    "first_aired": "2023-09-15",
                    "overview": "A compelling drama about software testing.",
                    "network": "Test Network",
                    "runtime_minutes": 45,
                    "poster_url": "https://artworks.thetvdb.com/banners/series/345678/posters/test.jpg",
                    "country": "usa",
                    "canonical_tags": [
                        {
                            "key": "canonical:genre:drama",
                            "category": "genre",
                            "name": "Drama",
                            "confidence": 1.0
                        },
                        {
                            "key": "canonical:genre:thriller",
                            "category": "genre",
                            "name": "Thriller",
                            "confidence": 1.0
                        }
                    ],
                    "aliases": ["Testing Show", "QA Chronicles"],
                    "tagged_aliases": [],
                    "artworks": [],
                    "seasons": [
                        {
                            "tvdb_id": 1000001,
                            "number": 1,
                            "label": "Season 1",
                            "episode_type": "default"
                        }
                    ],
                    "episodes": [
                        {
                            "tvdb_id": 2000001,
                            "episode_number": 1,
                            "season_number": 1,
                            "name": "Pilot",
                            "aired": "2023-09-15",
                            "runtime_minutes": 60,
                            "is_filler": false,
                            "is_recap": false,
                            "overview": "The team assembles.",
                            "absolute_number": "1"
                        }
                    ],
                    "anime_mappings": [],
                    "anime_movies": []
                }]
            }
        }
    })
    .to_string();
    Mock::given(method("GET"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture.clone()))
        .mount(&ctx.smg_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture))
        .mount(&ctx.smg_server)
        .await;

    let media_root = tempfile::tempdir().expect("media root tempdir");
    let show_dir = media_root.path().join("Test Show Name");
    std::fs::create_dir_all(&show_dir).expect("create show dir");
    std::fs::write(
        show_dir.join("tvshow.nfo"),
        r#"<tvshow><title>Test Show Name</title><tvdbid>345678</tvdbid></tvshow>"#,
    )
    .expect("write tvshow.nfo");

    let update = gql(
        &ctx,
        r#"
        mutation UpdateLibraryPaths($input: UpdateLibraryPathsInput!) {
          updateLibraryPaths(input: $input) {
            moviePath
            seriesPath
            animePath
          }
        }
        "#,
        json!({
          "input": {
            "moviePath": "/tmp/movies-unused",
            "seriesPath": media_root.path().display().to_string(),
            "animePath": "/tmp/anime-unused"
          }
        }),
    )
    .await;
    assert_no_errors(&update);

    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    ctx.app
        .scan_library(&admin, MediaFacet::Series)
        .await
        .expect("scan library");

    let mut hydrated_title = None;
    for _ in 0..20 {
        let titles = ctx
            .titles
            .list(Some(MediaFacet::Series), None)
            .await
            .expect("list titles");
        assert_eq!(titles.len(), 1);
        if titles[0].metadata_fetched_at.is_some() {
            hydrated_title = Some(titles[0].clone());
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    let hydrated_title = hydrated_title.expect("title should hydrate");
    assert!(!hydrated_title.monitored);

    let (wanted_items, total) = ctx
        .app
        .list_acquisition_scope_states(
            &scryer_domain::User::new_admin("admin"),
            scryer_application::AcquisitionScopeStatesQuery {
                statuses: Vec::new(),
                media_types: Vec::new(),
                title_id: Some(hydrated_title.id.clone()),
                title_search: None,
                latest_decision_codes: Vec::new(),
                limit: 10,
                offset: 0,
                library_ids: Vec::new(),
            },
        )
        .await
        .expect("list wanted items");
    assert!(wanted_items.is_empty());
    assert_eq!(total, 0);
}

#[tokio::test]
async fn library_anime_scan_hydrates_and_relinks_files_from_discovered_folder_path() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;

    let fixture = json!({
        "data": {
            "metadataBulk": {
                "movies": [],
                "series": [{
                    "tvdb_id": 456789,
                    "name": "Hydrated Anime Title",
                    "sort_name": "Hydrated Anime Title",
                    "slug": "hydrated-anime-title",
                    "status": "Ended",
                    "year": 2021,
                    "first_aired": "2021-01-10",
                    "overview": "An anime hydration fixture.",
                    "network": "Tokyo MX",
                    "runtime_minutes": 24,
                    "poster_url": "https://artworks.thetvdb.com/banners/series/456789/posters/test.jpg",
                    "country": "jpn",
                    "canonical_tags": [
                        {
                            "key": "canonical:genre:animation",
                            "category": "genre",
                            "name": "Animation",
                            "confidence": 1.0
                        }
                    ],
                    "aliases": ["Hydrated Anime Alias"],
                    "tagged_aliases": [],
                    "artworks": [],
                    "seasons": [
                        {
                            "tvdb_id": 1001001,
                            "number": 1,
                            "label": "Season 1",
                            "episode_type": "default"
                        }
                    ],
                    "episodes": [
                        {
                            "tvdb_id": 2001001,
                            "episode_number": 1,
                            "season_number": 1,
                            "name": "Episode 1",
                            "aired": "2021-01-10",
                            "runtime_minutes": 24,
                            "is_filler": false,
                            "is_recap": false,
                            "overview": "Episode 1 overview.",
                            "absolute_number": "1"
                        }
                    ],
                    "anime_mappings": [],
                    "anime_movies": []
                }]
            }
        }
    })
    .to_string();
    Mock::given(method("GET"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture.clone()))
        .mount(&ctx.smg_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture))
        .mount(&ctx.smg_server)
        .await;

    let media_root = tempfile::tempdir().expect("media root tempdir");
    let show_dir = media_root.path().join("Anime Scan [SubsPlease]");
    let season_dir = show_dir.join("Season 01");
    std::fs::create_dir_all(&season_dir).expect("create season dir");
    std::fs::write(
        show_dir.join("tvshow.nfo"),
        r#"<tvshow><title>Anime Scan</title><tvdbid>456789</tvdbid></tvshow>"#,
    )
    .expect("write tvshow.nfo");
    let file_path = season_dir.join("Anime.Scan.S01E01.1080p.WEB-DL.mkv");
    std::fs::write(&file_path, b"not-a-real-video").expect("write fake video");

    let update = gql(
        &ctx,
        r#"
        mutation UpdateLibraryPaths($input: UpdateLibraryPathsInput!) {
          updateLibraryPaths(input: $input) {
            moviePath
            seriesPath
            animePath
          }
        }
        "#,
        json!({
          "input": {
            "moviePath": "/tmp/movies-unused",
            "seriesPath": "/tmp/series-unused",
            "animePath": media_root.path().display().to_string()
          }
        }),
    )
    .await;
    assert_no_errors(&update);

    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let summary = ctx
        .app
        .scan_library(&admin, MediaFacet::Anime)
        .await
        .expect("scan anime library");
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.imported, 1);
    assert_eq!(summary.skipped, 0);

    let mut hydrated_title = None;
    let mut linked_files = Vec::new();
    for _ in 0..100 {
        let titles = ctx
            .titles
            .list(Some(MediaFacet::Anime), None)
            .await
            .expect("list anime titles");
        assert_eq!(titles.len(), 1);
        let files = ctx
            .media_files
            .list_media_files_for_title(&titles[0].id)
            .await
            .expect("list media files");
        if titles[0].metadata_fetched_at.is_some()
            && files.iter().any(|file| file.episode_id.is_some())
        {
            hydrated_title = Some(titles[0].clone());
            linked_files = files;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    let hydrated_title = hydrated_title.expect("anime title should hydrate and relink files");
    assert_eq!(hydrated_title.name, "Hydrated Anime Title");
    assert!(hydrated_title.metadata_fetched_at.is_some());
    assert_eq!(
        hydrated_title.folder_path.as_deref(),
        Some(show_dir.to_string_lossy().as_ref())
    );

    assert_eq!(linked_files.len(), 1);
    assert_eq!(
        linked_files[0].file_path,
        file_path.to_string_lossy().to_string()
    );
    assert!(
        linked_files[0].episode_id.is_some(),
        "linked file should target a hydrated episode"
    );
    assert_eq!(linked_files[0].scan_status, "scan_failed");
}

#[tokio::test]
async fn library_anime_scan_prefers_tvshow_nfo_identity_for_nightfall_fixture() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;

    let fixture = json!({
        "data": {
            "metadataBulk": {
                "movies": [],
                "series": [{
                    "tvdb_id": 415677,
                    "name": "Nightfall!! Correct Match",
                    "sort_name": "Nightfall!! Correct Match",
                    "slug": "nightfall-correct-match",
                    "status": "Ended",
                    "year": 2022,
                    "first_aired": "2022-06-30",
                    "overview": "A regression fixture for the Nightfall!! anime scan path.",
                    "network": "Netflix",
                    "runtime_minutes": 24,
                    "poster_url": "https://artworks.thetvdb.com/banners/series/415677/posters/test.jpg",
                    "country": "jpn",
                    "canonical_tags": [
                        {
                            "key": "canonical:genre:animation",
                            "category": "genre",
                            "name": "Animation",
                            "confidence": 1.0
                        },
                        {
                            "key": "canonical:genre:fantasy",
                            "category": "genre",
                            "name": "Fantasy",
                            "confidence": 1.0
                        }
                    ],
                    "aliases": ["Nightfall!! Kage no Requiem"],
                    "tagged_aliases": [],
                    "artworks": [],
                    "seasons": [
                        {
                            "tvdb_id": 14156771,
                            "number": 1,
                            "label": "Season 1",
                            "episode_type": "default"
                        }
                    ],
                    "episodes": [
                        {
                            "tvdb_id": 24156771,
                            "episode_number": 1,
                            "season_number": 1,
                            "name": "Episode 1",
                            "aired": "2022-06-30",
                            "runtime_minutes": 24,
                            "is_filler": false,
                            "is_recap": false,
                            "overview": "Episode 1 overview.",
                            "absolute_number": "1"
                        }
                    ],
                    "anime_mappings": [],
                    "anime_movies": []
                }]
            }
        }
    })
    .to_string();
    Mock::given(method("GET"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture.clone()))
        .mount(&ctx.smg_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture))
        .mount(&ctx.smg_server)
        .await;

    let nightfall_tvshow_nfo = r#"<?xml version="1.0" encoding="utf-8" standalone="yes"?>
<tvshow>
  <plot>Nightfall!! follows the remnant wardens of a ruined sky-kingdom as they try to stop a shard-born eclipse from swallowing the last inhabited cities.</plot>
  <outline>Nightfall!! follows the remnant wardens of a ruined sky-kingdom as they try to stop a shard-born eclipse from swallowing the last inhabited cities.</outline>
  <lockdata>false</lockdata>
  <dateadded>2026-04-21 04:22:41</dateadded>
  <title>Nightfall!!</title>
  <originaltitle>Nightfall!! Kage no Requiem</originaltitle>
  <trailer>plugin://plugin.video.youtube/play/?video_id=_Iqc-dG8peA</trailer>
  <trailer>plugin://plugin.video.youtube/play/?video_id=Vt4zSf3CfRA</trailer>
  <rating>5</rating>
  <year>2022</year>
  <mpaa>TV-MA</mpaa>
  <collectionnumber>156898</collectionnumber>
  <imdb_id>tt17736234</imdb_id>
  <tmdbid>156898</tmdbid>
  <premiered>1992-08-25</premiered>
  <releasedate>1992-08-25</releasedate>
  <enddate>1993-06-25</enddate>
  <runtime>25</runtime>
  <genre>Anime</genre>
  <genre>magic</genre>
  <genre>stereotypes</genre>
  <genre>super power</genre>
  <genre>violence</genre>
  <studio />
  <studio>Netflix</studio>
  <tag>anime</tag>
  <tag>based on manga</tag>
  <tag>combat</tag>
  <tag>dark fantasy</tag>
  <tag>ecchi</tag>
  <tag>heavy metal</tag>
  <tag>magic</tag>
  <tag>original net animation (ona)</tag>
  <tag>remake</tag>
  <tag>seinen</tag>
  <anidbid>10</anidbid>
  <tvdbid>415677</tvdbid>
  <tvdbslugid>nightfall-2022</tvdbslugid>
  <art>
    <poster>/config/metadata/library/df/df254e34942e2f83823ce24206a65630/poster.jpg</poster>
    <fanart>/config/metadata/library/df/df254e34942e2f83823ce24206a65630/backdrop.jpg</fanart>
  </art>
  <id>415677</id>
  <episodeguide>
    <url cache="415677.xml">http://www.thetvdb.com/api/1D62F2F90030C444/series/415677/all/en.zip</url>
  </episodeguide>
  <season>-1</season>
  <episode>-1</episode>
  <status>Ended</status>
</tvshow>"#;

    let media_root = tempfile::tempdir().expect("media root tempdir");
    let show_dir = media_root.path().join("Nightfall!! (2022)");
    let season_dir = show_dir.join("Season 01");
    std::fs::create_dir_all(&season_dir).expect("create season dir");
    std::fs::write(show_dir.join("tvshow.nfo"), nightfall_tvshow_nfo).expect("write tvshow.nfo");
    std::fs::write(
        season_dir.join("Nightfall!! (2022) - S01E01 (1) - 1080p.mkv"),
        b"not-a-real-video",
    )
    .expect("write fake video");
    std::fs::write(
        season_dir.join("Nightfall!! (2022) - S01E01 (1) - 1080p.nfo"),
        b"<episodedetails><title>Episode 1</title></episodedetails>",
    )
    .expect("write episode nfo");
    std::fs::write(season_dir.join("season.nfo"), b"<season></season>").expect("write season nfo");

    let update = gql(
        &ctx,
        r#"
        mutation UpdateLibraryPaths($input: UpdateLibraryPathsInput!) {
          updateLibraryPaths(input: $input) {
            moviePath
            seriesPath
            animePath
          }
        }
        "#,
        json!({
          "input": {
            "moviePath": "/tmp/movies-unused",
            "seriesPath": "/tmp/series-unused",
            "animePath": media_root.path().display().to_string()
          }
        }),
    )
    .await;
    assert_no_errors(&update);

    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let summary = ctx
        .app
        .scan_library(&admin, MediaFacet::Anime)
        .await
        .expect("scan anime library");
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.imported, 1);
    assert_eq!(summary.unmatched, 0);

    let mut hydrated_title = None;
    for _ in 0..100 {
        let titles = ctx
            .titles
            .list(Some(MediaFacet::Anime), None)
            .await
            .expect("list anime titles");
        assert_eq!(titles.len(), 1);
        let title = &titles[0];
        if title.metadata_fetched_at.is_some() {
            hydrated_title = Some(title.clone());
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    let hydrated_title =
        hydrated_title.expect("anime title should hydrate from tvshow.nfo identity");
    assert_eq!(hydrated_title.name, "Nightfall!! Correct Match");
    assert!(hydrated_title.metadata_fetched_at.is_some());
    assert_eq!(
        hydrated_title.folder_path.as_deref(),
        Some(show_dir.to_string_lossy().as_ref())
    );
    assert!(
        hydrated_title
            .external_ids
            .iter()
            .any(|id| id.source == "tvdb" && id.value == "415677"),
        "hydrated title should preserve the Nightfall!! TVDB identity"
    );
}

#[tokio::test]
async fn library_anime_scan_relinks_existing_hydrated_titles_from_discovered_folder_path() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let media_root = tempfile::tempdir().expect("media root tempdir");
    update_library_paths_for_scan(
        &ctx,
        "/tmp/movies-unused",
        "/tmp/series-unused",
        media_root.path().to_string_lossy().as_ref(),
    )
    .await;

    let title = create_catalog_title(
        &ctx,
        "Existing Anime",
        MediaFacet::Anime,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "567890".to_string(),
        }],
        vec![],
        false,
    )
    .await;

    let collection = Collection {
        id: Id::new().0,
        title_id: title.id.clone(),
        collection_type: scryer_domain::CollectionType::Season,
        collection_index: "1".to_string(),
        label: Some("Season 1".to_string()),
        ordered_path: None,
        narrative_order: None,
        first_episode_number: Some("1".to_string()),
        last_episode_number: Some("1".to_string()),
        monitored: false,
        created_at: chrono::Utc::now(),
    };
    let collection = ctx
        .shows
        .create_collection(collection)
        .await
        .expect("create collection");
    let episode = create_series_scan_episode(&ctx, &title, &collection, "1", "1", "S01E01").await;

    let show_dir = media_root.path().join("Existing Anime [BD]");
    let season_dir = show_dir.join("Season 01");
    std::fs::create_dir_all(&season_dir).expect("create season dir");
    std::fs::write(
        show_dir.join("tvshow.nfo"),
        r#"<tvshow><title>Existing Anime</title><tvdbid>567890</tvdbid></tvshow>"#,
    )
    .expect("write tvshow.nfo");
    let file_path = season_dir.join("Existing.Anime.S01E01.1080p.WEB-DL.mkv");
    std::fs::write(&file_path, b"not-a-real-video").expect("write fake video");

    let update = gql(
        &ctx,
        r#"
        mutation UpdateLibraryPaths($input: UpdateLibraryPathsInput!) {
          updateLibraryPaths(input: $input) {
            moviePath
            seriesPath
            animePath
          }
        }
        "#,
        json!({
          "input": {
            "moviePath": "/tmp/movies-unused",
            "seriesPath": "/tmp/series-unused",
            "animePath": media_root.path().display().to_string()
          }
        }),
    )
    .await;
    assert_no_errors(&update);

    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let summary = ctx
        .app
        .scan_library(&admin, MediaFacet::Anime)
        .await
        .expect("scan anime library");
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.imported, 0);
    assert_eq!(summary.skipped, 0);

    let mut linked_files = Vec::new();
    for _ in 0..100 {
        linked_files = ctx
            .media_files
            .list_media_files_for_title(&title.id)
            .await
            .expect("list media files");
        if !linked_files.is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }

    let refreshed_title = ctx
        .titles
        .get_by_id(&title.id)
        .await
        .expect("load title")
        .expect("title exists");
    assert_eq!(refreshed_title.name, "Existing Anime");
    assert_eq!(
        refreshed_title.folder_path.as_deref(),
        Some(show_dir.to_string_lossy().as_ref())
    );

    assert_eq!(linked_files.len(), 1);
    assert_eq!(
        linked_files[0].file_path,
        file_path.to_string_lossy().to_string()
    );
    assert_eq!(
        linked_files[0].episode_id.as_deref(),
        Some(episode.id.as_str())
    );
    assert_eq!(linked_files[0].scan_status, "scan_failed");
}

#[tokio::test]
async fn library_series_scan_relinks_existing_hydrated_titles_from_discovered_folder_path() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let media_root = tempfile::tempdir().expect("media root tempdir");
    update_library_paths_for_scan(
        &ctx,
        "/tmp/movies-unused",
        media_root.path().to_string_lossy().as_ref(),
        "/tmp/anime-unused",
    )
    .await;

    let title = create_catalog_title(
        &ctx,
        "Existing Series",
        MediaFacet::Series,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "345678".to_string(),
        }],
        vec![],
        false,
    )
    .await;

    let collection = Collection {
        id: Id::new().0,
        title_id: title.id.clone(),
        collection_type: scryer_domain::CollectionType::Season,
        collection_index: "1".to_string(),
        label: Some("Season 1".to_string()),
        ordered_path: None,
        narrative_order: None,
        first_episode_number: Some("1".to_string()),
        last_episode_number: Some("1".to_string()),
        monitored: false,
        created_at: chrono::Utc::now(),
    };
    let collection = ctx
        .shows
        .create_collection(collection)
        .await
        .expect("create collection");
    let episode = create_series_scan_episode(&ctx, &title, &collection, "1", "1", "S01E01").await;

    let show_dir = media_root.path().join("Existing Series [WEB-DL]");
    let season_dir = show_dir.join("Season 01");
    std::fs::create_dir_all(&season_dir).expect("create season dir");
    std::fs::write(
        show_dir.join("tvshow.nfo"),
        r#"<tvshow><title>Existing Series</title><tvdbid>345678</tvdbid></tvshow>"#,
    )
    .expect("write tvshow.nfo");
    let file_path = season_dir.join("Existing.Series.S01E01.1080p.WEB-DL.mkv");
    std::fs::write(&file_path, b"not-a-real-video").expect("write fake video");

    let update = gql(
        &ctx,
        r#"
        mutation UpdateLibraryPaths($input: UpdateLibraryPathsInput!) {
          updateLibraryPaths(input: $input) {
            moviePath
            seriesPath
            animePath
          }
        }
        "#,
        json!({
          "input": {
            "moviePath": "/tmp/movies-unused",
            "seriesPath": media_root.path().display().to_string(),
            "animePath": "/tmp/anime-unused"
          }
        }),
    )
    .await;
    assert_no_errors(&update);

    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let summary = ctx
        .app
        .scan_library(&admin, MediaFacet::Series)
        .await
        .expect("scan series library");
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.imported, 0);
    assert_eq!(summary.skipped, 0);

    let mut linked_files = Vec::new();
    for _ in 0..100 {
        linked_files = ctx
            .media_files
            .list_media_files_for_title(&title.id)
            .await
            .expect("list media files");
        if !linked_files.is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }

    let refreshed_title = ctx
        .titles
        .get_by_id(&title.id)
        .await
        .expect("load title")
        .expect("title exists");
    assert_eq!(refreshed_title.name, "Existing Series");
    assert_eq!(
        refreshed_title.folder_path.as_deref(),
        Some(show_dir.to_string_lossy().as_ref())
    );

    assert_eq!(linked_files.len(), 1);
    assert_eq!(
        linked_files[0].file_path,
        file_path.to_string_lossy().to_string()
    );
    assert_eq!(
        linked_files[0].episode_id.as_deref(),
        Some(episode.id.as_str())
    );
    assert_eq!(linked_files[0].scan_status, "scan_failed");
}

#[tokio::test]
async fn library_series_scan_existing_unhydrated_title_without_episodes_completes_session() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let media_root = tempfile::tempdir().expect("media root tempdir");
    update_library_paths_for_scan(
        &ctx,
        "/tmp/movies-unused",
        media_root.path().to_string_lossy().as_ref(),
        "/tmp/anime-unused",
    )
    .await;

    let title = ctx
        .titles
        .create(Title {
            id: Id::new().0,
            name: "Pending Series".to_string(),
            facet: MediaFacet::Series,
            library_id: scryer_domain::default_library_id_for_facet(&MediaFacet::Series),
            monitored: false,
            tags: vec![],
            canonical_tags: vec![],
            external_ids: vec![ExternalId {
                source: "tvdb".to_string(),
                value: "345679".to_string(),
            }],
            root_folder_id: scryer_domain::root_folder_id_for_path("/data/series"),
            created_by: None,
            created_at: Utc::now(),
            year: Some(2024),
            overview: Some("Pending hydration title".to_string()),
            poster_url: None,
            poster_source_url: None,
            background_url: None,
            background_source_url: None,
            sort_title: Some("Pending Series".to_string()),
            catalog_sort_key: String::new(),
            slug: Some("pending-series".to_string()),
            imdb_id: None,
            runtime_minutes: None,
            popularity: None,
            content_status: None,
            language: None,
            first_aired: None,
            network: None,
            studio: None,
            country: None,
            aliases: vec![],
            tagged_aliases: vec![],
            metadata_language: Some("eng".to_string()),
            metadata_fetched_at: None,
            min_availability: None,
            digital_release_date: None,
            folder_path: None,
        })
        .await
        .expect("create pending title");

    let show_dir = media_root.path().join("Pending Series [WEB-DL]");
    let season_dir = show_dir.join("Season 01");
    std::fs::create_dir_all(&season_dir).expect("create season dir");
    std::fs::write(
        show_dir.join("tvshow.nfo"),
        r#"<tvshow><title>Pending Series</title><tvdbid>345679</tvdbid></tvshow>"#,
    )
    .expect("write tvshow.nfo");
    let file_path = season_dir.join("Pending.Series.S01E01.1080p.WEB-DL.mkv");
    std::fs::write(&file_path, b"not-a-real-video").expect("write fake video");

    let update = gql(
        &ctx,
        r#"
        mutation UpdateLibraryPaths($input: UpdateLibraryPathsInput!) {
          updateLibraryPaths(input: $input) {
            moviePath
            seriesPath
            animePath
          }
        }
        "#,
        json!({
          "input": {
            "moviePath": "/tmp/movies-unused",
            "seriesPath": media_root.path().display().to_string(),
            "animePath": "/tmp/anime-unused"
          }
        }),
    )
    .await;
    assert_no_errors(&update);

    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let summary = ctx
        .app
        .scan_library(&admin, MediaFacet::Series)
        .await
        .expect("scan series library");
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.imported, 0);
    assert_eq!(summary.skipped, 0);

    let refreshed_title = ctx
        .titles
        .get_by_id(&title.id)
        .await
        .expect("load title")
        .expect("title exists");
    let media_files = ctx
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files");
    let episodes = ctx
        .shows
        .list_episodes_for_title(&title.id)
        .await
        .expect("list episodes");
    assert_eq!(
        refreshed_title.folder_path.as_deref(),
        Some(show_dir.to_string_lossy().as_ref())
    );
    assert!(
        ctx.app.active_library_scan_sessions().await.is_empty(),
        "scan session should complete when an existing unhydrated title is skipped",
    );
    assert!(media_files.is_empty());
    assert!(episodes.is_empty());
}

#[tokio::test]
async fn library_series_scan_creates_unmonitored_titles() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;

    let media_root = tempfile::tempdir().expect("media root tempdir");
    let show_dir = media_root.path().join("Harbor Pals");
    std::fs::create_dir_all(&show_dir).expect("create show dir");
    std::fs::write(
        show_dir.join("tvshow.nfo"),
        r#"<tvshow><title>Harbor Pals</title><tvdbid>81189</tvdbid></tvshow>"#,
    )
    .expect("write tvshow.nfo");

    let update = gql(
        &ctx,
        r#"
        mutation UpdateLibraryPaths($input: UpdateLibraryPathsInput!) {
          updateLibraryPaths(input: $input) {
            moviePath
            seriesPath
            animePath
          }
        }
        "#,
        json!({
          "input": {
            "moviePath": "/tmp/movies-unused",
            "seriesPath": media_root.path().display().to_string(),
            "animePath": "/tmp/anime-unused"
          }
        }),
    )
    .await;
    assert_no_errors(&update);

    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let summary = ctx
        .app
        .scan_library(&admin, MediaFacet::Series)
        .await
        .expect("scan library");

    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.imported, 1);
    assert_eq!(summary.skipped, 0);

    let titles = ctx
        .titles
        .list(Some(MediaFacet::Series), None)
        .await
        .expect("list titles");
    assert_eq!(titles.len(), 1);
    assert_eq!(titles[0].name, "Harbor Pals");
    assert!(!titles[0].monitored);
}

#[tokio::test]
async fn library_series_scan_counts_new_title_files_before_post_hydration_scan_progress() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;

    let media_root = tempfile::tempdir().expect("media root tempdir");
    let show_dir = media_root.path().join("Harbor Pals");
    let season_dir = show_dir.join("Season 01");
    std::fs::create_dir_all(&season_dir).expect("create show dir");
    std::fs::write(
        show_dir.join("tvshow.nfo"),
        r#"<tvshow><title>Harbor Pals</title><tvdbid>81189</tvdbid></tvshow>"#,
    )
    .expect("write tvshow.nfo");
    std::fs::write(
        season_dir.join("Harbor.Pals.S01E01.720p.WEB-DL.mkv"),
        b"not-a-real-video",
    )
    .expect("write fake episode");

    let update = gql(
        &ctx,
        r#"
        mutation UpdateLibraryPaths($input: UpdateLibraryPathsInput!) {
          updateLibraryPaths(input: $input) {
            moviePath
            seriesPath
            animePath
          }
        }
        "#,
        json!({
          "input": {
            "moviePath": "/tmp/movies-unused",
            "seriesPath": media_root.path().display().to_string(),
            "animePath": "/tmp/anime-unused"
          }
        }),
    )
    .await;
    assert_no_errors(&update);

    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let summary = ctx
        .app
        .scan_library(&admin, MediaFacet::Series)
        .await
        .expect("scan library");

    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.imported, 1);

    assert!(
        ctx.app.active_library_scan_sessions().await.is_empty(),
        "scan session should complete before the synchronous scan call returns",
    );
}

#[tokio::test]
async fn library_movie_scan_records_owned_folder_conflict_without_rehoming_title() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let media_root = tempfile::tempdir().expect("media root tempdir");
    update_library_paths_for_scan(
        &ctx,
        media_root.path().to_string_lossy().as_ref(),
        "/tmp/series-unused",
        "/tmp/anime-unused",
    )
    .await;

    let title = create_catalog_title(
        &ctx,
        "Existing Movie",
        MediaFacet::Movie,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "123456".to_string(),
        }],
        vec![],
        false,
    )
    .await;

    let stale_root = tempfile::tempdir().expect("stale root tempdir");
    let stale_folder = stale_root.path().join("Existing Movie");
    std::fs::create_dir_all(&stale_folder).expect("create stale folder");
    ctx.titles
        .set_folder_path(&title.id, stale_folder.to_string_lossy().as_ref())
        .await
        .expect("set stale folder path");

    let movie_dir = media_root.path().join("Existing Movie [2160p]");
    std::fs::create_dir_all(&movie_dir).expect("create movie dir");
    let movie_path = movie_dir.join("Existing.Movie.2024.2160p.WEB-DL.mkv");
    let movie_file = std::fs::File::create(&movie_path).expect("create movie file");
    movie_file
        .set_len(60 * 1024 * 1024)
        .expect("set movie file size");
    std::fs::write(
        movie_dir.join("movie.nfo"),
        r#"<movie><title>Existing Movie</title><tvdbid>123456</tvdbid><year>2024</year></movie>"#,
    )
    .expect("write movie.nfo");

    let update = gql(
        &ctx,
        r#"
        mutation UpdateLibraryPaths($input: UpdateLibraryPathsInput!) {
          updateLibraryPaths(input: $input) {
            moviePath
            seriesPath
            animePath
          }
        }
        "#,
        json!({
          "input": {
            "moviePath": media_root.path().display().to_string(),
            "seriesPath": "/tmp/series-unused",
            "animePath": "/tmp/anime-unused"
          }
        }),
    )
    .await;
    assert_no_errors(&update);

    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let summary = ctx
        .app
        .scan_library(&admin, MediaFacet::Movie)
        .await
        .expect("scan movie library");

    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.matched, 0);
    assert_eq!(summary.imported, 0);
    assert_eq!(summary.skipped, 0);
    assert_eq!(summary.unmatched, 1);

    let pending = gql(
        &ctx,
        r#"
        query PendingMovieFolderOwnershipConflicts {
          pendingImports(facet: MOVIE, status: PENDING) {
            totalCount
            items {
              titleId
              titleName
              path
              reason
            }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&pending);
    assert_eq!(pending["data"]["pendingImports"]["totalCount"], 1);
    let pending_item = &pending["data"]["pendingImports"]["items"][0];
    assert_eq!(pending_item["titleId"], title.id);
    assert_eq!(pending_item["titleName"], title.name);
    assert_eq!(pending_item["path"], movie_path.to_string_lossy().as_ref());
    assert_eq!(pending_item["reason"], "title_already_owns_another_folder");

    let refreshed_title = ctx
        .titles
        .get_by_id(&title.id)
        .await
        .expect("load title")
        .expect("title exists");
    assert_eq!(refreshed_title.name, "Existing Movie");
    assert_eq!(
        refreshed_title.folder_path.as_deref(),
        Some(stale_folder.to_string_lossy().as_ref())
    );

    let titles = ctx
        .titles
        .list(Some(MediaFacet::Movie), None)
        .await
        .expect("list movie titles");
    assert_eq!(titles.len(), 1, "scan must not create a duplicate title");

    let collections = ctx
        .shows
        .list_collections_for_title(&title.id)
        .await
        .expect("list collections");
    assert!(collections.is_empty());

    let media_files = ctx
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files");
    assert!(media_files.is_empty());
}

#[tokio::test]
async fn library_movie_scan_matches_existing_title_from_movie_nfo_when_folder_missing() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let media_root = tempfile::tempdir().expect("media root tempdir");
    update_library_paths_for_scan(
        &ctx,
        media_root.path().to_string_lossy().as_ref(),
        "/tmp/series-unused",
        "/tmp/anime-unused",
    )
    .await;

    let title = create_catalog_title(
        &ctx,
        "Existing Movie",
        MediaFacet::Movie,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "123456".to_string(),
        }],
        vec![],
        false,
    )
    .await;

    let movie_dir = media_root.path().join("Existing Movie [2160p]");
    std::fs::create_dir_all(&movie_dir).expect("create movie dir");
    let movie_path = movie_dir.join("Existing.Movie.2024.2160p.WEB-DL.mkv");
    let movie_file = std::fs::File::create(&movie_path).expect("create movie file");
    movie_file
        .set_len(60 * 1024 * 1024)
        .expect("set movie file size");
    std::fs::write(
        movie_dir.join("movie.nfo"),
        r#"<movie><title>Existing Movie</title><tvdbid>123456</tvdbid><year>2024</year></movie>"#,
    )
    .expect("write movie.nfo");

    let update = gql(
        &ctx,
        r#"
        mutation UpdateLibraryPaths($input: UpdateLibraryPathsInput!) {
          updateLibraryPaths(input: $input) {
            moviePath
            seriesPath
            animePath
          }
        }
        "#,
        json!({
          "input": {
            "moviePath": media_root.path().display().to_string(),
            "seriesPath": "/tmp/series-unused",
            "animePath": "/tmp/anime-unused"
          }
        }),
    )
    .await;
    assert_no_errors(&update);

    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let summary = ctx
        .app
        .scan_library(&admin, MediaFacet::Movie)
        .await
        .expect("scan movie library");

    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.matched, 1);
    assert_eq!(summary.imported, 1);
    assert_eq!(summary.skipped, 0);
    assert_eq!(summary.unmatched, 0);

    let refreshed_title = ctx
        .titles
        .get_by_id(&title.id)
        .await
        .expect("load title")
        .expect("title exists");
    assert_eq!(refreshed_title.name, "Existing Movie");
    assert_eq!(
        refreshed_title.folder_path.as_deref(),
        Some(movie_dir.to_string_lossy().as_ref())
    );

    let collections = ctx
        .shows
        .list_collections_for_title(&title.id)
        .await
        .expect("list collections");
    assert_eq!(collections.len(), 1);
    assert_eq!(
        collections[0].ordered_path.as_deref(),
        Some(movie_path.to_string_lossy().as_ref())
    );

    let media_files = ctx
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files");
    assert_eq!(media_files.len(), 1);
    assert_eq!(
        media_files[0].file_path,
        movie_path.to_string_lossy().to_string()
    );
    assert_eq!(media_files[0].scan_status, "scan_failed");
}

#[tokio::test]
async fn library_movie_scan_creates_unmonitored_title_and_collection() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    mount_smg_mocks(&ctx, "smg/metadata_bulk_movie.json").await;

    let media_root = tempfile::tempdir().expect("media root tempdir");
    let movie_dir = media_root.path().join("Test Movie Title (2024)");
    std::fs::create_dir_all(&movie_dir).expect("create movie dir");
    let movie_path = movie_dir.join("Test.Movie.Title.2024.1080p.WEB-DL.mkv");
    let movie_file = std::fs::File::create(&movie_path).expect("create movie file");
    movie_file
        .set_len(60 * 1024 * 1024)
        .expect("set movie file size");
    std::fs::write(
        movie_dir.join("movie.nfo"),
        r#"<movie><title>Test Movie Title</title><tvdbid>123456</tvdbid><year>2024</year></movie>"#,
    )
    .expect("write movie.nfo");

    let update = gql(
        &ctx,
        r#"
        mutation UpdateLibraryPaths($input: UpdateLibraryPathsInput!) {
          updateLibraryPaths(input: $input) {
            moviePath
            seriesPath
            animePath
          }
        }
        "#,
        json!({
          "input": {
            "moviePath": media_root.path().display().to_string(),
            "seriesPath": "/tmp/series-unused",
            "animePath": "/tmp/anime-unused"
          }
        }),
    )
    .await;
    assert_no_errors(&update);

    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let summary = ctx
        .app
        .scan_library(&admin, MediaFacet::Movie)
        .await
        .expect("scan movie library");

    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.matched, 1);
    assert_eq!(summary.imported, 1);
    assert_eq!(summary.skipped, 0);
    assert_eq!(summary.unmatched, 0);

    let mut hydrated_title = None;
    for _ in 0..20 {
        let titles = ctx
            .titles
            .list(Some(MediaFacet::Movie), None)
            .await
            .expect("list titles");
        assert_eq!(titles.len(), 1);
        if titles[0].metadata_fetched_at.is_some() {
            hydrated_title = Some(titles[0].clone());
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let hydrated_title = hydrated_title.expect("movie title should hydrate");
    assert_eq!(hydrated_title.name, "Test Movie Title");
    assert!(!hydrated_title.monitored);

    let collections = ctx
        .shows
        .list_collections_for_title(&hydrated_title.id)
        .await
        .expect("list collections");
    assert_eq!(collections.len(), 1);
    assert!(!collections[0].monitored);
    assert_eq!(
        collections[0].ordered_path.as_deref(),
        Some(movie_path.to_string_lossy().as_ref())
    );

    let media_files = ctx
        .media_files
        .list_media_files_for_title(&hydrated_title.id)
        .await
        .expect("list media files");
    assert_eq!(media_files.len(), 1);
    assert_eq!(
        media_files[0].file_path,
        movie_path.to_string_lossy().to_string()
    );
    assert_eq!(media_files[0].scan_status, "scan_failed");

    let (wanted_items, total) = ctx
        .app
        .list_acquisition_scope_states(
            &scryer_domain::User::new_admin("admin"),
            scryer_application::AcquisitionScopeStatesQuery {
                statuses: Vec::new(),
                media_types: Vec::new(),
                title_id: Some(hydrated_title.id.clone()),
                title_search: None,
                latest_decision_codes: Vec::new(),
                limit: 10,
                offset: 0,
                library_ids: Vec::new(),
            },
        )
        .await
        .expect("list wanted items");
    assert!(wanted_items.is_empty());
    assert_eq!(total, 0);
}

#[tokio::test]
async fn library_series_scan_handles_more_than_one_batch_of_titles() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;

    let media_root = tempfile::tempdir().expect("media root tempdir");
    for index in 0..300 {
        let folder = media_root.path().join(format!("Show {index:04}"));
        std::fs::create_dir_all(&folder).expect("create show dir");
        std::fs::write(
            folder.join("tvshow.nfo"),
            format!(
                "<tvshow><title>Show {index:04}</title><tvdbid>{}</tvdbid></tvshow>",
                900_000 + index
            ),
        )
        .expect("write tvshow.nfo");
    }

    let update = gql(
        &ctx,
        r#"
        mutation UpdateLibraryPaths($input: UpdateLibraryPathsInput!) {
          updateLibraryPaths(input: $input) {
            moviePath
            seriesPath
            animePath
          }
        }
        "#,
        json!({
          "input": {
            "moviePath": "/tmp/movies-unused",
            "seriesPath": media_root.path().display().to_string(),
            "animePath": "/tmp/anime-unused"
          }
        }),
    )
    .await;
    assert_no_errors(&update);

    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let summary = ctx
        .app
        .scan_library(&admin, MediaFacet::Series)
        .await
        .expect("scan library");

    assert_eq!(summary.scanned, 300);
    assert_eq!(summary.imported, 300);
    assert_eq!(summary.skipped, 0);
    assert_eq!(summary.unmatched, 0);

    let titles = ctx
        .titles
        .list(Some(MediaFacet::Series), None)
        .await
        .expect("list titles");
    assert_eq!(titles.len(), 300);
    assert!(titles.iter().all(|title| !title.monitored));
}

#[tokio::test]
async fn library_movie_scan_handles_more_than_one_batch_of_titles() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;

    let media_root = tempfile::tempdir().expect("media root tempdir");
    for index in 0..300 {
        let display_name = format!("Movie.Title.{index:04}.2024");
        let movie_folder = media_root.path().join(&display_name);
        std::fs::create_dir(&movie_folder).expect("create movie folder");
        let video_path = movie_folder.join(format!("{display_name}.mkv"));
        std::fs::write(&video_path, b"video").expect("write movie");
        std::fs::write(
            movie_folder.join("movie.nfo"),
            format!(
                "<movie><title>Movie {index:04}</title><tvdbid>{}</tvdbid><year>2024</year></movie>",
                800_000 + index
            ),
        )
        .expect("write movie nfo");
    }

    let update = gql(
        &ctx,
        r#"
        mutation UpdateLibraryPaths($input: UpdateLibraryPathsInput!) {
          updateLibraryPaths(input: $input) {
            moviePath
            seriesPath
            animePath
          }
        }
        "#,
        json!({
          "input": {
            "moviePath": media_root.path().display().to_string(),
            "seriesPath": "/tmp/series-unused",
            "animePath": "/tmp/anime-unused"
          }
        }),
    )
    .await;
    assert_no_errors(&update);

    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let summary = ctx
        .app
        .scan_library(&admin, MediaFacet::Movie)
        .await
        .expect("scan movie library");

    assert_eq!(summary.scanned, 300);
    assert_eq!(summary.matched, 300);
    assert_eq!(summary.imported, 300);
    assert_eq!(summary.skipped, 0);
    assert_eq!(summary.unmatched, 0);

    let titles = ctx
        .titles
        .list(Some(MediaFacet::Movie), None)
        .await
        .expect("list titles");
    assert_eq!(titles.len(), 300);
    assert!(titles.iter().all(|title| !title.monitored));
}

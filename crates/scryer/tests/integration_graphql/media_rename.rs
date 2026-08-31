use super::*;

#[tokio::test]
async fn graphql_media_rename_preview_for_anime_uses_media_file_rows() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let media_root = tempfile::tempdir().expect("media root tempdir");
    configure_default_library_root(&ctx, MediaFacet::Anime, media_root.path()).await;

    let title = create_catalog_title(
        &ctx,
        "Rename Preview Show",
        MediaFacet::Anime,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "91001".to_string(),
        }],
        vec![],
        true,
    )
    .await;

    let collection = ctx
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: scryer_domain::CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: Some("3".to_string()),
            last_episode_number: Some("3".to_string()),
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create season collection");

    let episode = ctx
        .shows
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(collection.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("3".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E03".to_string()),
            title: Some("Arrival".to_string()),
            air_date: None,
            duration_seconds: Some(1440),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: Some("12".to_string()),
            overview: None,
            tvdb_id: Some("9100103".to_string()),
            image_url: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create episode");

    let season_dir = media_root
        .path()
        .join("Rename Preview Show")
        .join("Season 01");
    std::fs::create_dir_all(&season_dir).expect("create season dir");
    set_title_folder_path(&ctx, &title.id, season_dir.parent().expect("title folder")).await;
    let file_path = season_dir.join("[SubsPlease] Rename Preview Show - 03 (1080p).mkv");
    std::fs::write(&file_path, b"anime-preview").expect("write preview file");

    let file_id = ctx
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: file_path.to_string_lossy().to_string(),
            size_bytes: 2048,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert media file");
    ctx.link_primary_file_to_episode(&title.id, &file_id, &episode.id)
        .await
        .expect("link file to episode");

    let body = gql(
        &ctx,
        r#"
        query($input: MediaRenamePreviewInput!) {
          mediaRenamePreview(input: $input) {
            total
            renamable
            noop
            conflicts
            errors
            items {
              collectionId
              currentPath
              proposedPath
              writeAction
              reasonCode
            }
          }
        }
        "#,
        json!({
            "input": {
                "facet": "ANIME",
                "titleId": title.id,
                "dryRun": true
            }
        }),
    )
    .await;
    assert_no_errors(&body);

    let plan = &body["data"]["mediaRenamePreview"];
    assert_eq!(plan["total"].as_i64(), Some(1));
    assert_eq!(plan["renamable"].as_i64(), Some(1));
    assert_eq!(plan["noop"].as_i64(), Some(0));
    assert_eq!(plan["conflicts"].as_i64(), Some(0));
    assert_eq!(plan["errors"].as_i64(), Some(0));

    let item = &plan["items"][0];
    assert_eq!(item["collectionId"], Value::Null);
    assert_eq!(
        item["currentPath"],
        json!(file_path.to_string_lossy().to_string())
    );
    assert_eq!(
        item["proposedPath"],
        json!(
            media_root
                .path()
                .join("Rename Preview Show (2024)")
                .join("Season 1")
                .join("Rename Preview Show - S01E03 (012) - 1080p.mkv")
                .to_string_lossy()
                .to_string()
        )
    );
    assert_eq!(item["writeAction"], "move");
    assert_eq!(item["reasonCode"], "rename_move");
}

#[tokio::test]
async fn graphql_media_rename_preview_for_anime_uses_saved_anime_template() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let media_root = tempfile::tempdir().expect("media root tempdir");
    configure_default_library_root(&ctx, MediaFacet::Anime, media_root.path()).await;

    let title = create_catalog_title(
        &ctx,
        "Template Scope Show",
        MediaFacet::Anime,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "91567".to_string(),
        }],
        vec![],
        true,
    )
    .await;

    let season_collection = ctx
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("1".to_string()),
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create season collection");

    let season_dir = media_root
        .path()
        .join("Template Scope Show")
        .join("Season 01");
    std::fs::create_dir_all(&season_dir).expect("create season dir");
    set_title_folder_path(&ctx, &title.id, season_dir.parent().expect("title folder")).await;
    let file_path = season_dir.join("Template.Scope.Show.S01E01.1080p.WEB-DL.mkv");
    std::fs::write(&file_path, b"anime-template-preview").expect("write preview file");

    let episode = ctx
        .shows
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(season_collection.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("1".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E01".to_string()),
            title: Some("Binary Bloom".to_string()),
            air_date: None,
            duration_seconds: Some(1440),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: Some("7".to_string()),
            overview: None,
            tvdb_id: Some("9156701".to_string()),
            image_url: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create episode");

    let file_id = ctx
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: file_path.to_string_lossy().to_string(),
            size_bytes: 2048,
            quality_label: Some("1080p".to_string()),
            release_group: Some("SkyGroup".to_string()),
            source_type: Some("WEB-DL".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert media file");
    ctx.link_primary_file_to_episode(&title.id, &file_id, &episode.id)
        .await
        .expect("link file to episode");

    let update = gql(
        &ctx,
        r#"
        mutation UpdateMediaSettings($input: UpdateMediaSettingsInput!) {
          updateMediaSettings(input: $input) {
            scope
            renameTemplate
            renameCollisionPolicy
            renameMissingMetadataPolicy
          }
        }
        "#,
        json!({
            "input": {
                "scope": "ANIME",
                "renameTemplate": "{title} - {episode_title} - {source} - {group} - {quality}.{ext}",
                "renameCollisionPolicy": "REPLACE_IF_BETTER",
                "renameMissingMetadataPolicy": "SKIP"
            }
        }),
    )
    .await;
    assert_no_errors(&update);
    assert_eq!(update["data"]["updateMediaSettings"]["scope"], "ANIME");
    assert_eq!(
        update["data"]["updateMediaSettings"]["renameTemplate"],
        "{title} - {episode_title} - {source} - {group} - {quality}.{ext}"
    );

    let body = gql(
        &ctx,
        r#"
        query($input: MediaRenamePreviewInput!) {
          mediaRenamePreview(input: $input) {
            total
            renamable
            items {
              currentPath
              proposedPath
            }
          }
        }
        "#,
        json!({
            "input": {
                "facet": "ANIME",
                "titleId": title.id,
                "dryRun": true
            }
        }),
    )
    .await;
    assert_no_errors(&body);

    let plan = &body["data"]["mediaRenamePreview"];
    assert_eq!(plan["total"].as_i64(), Some(1));
    assert_eq!(plan["renamable"].as_i64(), Some(1));
    assert_eq!(
        plan["items"][0]["currentPath"],
        json!(file_path.to_string_lossy().to_string())
    );
    assert_eq!(
        plan["items"][0]["proposedPath"],
        json!(
            media_root
                .path()
                .join("Template Scope Show (2024)")
                .join("Season 1")
                .join("Template Scope Show - Binary Bloom - WEB-DL - SkyGroup - 1080p.mkv")
                .to_string_lossy()
                .to_string()
        )
    );
}

#[tokio::test]
async fn graphql_media_rename_preview_for_anime_series_movie_uses_season_zero_numbering() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let media_root = tempfile::tempdir().expect("media root tempdir");
    configure_default_library_root(&ctx, MediaFacet::Anime, media_root.path()).await;

    let title = create_catalog_title(
        &ctx,
        "Festival Saga",
        MediaFacet::Anime,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "92001".to_string(),
        }],
        vec![],
        true,
    )
    .await;

    let season_zero_dir = media_root.path().join("Festival Saga").join("Season 00");
    std::fs::create_dir_all(&season_zero_dir).expect("create season zero dir");
    set_title_folder_path(
        &ctx,
        &title.id,
        season_zero_dir.parent().expect("title folder"),
    )
    .await;
    let file_path = season_zero_dir.join("Festival.Saga.Movie.Special.1080p.mkv");
    std::fs::write(&file_path, b"anime-series-movie").expect("write series movie file");

    let specials = ctx
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: scryer_domain::CollectionType::Specials,
            collection_index: "0".to_string(),
            label: Some("Specials".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: None,
            last_episode_number: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create specials collection");
    let special_episode = create_series_movie_special_episode(
        &ctx,
        &title,
        &specials,
        "3",
        "Festival Film",
        "9200103",
    )
    .await;
    let series_movie_link = create_test_series_movie_link(
        &ctx,
        &title,
        "Festival Film",
        "9200103",
        Some(special_episode.id.clone()),
        None,
    )
    .await;

    let file_id = ctx
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: file_path.to_string_lossy().to_string(),
            size_bytes: 4096,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert series movie file");
    ctx.link_primary_file_to_episode(&title.id, &file_id, &special_episode.id)
        .await
        .expect("link series movie special episode");
    ctx.media_files
        .link_file_to_series_movie(&file_id, &series_movie_link.id)
        .await
        .expect("link series movie file");

    let body = gql(
        &ctx,
        r#"
        query($input: MediaRenamePreviewInput!) {
          mediaRenamePreview(input: $input) {
            total
            renamable
            items {
              collectionId
              seriesMovieLinkIds
              currentPath
              proposedPath
              writeAction
            }
          }
        }
        "#,
        json!({
            "input": {
                "facet": "ANIME",
                "titleId": title.id,
                "dryRun": true
            }
        }),
    )
    .await;
    assert_no_errors(&body);

    let plan = &body["data"]["mediaRenamePreview"];
    assert_eq!(plan["total"].as_i64(), Some(1));
    assert_eq!(plan["renamable"].as_i64(), Some(1));

    let item = &plan["items"][0];
    assert_eq!(item["collectionId"], serde_json::Value::Null);
    assert_eq!(item["seriesMovieLinkIds"], json!([series_movie_link.id]));
    assert_eq!(
        item["currentPath"],
        json!(file_path.to_string_lossy().to_string())
    );
    assert_eq!(
        item["proposedPath"],
        json!(
            media_root
                .path()
                .join("Festival Saga (2024)")
                .join("Specials")
                .join("Festival Saga - S00E03 (003) - 1080p.mkv")
                .to_string_lossy()
                .to_string()
        )
    );
    assert_eq!(item["writeAction"], "move");
}

#[tokio::test]
async fn apply_media_rename_for_anime_updates_media_files_and_series_movie_specials() {
    let mut ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    ctx.app = ctx.app.with_test_overrides(|builder| {
        builder.with_library_renamer(std::sync::Arc::new(FileSystemLibraryRenamer::new()))
    });
    let media_root = tempfile::tempdir().expect("media root tempdir");
    configure_default_library_root(&ctx, MediaFacet::Anime, media_root.path()).await;

    let title = create_catalog_title(
        &ctx,
        "Anime Apply Show",
        MediaFacet::Anime,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "93001".to_string(),
        }],
        vec![],
        true,
    )
    .await;

    let season_collection = ctx
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: scryer_domain::CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("1".to_string()),
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create season collection");

    let episode = ctx
        .shows
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(season_collection.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("1".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E01".to_string()),
            title: Some("Pilot".to_string()),
            air_date: None,
            duration_seconds: Some(1440),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: Some("1".to_string()),
            overview: None,
            tvdb_id: Some("9300101".to_string()),
            image_url: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create episode");

    let season_dir = media_root.path().join("Anime Apply Show").join("Season 01");
    std::fs::create_dir_all(&season_dir).expect("create season dir");
    set_title_folder_path(&ctx, &title.id, season_dir.parent().expect("title folder")).await;
    let regular_file_path = season_dir.join("Anime.Apply.Show.Episode.One.1080p.mkv");
    std::fs::write(&regular_file_path, b"anime-apply-episode").expect("write regular file");

    let regular_file_id = ctx
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: regular_file_path.to_string_lossy().to_string(),
            size_bytes: 1024,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert regular file");
    ctx.link_primary_file_to_episode(&title.id, &regular_file_id, &episode.id)
        .await
        .expect("link regular file");

    let season_zero_dir = media_root.path().join("Anime Apply Show").join("Season 00");
    std::fs::create_dir_all(&season_zero_dir).expect("create season zero dir");
    let series_movie_file_path = season_zero_dir.join("Anime.Apply.Show.Movie.Special.1080p.mkv");
    std::fs::write(&series_movie_file_path, b"anime-apply-series-movie")
        .expect("write series movie file");

    let specials_collection = ctx
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: scryer_domain::CollectionType::Specials,
            collection_index: "0".to_string(),
            label: Some("Specials".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: None,
            last_episode_number: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create specials collection");
    let series_movie_episode = create_series_movie_special_episode(
        &ctx,
        &title,
        &specials_collection,
        "3",
        "Pilot Movie",
        "9300103",
    )
    .await;
    let series_movie_link = create_test_series_movie_link(
        &ctx,
        &title,
        "Pilot Movie",
        "9300103",
        Some(series_movie_episode.id.clone()),
        None,
    )
    .await;

    let series_movie_file_id = ctx
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: series_movie_file_path.to_string_lossy().to_string(),
            size_bytes: 2048,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert series movie media file");
    ctx.link_primary_file_to_episode(&title.id, &series_movie_file_id, &series_movie_episode.id)
        .await
        .expect("link series movie special");
    ctx.media_files
        .link_file_to_series_movie(&series_movie_file_id, &series_movie_link.id)
        .await
        .expect("link series movie file");

    let actor = ctx
        .app
        .find_or_create_default_user()
        .await
        .expect("default user");
    let preview = ctx
        .app
        .preview_rename_for_title(&actor, &title.id, MediaFacet::Anime)
        .await
        .expect("preview rename plan");
    assert_eq!(preview.renamable, 2);

    let result = ctx
        .app
        .apply_rename_for_title(&actor, &title.id, MediaFacet::Anime, &preview.fingerprint)
        .await
        .expect("apply rename");
    assert_eq!(result.applied, 2);
    assert_eq!(result.failed, 0);

    let expected_regular_path = media_root
        .path()
        .join("Anime Apply Show (2024)")
        .join("Season 1")
        .join("Anime Apply Show - S01E01 (001) - 1080p.mkv")
        .to_string_lossy()
        .to_string();
    let expected_series_movie_path = media_root
        .path()
        .join("Anime Apply Show (2024)")
        .join("Specials")
        .join("Anime Apply Show - S00E03 (003) - 1080p.mkv")
        .to_string_lossy()
        .to_string();

    let updated_regular_file = ctx
        .media_files
        .get_media_file_by_id(&regular_file_id)
        .await
        .expect("load updated regular media file")
        .expect("regular media file");
    let updated_series_movie_file = ctx
        .media_files
        .get_media_file_by_id(&series_movie_file_id)
        .await
        .expect("load updated series movie media file")
        .expect("series movie media file");
    let refreshed_season_collection = ctx
        .shows
        .get_collection_by_id(&season_collection.id)
        .await
        .expect("load season collection")
        .expect("season collection");
    let refreshed_specials_collection = ctx
        .shows
        .get_collection_by_id(&specials_collection.id)
        .await
        .expect("load specials collection")
        .expect("specials collection");

    assert_eq!(updated_regular_file.file_path, expected_regular_path);
    assert_eq!(
        updated_series_movie_file.file_path,
        expected_series_movie_path
    );
    assert_eq!(refreshed_season_collection.ordered_path, None);
    assert_eq!(refreshed_specials_collection.ordered_path, None);
    assert!(std::path::Path::new(&expected_regular_path).exists());
    assert!(std::path::Path::new(&expected_series_movie_path).exists());
    assert!(!regular_file_path.exists());
    assert!(!series_movie_file_path.exists());
}

#[tokio::test]
async fn graphql_media_rename_preview_for_movies_stays_collection_based() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let media_root = tempfile::tempdir().expect("media root tempdir");
    configure_default_library_root(&ctx, MediaFacet::Movie, media_root.path()).await;

    let title = create_catalog_title(
        &ctx,
        "Regression Movie (2024)",
        MediaFacet::Movie,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "94001".to_string(),
        }],
        vec![],
        true,
    )
    .await;

    let movie_dir = media_root.path().join("Regression Movie (2024)");
    std::fs::create_dir_all(&movie_dir).expect("create movie dir");
    set_title_folder_path(&ctx, &title.id, &movie_dir).await;
    let file_path = movie_dir.join("Regression.Movie.2024.1080p.WEB-DL.mkv");
    std::fs::write(&file_path, b"movie-rename-preview").expect("write movie file");

    let collection = ctx
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: scryer_domain::CollectionType::Movie,
            collection_index: "1".to_string(),
            label: Some("1080p".to_string()),
            ordered_path: Some(file_path.to_string_lossy().to_string()),
            narrative_order: None,
            first_episode_number: None,
            last_episode_number: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create movie collection");
    let _file_id = ctx
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: file_path.to_string_lossy().to_string(),
            size_bytes: 4096,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert movie media file");

    let body = gql(
        &ctx,
        r#"
        query($input: MediaRenamePreviewInput!) {
          mediaRenamePreview(input: $input) {
            total
            renamable
            items {
              collectionId
              currentPath
              proposedPath
              writeAction
            }
          }
        }
        "#,
        json!({
            "input": {
                "facet": "MOVIE",
                "titleId": title.id,
                "dryRun": true
            }
        }),
    )
    .await;
    assert_no_errors(&body);

    let plan = &body["data"]["mediaRenamePreview"];
    assert_eq!(plan["total"].as_i64(), Some(1));
    assert_eq!(plan["renamable"].as_i64(), Some(1));

    let item = &plan["items"][0];
    assert_eq!(item["collectionId"], json!(collection.id));
    assert_eq!(
        item["currentPath"],
        json!(file_path.to_string_lossy().to_string())
    );
    assert_eq!(
        item["proposedPath"],
        json!(
            movie_dir
                .join("Regression Movie (2024) - 1080p.mkv")
                .to_string_lossy()
                .to_string()
        )
    );
    assert_eq!(item["writeAction"], "move");
}

#[tokio::test]
async fn apply_media_rename_for_movies_updates_collection_and_media_file_paths() {
    let mut ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    ctx.app = ctx.app.with_test_overrides(|builder| {
        builder.with_library_renamer(std::sync::Arc::new(FileSystemLibraryRenamer::new()))
    });
    let media_root = tempfile::tempdir().expect("media root tempdir");
    configure_default_library_root(&ctx, MediaFacet::Movie, media_root.path()).await;

    let title = create_catalog_title(
        &ctx,
        "Movie Apply Sync (2024)",
        MediaFacet::Movie,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "94002".to_string(),
        }],
        vec![],
        true,
    )
    .await;

    let movie_dir = media_root.path().join("Movie Apply Sync (2024)");
    std::fs::create_dir_all(&movie_dir).expect("create movie dir");
    set_title_folder_path(&ctx, &title.id, &movie_dir).await;
    let source_path = movie_dir.join("Movie.Apply.Sync.2024.1080p.WEB-DL.mkv");
    std::fs::write(&source_path, b"movie-apply-sync").expect("write movie file");

    let collection = ctx
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: scryer_domain::CollectionType::Movie,
            collection_index: "1".to_string(),
            label: Some("1080p".to_string()),
            ordered_path: Some(source_path.to_string_lossy().to_string()),
            narrative_order: None,
            first_episode_number: None,
            last_episode_number: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create movie collection");
    let file_id = ctx
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: source_path.to_string_lossy().to_string(),
            size_bytes: 8192,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert movie media file");

    let actor = ctx
        .app
        .find_or_create_default_user()
        .await
        .expect("default user");
    let preview = ctx
        .app
        .preview_rename_for_title(&actor, &title.id, MediaFacet::Movie)
        .await
        .expect("preview rename plan");
    assert_eq!(preview.renamable, 1);
    assert_eq!(
        preview.items[0].media_file_id.as_deref(),
        Some(file_id.as_str())
    );

    let result = ctx
        .app
        .apply_rename_for_title(&actor, &title.id, MediaFacet::Movie, &preview.fingerprint)
        .await
        .expect("apply rename");
    assert_eq!(result.applied, 1);
    assert_eq!(result.failed, 0);

    let expected_path = movie_dir
        .join("Movie Apply Sync (2024) - 1080p.mkv")
        .to_string_lossy()
        .to_string();
    let updated_collection = ctx
        .shows
        .get_collection_by_id(&collection.id)
        .await
        .expect("load movie collection")
        .expect("movie collection");
    let updated_file = ctx
        .media_files
        .get_media_file_by_id(&file_id)
        .await
        .expect("load movie media file")
        .expect("movie media file");

    assert_eq!(
        updated_collection.ordered_path.as_deref(),
        Some(expected_path.as_str())
    );
    assert_eq!(updated_file.file_path, expected_path);
}

#[tokio::test]
async fn apply_media_rename_for_movies_uses_folder_template_and_updates_title_folder_path() {
    let mut ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    set_folder_template(&ctx, "MOVIE", "{title} ({year})").await;
    ctx.app = ctx.app.with_test_overrides(|builder| {
        builder.with_library_renamer(std::sync::Arc::new(FileSystemLibraryRenamer::new()))
    });
    let media_root = tempfile::tempdir().expect("media root tempdir");
    configure_default_library_root(&ctx, MediaFacet::Movie, media_root.path()).await;

    let title = create_catalog_title(
        &ctx,
        "Movie Apply Folder",
        MediaFacet::Movie,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "94003".to_string(),
        }],
        vec![],
        true,
    )
    .await;

    let old_movie_dir = media_root.path().join("Movie Apply Folder");
    std::fs::create_dir_all(&old_movie_dir).expect("create old movie dir");
    set_title_folder_path(&ctx, &title.id, &old_movie_dir).await;
    let source_path = old_movie_dir.join("Movie.Apply.Folder.2024.1080p.WEB-DL.mkv");
    std::fs::write(&source_path, b"movie-apply-folder").expect("write movie file");

    let collection = ctx
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: scryer_domain::CollectionType::Movie,
            collection_index: "1".to_string(),
            label: Some("1080p".to_string()),
            ordered_path: Some(source_path.to_string_lossy().to_string()),
            narrative_order: None,
            first_episode_number: None,
            last_episode_number: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create movie collection");
    let file_id = ctx
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: source_path.to_string_lossy().to_string(),
            size_bytes: 8192,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert movie media file");

    let actor = ctx
        .app
        .find_or_create_default_user()
        .await
        .expect("default user");
    let preview = ctx
        .app
        .preview_rename_for_title(&actor, &title.id, MediaFacet::Movie)
        .await
        .expect("preview rename plan");
    assert_eq!(preview.renamable, 1);

    let result = ctx
        .app
        .apply_rename_for_title(&actor, &title.id, MediaFacet::Movie, &preview.fingerprint)
        .await
        .expect("apply rename");
    assert_eq!(result.applied, 1);
    assert_eq!(result.failed, 0);

    let new_movie_dir = media_root.path().join("Movie Apply Folder (2024)");
    let expected_path = new_movie_dir
        .join("Movie Apply Folder (2024) - 1080p.mkv")
        .to_string_lossy()
        .to_string();
    let updated_title = ctx
        .titles
        .get_by_id(&title.id)
        .await
        .expect("load title")
        .expect("title");
    let updated_collection = ctx
        .shows
        .get_collection_by_id(&collection.id)
        .await
        .expect("load movie collection")
        .expect("movie collection");
    let updated_file = ctx
        .media_files
        .get_media_file_by_id(&file_id)
        .await
        .expect("load movie media file")
        .expect("movie media file");

    assert_eq!(
        updated_title.folder_path.as_deref(),
        Some(new_movie_dir.to_string_lossy().as_ref())
    );
    assert_eq!(
        updated_collection.ordered_path.as_deref(),
        Some(expected_path.as_str())
    );
    assert_eq!(updated_file.file_path, expected_path);
}

#[tokio::test]
async fn graphql_media_rename_preview_for_anime_tracked_destination_returns_error_not_replace() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    set_rename_collision_policy(&ctx, "ANIME", "REPLACE_IF_BETTER").await;
    let media_root = tempfile::tempdir().expect("media root tempdir");
    configure_default_library_root(&ctx, MediaFacet::Anime, media_root.path()).await;

    let title = create_catalog_title(
        &ctx,
        "Tracked Collision Anime",
        MediaFacet::Anime,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "95001".to_string(),
        }],
        vec![],
        true,
    )
    .await;

    let collection = ctx
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: scryer_domain::CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: Some("3".to_string()),
            last_episode_number: Some("3".to_string()),
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create season collection");

    let episode = ctx
        .shows
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(collection.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("3".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E03".to_string()),
            title: Some("Arrival".to_string()),
            air_date: None,
            duration_seconds: Some(1440),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: Some("12".to_string()),
            overview: None,
            tvdb_id: Some("9500103".to_string()),
            image_url: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create episode");

    let season_dir = media_root
        .path()
        .join("Tracked Collision Anime")
        .join("Season 01");
    std::fs::create_dir_all(&season_dir).expect("create season dir");
    set_title_folder_path(&ctx, &title.id, season_dir.parent().expect("title folder")).await;
    let source_path = season_dir.join("[SubsPlease] Tracked Collision Anime - 03 (1080p).mkv");
    std::fs::write(&source_path, b"tracked-collision-source").expect("write source file");
    let destination_path = media_root
        .path()
        .join("Tracked Collision Anime (2024)")
        .join("Season 1")
        .join("Tracked Collision Anime - S01E03 (012) - 1080p.mkv");

    let file_id = ctx
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: source_path.to_string_lossy().to_string(),
            size_bytes: 2048,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert source media file");
    ctx.link_primary_file_to_episode(&title.id, &file_id, &episode.id)
        .await
        .expect("link file to episode");

    let owning_title = create_catalog_title(
        &ctx,
        "Tracked Collision Owner",
        MediaFacet::Anime,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "95002".to_string(),
        }],
        vec![],
        true,
    )
    .await;
    ctx.media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: owning_title.id,
            file_path: destination_path.to_string_lossy().to_string(),
            size_bytes: 4096,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert tracked destination");

    let body = gql(
        &ctx,
        r#"
        query($input: MediaRenamePreviewInput!) {
          mediaRenamePreview(input: $input) {
            total
            renamable
            conflicts
            errors
            items {
              writeAction
              reasonCode
            }
          }
        }
        "#,
        json!({
            "input": {
                "facet": "ANIME",
                "titleId": title.id,
                "dryRun": true
            }
        }),
    )
    .await;
    assert_no_errors(&body);

    let plan = &body["data"]["mediaRenamePreview"];
    assert_eq!(plan["total"].as_i64(), Some(1));
    assert_eq!(plan["renamable"].as_i64(), Some(0));
    assert_eq!(plan["conflicts"].as_i64(), Some(1));
    assert_eq!(plan["errors"].as_i64(), Some(1));
    assert_eq!(plan["items"][0]["writeAction"], "error");
    assert_eq!(plan["items"][0]["reasonCode"], "collision_existing_tracked");
    assert!(
        plan["items"]
            .as_array()
            .expect("items array")
            .iter()
            .all(|item| item["writeAction"] != "replace")
    );
}

#[tokio::test]
async fn graphql_media_rename_preview_for_movies_tracked_destination_returns_error_not_replace() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    set_rename_collision_policy(&ctx, "MOVIE", "REPLACE_IF_BETTER").await;
    let media_root = tempfile::tempdir().expect("media root tempdir");
    configure_default_library_root(&ctx, MediaFacet::Movie, media_root.path()).await;

    let title = create_catalog_title(
        &ctx,
        "Tracked Collision Movie (2024)",
        MediaFacet::Movie,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "96001".to_string(),
        }],
        vec![],
        true,
    )
    .await;

    let movie_dir = media_root.path().join("Tracked Collision Movie (2024)");
    std::fs::create_dir_all(&movie_dir).expect("create movie dir");
    set_title_folder_path(&ctx, &title.id, &movie_dir).await;
    let source_path = movie_dir.join("Tracked.Collision.Movie.2024.1080p.WEB-DL.mkv");
    std::fs::write(&source_path, b"tracked-movie-source").expect("write movie source");
    let destination_path = movie_dir.join("Tracked Collision Movie (2024) - 1080p.mkv");

    ctx.shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: scryer_domain::CollectionType::Movie,
            collection_index: "1".to_string(),
            label: Some("1080p".to_string()),
            ordered_path: Some(source_path.to_string_lossy().to_string()),
            narrative_order: None,
            first_episode_number: None,
            last_episode_number: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create movie collection");

    let owning_title = create_catalog_title(
        &ctx,
        "Tracked Collision Owner Movie (2024)",
        MediaFacet::Movie,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "96002".to_string(),
        }],
        vec![],
        true,
    )
    .await;
    ctx.shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: owning_title.id,
            collection_type: scryer_domain::CollectionType::Movie,
            collection_index: "1".to_string(),
            label: Some("1080p".to_string()),
            ordered_path: Some(destination_path.to_string_lossy().to_string()),
            narrative_order: None,
            first_episode_number: None,
            last_episode_number: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create tracked destination collection");

    let body = gql(
        &ctx,
        r#"
        query($input: MediaRenamePreviewInput!) {
          mediaRenamePreview(input: $input) {
            total
            renamable
            conflicts
            errors
            items {
              writeAction
              reasonCode
            }
          }
        }
        "#,
        json!({
            "input": {
                "facet": "MOVIE",
                "titleId": title.id,
                "dryRun": true
            }
        }),
    )
    .await;
    assert_no_errors(&body);

    let plan = &body["data"]["mediaRenamePreview"];
    assert_eq!(plan["total"].as_i64(), Some(1));
    assert_eq!(plan["renamable"].as_i64(), Some(0));
    assert_eq!(plan["conflicts"].as_i64(), Some(1));
    assert_eq!(plan["errors"].as_i64(), Some(1));
    assert_eq!(plan["items"][0]["writeAction"], "error");
    assert_eq!(plan["items"][0]["reasonCode"], "collision_existing_tracked");
    assert!(
        plan["items"]
            .as_array()
            .expect("items array")
            .iter()
            .all(|item| item["writeAction"] != "replace")
    );
}

#[tokio::test]
async fn graphql_media_rename_preview_for_anime_multi_episode_file_uses_episode_range() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let media_root = tempfile::tempdir().expect("media root tempdir");
    configure_default_library_root(&ctx, MediaFacet::Anime, media_root.path()).await;

    let title = create_catalog_title(
        &ctx,
        "Range Preview Show",
        MediaFacet::Anime,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "97002".to_string(),
        }],
        vec![],
        true,
    )
    .await;

    let collection = ctx
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: scryer_domain::CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("2".to_string()),
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create season collection");
    let episode_one =
        create_series_scan_episode(&ctx, &title, &collection, "1", "1", "S01E01").await;
    let episode_two =
        create_series_scan_episode(&ctx, &title, &collection, "1", "2", "S01E02").await;

    let season_dir = media_root
        .path()
        .join("Range Preview Show")
        .join("Season 01");
    std::fs::create_dir_all(&season_dir).expect("create season dir");
    set_title_folder_path(&ctx, &title.id, season_dir.parent().expect("title folder")).await;
    let file_path = season_dir.join("Range.Preview.Show.S01E01-E02.1080p.mkv");
    std::fs::write(&file_path, b"anime-range-preview").expect("write preview file");

    let file_id = ctx
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: file_path.to_string_lossy().to_string(),
            size_bytes: 4096,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert media file");
    ctx.link_primary_file_to_episode(&title.id, &file_id, &episode_one.id)
        .await
        .expect("link first episode");
    ctx.link_primary_file_to_episode(&title.id, &file_id, &episode_two.id)
        .await
        .expect("link second episode");

    let body = gql(
        &ctx,
        r#"
        query($input: MediaRenamePreviewInput!) {
          mediaRenamePreview(input: $input) {
            total
            renamable
            items {
              proposedPath
              writeAction
            }
          }
        }
        "#,
        json!({
            "input": {
                "facet": "ANIME",
                "titleId": title.id,
                "dryRun": true
            }
        }),
    )
    .await;
    assert_no_errors(&body);

    let plan = &body["data"]["mediaRenamePreview"];
    assert_eq!(plan["total"].as_i64(), Some(1));
    assert_eq!(plan["renamable"].as_i64(), Some(1));
    assert_eq!(plan["items"][0]["writeAction"], "move");
    assert_eq!(
        plan["items"][0]["proposedPath"],
        json!(
            media_root
                .path()
                .join("Range Preview Show (2024)")
                .join("Season 1")
                .join("Range Preview Show - S01E01-02 (01-02) - 1080p.mkv")
                .to_string_lossy()
                .to_string()
        )
    );
}

#[tokio::test]
async fn apply_media_rename_refuses_an_untracked_existing_target() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    set_rename_collision_policy(&ctx, "MOVIE", "REPLACE_IF_BETTER").await;
    let media_root = tempfile::tempdir().expect("media root tempdir");
    configure_default_library_root(&ctx, MediaFacet::Movie, media_root.path()).await;

    let title = create_catalog_title(
        &ctx,
        "Untracked Collision Movie (2024)",
        MediaFacet::Movie,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "97001".to_string(),
        }],
        vec![],
        true,
    )
    .await;

    let movie_dir = media_root.path().join("Untracked Collision Movie (2024)");
    std::fs::create_dir_all(&movie_dir).expect("create movie dir");
    set_title_folder_path(&ctx, &title.id, &movie_dir).await;
    let source_path = movie_dir.join("Untracked.Collision.Movie.2024.1080p.WEB-DL.mkv");
    std::fs::write(&source_path, b"untracked-movie-source").expect("write movie source");
    let destination_path = movie_dir.join("Untracked Collision Movie (2024) - 1080p.mkv");
    std::fs::write(&destination_path, b"untracked-movie-destination")
        .expect("write untracked destination");

    ctx.shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: scryer_domain::CollectionType::Movie,
            collection_index: "1".to_string(),
            label: Some("1080p".to_string()),
            ordered_path: Some(source_path.to_string_lossy().to_string()),
            narrative_order: None,
            first_episode_number: None,
            last_episode_number: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create movie collection");

    // Planning is database-only, so an untracked file squatting on the target
    // is not visible until apply, which is where the filesystem is read.
    let body = gql(
        &ctx,
        r#"
        query($input: MediaRenamePreviewInput!) {
          mediaRenamePreview(input: $input) {
            fingerprint
            total
            renamable
            items {
              writeAction
            }
          }
        }
        "#,
        json!({
            "input": {
                "facet": "MOVIE",
                "titleId": title.id,
                "dryRun": true
            }
        }),
    )
    .await;
    assert_no_errors(&body);

    let plan = &body["data"]["mediaRenamePreview"];
    assert_eq!(plan["total"].as_i64(), Some(1));
    // Never `replace`: overwriting a file the catalog does not know about is
    // exactly what this guards, whichever collision policy is configured.
    assert!(
        plan["items"]
            .as_array()
            .expect("items array")
            .iter()
            .all(|item| item["writeAction"] != "replace")
    );
    let fingerprint = plan["fingerprint"]
        .as_str()
        .expect("fingerprint")
        .to_string();

    // Apply reads the filesystem and must refuse rather than clobber it.
    let apply = gql(
        &ctx,
        r#"
        mutation($input: MediaRenameApplyInput!) {
          applyMediaRename(input: $input) {
            applied
            failed
          }
        }
        "#,
        json!({
            "input": {
                "facet": "MOVIE",
                "titleId": title.id,
                "fingerprint": fingerprint
            }
        }),
    )
    .await;
    let refused = apply["errors"].is_array()
        || apply["data"]["applyMediaRename"]["applied"].as_i64() == Some(0);
    assert!(refused, "apply should refuse an occupied target: {apply}");

    assert_eq!(
        std::fs::read(&destination_path).expect("destination still present"),
        b"untracked-movie-destination",
        "the untracked destination must not be overwritten"
    );
    assert!(source_path.exists(), "the source must stay put");
}

#[tokio::test]
async fn apply_media_rename_for_anime_rolls_back_when_media_file_update_fails() {
    let mut ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    ctx.app = ctx.app.with_test_overrides(|builder| {
        builder.with_library_renamer(std::sync::Arc::new(FileSystemLibraryRenamer::new()))
    });
    let media_root = tempfile::tempdir().expect("media root tempdir");
    configure_default_library_root(&ctx, MediaFacet::Anime, media_root.path()).await;

    let title = create_catalog_title(
        &ctx,
        "Anime Media Rollback",
        MediaFacet::Anime,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "98001".to_string(),
        }],
        vec![],
        true,
    )
    .await;

    let collection = ctx
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: scryer_domain::CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("1".to_string()),
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create season collection");

    let episode = ctx
        .shows
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(collection.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("1".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E01".to_string()),
            title: Some("Pilot".to_string()),
            air_date: None,
            duration_seconds: Some(1440),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: Some("1".to_string()),
            overview: None,
            tvdb_id: Some("9800101".to_string()),
            image_url: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create episode");

    let season_dir = media_root
        .path()
        .join("Anime Media Rollback")
        .join("Season 01");
    std::fs::create_dir_all(&season_dir).expect("create season dir");
    set_title_folder_path(&ctx, &title.id, season_dir.parent().expect("title folder")).await;
    let source_path = season_dir.join("Anime.Media.Rollback.Episode.One.1080p.mkv");
    std::fs::write(&source_path, b"anime-media-rollback").expect("write source file");

    let file_id = ctx
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: source_path.to_string_lossy().to_string(),
            size_bytes: 1024,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert media file");
    ctx.link_primary_file_to_episode(&title.id, &file_id, &episode.id)
        .await
        .expect("link file to episode");

    ctx.app = ctx.app.with_test_overrides(|builder| {
        builder.with_media_files(std::sync::Arc::new(FailingMediaFileRepo {
            inner: ctx.media_files.clone(),
            fail_file_id: file_id.clone(),
        }))
    });

    let actor = ctx
        .app
        .find_or_create_default_user()
        .await
        .expect("default user");
    let preview = ctx
        .app
        .preview_rename_for_title(&actor, &title.id, MediaFacet::Anime)
        .await
        .expect("preview rename plan");
    assert_eq!(preview.renamable, 1);
    assert!(
        preview
            .items
            .iter()
            .all(|item| item.write_action != scryer_application::RenameWriteAction::Replace)
    );

    let result = ctx
        .app
        .apply_rename_for_title(&actor, &title.id, MediaFacet::Anime, &preview.fingerprint)
        .await
        .expect("apply rename");
    assert_eq!(result.applied, 0);
    assert_eq!(result.failed, 1);
    assert!(
        result
            .items
            .iter()
            .all(|item| item.write_action != scryer_application::RenameWriteAction::Replace)
    );

    let expected_path = media_root
        .path()
        .join("Anime Media Rollback (2024)")
        .join("Season 1")
        .join("Anime Media Rollback - S01E01 (001) - 1080p.mkv")
        .to_string_lossy()
        .to_string();
    let item = &result.items[0];
    assert_eq!(item.status.as_str(), "failed");
    assert_eq!(item.reason_code, "db_update_failed");
    assert_eq!(
        item.final_path.as_deref(),
        Some(source_path.to_string_lossy().as_ref())
    );
    assert!(
        item.error_message
            .as_deref()
            .is_some_and(|message| message.contains("rollback succeeded"))
    );

    let stored = ctx
        .media_files
        .get_media_file_by_id(&file_id)
        .await
        .expect("load media file")
        .expect("media file present");
    assert_eq!(stored.file_path, source_path.to_string_lossy().to_string());
    assert!(source_path.exists());
    assert!(!std::path::Path::new(&expected_path).exists());
}

#[tokio::test]
async fn graphql_media_rename_preview_scopes_returned_items_without_changing_counts() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let media_root = tempfile::tempdir().expect("media root tempdir");
    configure_default_library_root(&ctx, MediaFacet::Anime, media_root.path()).await;

    let title = create_catalog_title(
        &ctx,
        "Rename Preview Show",
        MediaFacet::Anime,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "91001".to_string(),
        }],
        vec![],
        true,
    )
    .await;

    let collection = ctx
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: scryer_domain::CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: Some("3".to_string()),
            last_episode_number: Some("4".to_string()),
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create season collection");

    let make_episode = |episode_number: &str, absolute_number: &str| {
        let ctx = &ctx;
        let title_id = title.id.clone();
        let collection_id = collection.id.clone();
        let episode_number = episode_number.to_string();
        let absolute_number = absolute_number.to_string();
        async move {
            ctx.shows
                .create_episode(Episode {
                    id: Id::new().0,
                    title_id,
                    collection_id: Some(collection_id),
                    episode_type: scryer_domain::EpisodeType::Standard,
                    episode_number: Some(episode_number.clone()),
                    season_number: Some("1".to_string()),
                    episode_label: Some(format!("S01E0{episode_number}")),
                    title: Some("Arrival".to_string()),
                    air_date: None,
                    duration_seconds: Some(1440),
                    has_multi_audio: false,
                    has_subtitle: false,
                    is_filler: false,
                    is_recap: false,
                    absolute_number: Some(absolute_number),
                    overview: None,
                    tvdb_id: None,
                    image_url: None,
                    monitored: true,
                    created_at: chrono::Utc::now(),
                })
                .await
                .expect("create episode")
        }
    };

    let misnamed_episode = make_episode("3", "12").await;
    let named_episode = make_episode("4", "13").await;

    let season_dir = media_root
        .path()
        .join("Rename Preview Show")
        .join("Season 01");
    std::fs::create_dir_all(&season_dir).expect("create season dir");
    set_title_folder_path(&ctx, &title.id, season_dir.parent().expect("title folder")).await;

    let misnamed_path = season_dir.join("[SubsPlease] Rename Preview Show - 03 (1080p).mkv");
    std::fs::write(&misnamed_path, b"anime-preview").expect("write misnamed file");

    // Already sitting at the path the template renders, so it plans as a noop.
    let named_dir = media_root
        .path()
        .join("Rename Preview Show (2024)")
        .join("Season 1");
    std::fs::create_dir_all(&named_dir).expect("create renamed season dir");
    let named_path = named_dir.join("Rename Preview Show - S01E04 (013) - 1080p.mkv");
    std::fs::write(&named_path, b"anime-preview-named").expect("write named file");

    for (path, episode) in [
        (&misnamed_path, &misnamed_episode),
        (&named_path, &named_episode),
    ] {
        let file_id = ctx
            .media_files
            .insert_media_file(&InsertMediaFileInput {
                title_id: title.id.clone(),
                file_path: path.to_string_lossy().to_string(),
                size_bytes: 2048,
                quality_label: Some("1080p".to_string()),
                ..Default::default()
            })
            .await
            .expect("insert media file");
        ctx.link_primary_file_to_episode(&title.id, &file_id, &episode.id)
            .await
            .expect("link file to episode");
    }

    let preview = |scope: Value| {
        let ctx = &ctx;
        let title_id = title.id.clone();
        async move {
            let mut input = json!({
                "facet": "ANIME",
                "titleId": title_id,
                "dryRun": true
            });
            if let (Some(input), Some(scope)) = (input.as_object_mut(), scope.as_object()) {
                for (key, value) in scope {
                    input.insert(key.clone(), value.clone());
                }
            }
            let body = gql(
                ctx,
                r#"
                query($input: MediaRenamePreviewInput!) {
                  mediaRenamePreview(input: $input) {
                    fingerprint
                    total
                    renamable
                    noop
                    items {
                      currentPath
                      writeAction
                    }
                  }
                }
                "#,
                json!({ "input": input }),
            )
            .await;
            assert_no_errors(&body);
            body["data"]["mediaRenamePreview"].clone()
        }
    };

    let full = preview(json!({})).await;
    assert_eq!(full["total"].as_i64(), Some(2));
    assert_eq!(full["renamable"].as_i64(), Some(1));
    assert_eq!(full["noop"].as_i64(), Some(1));
    assert_eq!(full["items"].as_array().map(Vec::len), Some(2));
    let fingerprint = full["fingerprint"]
        .as_str()
        .expect("fingerprint")
        .to_string();

    let renamable_only = preview(json!({ "renamableOnly": true })).await;
    assert_eq!(renamable_only["fingerprint"].as_str(), Some(&*fingerprint));
    assert_eq!(renamable_only["total"].as_i64(), Some(2));
    assert_eq!(renamable_only["renamable"].as_i64(), Some(1));
    assert_eq!(renamable_only["noop"].as_i64(), Some(1));
    let items = renamable_only["items"].as_array().expect("items");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["writeAction"], "move");
    assert_eq!(
        items[0]["currentPath"],
        json!(misnamed_path.to_string_lossy().to_string())
    );

    let capped = preview(json!({ "renamableOnly": true, "maxItems": 0 })).await;
    assert_eq!(capped["fingerprint"].as_str(), Some(&*fingerprint));
    assert_eq!(capped["total"].as_i64(), Some(2));
    assert_eq!(capped["renamable"].as_i64(), Some(1));
    assert_eq!(capped["items"].as_array().map(Vec::len), Some(0));
}

#[tokio::test]
async fn graphql_media_rename_preview_does_not_refresh_stale_title_metadata_language() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let media_root = tempfile::tempdir().expect("media root tempdir");
    configure_default_library_root(&ctx, MediaFacet::Movie, media_root.path()).await;

    let localized_movie_response = json!({
        "data": {
            "movie": {
                "movie": {
                    "tvdb_id": 94123,
                    "name": "現地化された映画",
                    "slug": "localized-rename-movie",
                    "year": 2024,
                    "status": "Released",
                    "overview": "",
                    "poster_url": "",
                    "language": "jpn",
                    "original_language": "jpn",
                    "runtime_minutes": 120,
                    "sort_title": "現地化された映画",
                    "imdb_id": "",
                    "tmdb_id": null,
                    "tmdb_popularity": null,
                    "anidb_id": null,
                    "canonical_tags": [],
                    "studio": "",
                    "tmdb_release_date": null,
                    "rating": null,
                    "rating_sources": [],
                    "external_ratings": [],
                    "credits": [],
                    "artworks": []
                }
            }
        }
    });
    Mock::given(method("GET"))
        .and(path("/graphql"))
        .and(query_param("operationName", "GetMovie"))
        .respond_with(ResponseTemplate::new(200).set_body_json(localized_movie_response.clone()))
        .with_priority(1)
        .mount(&ctx.smg_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .and(body_string_contains("GetMovie"))
        .respond_with(ResponseTemplate::new(200).set_body_json(localized_movie_response))
        .with_priority(1)
        .mount(&ctx.smg_server)
        .await;
    // This fixture models an older gateway: title-id lookup is rejected, so a
    // TVDB-backed movie must fall back to the legacy GetMovie operation.
    Mock::given(method("GET"))
        .and(path("/graphql"))
        .and(query_param("operationName", "ResolveTitles"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "errors": [{
                "message": "Cannot query field \"resolveTitles\" on type \"Query\"."
            }]
        })))
        .with_priority(2)
        .mount(&ctx.smg_server)
        .await;

    let title = create_catalog_title(
        &ctx,
        "Saved English Title",
        MediaFacet::Movie,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "94123".to_string(),
        }],
        vec![],
        true,
    )
    .await;
    let movie_dir = media_root.path().join("Saved English Title");
    std::fs::create_dir_all(&movie_dir).expect("create movie dir");
    set_title_folder_path(&ctx, &title.id, &movie_dir).await;
    let file_path = movie_dir.join("Saved.English.Title.2024.mkv");
    std::fs::write(&file_path, b"movie-rename-preview").expect("write movie file");
    ctx.shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: scryer_domain::CollectionType::Movie,
            collection_index: "1".to_string(),
            label: Some("1080p".to_string()),
            ordered_path: Some(file_path.to_string_lossy().to_string()),
            narrative_order: None,
            first_episode_number: None,
            last_episode_number: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create movie collection");
    ctx.media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: file_path.to_string_lossy().to_string(),
            size_bytes: 4096,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert movie media file");

    let actor = ctx
        .app
        .find_or_create_default_user()
        .await
        .expect("default user");
    ctx.app
        .set_title_metadata_language_override(&actor, &title.id, Some("jpn".to_string()))
        .await
        .expect("set title metadata language override");

    let preview = || async {
        gql(
            &ctx,
            r#"
            query($input: MediaRenamePreviewInput!) {
              mediaRenamePreview(input: $input) {
                fingerprint
                items { proposedPath }
              }
            }
            "#,
            json!({ "input": { "facet": "MOVIE", "titleId": title.id, "dryRun": true } }),
        )
        .await
    };

    let first = preview().await;
    assert_no_errors(&first);
    let metadata_requests = ctx
        .smg_server
        .received_requests()
        .await
        .expect("read metadata requests");
    assert!(
        metadata_requests.is_empty(),
        "preview must not hydrate: {metadata_requests:?}"
    );
    assert!(
        first["data"]["mediaRenamePreview"]["items"][0]["proposedPath"]
            .as_str()
            .is_some_and(|path| path.contains("Saved English Title")),
        "expected persisted metadata path, got {first}"
    );
    let persisted = ctx
        .titles
        .get_by_id(&title.id)
        .await
        .expect("load title")
        .expect("persisted title");
    assert_eq!(persisted.name, "Saved English Title");
    assert_eq!(persisted.metadata_language.as_deref(), Some("eng"));

    let request_count = metadata_requests.len();
    let second = preview().await;
    assert_no_errors(&second);
    assert_eq!(
        ctx.smg_server
            .received_requests()
            .await
            .expect("read metadata requests")
            .len(),
        request_count,
        "preview must not issue a metadata request"
    );
}

#[tokio::test]
async fn graphql_media_rename_preview_bulk_matches_per_title_previews() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let media_root = tempfile::tempdir().expect("media root tempdir");
    configure_default_library_root(&ctx, MediaFacet::Anime, media_root.path()).await;

    let mut expected = Vec::new();
    for (index, name) in ["Bulk Preview One", "Bulk Preview Two"].iter().enumerate() {
        let title = create_catalog_title(
            &ctx,
            name,
            MediaFacet::Anime,
            vec![ExternalId {
                source: "tvdb".to_string(),
                value: format!("9200{index}"),
            }],
            vec![],
            true,
        )
        .await;

        let collection = ctx
            .shows
            .create_collection(Collection {
                id: Id::new().0,
                title_id: title.id.clone(),
                collection_type: scryer_domain::CollectionType::Season,
                collection_index: "1".to_string(),
                label: Some("Season 1".to_string()),
                ordered_path: None,
                narrative_order: None,
                first_episode_number: Some("3".to_string()),
                last_episode_number: Some("3".to_string()),
                monitored: true,
                created_at: chrono::Utc::now(),
            })
            .await
            .expect("create season collection");

        let episode = ctx
            .shows
            .create_episode(Episode {
                id: Id::new().0,
                title_id: title.id.clone(),
                collection_id: Some(collection.id.clone()),
                episode_type: scryer_domain::EpisodeType::Standard,
                episode_number: Some("3".to_string()),
                season_number: Some("1".to_string()),
                episode_label: Some("S01E03".to_string()),
                title: Some("Arrival".to_string()),
                air_date: None,
                duration_seconds: Some(1440),
                has_multi_audio: false,
                has_subtitle: false,
                is_filler: false,
                is_recap: false,
                absolute_number: Some("12".to_string()),
                overview: None,
                tvdb_id: None,
                image_url: None,
                monitored: true,
                created_at: chrono::Utc::now(),
            })
            .await
            .expect("create episode");

        let season_dir = media_root.path().join(name).join("Season 01");
        std::fs::create_dir_all(&season_dir).expect("create season dir");
        set_title_folder_path(&ctx, &title.id, season_dir.parent().expect("title folder")).await;
        let file_path = season_dir.join(format!("[SubsPlease] {name} - 03 (1080p).mkv"));
        std::fs::write(&file_path, b"anime-preview").expect("write preview file");

        let file_id = ctx
            .media_files
            .insert_media_file(&InsertMediaFileInput {
                title_id: title.id.clone(),
                file_path: file_path.to_string_lossy().to_string(),
                size_bytes: 2048,
                quality_label: Some("1080p".to_string()),
                ..Default::default()
            })
            .await
            .expect("insert media file");
        ctx.link_primary_file_to_episode(&title.id, &file_id, &episode.id)
            .await
            .expect("link file to episode");

        expected.push(title.id.clone());
    }

    // Per-title previews are the oracle: batching may not change a single plan.
    let mut per_title = Vec::new();
    for title_id in &expected {
        let body = gql(
            &ctx,
            r#"
            query($input: MediaRenamePreviewInput!) {
              mediaRenamePreview(input: $input) {
                titleId
                fingerprint
                total
                renamable
                items { currentPath proposedPath }
              }
            }
            "#,
            json!({ "input": { "facet": "ANIME", "titleId": title_id, "dryRun": true } }),
        )
        .await;
        assert_no_errors(&body);
        per_title.push(body["data"]["mediaRenamePreview"].clone());
    }

    let bulk_body = gql(
        &ctx,
        r#"
        query($input: MediaRenamePreviewBulkInput!) {
          mediaRenamePreviewBulk(input: $input) {
            titleId
            fingerprint
            total
            renamable
            items { currentPath proposedPath }
          }
        }
        "#,
        json!({ "input": { "facet": "ANIME", "titleIds": expected } }),
    )
    .await;
    assert_no_errors(&bulk_body);
    let bulk = bulk_body["data"]["mediaRenamePreviewBulk"]
        .as_array()
        .expect("bulk plans");
    assert_eq!(bulk.len(), per_title.len());
    for (index, plan) in bulk.iter().enumerate() {
        assert_eq!(plan, &per_title[index], "plan {index} should match");
    }

    // The sampling budget spans the batch instead of applying per title.
    let sampled_body = gql(
        &ctx,
        r#"
        query($input: MediaRenamePreviewBulkInput!) {
          mediaRenamePreviewBulk(input: $input) {
            titleId
            fingerprint
            renamable
            items { currentPath }
          }
        }
        "#,
        json!({
            "input": {
                "facet": "ANIME",
                "titleIds": expected,
                "renamableOnly": true,
                "maxItems": 1
            }
        }),
    )
    .await;
    assert_no_errors(&sampled_body);
    let sampled = sampled_body["data"]["mediaRenamePreviewBulk"]
        .as_array()
        .expect("sampled plans");
    assert_eq!(sampled.len(), 2);
    assert_eq!(sampled[0]["items"].as_array().map(Vec::len), Some(1));
    assert_eq!(sampled[1]["items"].as_array().map(Vec::len), Some(0));
    for (index, plan) in sampled.iter().enumerate() {
        assert_eq!(plan["renamable"].as_i64(), Some(1));
        assert_eq!(plan["fingerprint"], per_title[index]["fingerprint"]);
    }
}

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
        vec![ExternalId::new("tvdb".to_string(), "91001".to_string())],
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
        vec![ExternalId::new("tvdb".to_string(), "91567".to_string())],
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
        vec![ExternalId::new("tvdb".to_string(), "92001".to_string())],
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
        vec![ExternalId::new("tvdb".to_string(), "93001".to_string())],
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
        vec![ExternalId::new("tvdb".to_string(), "94001".to_string())],
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
        vec![ExternalId::new("tvdb".to_string(), "94002".to_string())],
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
        vec![ExternalId::new("tvdb".to_string(), "94003".to_string())],
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
        vec![ExternalId::new("tvdb".to_string(), "95001".to_string())],
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
        vec![ExternalId::new("tvdb".to_string(), "95002".to_string())],
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
        vec![ExternalId::new("tvdb".to_string(), "96001".to_string())],
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
        vec![ExternalId::new("tvdb".to_string(), "96002".to_string())],
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
        vec![ExternalId::new("tvdb".to_string(), "97002".to_string())],
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
        vec![ExternalId::new("tvdb".to_string(), "97001".to_string())],
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
        vec![ExternalId::new("tvdb".to_string(), "98001".to_string())],
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
        vec![ExternalId::new("tvdb".to_string(), "91001".to_string())],
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
        vec![ExternalId::new("tvdb".to_string(), "94123".to_string())],
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
            vec![ExternalId::new("tvdb".to_string(), format!("9200{index}"))],
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

// ── #224: rename stays inside the root the files are actually under ───────

/// Configure `paths` as the facet library's roots (first one default) and hand
/// back their allocated ids in the same order.
async fn configure_library_roots(
    ctx: &TestContext,
    facet: MediaFacet,
    paths: &[&std::path::Path],
) -> Vec<String> {
    let library_id = scryer_domain::default_library_id_for_facet(&facet);
    let library = ctx
        .libraries
        .get_by_id(&library_id)
        .await
        .expect("lookup default library")
        .expect("default library exists");
    let drafts = paths
        .iter()
        .enumerate()
        .map(|(index, path)| LibraryRootDraft {
            path: path.to_string_lossy().to_string(),
            is_default: index == 0,
        })
        .collect::<Vec<_>>();
    let updated = ctx
        .libraries
        .update(&library_id, library.name, library.slug, drafts)
        .await
        .expect("configure library roots");
    paths
        .iter()
        .map(|path| {
            let wanted = path.to_string_lossy().to_string();
            updated
                .roots
                .iter()
                .find(|root| root.path == wanted)
                .map(|root| root.id.clone())
                .expect("configured root should be readable back")
        })
        .collect()
}

async fn seed_movie_file(
    ctx: &TestContext,
    title: &Title,
    source_path: &std::path::Path,
) -> String {
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
    ctx.media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: source_path.to_string_lossy().to_string(),
            size_bytes: 8192,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert movie media file")
}

/// A title the scan left on the library default root while its folder lives
/// under the second root must be renamed *inside* that second root. Before
/// #224 the plan proposed a path under the default root, i.e. a silent
/// cross-root move performed by a rename.
#[tokio::test]
async fn rename_preview_plans_inside_the_root_holding_the_files_not_the_recorded_root() {
    let mut ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    set_folder_template(&ctx, "MOVIE", "{title} ({year})").await;
    ctx.app = ctx.app.with_test_overrides(|builder| {
        builder.with_library_renamer(std::sync::Arc::new(FileSystemLibraryRenamer::new()))
    });

    let roots = tempfile::tempdir().expect("roots tempdir");
    let default_root = roots.path().join("movies-default");
    let second_root = roots.path().join("movies-second");
    std::fs::create_dir_all(&default_root).expect("create default root");
    std::fs::create_dir_all(&second_root).expect("create second root");
    let root_ids =
        configure_library_roots(&ctx, MediaFacet::Movie, &[&default_root, &second_root]).await;

    let title = create_catalog_title(
        &ctx,
        "Harbor Kestrels",
        MediaFacet::Movie,
        vec![ExternalId::new("tvdb".to_string(), "94101".to_string())],
        vec![],
        true,
    )
    .await;
    assert_eq!(title.root_folder_id, root_ids[0]);

    let movie_dir = second_root.join("Harbor Kestrels");
    std::fs::create_dir_all(&movie_dir).expect("create movie dir");
    set_title_folder_path(&ctx, &title.id, &movie_dir).await;
    let source_path = movie_dir.join("Harbor.Kestrels.2024.1080p.WEB-DL.mkv");
    std::fs::write(&source_path, b"harbor-kestrels").expect("write movie file");
    seed_movie_file(&ctx, &title, &source_path).await;

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
    let proposed = preview.items[0]
        .proposed_path
        .as_deref()
        .expect("proposed path");
    assert!(
        proposed.starts_with(second_root.to_string_lossy().as_ref() as &str),
        "a rename must stay inside the root holding the files, got {proposed}"
    );
    assert!(
        !proposed.starts_with(default_root.to_string_lossy().as_ref() as &str),
        "a rename must never propose a cross-root move"
    );

    // Applying brings the recorded root id in line with where the files are.
    ctx.app
        .apply_rename_for_title(&actor, &title.id, MediaFacet::Movie, &preview.fingerprint)
        .await
        .expect("apply rename");
    let updated = ctx
        .titles
        .get_by_id(&title.id)
        .await
        .expect("load title")
        .expect("title");
    assert_eq!(updated.root_folder_id, root_ids[1]);
}

/// Files under no configured root are a Change Folder / root move, not a
/// rename into the library default root.
#[tokio::test]
async fn rename_preview_skips_a_title_whose_files_are_under_no_configured_root() {
    let mut ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    set_folder_template(&ctx, "MOVIE", "{title} ({year})").await;
    ctx.app = ctx.app.with_test_overrides(|builder| {
        builder.with_library_renamer(std::sync::Arc::new(FileSystemLibraryRenamer::new()))
    });

    let roots = tempfile::tempdir().expect("roots tempdir");
    let default_root = roots.path().join("movies-default");
    std::fs::create_dir_all(&default_root).expect("create default root");
    configure_library_roots(&ctx, MediaFacet::Movie, &[&default_root]).await;

    let title = create_catalog_title(
        &ctx,
        "Cobalt Meridian",
        MediaFacet::Movie,
        vec![ExternalId::new("tvdb".to_string(), "94102".to_string())],
        vec![],
        true,
    )
    .await;

    let stray_dir = roots.path().join("unconfigured").join("Cobalt Meridian");
    std::fs::create_dir_all(&stray_dir).expect("create stray dir");
    set_title_folder_path(&ctx, &title.id, &stray_dir).await;
    let source_path = stray_dir.join("Cobalt.Meridian.2024.1080p.WEB-DL.mkv");
    std::fs::write(&source_path, b"cobalt-meridian").expect("write movie file");
    seed_movie_file(&ctx, &title, &source_path).await;

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
    assert_eq!(preview.renamable, 0);
    assert_eq!(preview.items.len(), 1);
    assert_eq!(preview.items[0].reason_code, "title_outside_library_roots");
    assert!(preview.items[0].proposed_path.is_none());
    assert!(std::fs::metadata(&source_path).is_ok(), "nothing moved");
}

/// The defensive backstop: a title whose recorded folder and whose file sit
/// under *different* roots would otherwise produce a destination outside the
/// source's root. Such an item is refused, never executed.
#[tokio::test]
async fn rename_preview_refuses_an_item_whose_destination_leaves_its_source_root() {
    let mut ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    set_folder_template(&ctx, "MOVIE", "{title} ({year})").await;
    ctx.app = ctx.app.with_test_overrides(|builder| {
        builder.with_library_renamer(std::sync::Arc::new(FileSystemLibraryRenamer::new()))
    });

    let roots = tempfile::tempdir().expect("roots tempdir");
    let default_root = roots.path().join("movies-default");
    let second_root = roots.path().join("movies-second");
    std::fs::create_dir_all(&default_root).expect("create default root");
    std::fs::create_dir_all(&second_root).expect("create second root");
    configure_library_roots(&ctx, MediaFacet::Movie, &[&default_root, &second_root]).await;

    let title = create_catalog_title(
        &ctx,
        "Lantern Drift",
        MediaFacet::Movie,
        vec![ExternalId::new("tvdb".to_string(), "94103".to_string())],
        vec![],
        true,
    )
    .await;

    // The folder says root two; the file is still sitting in root one.
    let folder_dir = second_root.join("Lantern Drift");
    std::fs::create_dir_all(&folder_dir).expect("create folder dir");
    set_title_folder_path(&ctx, &title.id, &folder_dir).await;
    let stranded_dir = default_root.join("Lantern Drift");
    std::fs::create_dir_all(&stranded_dir).expect("create stranded dir");
    let source_path = stranded_dir.join("Lantern.Drift.2024.1080p.WEB-DL.mkv");
    std::fs::write(&source_path, b"lantern-drift").expect("write movie file");
    seed_movie_file(&ctx, &title, &source_path).await;

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
    assert_eq!(preview.renamable, 0);
    assert_eq!(preview.items[0].reason_code, "cross_root_rename_refused");

    ctx.app
        .apply_rename_for_title(&actor, &title.id, MediaFacet::Movie, &preview.fingerprint)
        .await
        .expect("apply rename");
    assert!(
        std::fs::metadata(&source_path).is_ok(),
        "a refused cross-root item is never executed"
    );
}

/// A folder-renaming plan takes the file's sidecars with it and leaves no husk
/// behind: the `.hi.srt` keeps its language/hearing-impaired suffix on the new
/// stem, `movie.nfo` follows the folder, and the emptied folder is removed.
#[tokio::test]
async fn apply_media_rename_moves_companions_and_removes_the_emptied_folder() {
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
        "Tidewater Signals",
        MediaFacet::Movie,
        vec![ExternalId::new("tvdb".to_string(), "94104".to_string())],
        vec![],
        true,
    )
    .await;

    let old_dir = media_root.path().join("Tidewater Signals");
    std::fs::create_dir_all(&old_dir).expect("create old movie dir");
    set_title_folder_path(&ctx, &title.id, &old_dir).await;
    let source_stem = "Tidewater.Signals.2024.1080p.WEB-DL";
    let source_path = old_dir.join(format!("{source_stem}.mkv"));
    std::fs::write(&source_path, b"tidewater-signals").expect("write movie file");
    std::fs::write(old_dir.join(format!("{source_stem}.hi.srt")), b"subs").expect("write srt");
    std::fs::write(old_dir.join(format!("{source_stem}.nfo")), b"nfo").expect("write file nfo");
    std::fs::write(old_dir.join("movie.nfo"), b"folder-nfo").expect("write folder nfo");
    seed_movie_file(&ctx, &title, &source_path).await;

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
    assert_eq!(
        preview
            .items
            .iter()
            .filter(|item| item.companion_of.is_some())
            .count(),
        3,
        "the stem-matched srt and nfo plus the folder nfo follow the media file"
    );
    assert_eq!(preview.renamable, 4);

    let result = ctx
        .app
        .apply_rename_for_title(&actor, &title.id, MediaFacet::Movie, &preview.fingerprint)
        .await
        .expect("apply rename");
    assert_eq!(result.applied, 4);
    assert_eq!(result.failed, 0);

    let new_dir = media_root.path().join("Tidewater Signals (2024)");
    let new_stem = "Tidewater Signals (2024) - 1080p";
    assert!(new_dir.join(format!("{new_stem}.mkv")).is_file());
    assert!(
        new_dir.join(format!("{new_stem}.hi.srt")).is_file(),
        "the external subtitle keeps its suffix on the new stem"
    );
    assert!(new_dir.join(format!("{new_stem}.nfo")).is_file());
    assert!(new_dir.join("movie.nfo").is_file());

    // Everything the title owned moved, so no husk is left behind.
    assert!(
        !old_dir.exists(),
        "the emptied source folder must be removed"
    );
    let mut moved = std::fs::read_dir(&new_dir)
        .expect("read new dir")
        .map(|entry| {
            entry
                .expect("dir entry")
                .file_name()
                .to_string_lossy()
                .to_string()
        })
        .collect::<Vec<_>>();
    moved.sort();
    assert_eq!(
        moved,
        vec![
            format!("{new_stem}.hi.srt"),
            format!("{new_stem}.mkv"),
            format!("{new_stem}.nfo"),
            "movie.nfo".to_string(),
        ]
    );
}

/// A rename that moves a sidecar has to move its row with it. The companion
/// item carries no database identity, so before #226 the file landed under
/// its new name while `subtitle_downloads` kept the old path: the title listed
/// a subtitle that no longer existed, and no refresh could correct it.
///
/// The row is a *downloaded* subtitle on purpose. It must be re-pointed in
/// place — same id, provider, score and sync state — not deleted and
/// rediscovered as an anonymous sidecar.
#[tokio::test]
async fn apply_media_rename_repoints_external_subtitle_rows_at_the_moved_sidecar() {
    use scryer_application::SubtitleDownloadRepository;
    use scryer_domain::{ExternalSubtitleSourceKind, SubtitleDownload};

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
        "Lanternfall Harbor",
        MediaFacet::Movie,
        vec![ExternalId::new("tvdb".to_string(), "94117".to_string())],
        vec![],
        true,
    )
    .await;

    let old_dir = media_root.path().join("Lanternfall Harbor");
    std::fs::create_dir_all(&old_dir).expect("create old movie dir");
    set_title_folder_path(&ctx, &title.id, &old_dir).await;
    let source_stem = "Lanternfall.Harbor.2024.1080p.WEB-DL";
    let source_path = old_dir.join(format!("{source_stem}.mkv"));
    let old_subtitle = old_dir.join(format!("{source_stem}.hi.srt"));
    std::fs::write(&source_path, b"lanternfall-harbor").expect("write movie file");
    std::fs::write(&old_subtitle, b"subs").expect("write srt");
    let media_file_id = seed_movie_file(&ctx, &title, &source_path).await;

    ctx.library_state
        .subtitle_downloads
        .insert(&SubtitleDownload {
            id: "subtitle-lanternfall".to_string(),
            media_file_id: media_file_id.clone(),
            title_id: title.id.clone(),
            episode_id: None,
            source_kind: ExternalSubtitleSourceKind::Downloaded,
            language: "eng".to_string(),
            provider: Some("synthetic-provider".to_string()),
            provider_file_id: Some("synthetic-file-1".to_string()),
            file_path: old_subtitle.to_string_lossy().to_string(),
            score: Some(97),
            hearing_impaired: true,
            forced: false,
            ai_translated: false,
            machine_translated: false,
            uploader: None,
            release_info: Some(format!("{source_stem}.hi")),
            synced: true,
            downloaded_at: "2026-09-01T00:00:00Z".to_string(),
        })
        .await
        .expect("seed downloaded subtitle");

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
    let result = ctx
        .app
        .apply_rename_for_title(&actor, &title.id, MediaFacet::Movie, &preview.fingerprint)
        .await
        .expect("apply rename");
    assert_eq!(result.failed, 0);

    let new_subtitle = media_root
        .path()
        .join("Lanternfall Harbor (2024)")
        .join("Lanternfall Harbor (2024) - 1080p.hi.srt");
    assert!(new_subtitle.is_file(), "the sidecar moved on disk");
    assert!(!old_subtitle.exists());

    let rows = ctx
        .library_state
        .subtitle_downloads
        .list_for_title(&title.id)
        .await
        .expect("list subtitles");
    assert_eq!(rows.len(), 1, "re-pointed, never duplicated: {rows:?}");
    let row = &rows[0];
    assert_eq!(row.id, "subtitle-lanternfall", "rewritten in place");
    assert_eq!(
        std::path::PathBuf::from(&row.file_path),
        new_subtitle,
        "the row follows the file"
    );
    assert_eq!(row.source_kind, ExternalSubtitleSourceKind::Downloaded);
    assert_eq!(row.provider.as_deref(), Some("synthetic-provider"));
    assert_eq!(row.score, Some(97));
    assert!(row.synced, "sync state survives the rename");
    assert!(row.hearing_impaired);
}

/// Extras that are not stem-matched companions are a folder move, not a
/// rename: they stay put, and the folder that still holds them survives.
#[tokio::test]
async fn apply_media_rename_leaves_unrelated_extras_and_their_folder_alone() {
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
        "Wrenfield Passage",
        MediaFacet::Movie,
        vec![ExternalId::new("tvdb".to_string(), "94106".to_string())],
        vec![],
        true,
    )
    .await;

    let old_dir = media_root.path().join("Wrenfield Passage");
    std::fs::create_dir_all(&old_dir).expect("create old movie dir");
    set_title_folder_path(&ctx, &title.id, &old_dir).await;
    let source_path = old_dir.join("Wrenfield.Passage.2024.1080p.WEB-DL.mkv");
    std::fs::write(&source_path, b"wrenfield-passage").expect("write movie file");
    std::fs::write(old_dir.join("unrelated-extra.mkv"), b"extra").expect("write extra");
    seed_movie_file(&ctx, &title, &source_path).await;

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
    assert_eq!(preview.renamable, 1, "the extra is never planned");
    let result = ctx
        .app
        .apply_rename_for_title(&actor, &title.id, MediaFacet::Movie, &preview.fingerprint)
        .await
        .expect("apply rename");
    assert_eq!(result.applied, 1);

    assert!(old_dir.join("unrelated-extra.mkv").is_file());
    assert!(
        old_dir.is_dir(),
        "a folder that still holds files is never removed"
    );
}

/// A companion whose destination is already taken fails on its own; the media
/// file still moves.
#[tokio::test]
async fn apply_media_rename_skips_an_occupied_companion_target_without_blocking_the_media_move() {
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
        "Seagrass Ledger",
        MediaFacet::Movie,
        vec![ExternalId::new("tvdb".to_string(), "94105".to_string())],
        vec![],
        true,
    )
    .await;

    let old_dir = media_root.path().join("Seagrass Ledger");
    std::fs::create_dir_all(&old_dir).expect("create old movie dir");
    set_title_folder_path(&ctx, &title.id, &old_dir).await;
    let source_stem = "Seagrass.Ledger.2024.1080p.WEB-DL";
    let source_path = old_dir.join(format!("{source_stem}.mkv"));
    std::fs::write(&source_path, b"seagrass-ledger").expect("write movie file");
    std::fs::write(old_dir.join(format!("{source_stem}.hi.srt")), b"subs").expect("write srt");
    seed_movie_file(&ctx, &title, &source_path).await;

    // Squat the companion's destination with a different file.
    let new_dir = media_root.path().join("Seagrass Ledger (2024)");
    std::fs::create_dir_all(&new_dir).expect("create new movie dir");
    let occupied = new_dir.join("Seagrass Ledger (2024) - 1080p.hi.srt");
    std::fs::write(&occupied, b"someone-elses-subs").expect("write occupied companion");

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
    let result = ctx
        .app
        .apply_rename_for_title(&actor, &title.id, MediaFacet::Movie, &preview.fingerprint)
        .await
        .expect("apply rename");

    assert_eq!(result.applied, 1, "the media file still moves");
    assert_eq!(result.failed, 1);
    let companion = result
        .items
        .iter()
        .find(|item| item.current_path.ends_with(".hi.srt"))
        .expect("companion item");
    assert_eq!(companion.reason_code, "target_exists");
    assert_eq!(
        std::fs::read(&occupied).expect("occupied companion intact"),
        b"someone-elses-subs"
    );
    assert!(new_dir.join("Seagrass Ledger (2024) - 1080p.mkv").is_file());
    let updated_file_path = ctx
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files")
        .into_iter()
        .map(|file| file.file_path)
        .collect::<Vec<_>>();
    assert_eq!(
        updated_file_path,
        vec![
            new_dir
                .join("Seagrass Ledger (2024) - 1080p.mkv")
                .to_string_lossy()
                .to_string()
        ]
    );
}

/// A companion must never outlive its media file's failure. When the primary's
/// destination is occupied the media file stays put, so the sidecars must stay
/// with it rather than moving to a folder with no video in it.
#[tokio::test]
async fn apply_media_rename_leaves_companions_behind_when_the_primary_cannot_move() {
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
        "Marrowgate Hollow",
        MediaFacet::Movie,
        vec![ExternalId::new("tvdb".to_string(), "94107".to_string())],
        vec![],
        true,
    )
    .await;

    let old_dir = media_root.path().join("Marrowgate Hollow");
    std::fs::create_dir_all(&old_dir).expect("create old movie dir");
    set_title_folder_path(&ctx, &title.id, &old_dir).await;
    let source_stem = "Marrowgate.Hollow.2024.1080p.WEB-DL";
    let source_path = old_dir.join(format!("{source_stem}.mkv"));
    let source_srt = old_dir.join(format!("{source_stem}.hi.srt"));
    let source_nfo = old_dir.join(format!("{source_stem}.nfo"));
    std::fs::write(&source_path, b"marrowgate-hollow").expect("write movie file");
    std::fs::write(&source_srt, b"subs").expect("write srt");
    std::fs::write(&source_nfo, b"nfo").expect("write nfo");
    seed_movie_file(&ctx, &title, &source_path).await;

    // Squat the *media file's* destination so its move fails.
    let new_dir = media_root.path().join("Marrowgate Hollow (2024)");
    std::fs::create_dir_all(&new_dir).expect("create new movie dir");
    let occupied = new_dir.join("Marrowgate Hollow (2024) - 1080p.mkv");
    std::fs::write(&occupied, b"someone-elses-movie").expect("write occupied media target");

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
    let result = ctx
        .app
        .apply_rename_for_title(&actor, &title.id, MediaFacet::Movie, &preview.fingerprint)
        .await
        .expect("apply rename");

    assert_eq!(result.applied, 0, "the media file could not move");
    assert_eq!(result.failed, 1);
    assert_eq!(result.skipped, 2, "both companions stood down");
    let primary = result
        .items
        .iter()
        .find(|item| item.current_path.ends_with(".mkv"))
        .expect("primary item");
    assert_eq!(primary.reason_code, "target_exists");
    for companion in result
        .items
        .iter()
        .filter(|item| !item.current_path.ends_with(".mkv"))
    {
        assert_eq!(companion.status.as_str(), "skipped");
        assert_eq!(companion.reason_code, "companion_primary_not_applied");
    }

    assert!(source_path.is_file(), "the media file stayed put");
    assert!(source_srt.is_file(), "the subtitle stayed with its video");
    assert!(source_nfo.is_file(), "the sidecar stayed with its video");
    assert!(old_dir.is_dir(), "nothing emptied the source folder");
    assert_eq!(
        std::fs::read(&occupied).expect("occupied target intact"),
        b"someone-elses-movie"
    );
}

/// When the media file's move is rolled back because its database update
/// failed, the companions that already moved come back with it.
#[tokio::test]
async fn apply_media_rename_rolls_companions_back_with_their_primary_on_a_db_failure() {
    let mut ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    set_folder_template(&ctx, "MOVIE", "{title} ({year})").await;
    let media_root = tempfile::tempdir().expect("media root tempdir");
    configure_default_library_root(&ctx, MediaFacet::Movie, media_root.path()).await;

    let title = create_catalog_title(
        &ctx,
        "Pellucid Quarry",
        MediaFacet::Movie,
        vec![ExternalId::new("tvdb".to_string(), "94108".to_string())],
        vec![],
        true,
    )
    .await;

    let old_dir = media_root.path().join("Pellucid Quarry");
    std::fs::create_dir_all(&old_dir).expect("create old movie dir");
    set_title_folder_path(&ctx, &title.id, &old_dir).await;
    let source_stem = "Pellucid.Quarry.2024.1080p.WEB-DL";
    let source_path = old_dir.join(format!("{source_stem}.mkv"));
    let source_srt = old_dir.join(format!("{source_stem}.hi.srt"));
    let source_nfo = old_dir.join(format!("{source_stem}.nfo"));
    std::fs::write(&source_path, b"pellucid-quarry").expect("write movie file");
    std::fs::write(&source_srt, b"subs").expect("write srt");
    std::fs::write(&source_nfo, b"nfo").expect("write nfo");
    let file_id = seed_movie_file(&ctx, &title, &source_path).await;

    ctx.app = ctx.app.with_test_overrides(|builder| {
        builder
            .with_library_renamer(std::sync::Arc::new(FileSystemLibraryRenamer::new()))
            .with_media_files(std::sync::Arc::new(FailingMediaFileRepo {
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
        .preview_rename_for_title(&actor, &title.id, MediaFacet::Movie)
        .await
        .expect("preview rename plan");
    assert_eq!(preview.renamable, 3);
    let result = ctx
        .app
        .apply_rename_for_title(&actor, &title.id, MediaFacet::Movie, &preview.fingerprint)
        .await
        .expect("apply rename");

    assert_eq!(result.applied, 0);
    assert_eq!(result.failed, 3, "the primary and both companions failed");
    let primary = result
        .items
        .iter()
        .find(|item| item.current_path.ends_with(".mkv"))
        .expect("primary item");
    assert_eq!(primary.reason_code, "db_update_failed");
    assert!(
        primary
            .error_message
            .as_deref()
            .is_some_and(|message| message.contains("2 companion(s) restored")),
        "the primary reports how many companions came back: {:?}",
        primary.error_message
    );
    for companion in result
        .items
        .iter()
        .filter(|item| !item.current_path.ends_with(".mkv"))
    {
        assert_eq!(companion.status.as_str(), "failed");
        assert_eq!(companion.reason_code, "companion_rolled_back_with_primary");
    }

    assert!(source_path.is_file(), "the media file is back");
    assert!(source_srt.is_file(), "the subtitle is back");
    assert!(source_nfo.is_file(), "the sidecar is back");
    let new_dir = media_root.path().join("Pellucid Quarry (2024)");
    let leftovers = std::fs::read_dir(&new_dir)
        .map(|entries| entries.count())
        .unwrap_or(0);
    assert_eq!(leftovers, 0, "nothing was left at the new location");
    let stored = ctx
        .media_files
        .get_media_file_by_id(&file_id)
        .await
        .expect("load media file")
        .expect("media file present");
    assert_eq!(stored.file_path, source_path.to_string_lossy().to_string());
}

/// A renamer whose rollback refuses one companion, so that companion stays at
/// the rename's destination while everything else is put back.
struct CompanionRollbackRefusingRenamer {
    inner: FileSystemLibraryRenamer,
    refuse_suffix: &'static str,
}

#[async_trait]
impl scryer_application::LibraryRenamer for CompanionRollbackRefusingRenamer {
    async fn validate_targets(&self, plan: &scryer_application::RenamePlan) -> AppResult<()> {
        self.inner.validate_targets(plan).await
    }

    async fn apply_plan(
        &self,
        plan: &scryer_application::RenamePlan,
        permissions: &scryer_application::ImportFilePermissions,
    ) -> AppResult<Vec<scryer_application::RenameApplyItemResult>> {
        self.inner.apply_plan(plan, permissions).await
    }

    async fn rollback(
        &self,
        applied_items: &[scryer_application::RenameApplyItemResult],
    ) -> AppResult<Vec<scryer_application::RenameApplyItemResult>> {
        if applied_items
            .iter()
            .any(|item| item.current_path.ends_with(self.refuse_suffix))
        {
            return Err(AppError::Repository("synthetic rollback refusal".into()));
        }
        self.inner.rollback(applied_items).await
    }
}

/// A companion whose rollback fails is reported where it actually is: the
/// rename's destination, with the rollback error, not as restored.
#[tokio::test]
async fn apply_media_rename_reports_a_companion_that_could_not_be_rolled_back_at_its_destination() {
    let mut ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    set_folder_template(&ctx, "MOVIE", "{title} ({year})").await;
    let media_root = tempfile::tempdir().expect("media root tempdir");
    configure_default_library_root(&ctx, MediaFacet::Movie, media_root.path()).await;

    let title = create_catalog_title(
        &ctx,
        "Tessellate Ferry",
        MediaFacet::Movie,
        vec![ExternalId::new("tvdb".to_string(), "94109".to_string())],
        vec![],
        true,
    )
    .await;

    let old_dir = media_root.path().join("Tessellate Ferry");
    std::fs::create_dir_all(&old_dir).expect("create old movie dir");
    set_title_folder_path(&ctx, &title.id, &old_dir).await;
    let source_stem = "Tessellate.Ferry.2024.1080p.WEB-DL";
    let source_path = old_dir.join(format!("{source_stem}.mkv"));
    let source_srt = old_dir.join(format!("{source_stem}.hi.srt"));
    let source_nfo = old_dir.join(format!("{source_stem}.nfo"));
    std::fs::write(&source_path, b"tessellate-ferry").expect("write movie file");
    std::fs::write(&source_srt, b"subs").expect("write srt");
    std::fs::write(&source_nfo, b"nfo").expect("write nfo");
    let file_id = seed_movie_file(&ctx, &title, &source_path).await;

    ctx.app = ctx.app.with_test_overrides(|builder| {
        builder
            .with_library_renamer(std::sync::Arc::new(CompanionRollbackRefusingRenamer {
                inner: FileSystemLibraryRenamer::new(),
                refuse_suffix: ".srt",
            }))
            .with_media_files(std::sync::Arc::new(FailingMediaFileRepo {
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
        .preview_rename_for_title(&actor, &title.id, MediaFacet::Movie)
        .await
        .expect("preview rename plan");
    assert_eq!(preview.renamable, 3);
    let result = ctx
        .app
        .apply_rename_for_title(&actor, &title.id, MediaFacet::Movie, &preview.fingerprint)
        .await
        .expect("apply rename");

    assert_eq!(result.applied, 0);
    assert_eq!(result.failed, 3);
    let primary = result
        .items
        .iter()
        .find(|item| item.current_path.ends_with(".mkv"))
        .expect("primary item");
    assert!(
        primary.error_message.as_deref().is_some_and(|message| {
            message.contains("1 companion(s) restored, 1 could not be restored")
        }),
        "the primary reports the split: {:?}",
        primary.error_message
    );

    let srt = result
        .items
        .iter()
        .find(|item| item.current_path.ends_with(".srt"))
        .expect("subtitle item");
    assert_eq!(srt.status.as_str(), "failed");
    assert_eq!(srt.reason_code, "companion_rollback_failed");
    let stranded_at = srt.final_path.clone().expect("destination kept");
    assert_ne!(stranded_at, srt.current_path);
    assert!(
        std::path::Path::new(&stranded_at).is_file(),
        "the reported path is where the subtitle really is"
    );
    assert!(
        srt.error_message
            .as_deref()
            .is_some_and(|message| message.contains("synthetic rollback refusal")),
        "the rollback error is carried: {:?}",
        srt.error_message
    );

    let nfo = result
        .items
        .iter()
        .find(|item| item.current_path.ends_with(".nfo"))
        .expect("sidecar item");
    assert_eq!(nfo.reason_code, "companion_rolled_back_with_primary");
    assert_eq!(nfo.final_path.as_deref(), Some(nfo.current_path.as_str()));
    assert!(source_nfo.is_file(), "the sidecar is back");
    assert!(source_path.is_file(), "the media file is back");
}

/// The emptied-folder cleanup stops at the title folder. An operator's
/// organising directory above it is not the rename's to remove, even when the
/// rename happens to leave it empty.
#[tokio::test]
async fn apply_media_rename_removes_the_title_folder_but_not_the_directory_above_it() {
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
        "Kestrel Hollow",
        MediaFacet::Movie,
        vec![ExternalId::new("tvdb".to_string(), "94109".to_string())],
        vec![],
        true,
    )
    .await;

    let organising_dir = media_root.path().join("Unsorted Imports");
    let old_dir = organising_dir.join("Kestrel Hollow");
    std::fs::create_dir_all(&old_dir).expect("create old movie dir");
    set_title_folder_path(&ctx, &title.id, &old_dir).await;
    let source_path = old_dir.join("Kestrel.Hollow.2024.1080p.WEB-DL.mkv");
    std::fs::write(&source_path, b"kestrel-hollow").expect("write movie file");
    seed_movie_file(&ctx, &title, &source_path).await;

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
    let result = ctx
        .app
        .apply_rename_for_title(&actor, &title.id, MediaFacet::Movie, &preview.fingerprint)
        .await
        .expect("apply rename");
    assert_eq!(result.applied, 1);

    assert!(
        media_root
            .path()
            .join("Kestrel Hollow (2024)")
            .join("Kestrel Hollow (2024) - 1080p.mkv")
            .is_file()
    );
    assert!(!old_dir.exists(), "the emptied title folder is removed");
    assert!(
        organising_dir.is_dir(),
        "the operator's organising directory is left alone even though it is now empty"
    );
    assert_eq!(
        std::fs::read_dir(&organising_dir)
            .expect("read organising dir")
            .count(),
        0
    );
}

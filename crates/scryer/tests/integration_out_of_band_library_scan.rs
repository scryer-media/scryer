#![recursion_limit = "256"]

mod common;

use std::path::{Path, PathBuf};

use common::TestContext;
use scryer_application::{
    InsertMediaFileInput, LibraryRootDraft, LibraryScanUnmatchedItem,
    LibraryScanUnmatchedItemRepository, MediaFileRepository, MediaFileRole, PendingImportStatus,
    ShowRepository, TitleRepository,
};
use scryer_domain::{Collection, Episode, ExternalId, Id, MediaFacet, NewTitle, Title, User};
use scryer_infrastructure_sql::types::SettingDefinitionSeed;

fn admin() -> User {
    let mut user = User::new_admin("admin");
    user.authorization = scryer_domain::UserAuthorization {
        app: scryer_domain::AppPermissionMask::from_permissions([
            scryer_domain::AppPermission::ManageCatalogSettings,
        ]),
        default_library: scryer_domain::LibraryPermissionMask::from_permissions([
            scryer_domain::LibraryPermission::View,
            scryer_domain::LibraryPermission::ManageTitles,
            scryer_domain::LibraryPermission::ResolveImports,
            scryer_domain::LibraryPermission::ManageLibrary,
        ]),
        actor_capabilities: scryer_domain::ActorCapabilityMask::MANAGE_OWN_ACCOUNT,
        loaded: true,
        ..Default::default()
    };
    user
}

fn pending_import_title_request(facet: MediaFacet, name: &str, tvdb_id: &str) -> NewTitle {
    NewTitle {
        name: name.to_string(),
        facet,
        monitored: false,
        tags: Vec::new(),
        external_ids: vec![ExternalId {
            source: "tvdb".to_string(),
            value: tvdb_id.to_string(),
        }],
        root_folder_id: None,
        min_availability: None,
        poster_url: None,
        year: None,
        overview: None,
        sort_title: Some(name.to_string()),
        slug: Some(name.to_ascii_lowercase().replace(' ', "-")),
        runtime_minutes: None,
        language: None,
        content_status: None,
    }
}

async fn seed_media_path_settings(ctx: &TestContext) {
    ctx.settings_store
        .batch_ensure_setting_definitions(vec![
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "media".into(),
                key_name: "series.path".into(),
                data_type: "string".into(),
                default_value_json: "\"/data/series\"".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "media".into(),
                key_name: "movies.path".into(),
                data_type: "string".into(),
                default_value_json: "\"/data/movies\"".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "media".into(),
                key_name: "anime.path".into(),
                data_type: "string".into(),
                default_value_json: "\"/data/anime\"".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "media".into(),
                key_name: "series.root_folders".into(),
                data_type: "json".into(),
                default_value_json: "[]".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "media".into(),
                key_name: "movies.root_folders".into(),
                data_type: "json".into(),
                default_value_json: "[]".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "media".into(),
                key_name: "anime.root_folders".into(),
                data_type: "json".into(),
                default_value_json: "[]".into(),
                is_sensitive: false,
                validation_json: None,
            },
        ])
        .await
        .expect("seed media path setting definitions");
}

async fn set_media_path(ctx: &TestContext, key_name: &str, value: &str) {
    ctx.settings_store
        .upsert_setting_value(
            "media",
            key_name,
            None,
            serde_json::to_string(value).expect("serialize setting value"),
            "integration_test",
            None,
        )
        .await
        .expect("upsert media path setting");
}

async fn set_default_library_root(ctx: &TestContext, facet: MediaFacet, root: &Path) {
    let library_id = scryer_domain::default_library_id_for_facet(&facet);
    ctx.app
        .update_library(
            &admin(),
            &library_id,
            None,
            Some(vec![LibraryRootDraft {
                path: root.to_string_lossy().to_string(),
                is_default: true,
            }]),
            None,
        )
        .await
        .expect("update default library root");
}

async fn seed_series_title(
    ctx: &TestContext,
    id: &str,
    name: &str,
    facet: MediaFacet,
    media_root: &Path,
    folder_path: Option<&Path>,
    tvdb_id: Option<&str>,
) -> Title {
    let title = Title {
        id: id.to_string(),
        name: name.to_string(),
        library_id: scryer_domain::default_library_id_for_facet(&facet),
        facet,
        monitored: true,
        tags: vec![],
        canonical_tags: vec![],
        external_ids: tvdb_id
            .map(|value| {
                vec![ExternalId {
                    source: "tvdb".to_string(),
                    value: value.to_string(),
                }]
            })
            .unwrap_or_default(),
        root_folder_id: scryer_domain::root_folder_id_for_path(
            media_root.to_string_lossy().as_ref(),
        ),
        created_by: None,
        created_at: chrono::Utc::now(),
        year: Some(2024),
        overview: None,
        poster_url: None,
        poster_source_url: None,
        background_url: None,
        background_source_url: None,
        sort_title: None,
        catalog_sort_key: String::new(),
        slug: None,
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
        metadata_language: None,
        metadata_fetched_at: None,
        min_availability: None,
        digital_release_date: None,
        folder_path: folder_path.map(|path| path.to_string_lossy().to_string()),
    };
    ctx.titles.create(title.clone()).await.expect("seed title");
    title
}

async fn seed_collection(ctx: &TestContext, title: &Title, index: u32) -> Collection {
    let collection = Collection {
        id: Id::new().0,
        title_id: title.id.clone(),
        collection_type: scryer_domain::CollectionType::Season,
        collection_index: index.to_string(),
        label: Some(format!("Season {index}")),
        ordered_path: None,
        narrative_order: None,
        first_episode_number: Some("1".to_string()),
        last_episode_number: Some("99".to_string()),
        monitored: true,
        created_at: chrono::Utc::now(),
    };
    ctx.shows
        .create_collection(collection.clone())
        .await
        .expect("create collection");
    collection
}

async fn seed_episode(
    ctx: &TestContext,
    title: &Title,
    collection: &Collection,
    episode_number: u32,
) -> Episode {
    let episode = Episode {
        id: Id::new().0,
        title_id: title.id.clone(),
        collection_id: Some(collection.id.clone()),
        episode_type: scryer_domain::EpisodeType::Standard,
        episode_number: Some(episode_number.to_string()),
        season_number: Some(collection.collection_index.clone()),
        episode_label: Some(format!(
            "S{:02}E{:02}",
            collection.collection_index.parse::<u32>().unwrap_or(1),
            episode_number
        )),
        title: Some(format!("Episode {episode_number}")),
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
    };
    ctx.shows
        .create_episode(episode.clone())
        .await
        .expect("create episode");
    episode
}

fn write_fake_media_file(path: &Path) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create parent directory");
    }
    std::fs::write(path, b"not a real video").expect("write fake media file");
}

#[tokio::test]
async fn known_title_unmatched_file_becomes_title_bound_pending_import_and_can_bind() {
    let ctx = TestContext::new().await;
    seed_media_path_settings(&ctx).await;

    let media_root = tempfile::tempdir().expect("series root");
    set_media_path(
        &ctx,
        "series.path",
        media_root.path().to_string_lossy().as_ref(),
    )
    .await;
    set_default_library_root(&ctx, MediaFacet::Series, media_root.path()).await;

    let title_dir = media_root.path().join("Known Show");
    std::fs::create_dir_all(&title_dir).expect("create title directory");
    let title = seed_series_title(
        &ctx,
        "title-known",
        "Known Show",
        MediaFacet::Series,
        media_root.path(),
        Some(&title_dir),
        None,
    )
    .await;
    let season = seed_collection(&ctx, &title, 1).await;
    let first_episode = seed_episode(&ctx, &title, &season, 1).await;
    let _second_episode = seed_episode(&ctx, &title, &season, 2).await;

    let unmanaged_file = title_dir.join("Known Show - bonus feature.mkv");
    write_fake_media_file(&unmanaged_file);

    let actor = admin();
    let summary = ctx
        .app
        .scan_title_library(&actor, &title.id)
        .await
        .expect("scan title library");
    assert_eq!(summary.unmatched, 1);

    let pending = ctx
        .app
        .pending_imports(
            &actor,
            MediaFacet::Series,
            None,
            PendingImportStatus::Pending,
            20,
            0,
        )
        .await
        .expect("list pending imports");
    assert_eq!(pending.total, 1);
    assert_eq!(pending.items.len(), 1);

    let item = &pending.items[0];
    assert_eq!(item.title_id.as_deref(), Some(title.id.as_str()));
    assert_eq!(PathBuf::from(&item.path), unmanaged_file);

    let preview = ctx
        .app
        .preview_title_bound_pending_import(&actor, &item.id)
        .await
        .expect("preview title-bound pending import");
    assert_eq!(preview.title.id, title.id);
    assert!(preview.file.suggested_episode_ids.is_empty());
    assert_eq!(preview.available_episodes.len(), 2);

    let bind_result = ctx
        .app
        .bind_title_bound_pending_import(
            &actor,
            &item.id,
            None,
            std::slice::from_ref(&first_episode.id),
        )
        .await
        .expect("bind title-bound pending import");
    assert!(!bind_result.created);

    let pending_after = ctx
        .app
        .pending_imports(
            &actor,
            MediaFacet::Series,
            None,
            PendingImportStatus::Pending,
            20,
            0,
        )
        .await
        .expect("list pending imports after bind");
    assert_eq!(pending_after.total, 0);

    let media_files = ctx
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list title media files");
    assert_eq!(media_files.len(), 1);
    assert_eq!(
        media_files[0].episode_id.as_deref(),
        Some(first_episode.id.as_str())
    );
    assert_eq!(PathBuf::from(&media_files[0].file_path), unmanaged_file);
}

#[tokio::test]
async fn known_title_pending_import_row_is_cleared_when_file_is_removed() {
    let ctx = TestContext::new().await;
    seed_media_path_settings(&ctx).await;

    let media_root = tempfile::tempdir().expect("series root");
    set_media_path(
        &ctx,
        "series.path",
        media_root.path().to_string_lossy().as_ref(),
    )
    .await;
    set_default_library_root(&ctx, MediaFacet::Series, media_root.path()).await;

    let title_dir = media_root.path().join("Removal Show");
    std::fs::create_dir_all(&title_dir).expect("create title directory");
    let title = seed_series_title(
        &ctx,
        "title-removal",
        "Removal Show",
        MediaFacet::Series,
        media_root.path(),
        Some(&title_dir),
        None,
    )
    .await;
    let season = seed_collection(&ctx, &title, 1).await;
    let _first_episode = seed_episode(&ctx, &title, &season, 1).await;

    let unmanaged_file = title_dir.join("Removal Show - notes.mkv");
    write_fake_media_file(&unmanaged_file);

    let actor = admin();
    ctx.app
        .scan_title_library(&actor, &title.id)
        .await
        .expect("initial title scan");

    let pending_before = ctx
        .app
        .pending_imports(
            &actor,
            MediaFacet::Series,
            None,
            PendingImportStatus::Pending,
            20,
            0,
        )
        .await
        .expect("list pending imports before delete");
    assert_eq!(pending_before.total, 1);

    std::fs::remove_file(&unmanaged_file).expect("remove unmanaged file");

    ctx.app
        .scan_title_library(&actor, &title.id)
        .await
        .expect("rescan title after delete");

    let pending_after = ctx
        .app
        .pending_imports(
            &actor,
            MediaFacet::Series,
            None,
            PendingImportStatus::Pending,
            20,
            0,
        )
        .await
        .expect("list pending imports after delete");
    assert_eq!(pending_after.total, 0);
}

#[tokio::test]
async fn loose_root_series_file_is_skipped_without_claiming_library_root() {
    let ctx = TestContext::new().await;
    seed_media_path_settings(&ctx).await;

    let media_root = tempfile::tempdir().expect("series root");
    set_media_path(
        &ctx,
        "series.path",
        media_root.path().to_string_lossy().as_ref(),
    )
    .await;

    let title = seed_series_title(
        &ctx,
        "title-loose",
        "Loose Show",
        MediaFacet::Series,
        media_root.path(),
        None,
        None,
    )
    .await;
    let season = seed_collection(&ctx, &title, 1).await;
    let _first_episode = seed_episode(&ctx, &title, &season, 1).await;

    let loose_file = media_root.path().join("Loose.Show.S01E01.mkv");
    write_fake_media_file(&loose_file);

    let actor = admin();
    set_default_library_root(&ctx, MediaFacet::Series, media_root.path()).await;
    let summary = ctx
        .app
        .scan_library(&actor, MediaFacet::Series)
        .await
        .expect("full library scan");
    assert_eq!(summary.unmatched, 0);

    let media_files = ctx
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list title media files");
    assert!(media_files.is_empty());
    assert!(loose_file.exists());

    let refreshed_title = ctx
        .titles
        .get_by_id(&title.id)
        .await
        .expect("load refreshed title")
        .expect("title should exist");
    assert_eq!(refreshed_title.folder_path, None);
}

#[tokio::test]
async fn full_rescan_preserves_existing_match_for_loose_series_file() {
    let ctx = TestContext::new().await;
    seed_media_path_settings(&ctx).await;

    let media_root = tempfile::tempdir().expect("series root");
    set_media_path(
        &ctx,
        "series.path",
        media_root.path().to_string_lossy().as_ref(),
    )
    .await;

    let title = seed_series_title(
        &ctx,
        "title-preserved-match",
        "Known Show",
        MediaFacet::Series,
        media_root.path(),
        None,
        None,
    )
    .await;
    let season = seed_collection(&ctx, &title, 1).await;
    let episode = seed_episode(&ctx, &title, &season, 1).await;

    let loose_file = media_root
        .path()
        .join("Completely.Different.Name.S01E01.mkv");
    write_fake_media_file(&loose_file);

    let file_id = ctx
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: loose_file.to_string_lossy().to_string(),
            size_bytes: 16,
            announced_size_bytes: None,
            role: MediaFileRole::Primary,
            source_signature_scheme: None,
            source_signature_value: None,
            quality_label: Some("720p".to_string()),
            scene_name: None,
            release_group: None,
            source_type: None,
            resolution: None,
            video_codec_parsed: None,
            audio_codec_parsed: None,
            audio_channels_parsed: None,
            acquisition_score: None,
            scoring_log: None,
            indexer_source: None,
            grabbed_release_title: None,
            grabbed_at: None,
            edition: None,
            original_file_path: None,
            release_hash: None,
        })
        .await
        .expect("insert media file");
    ctx.media_files
        .link_file_to_episode(&file_id, &episode.id)
        .await
        .expect("link file to episode");

    let actor = admin();
    set_default_library_root(&ctx, MediaFacet::Series, media_root.path()).await;
    let summary = ctx
        .app
        .scan_library(&actor, MediaFacet::Series)
        .await
        .expect("full library scan");

    assert_eq!(summary.unmatched, 0);

    let media_files = ctx
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list title media files");
    assert_eq!(media_files.len(), 1);
    assert_eq!(media_files[0].id, file_id);
    assert_eq!(
        media_files[0].episode_id.as_deref(),
        Some(episode.id.as_str())
    );

    let unmatched_count = ctx
        .library_scan_unmatched
        .count_library_scan_unmatched_items(
            Some(MediaFacet::Series),
            Some(media_root.path().to_string_lossy().as_ref()),
            None,
        )
        .await
        .expect("count unmatched items");
    assert_eq!(unmatched_count, 0);
}

#[tokio::test]
async fn full_scan_uses_release_subfolder_when_series_file_name_is_obfuscated() {
    let ctx = TestContext::new().await;
    seed_media_path_settings(&ctx).await;

    let media_root = tempfile::tempdir().expect("series root");
    set_media_path(
        &ctx,
        "series.path",
        media_root.path().to_string_lossy().as_ref(),
    )
    .await;

    let title_dir = media_root.path().join("Harbor Pals");
    std::fs::create_dir_all(&title_dir).expect("create title directory");
    let title = seed_series_title(
        &ctx,
        "title-obfuscated-release-dir",
        "Harbor Pals",
        MediaFacet::Series,
        media_root.path(),
        Some(&title_dir),
        None,
    )
    .await;
    let season = seed_collection(&ctx, &title, 1).await;
    let first_episode = seed_episode(&ctx, &title, &season, 1).await;

    let release_dir = title_dir.join("Harbor.Pals.S01E01.720p.WEB-DL.AV1.AAC2.0-NTb");
    std::fs::create_dir_all(&release_dir).expect("create release directory");
    let obfuscated_file = release_dir.join("4f8e2c7a91b6d3e0.mkv");
    write_fake_media_file(&obfuscated_file);

    let actor = admin();
    set_default_library_root(&ctx, MediaFacet::Series, media_root.path()).await;
    ctx.app
        .scan_library(&actor, MediaFacet::Series)
        .await
        .expect("full library scan");

    let pending = ctx
        .app
        .pending_imports(
            &actor,
            MediaFacet::Series,
            None,
            PendingImportStatus::Pending,
            20,
            0,
        )
        .await
        .expect("list pending imports after scan");
    assert_eq!(pending.total, 0);

    let media_files = ctx
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list title media files");
    assert_eq!(media_files.len(), 1);
    assert_eq!(
        media_files[0].episode_id.as_deref(),
        Some(first_episode.id.as_str())
    );
    assert_eq!(PathBuf::from(&media_files[0].file_path), obfuscated_file);
}

#[tokio::test]
async fn full_scan_does_not_infer_episode_from_parent_when_release_folder_has_multiple_videos() {
    let ctx = TestContext::new().await;
    seed_media_path_settings(&ctx).await;

    let media_root = tempfile::tempdir().expect("series root");
    set_media_path(
        &ctx,
        "series.path",
        media_root.path().to_string_lossy().as_ref(),
    )
    .await;

    let title_dir = media_root.path().join("Harbor Pals");
    std::fs::create_dir_all(&title_dir).expect("create title directory");
    let title = seed_series_title(
        &ctx,
        "title-multi-file-release-dir",
        "Harbor Pals",
        MediaFacet::Series,
        media_root.path(),
        Some(&title_dir),
        None,
    )
    .await;
    let season = seed_collection(&ctx, &title, 1).await;
    let _first_episode = seed_episode(&ctx, &title, &season, 1).await;
    let _second_episode = seed_episode(&ctx, &title, &season, 2).await;

    let release_dir = title_dir.join("Harbor.Pals.S01E01.720p.WEB-DL.AV1.AAC2.0-NTb");
    std::fs::create_dir_all(&release_dir).expect("create release directory");
    let first_file = release_dir.join("4f8e2c7a91b6d3e0.mkv");
    let second_file = release_dir.join("9dbe2170c4aa87f1.mkv");
    write_fake_media_file(&first_file);
    write_fake_media_file(&second_file);

    let actor = admin();
    set_default_library_root(&ctx, MediaFacet::Series, media_root.path()).await;
    ctx.app
        .scan_library(&actor, MediaFacet::Series)
        .await
        .expect("full library scan");

    let pending = ctx
        .app
        .pending_imports(
            &actor,
            MediaFacet::Series,
            None,
            PendingImportStatus::Pending,
            20,
            0,
        )
        .await
        .expect("list pending imports after scan");
    assert_eq!(pending.total, 2);

    let media_files = ctx
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list title media files");
    assert!(media_files.is_empty());
}

#[tokio::test]
async fn resolve_pending_import_creates_title_and_clears_movie_row_without_scanning() {
    let ctx = TestContext::new().await;
    seed_media_path_settings(&ctx).await;

    let media_root = tempfile::tempdir().expect("movie root");
    set_media_path(
        &ctx,
        "movies.path",
        media_root.path().to_string_lossy().as_ref(),
    )
    .await;
    set_default_library_root(&ctx, MediaFacet::Movie, media_root.path()).await;

    let missing_movie_file = media_root.path().join("Fresh.Match.2026.mkv");
    let now = chrono::Utc::now().to_rfc3339();
    let pending_item = LibraryScanUnmatchedItem {
        id: Id::new().0,
        library_id: scryer_domain::default_library_id_for_facet(&MediaFacet::Movie),
        facet: MediaFacet::Movie,
        status: PendingImportStatus::Ignored,
        title_id: None,
        scan_session_id: "test-session".to_string(),
        scan_root: media_root.path().to_string_lossy().to_string(),
        item_path: missing_movie_file.to_string_lossy().to_string(),
        display_name: "Fresh.Match.2026".to_string(),
        query: "Fresh Match".to_string(),
        year_hint: Some(2026),
        reason_code: "test_match_without_scan".to_string(),
        error_message: None,
        search_attempts: Vec::new(),
        size_bytes: None,
        created_at: now.clone(),
        updated_at: now,
    };
    let pending_id = ctx
        .library_scan_unmatched
        .upsert_library_scan_unmatched_item(&pending_item)
        .await
        .expect("insert pending import");

    let actor = admin();
    let result = ctx
        .app
        .resolve_pending_import(
            &actor,
            &pending_id,
            pending_import_title_request(MediaFacet::Movie, "Fresh Match", "987654"),
            false,
        )
        .await
        .expect("resolve pending import");

    assert!(result.created);
    assert!(result.library_scan.is_none());

    let unmatched = ctx
        .library_scan_unmatched
        .get_library_scan_unmatched_item(&pending_id)
        .await
        .expect("load pending import after resolve");
    assert!(
        unmatched.is_none(),
        "movie pending import row should be cleared after match"
    );

    let media_files = ctx
        .media_files
        .list_media_files_for_title(&result.title.id)
        .await
        .expect("list title media files");
    assert!(media_files.is_empty());
}

#[tokio::test]
async fn resolve_pending_import_rejects_stale_movie_row_already_bound_to_title() {
    let ctx = TestContext::new().await;
    seed_media_path_settings(&ctx).await;

    let media_root = tempfile::tempdir().expect("movie root");
    set_media_path(
        &ctx,
        "movies.path",
        media_root.path().to_string_lossy().as_ref(),
    )
    .await;
    set_default_library_root(&ctx, MediaFacet::Movie, media_root.path()).await;

    let movie_dir = media_root.path().join("Emberline (2010)");
    std::fs::create_dir_all(&movie_dir).expect("create movie dir");
    let movie_file = movie_dir.join("Totally.Wrong.Name.2010.mkv");
    write_fake_media_file(&movie_file);

    let title = seed_series_title(
        &ctx,
        "title-emberline",
        "Emberline",
        MediaFacet::Movie,
        media_root.path(),
        Some(&movie_dir),
        Some("123456"),
    )
    .await;

    ctx.media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: movie_file.to_string_lossy().to_string(),
            size_bytes: 24,
            announced_size_bytes: None,
            role: MediaFileRole::Primary,
            source_signature_scheme: None,
            source_signature_value: None,
            quality_label: Some("1080p".to_string()),
            scene_name: None,
            release_group: None,
            source_type: None,
            resolution: None,
            video_codec_parsed: None,
            audio_codec_parsed: None,
            audio_channels_parsed: None,
            acquisition_score: None,
            scoring_log: None,
            indexer_source: None,
            grabbed_release_title: None,
            grabbed_at: None,
            edition: None,
            original_file_path: None,
            release_hash: None,
        })
        .await
        .expect("insert movie media file");

    let now = chrono::Utc::now().to_rfc3339();
    let pending_item = LibraryScanUnmatchedItem {
        id: Id::new().0,
        library_id: scryer_domain::default_library_id_for_facet(&MediaFacet::Movie),
        facet: MediaFacet::Movie,
        status: PendingImportStatus::Pending,
        title_id: None,
        scan_session_id: "test-session".to_string(),
        scan_root: media_root.path().to_string_lossy().to_string(),
        item_path: movie_file.to_string_lossy().to_string(),
        display_name: "Totally.Wrong.Name.2010".to_string(),
        query: "Emberline".to_string(),
        year_hint: Some(2010),
        reason_code: "stale_duplicate_pending_import".to_string(),
        error_message: None,
        search_attempts: Vec::new(),
        size_bytes: None,
        created_at: now.clone(),
        updated_at: now,
    };
    let pending_id = ctx
        .library_scan_unmatched
        .upsert_library_scan_unmatched_item(&pending_item)
        .await
        .expect("insert pending import");

    let actor = admin();
    let error = ctx
        .app
        .resolve_pending_import(
            &actor,
            &pending_id,
            pending_import_title_request(MediaFacet::Movie, "Emberline", "123456"),
            false,
        )
        .await
        .expect_err("stale pending import should be rejected when title already exists");
    assert!(
        error
            .to_string()
            .contains("title already exists in this library")
    );

    let unmatched = ctx
        .library_scan_unmatched
        .get_library_scan_unmatched_item(&pending_id)
        .await
        .expect("load pending import after resolve");
    assert!(
        unmatched.is_some(),
        "stale pending import row should stay until explicit bind or ignore"
    );

    let media_files = ctx
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list movie media files");
    assert_eq!(media_files.len(), 1);
    assert_eq!(media_files[0].file_path, movie_file.to_string_lossy());
}

#[tokio::test]
async fn resolving_existing_title_pending_import_does_not_clear_existing_title_folder_path() {
    let ctx = TestContext::new().await;
    seed_media_path_settings(&ctx).await;

    let media_root = tempfile::tempdir().expect("series root");
    set_media_path(
        &ctx,
        "series.path",
        media_root.path().to_string_lossy().as_ref(),
    )
    .await;

    let title_dir = media_root.path().join("Existing Folder Show");
    std::fs::create_dir_all(&title_dir).expect("create title directory");
    let title = seed_series_title(
        &ctx,
        "title-existing-folder",
        "Existing Folder Show",
        MediaFacet::Series,
        media_root.path(),
        Some(&title_dir),
        Some("123456"),
    )
    .await;
    let missing_file = media_root.path().join("Existing.Folder.Show.S01E01.mkv");
    let now = chrono::Utc::now().to_rfc3339();
    let pending_item = LibraryScanUnmatchedItem {
        id: Id::new().0,
        library_id: scryer_domain::default_library_id_for_facet(&MediaFacet::Series),
        facet: MediaFacet::Series,
        status: PendingImportStatus::Pending,
        title_id: None,
        scan_session_id: "test-session".to_string(),
        scan_root: media_root.path().to_string_lossy().to_string(),
        item_path: missing_file.to_string_lossy().to_string(),
        display_name: "Existing.Folder.Show.S01E01".to_string(),
        query: "Existing Folder Show".to_string(),
        year_hint: None,
        reason_code: "test_missing_loose_file".to_string(),
        error_message: None,
        search_attempts: Vec::new(),
        size_bytes: None,
        created_at: now.clone(),
        updated_at: now,
    };
    let pending_id = ctx
        .library_scan_unmatched
        .upsert_library_scan_unmatched_item(&pending_item)
        .await
        .expect("insert unmatched item");

    let actor = admin();
    let error = ctx
        .app
        .resolve_pending_import(
            &actor,
            &pending_id,
            pending_import_title_request(MediaFacet::Series, "Existing Folder Show", "123456"),
            false,
        )
        .await
        .expect_err("existing title should not resolve through match");
    assert!(
        error
            .to_string()
            .contains("title already exists in this library")
    );

    let refreshed_title = ctx
        .titles
        .get_by_id(&title.id)
        .await
        .expect("load refreshed title")
        .expect("title should exist");
    assert_eq!(
        refreshed_title.folder_path.as_deref(),
        Some(title_dir.to_string_lossy().as_ref())
    );
}

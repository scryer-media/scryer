#![recursion_limit = "256"]

mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use common::TestContext;
use scryer_application::testing::AppUseCaseTestExt;
use scryer_application::{
    AcquisitionScopeStateRepository, BlocklistRepository, ClientJobLocator,
    DownloadClientConfigRepository, DownloadSubmission, DownloadSubmissionPurpose,
    DownloadSubmissionRepository, ImportArtifactRepository, ImportRepository, LibraryRepository,
    LibraryRootDraft, MediaFileRepository, ReleaseAttemptRepository, SaveQualityProfileSettings,
    ShowRepository, SubmissionScope, TitleRepository, import_completed_download,
};
use scryer_domain::{
    Collection, CompletedDownload, DownloadClientConfig, DownloadClientStatus, Episode, Id,
    ImportDecision, ImportSkipReason, MediaFacet, Title,
};
use scryer_infrastructure_acquisition::downloads::config_store::DownloadClientConfigStore;
use scryer_infrastructure_sql::types::SettingDefinitionSeed;
use scryer_infrastructure_workflow::workflow::{
    file_importer::FsFileImporter,
    stores::{DownloadSubmissionStore, ImportStore},
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build an AppUseCase with a real SQLite import repository and filesystem
/// file importer and production-equivalent quality-profile configuration so
/// that tests can exercise the full import pipeline.
async fn app_with_real_imports(ctx: &TestContext) -> scryer_application::AppUseCase {
    ctx.settings_store
        .batch_ensure_setting_definitions(vec![
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "quality.profiles".into(),
                data_type: "json".into(),
                default_value_json: "[]".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "quality.profile_id".into(),
                data_type: "string".into(),
                default_value_json: "\"1080p\"".into(),
                is_sensitive: false,
                validation_json: None,
            },
        ])
        .await
        .expect("seed import quality-profile setting definitions");
    let user = ctx
        .app
        .find_or_create_default_user()
        .await
        .expect("create import test user");
    ctx.app
        .save_quality_profile_settings(
            &user,
            SaveQualityProfileSettings {
                profiles: vec![
                    scryer_application::builtin_default_quality_profile(),
                    scryer_application::builtin_4k_profile(),
                ],
                replace_existing: true,
                global_profile_id: Some(
                    scryer_application::BUILTIN_DEFAULT_QUALITY_PROFILE_ID.to_string(),
                ),
                category_selections: Vec::new(),
                global_scoring_persona: None,
                category_persona_selections: Vec::new(),
            },
        )
        .await
        .expect("seed import quality-profile configuration");

    let workflow_store = Arc::new(ImportStore::new(ctx.db.datastore()));
    ctx.app.with_test_overrides(|builder| {
        builder
            .with_imports(workflow_store)
            .with_file_importer(Arc::new(FsFileImporter::new()))
            .with_media_files(Arc::new(ctx.media_files.clone()))
            .with_acquisition_scope_states(Arc::new(ctx.library_state.clone()))
    })
}

/// Build a minimal CompletedDownload with scryer-origin parameters.
fn scryer_completed(
    item_id: &str,
    dest_dir: &str,
    title_id: &str,
    facet_id: &str,
) -> CompletedDownload {
    CompletedDownload {
        client_type: "nzbget".to_string(),
        client_id: "test-client".to_string(),
        download_client_item_id: item_id.to_string(),
        download_id: None,
        name: format!("Test.Download.{item_id}"),
        release_name: None,
        dest_dir: dest_dir.to_string(),
        category: None,
        size_bytes: None,
        completed_at: None,
        parameters: vec![
            ("*scryer_title_id".to_string(), title_id.to_string()),
            ("*scryer_facet".to_string(), facet_id.to_string()),
        ],
    }
}

async fn queue_import_record(ctx: &TestContext, completed: &CompletedDownload) -> String {
    ImportStore::new(ctx.db.datastore())
        .queue_import_request(
            ClientJobLocator::for_import_artifact(
                Some(&completed.client_id),
                &completed.client_type,
                &completed.download_client_item_id,
            ),
            "manual_import".to_string(),
            "{}".to_string(),
        )
        .await
        .expect("queue import record")
}

/// Record the durable grab-time submission a Scryer download carries in
/// production: the identity of `completed` bound to `title` with the indexer
/// release title. This is the release evidence import parses, scores, and —
/// on rejection — blocklists, so a rejected release can be recognised when the
/// indexer offers it again (a blocklist keyed by the client's display label
/// never could).
async fn record_movie_grab_submission(
    ctx: &TestContext,
    completed: &CompletedDownload,
    title: &Title,
    source_title: &str,
) -> scryer_domain::download_identity::DownloadId {
    let download_id = scryer_domain::download_identity::DownloadId::new();
    DownloadSubmissionStore::new(ctx.db.datastore())
        .record_submission(DownloadSubmission {
            download_id,
            title_id: title.id.clone(),
            facet: "movie".to_string(),
            download_client_id: Some(completed.client_id.clone()),
            download_client_type: completed.client_type.clone(),
            download_client_item_id: completed.download_client_item_id.clone(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: Some(source_title.to_string()),
            info_hash: None,
            release_size_bytes: None,
            request_signature: None,
            purpose: DownloadSubmissionPurpose::Standard,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("record grab submission");
    download_id
}

async fn configure_default_library_root(
    ctx: &TestContext,
    facet: MediaFacet,
    media_root: &str,
) -> String {
    let library_id = scryer_domain::default_library_id_for_facet(&facet);
    let library = ctx
        .libraries
        .get_by_id(&library_id)
        .await
        .expect("default library should load")
        .expect("default library should exist");
    let updated = ctx
        .libraries
        .update(
            &library_id,
            library.name,
            library.slug,
            vec![LibraryRootDraft {
                path: media_root.to_string(),
                is_default: true,
            }],
        )
        .await
        .expect("default library root should update");
    // Root ids are allocated, not derived from the path, so read the stored id back.
    updated
        .roots
        .iter()
        .find(|root| root.is_default)
        .or_else(|| updated.roots.first())
        .map(|root| root.id.clone())
        .expect("configured library should expose its root")
}

/// Add a minimal movie Title to the DB with `media_root` configured as the
/// destination library folder.
async fn add_movie_title(ctx: &TestContext, id: &str, name: &str, media_root: &str) -> Title {
    let root_folder_id = configure_default_library_root(ctx, MediaFacet::Movie, media_root).await;
    let title = Title {
        id: id.to_string(),
        name: name.to_string(),
        facet: MediaFacet::Movie,
        library_id: scryer_domain::default_library_id_for_facet(&MediaFacet::Movie),
        monitored: true,
        tags: vec![],
        canonical_tags: vec![],
        external_ids: vec![],
        root_folder_id,
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
        // These integration fixtures use tiny synthetic videos; mark them as
        // short-form so runtime-sample validation does not preempt unrelated
        // import-path, rule, dedupe, or symlink assertions.
        runtime_minutes: Some(1),
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
        folder_path: None,
    };
    ctx.titles.create(title).await.expect("add movie title")
}

async fn add_series_title(ctx: &TestContext, id: &str, name: &str, media_root: &str) -> Title {
    add_series_title_with_runtime(ctx, id, name, media_root, Some(24)).await
}

async fn add_short_form_series_title(
    ctx: &TestContext,
    id: &str,
    name: &str,
    media_root: &str,
) -> Title {
    add_series_title_with_runtime(ctx, id, name, media_root, Some(1)).await
}

async fn add_series_title_with_runtime(
    ctx: &TestContext,
    id: &str,
    name: &str,
    media_root: &str,
    runtime_minutes: Option<i32>,
) -> Title {
    let root_folder_id = configure_default_library_root(ctx, MediaFacet::Series, media_root).await;
    let title = Title {
        id: id.to_string(),
        name: name.to_string(),
        facet: MediaFacet::Series,
        library_id: scryer_domain::default_library_id_for_facet(&MediaFacet::Series),
        monitored: true,
        tags: vec![],
        canonical_tags: vec![],
        external_ids: vec![],
        root_folder_id,
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
        runtime_minutes,
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
        folder_path: None,
    };
    ctx.titles.create(title).await.expect("add series title")
}

async fn ensure_folder_template_setting_definition(ctx: &TestContext) {
    ctx.settings_store
        .batch_ensure_setting_definitions(vec![SettingDefinitionSeed {
            category: "media".into(),
            scope: "system".into(),
            key_name: "folder.template".into(),
            data_type: "string".into(),
            default_value_json: "null".into(),
            is_sensitive: false,
            validation_json: None,
        }])
        .await
        .expect("seed folder template setting definition");
}

async fn set_folder_template(ctx: &TestContext, facet: MediaFacet, template: &str) {
    ensure_folder_template_setting_definition(ctx).await;
    let actor = ctx.app.find_or_create_default_user().await.unwrap();
    ctx.app
        .update_media_settings(
            &actor,
            facet,
            scryer_application::UpdateMediaSettings {
                library_path: None,
                root_folders: None,
                required_audio_languages: None,
                folder_template: Some(template.to_string()),
                season_folder_template: None,
                specials_folder_template: None,
                use_season_folders: None,
                rename_enabled: None,
                rename_template: None,
                rename_collision_policy: None,
                rename_missing_metadata_policy: None,
                filler_policy: None,
                recap_policy: None,
                monitor_specials: None,
                inter_season_movies: None,
                monitor_filler_movies: None,
                nfo_write_on_import: None,
                plexmatch_write_on_import: None,
                import_mode: None,
                set_permissions_linux: None,
                file_chmod: None,
                folder_chmod: None,
                chown_group: None,
            },
        )
        .await
        .expect("update folder template");
}

fn mediainfo_fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("scryer-mediainfo")
        .join("tests")
        .join("media")
        .join(name)
}

fn copy_fixture(dest_dir: &Path, fixture_name: &str, dest_name: &str) -> PathBuf {
    let dest = dest_dir.join(dest_name);
    std::fs::copy(mediainfo_fixture(fixture_name), &dest).expect("copy fixture");
    dest
}

async fn seed_movie_wanted_item(
    ctx: &TestContext,
    title_id: &str,
    status: scryer_application::AcquisitionScopeStatus,
) -> scryer_application::AcquisitionScopeState {
    let item = scryer_application::AcquisitionScopeState {
        id: Id::new().0,
        title_id: title_id.to_string(),
        title_name: Some("Test Title".to_string()),
        title_slug: None,
        title_facet: Some("movie".to_string()),
        library_id: None,
        library_name: None,
        library_slug: None,
        episode_id: None,
        collection_id: None,
        series_movie_link_id: None,
        season_number: None,
        episode_number: None,
        media_type: "movie".to_string(),
        last_search_at: None,
        status,
        grabbed_release: None,
        landed_bar: None,
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: chrono::Utc::now().to_rfc3339(),
        updated_at: chrono::Utc::now().to_rfc3339(),
    };
    ctx.library_state
        .upsert_acquisition_scope_state(&item)
        .await
        .expect("seed movie wanted");
    item
}

async fn seed_series_episode(ctx: &TestContext, title: &Title) -> Episode {
    seed_series_episode_with_duration(ctx, title, Some(1440)).await
}

async fn seed_short_form_series_episode(ctx: &TestContext, title: &Title) -> Episode {
    seed_series_episode_with_duration(ctx, title, Some(60)).await
}

async fn seed_series_episode_with_duration(
    ctx: &TestContext,
    title: &Title,
    duration_seconds: Option<i64>,
) -> Episode {
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
        monitored: true,
        created_at: chrono::Utc::now(),
    };
    ctx.shows
        .create_collection(collection.clone())
        .await
        .expect("create collection");

    let episode = Episode {
        id: Id::new().0,
        title_id: title.id.clone(),
        collection_id: Some(collection.id.clone()),
        episode_type: scryer_domain::EpisodeType::Standard,
        episode_number: Some("1".to_string()),
        season_number: Some("1".to_string()),
        episode_label: Some("S01E01".to_string()),
        title: Some("Pilot".to_string()),
        air_date: None,
        duration_seconds,
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

async fn seed_episode_wanted_item(
    ctx: &TestContext,
    title: &Title,
    episode: &Episode,
    status: scryer_application::AcquisitionScopeStatus,
) -> scryer_application::AcquisitionScopeState {
    let item = scryer_application::AcquisitionScopeState {
        id: Id::new().0,
        title_id: title.id.clone(),
        title_name: Some(title.name.clone()),
        title_slug: title.slug.clone(),
        title_facet: Some(title.facet.as_str().to_string()),
        library_id: None,
        library_name: None,
        library_slug: None,
        episode_id: Some(episode.id.clone()),
        collection_id: None,
        series_movie_link_id: None,
        season_number: Some("1".to_string()),
        episode_number: episode.episode_number.clone(),
        media_type: "series".to_string(),
        last_search_at: None,
        status,
        grabbed_release: None,
        landed_bar: None,
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: chrono::Utc::now().to_rfc3339(),
        updated_at: chrono::Utc::now().to_rfc3339(),
    };
    ctx.library_state
        .upsert_acquisition_scope_state(&item)
        .await
        .expect("seed episode wanted");
    item
}

async fn install_rule(
    app: &scryer_application::AppUseCase,
    user: &scryer_domain::User,
    rego_source: &str,
    applied_facets: Vec<MediaFacet>,
) {
    app.create_rule_set(
        user,
        "Test Rule".to_string(),
        "integration test".to_string(),
        rego_source.to_string(),
        applied_facets,
        0,
        None,
    )
    .await
    .expect("create rule set");
}

fn pad_file_past_series_sample_threshold(path: &Path) {
    use std::io::{Seek, SeekFrom, Write};

    let target_len = 52 * 1024 * 1024;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .expect("open fixture for padding");
    file.seek(SeekFrom::Start(target_len))
        .expect("seek fixture");
    file.write_all(&[0])
        .expect("extend fixture beyond sample threshold");
}

async fn seed_second_series_episode(
    ctx: &TestContext,
    title: &Title,
    collection_id: &str,
) -> Episode {
    seed_series_episode_in_collection(ctx, title, collection_id, 2, Some(1440)).await
}

async fn seed_second_short_form_series_episode(
    ctx: &TestContext,
    title: &Title,
    collection_id: &str,
) -> Episode {
    seed_series_episode_in_collection(ctx, title, collection_id, 2, Some(60)).await
}

async fn seed_series_episode_in_collection(
    ctx: &TestContext,
    title: &Title,
    collection_id: &str,
    episode_number: u32,
    duration_seconds: Option<i64>,
) -> Episode {
    let episode = Episode {
        id: Id::new().0,
        title_id: title.id.clone(),
        collection_id: Some(collection_id.to_string()),
        episode_type: scryer_domain::EpisodeType::Standard,
        episode_number: Some(episode_number.to_string()),
        season_number: Some("1".to_string()),
        episode_label: Some(format!("S01E{episode_number:02}")),
        title: Some(format!("Episode {episode_number}")),
        air_date: None,
        duration_seconds,
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
        .expect("create seeded episode");
    episode
}

async fn seed_series_movie_link(
    ctx: &TestContext,
    title: &Title,
) -> scryer_domain::SeriesMovieLink {
    let now = chrono::Utc::now();
    let link = scryer_domain::SeriesMovieLink {
        id: Id::new().0,
        series_title_id: title.id.clone(),
        movie: scryer_domain::MovieEntity {
            id: Id::new().0,
            title: format!("{} Movie", title.name),
            sort_title: None,
            slug: None,
            year: Some(2024),
            overview: None,
            poster_url: None,
            background_url: None,
            language: None,
            runtime_minutes: Some(90),
            content_status: Some("released".to_string()),
            studio: None,
            digital_release_date: None,
            imdb_id: None,
            tvdb_id: None,
            tmdb_id: None,
            mal_id: None,
            anidb_id: None,
            ratings: None,
            credits: None,
            created_at: now,
            updated_at: now,
        },
        placement: None,
        narrative_order: None,
        after_season: None,
        before_season: None,
        linked_episode_id: None,
        association_confidence: None,
        continuity_status: None,
        movie_form: Some("movie".to_string()),
        confidence: None,
        signal_summary: None,
        source: Some("test".to_string()),
        monitoring_override: None,
        metadata_active: true,
        monitored: true,
        legacy_collection_id: None,
        created_at: now,
        updated_at: now,
    };
    ctx.shows
        .upsert_series_movie_link(link.clone())
        .await
        .expect("create seeded series-movie link");
    link
}

#[test]
fn completed_download_series_import_stack_subprocess_probe() {
    let exe = std::env::current_exe().expect("resolve current test executable");
    let mut child = Command::new(exe)
        .arg("--exact")
        .arg("completed_download_series_import_stack_subprocess_probe_child")
        .arg("--ignored")
        .arg("--nocapture")
        .env("RUST_TEST_THREADS", "1")
        .env("RUST_BACKTRACE", "1")
        .env("SCRYER_COMPLETED_IMPORT_STACK_PROBE_CHILD", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn completed import stack probe");

    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        if child
            .try_wait()
            .expect("poll completed import stack probe status")
            .is_some()
        {
            let output = child
                .wait_with_output()
                .expect("collect completed import stack probe output");
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(
                output.status.success(),
                "completed import stack probe child failed\nstatus: {:?}\nstdout:\n{}\nstderr:\n{}",
                output.status,
                stdout,
                stderr
            );
            return;
        }

        if Instant::now() >= deadline {
            let _ = child.kill();
            let output = child
                .wait_with_output()
                .expect("collect timed-out completed import stack probe output");
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            panic!(
                "completed import stack probe child timed out after {}s\nstdout:\n{}\nstderr:\n{}",
                deadline.elapsed().as_secs(),
                stdout,
                stderr
            );
        }

        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
#[ignore = "subprocess-only completed import stack probe child"]
fn completed_download_series_import_stack_subprocess_probe_child() {
    if std::env::var_os("SCRYER_COMPLETED_IMPORT_STACK_PROBE_CHILD").is_none() {
        return;
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(4)
        .build()
        .expect("build tokio runtime for completed import stack probe");

    runtime.block_on(async {
        tokio::time::timeout(
            Duration::from_secs(60),
            run_completed_download_series_import_stack_probe(),
        )
        .await
        .expect("completed import stack probe should finish within timeout");
    });
}

async fn run_completed_download_series_import_stack_probe() {
    let ctx = TestContext::new().await;
    let app = app_with_real_imports(&ctx).await;
    let user = ctx.app.find_or_create_default_user().await.unwrap();

    let source_dir = tempfile::tempdir().expect("source tempdir");
    let source_video = copy_fixture(
        source_dir.path(),
        "h264_aac.mkv",
        "Bluey.S01E01.720p.WEB-DL.H264.AAC2.0.mkv",
    );
    pad_file_past_series_sample_threshold(&source_video);

    let dest_root = tempfile::tempdir().expect("dest tempdir");
    let title = add_short_form_series_title(
        &ctx,
        "title-series-completed-import-stack-probe",
        "Bluey",
        dest_root.path().to_str().unwrap(),
    )
    .await;
    let episode = seed_short_form_series_episode(&ctx, &title).await;

    let completed = scryer_completed(
        "dl-series-completed-import-stack-probe",
        source_dir.path().to_str().unwrap(),
        &title.id,
        "series",
    );

    let result = import_completed_download(&app, &user, &completed)
        .await
        .expect("completed download series import");

    assert_eq!(
        result.decision,
        ImportDecision::Imported,
        "unexpected completed import result: {result:?}"
    );
    let media_files = ctx
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list imported media files");
    assert_eq!(media_files.len(), 1);
    assert_eq!(
        media_files[0].episode_id.as_deref(),
        Some(episode.id.as_str())
    );
}

// ---------------------------------------------------------------------------
// Deduplication
// ---------------------------------------------------------------------------

/// Directly mark a download as "completed" in the import repository, then
/// attempt to import the same download again.  The second call should be
/// short-circuited as AlreadyImported without re-running the pipeline.
#[tokio::test]
async fn import_returns_unmatched_when_title_not_found() {
    let ctx = TestContext::new().await;
    let app = app_with_real_imports(&ctx).await;
    let user = ctx.app.find_or_create_default_user().await.unwrap();

    let source_dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        source_dir.path().join("Unknown.Movie.2024.mkv"),
        b"fake video",
    )
    .expect("write");

    let completed = CompletedDownload {
        client_type: "nzbget".to_string(),
        client_id: "test-client".to_string(),
        download_client_item_id: "dl-no-title".to_string(),
        download_id: None,
        name: "Unknown.Movie.2024".to_string(),
        release_name: None,
        dest_dir: source_dir.path().to_str().unwrap().to_string(),
        category: None,
        size_bytes: None,
        completed_at: None,
        parameters: vec![("*scryer_title_id".to_string(), "nonexistent-id".to_string())],
    };

    let result = import_completed_download(&app, &user, &completed)
        .await
        .expect("import_completed_download");

    assert_eq!(result.decision, ImportDecision::Unmatched);
    assert_eq!(
        result.skip_reason,
        Some(ImportSkipReason::UnresolvedIdentity)
    );
}

// ---------------------------------------------------------------------------
// Video file detection
// ---------------------------------------------------------------------------

#[tokio::test]
async fn import_fails_when_no_video_files_in_dest_dir() {
    let ctx = TestContext::new().await;
    let app = app_with_real_imports(&ctx).await;
    let user = ctx.app.find_or_create_default_user().await.unwrap();

    // Source dir exists but contains only a text file — no video files.
    let source_dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(source_dir.path().join("readme.txt"), b"no video here").expect("write");

    let dest_dir = tempfile::tempdir().expect("tempdir");
    let title = add_movie_title(
        &ctx,
        "title-no-video",
        "No Video Movie",
        dest_dir.path().to_str().unwrap(),
    )
    .await;

    let completed = scryer_completed(
        "dl-no-video",
        source_dir.path().to_str().unwrap(),
        &title.id,
        "movie",
    );

    let result = import_completed_download(&app, &user, &completed)
        .await
        .expect("import_completed_download");

    assert_eq!(result.decision, ImportDecision::Skipped);
    assert_eq!(result.skip_reason, Some(ImportSkipReason::NoVideoFiles));
}

#[tokio::test]
async fn import_movie_strm_file_is_treated_as_video_artifact() {
    let ctx = TestContext::new().await;
    let app = app_with_real_imports(&ctx).await;
    let user = ctx.app.find_or_create_default_user().await.unwrap();

    let source_dir = tempfile::tempdir().expect("source tempdir");
    let strm = source_dir
        .path()
        .join("Test.Movie.2024.1080p.WEB-DL.H264.strm");
    std::fs::write(
        &strm,
        b"https://nzbdav.example/stream/Test.Movie.2024.1080p.WEB-DL.H264",
    )
    .expect("write strm");

    let dest_root = tempfile::tempdir().expect("dest tempdir");
    let title = add_movie_title(
        &ctx,
        "title-movie-strm-1",
        "Test Movie",
        dest_root.path().to_str().unwrap(),
    )
    .await;

    let completed = scryer_completed(
        "dl-movie-strm-1",
        source_dir.path().to_str().unwrap(),
        &title.id,
        "movie",
    );

    let result = import_completed_download(&app, &user, &completed)
        .await
        .expect("import_completed_download");

    assert_eq!(result.decision, ImportDecision::Imported);
    let dest_path = result.dest_path.expect("dest path");
    assert!(dest_path.ends_with(".strm"));
    assert!(std::path::Path::new(&dest_path).exists());
    assert_eq!(
        std::fs::read_to_string(&dest_path).expect("read imported strm"),
        "https://nzbdav.example/stream/Test.Movie.2024.1080p.WEB-DL.H264"
    );

    let media_files = ctx
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files");
    assert_eq!(media_files.len(), 1);
    assert_eq!(media_files[0].scan_status, "scanned");
    assert_eq!(media_files[0].container_format.as_deref(), Some("strm"));
    assert_eq!(media_files[0].video_height, Some(1080));
    assert_eq!(media_files[0].video_width, Some(1920));
    assert_eq!(media_files[0].video_codec, None);
}

#[tokio::test]
async fn import_movie_rejection_does_not_persist_title_folder_path() {
    let ctx = TestContext::new().await;
    let app = app_with_real_imports(&ctx).await;
    let user = ctx.app.find_or_create_default_user().await.unwrap();

    let source_dir = tempfile::tempdir().expect("source tempdir");
    copy_fixture(
        source_dir.path(),
        "h264_aac.mkv",
        "Rejected.Movie.2025.1080p.WEB-DL.sample.mkv",
    );

    let dest_root = tempfile::tempdir().expect("dest tempdir");
    let title = add_movie_title(
        &ctx,
        "title-movie-reject-folder-path",
        "Rejected Movie",
        dest_root.path().to_str().unwrap(),
    )
    .await;

    let completed = scryer_completed(
        "dl-movie-reject-folder-path",
        source_dir.path().to_str().unwrap(),
        &title.id,
        "movie",
    );

    let result = import_completed_download(&app, &user, &completed)
        .await
        .expect("import_completed_download");

    assert_ne!(result.decision, ImportDecision::Imported);
    let updated_title = ctx
        .titles
        .get_by_id(&title.id)
        .await
        .expect("load title")
        .expect("title");
    assert_eq!(updated_title.folder_path, None);
}

#[cfg(unix)]
#[tokio::test]
async fn import_movie_symlink_file_preserves_symlink_and_media_analysis() {
    let ctx = TestContext::new().await;
    let app = app_with_real_imports(&ctx).await;
    let user = ctx.app.find_or_create_default_user().await.unwrap();

    let backing_dir = tempfile::tempdir().expect("backing tempdir");
    let backing_video = copy_fixture(
        backing_dir.path(),
        "h264_aac.mkv",
        "Test.Movie.2024.2160p.WEB-DL.H265.DDP5.1.Atmos.mkv",
    );

    let source_dir = tempfile::tempdir().expect("source tempdir");
    let source_link = source_dir
        .path()
        .join("Test.Movie.2024.2160p.WEB-DL.H265.DDP5.1.Atmos.mkv");
    std::os::unix::fs::symlink(&backing_video, &source_link).expect("create source symlink");

    let dest_root = tempfile::tempdir().expect("dest tempdir");
    let title = add_movie_title(
        &ctx,
        "title-movie-symlink-1",
        "Test Movie",
        dest_root.path().to_str().unwrap(),
    )
    .await;

    let completed = scryer_completed(
        "dl-movie-symlink-1",
        source_dir.path().to_str().unwrap(),
        &title.id,
        "movie",
    );

    let result = import_completed_download(&app, &user, &completed)
        .await
        .expect("import_completed_download");

    assert_eq!(result.decision, ImportDecision::Imported);
    assert_eq!(
        result.link_type.map(|strategy| strategy.as_str()),
        Some("symlink")
    );
    let dest_path = result.dest_path.expect("dest path");
    assert!(
        std::fs::symlink_metadata(&dest_path)
            .expect("dest metadata")
            .file_type()
            .is_symlink()
    );

    let media_files = ctx
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files");
    assert_eq!(media_files.len(), 1);
    assert_eq!(media_files[0].video_height, Some(72));
    assert_eq!(media_files[0].video_width, Some(128));
    assert_eq!(
        media_files[0].video_codec.as_ref(),
        Some(&scryer_application::VideoCodec::H264)
    );
    assert_eq!(media_files[0].audio_codec.as_deref(), Some("aac"));
    assert_eq!(media_files[0].audio_channels, Some(2));
}

#[cfg(unix)]
#[tokio::test]
async fn import_movie_decypharr_symlink_release_folder_succeeds() {
    let ctx = TestContext::new().await;
    let app = app_with_real_imports(&ctx).await;
    let user = ctx.app.find_or_create_default_user().await.unwrap();

    let release_name =
        "Harbor.Pilot.and.the.Keeper.of.Portmere.2004.BluRay.1080p.AV1.Opus-nAV1gator";
    let file_name = format!("{release_name}.mkv");

    let backing_dir = tempfile::tempdir().expect("backing tempdir");
    let backing_video = copy_fixture(backing_dir.path(), "h264_aac.mkv", &file_name);

    let symlink_root = tempfile::tempdir().expect("symlink root tempdir");
    let release_dir = symlink_root.path().join("radarr").join(release_name);
    std::fs::create_dir_all(&release_dir).expect("create release dir");
    let source_link = release_dir.join(&file_name);
    std::os::unix::fs::symlink(&backing_video, &source_link).expect("create source symlink");

    let dest_root = tempfile::tempdir().expect("dest tempdir");
    let title = add_movie_title(
        &ctx,
        "title-movie-decypharr-1",
        "Harbor Pilot and the Keeper of Portmere",
        dest_root.path().to_str().unwrap(),
    )
    .await;

    let completed = CompletedDownload {
        client_type: "qbittorrent".to_string(),
        client_id: "test-client".to_string(),
        download_client_item_id: "dl-movie-decypharr-1".to_string(),
        download_id: None,
        name: release_name.to_string(),
        release_name: Some(release_name.to_string()),
        dest_dir: release_dir.to_string_lossy().to_string(),
        category: Some("radarr".to_string()),
        size_bytes: None,
        completed_at: None,
        parameters: vec![
            ("*scryer_title_id".to_string(), title.id.clone()),
            ("*scryer_facet".to_string(), "movie".to_string()),
        ],
    };

    let result = import_completed_download(&app, &user, &completed)
        .await
        .expect("import_completed_download");

    assert_eq!(result.decision, ImportDecision::Imported);
    assert_eq!(
        result.link_type.map(|strategy| strategy.as_str()),
        Some("symlink")
    );
    let dest_path = result.dest_path.expect("dest path");
    assert!(
        std::fs::symlink_metadata(&dest_path)
            .expect("dest metadata")
            .file_type()
            .is_symlink()
    );
}

#[cfg(unix)]
#[tokio::test]
async fn import_movie_decypharr_symlink_release_folder_uses_remote_path_mapping() {
    let ctx = TestContext::new().await;
    let app = app_with_real_imports(&ctx).await;
    let user = ctx.app.find_or_create_default_user().await.unwrap();

    let release_name =
        "Harbor.Pilot.and.the.Keeper.of.Portmere.2004.BluRay.1080p.AV1.Opus-nAV1gator";
    let file_name = format!("{release_name}.mkv");

    let backing_dir = tempfile::tempdir().expect("backing tempdir");
    let backing_video = copy_fixture(backing_dir.path(), "h264_aac.mkv", &file_name);

    let local_symlink_root = tempfile::tempdir().expect("local symlink root tempdir");
    let local_category_root = local_symlink_root.path().join("radarr");
    let local_release_dir = local_category_root.join(release_name);
    std::fs::create_dir_all(&local_release_dir).expect("create local release dir");
    let source_link = local_release_dir.join(&file_name);
    std::os::unix::fs::symlink(&backing_video, &source_link).expect("create source symlink");

    let remote_category_root = "/mnt/symlinks/radarr";
    let download_client_configs =
        DownloadClientConfigStore::new(ctx.db.datastore(), Arc::new(RwLock::new(None)));
    let config = download_client_configs
        .create(DownloadClientConfig {
            id: Id::new().0,
            name: "Decypharr".to_string(),
            client_type: "qbittorrent".to_string(),
            config_json: format!(
                r#"{{"remote_path_mappings":"{} => {}"}}"#,
                remote_category_root,
                local_category_root.to_string_lossy()
            ),
            is_enabled: true,
            status: DownloadClientStatus::Healthy,
            last_error: None,
            last_seen_at: None,
            client_priority: 0,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            proxy_config_id: None,
        })
        .await
        .expect("seed qbittorrent config");

    let dest_root = tempfile::tempdir().expect("dest tempdir");
    let title = add_movie_title(
        &ctx,
        "title-movie-decypharr-2",
        "Harbor Pilot and the Keeper of Portmere",
        dest_root.path().to_str().unwrap(),
    )
    .await;

    let completed = CompletedDownload {
        client_type: "qbittorrent".to_string(),
        client_id: config.id,
        download_client_item_id: "dl-movie-decypharr-2".to_string(),
        download_id: None,
        name: release_name.to_string(),
        release_name: Some(release_name.to_string()),
        dest_dir: format!("{remote_category_root}/{release_name}"),
        category: Some("radarr".to_string()),
        size_bytes: None,
        completed_at: None,
        parameters: vec![
            ("*scryer_title_id".to_string(), title.id.clone()),
            ("*scryer_facet".to_string(), "movie".to_string()),
        ],
    };

    let result = import_completed_download(&app, &user, &completed)
        .await
        .expect("import_completed_download");

    assert_eq!(result.decision, ImportDecision::Imported);
    assert_eq!(
        result.link_type.map(|strategy| strategy.as_str()),
        Some("symlink")
    );
    let dest_path = result.dest_path.expect("dest path");
    assert!(
        std::fs::symlink_metadata(&dest_path)
            .expect("dest metadata")
            .file_type()
            .is_symlink()
    );
}

// ---------------------------------------------------------------------------
// Happy path: movie import
// ---------------------------------------------------------------------------

#[tokio::test]
async fn import_movie_succeeds_and_copies_file() {
    let ctx = TestContext::new().await;
    let app = app_with_real_imports(&ctx).await;
    let user = ctx.app.find_or_create_default_user().await.unwrap();

    // Source: a temp dir containing a plausible movie .mkv file.
    let source_dir = tempfile::tempdir().expect("source tempdir");
    copy_fixture(
        source_dir.path(),
        "h264_aac.mkv",
        "Test.Movie.2024.1080p.WEB-DL.H264.mkv",
    );

    // Destination: a different temp dir used as the media library root.
    let dest_root = tempfile::tempdir().expect("dest tempdir");

    let title = add_movie_title(
        &ctx,
        "title-movie-1",
        "Test Movie",
        dest_root.path().to_str().unwrap(),
    )
    .await;

    let completed = scryer_completed(
        "dl-movie-1",
        source_dir.path().to_str().unwrap(),
        &title.id,
        "movie",
    );

    let result = import_completed_download(&app, &user, &completed)
        .await
        .expect("import_completed_download");

    assert_eq!(
        result.decision,
        ImportDecision::Imported,
        "expected Imported"
    );
    assert!(
        result.dest_path.is_some(),
        "dest_path should be set after import"
    );

    // The imported file must physically exist.
    let dest_path = result.dest_path.unwrap();
    assert!(
        std::path::Path::new(&dest_path).exists(),
        "imported file should exist at {dest_path}"
    );
}

// ---------------------------------------------------------------------------
// Dedup after a real successful import
// ---------------------------------------------------------------------------

/// Run a complete movie import, then confirm that a second attempt with the
/// same download_client_item_id is immediately short-circuited.
#[tokio::test]
async fn import_movie_second_attempt_is_deduped() {
    let ctx = TestContext::new().await;
    let app = app_with_real_imports(&ctx).await;
    let user = ctx.app.find_or_create_default_user().await.unwrap();

    let source_dir = tempfile::tempdir().expect("source tempdir");
    copy_fixture(source_dir.path(), "h264_aac.mkv", "Movie.2024.1080p.mkv");

    let dest_root = tempfile::tempdir().expect("dest tempdir");
    let title = add_movie_title(
        &ctx,
        "title-dedup-2",
        "Dedup Movie",
        dest_root.path().to_str().unwrap(),
    )
    .await;

    let completed = scryer_completed(
        "dl-dedup-2",
        source_dir.path().to_str().unwrap(),
        &title.id,
        "movie",
    );

    // First import — should succeed.
    let first = import_completed_download(&app, &user, &completed)
        .await
        .expect("first import");
    assert_eq!(first.decision, ImportDecision::Imported);

    // Second import — same download_client_item_id → AlreadyImported.
    let second = import_completed_download(&app, &user, &completed)
        .await
        .expect("second import");
    assert_eq!(second.decision, ImportDecision::Skipped);
    assert_eq!(second.skip_reason, Some(ImportSkipReason::AlreadyImported));
}

#[tokio::test]
async fn import_movie_rejected_by_post_download_rule_leaves_no_library_file_and_blocklists_release()
{
    let ctx = TestContext::new().await;
    let app = app_with_real_imports(&ctx).await;
    let user = ctx.app.find_or_create_default_user().await.unwrap();
    let source_dir = tempfile::tempdir().expect("source tempdir");
    copy_fixture(
        source_dir.path(),
        "h264_aac.mkv",
        "Blocked.Movie.2024.1080p.WEB-DL.H264.mkv",
    );
    let dest_root = tempfile::tempdir().expect("dest tempdir");
    let title = add_movie_title(
        &ctx,
        "title-rule-blocked",
        "Blocked Movie",
        dest_root.path().to_str().unwrap(),
    )
    .await;
    let wanted = seed_movie_wanted_item(
        &ctx,
        &title.id,
        scryer_application::AcquisitionScopeStatus::Grabbed,
    )
    .await;

    install_rule(
        &app,
        &user,
        r#"
import rego.v1

score_entry["too_few_chapters"] := scryer.block_score() if {
    input.file != null
    input.file.num_chapters < 2
}
"#,
        vec![MediaFacet::Movie],
    )
    .await;

    let completed = scryer_completed(
        "dl-rule-blocked",
        source_dir.path().to_str().unwrap(),
        &title.id,
        "movie",
    );
    // The grab-time indexer release title is the durable release evidence; the
    // blocklist must carry it (lowercased), never the client's display label.
    let grabbed_release = "Blocked.Movie.2024.1080p.WEB-DL.H264-GRP";
    record_movie_grab_submission(&ctx, &completed, &title, grabbed_release).await;
    let blocklisted_title = grabbed_release.to_ascii_lowercase();

    let result = import_completed_download(&app, &user, &completed)
        .await
        .expect("import completed download");

    assert_eq!(result.decision, ImportDecision::Rejected);
    assert_eq!(
        result.skip_reason,
        Some(ImportSkipReason::PostDownloadRuleBlocked)
    );
    assert!(
        result.dest_path.is_none(),
        "rejected import should not report a finalized destination path"
    );
    assert!(
        std::fs::read_dir(dest_root.path())
            .expect("read destination root")
            .next()
            .is_none(),
        "rejected movie should not create library artifacts"
    );
    assert!(
        ctx.media_files
            .list_media_files_for_title(&title.id)
            .await
            .expect("list media files")
            .is_empty(),
        "rejected movie should not leave a finalized media file"
    );

    let updated_wanted = ctx
        .library_state
        .get_acquisition_scope_state_for_title(&title.id, None)
        .await
        .expect("get wanted")
        .expect("wanted item");
    assert_eq!(updated_wanted.id, wanted.id);
    assert_eq!(
        updated_wanted.status,
        scryer_application::AcquisitionScopeStatus::Wanted
    );

    let failures = scryer_infrastructure_workflow::workflow::release_store::ReleaseStore::new(
        ctx.db.datastore(),
        ctx.db.encryption_key_state(),
    )
    .list_failed_release_signatures_for_title(&title.id, 10)
    .await
    .expect("failed signatures");
    assert!(
        failures.iter().any(|failure| {
            failure.source_title.as_deref() == Some(blocklisted_title.as_str())
                && failure
                    .error_message
                    .as_deref()
                    .is_some_and(|message| message.contains("too_few_chapters"))
        }),
        "failed-release signature must carry the grabbed indexer title: {failures:#?}"
    );

    let blocklist = ctx
        .library_state
        .list_for_title(&title.id, 10)
        .await
        .expect("list blocklist");
    assert!(
        blocklist.iter().any(|entry| {
            entry.normalized_release_name == blocklisted_title
                && entry
                    .reason
                    .as_deref()
                    .is_some_and(|reason| reason.contains("too_few_chapters"))
        }),
        "blocklist entry must carry the grabbed indexer title: {blocklist:#?}"
    );
}

#[tokio::test]
async fn import_series_rejected_by_post_download_rule_resets_episode_wanted_item() {
    let ctx = TestContext::new().await;
    let app = app_with_real_imports(&ctx).await;
    let user = ctx.app.find_or_create_default_user().await.unwrap();
    let source_dir = tempfile::tempdir().expect("source tempdir");
    let source_file = copy_fixture(
        source_dir.path(),
        "h264_aac.mkv",
        "Blocked.Show.S01E01.1080p.WEB-DL.H264.mkv",
    );
    pad_file_past_series_sample_threshold(&source_file);
    let dest_root = tempfile::tempdir().expect("dest tempdir");
    let title = add_short_form_series_title(
        &ctx,
        "title-series-rule-blocked",
        "Blocked Show",
        dest_root.path().to_str().unwrap(),
    )
    .await;
    let episode = seed_short_form_series_episode(&ctx, &title).await;
    let wanted = seed_episode_wanted_item(
        &ctx,
        &title,
        &episode,
        scryer_application::AcquisitionScopeStatus::Grabbed,
    )
    .await;

    install_rule(
        &app,
        &user,
        r#"
import rego.v1

score_entry["too_few_chapters"] := scryer.block_score() if {
    input.file != null
    input.file.num_chapters < 2
}
"#,
        vec![MediaFacet::Series],
    )
    .await;

    let completed = scryer_completed(
        "dl-series-rule-blocked",
        source_dir.path().to_str().unwrap(),
        &title.id,
        "series",
    );

    let result = import_completed_download(&app, &user, &completed)
        .await
        .expect("import completed download");

    assert_eq!(result.decision, ImportDecision::Rejected);
    assert_eq!(
        result.skip_reason,
        Some(ImportSkipReason::PostDownloadRuleBlocked)
    );
    assert!(
        ctx.media_files
            .list_media_files_for_title(&title.id)
            .await
            .expect("list media files")
            .is_empty(),
        "rejected episode should not leave a finalized media file"
    );

    let updated_wanted = ctx
        .library_state
        .get_acquisition_scope_state_for_title(&title.id, Some(&episode.id))
        .await
        .expect("get wanted")
        .expect("wanted item");
    assert_eq!(updated_wanted.id, wanted.id);
    assert_eq!(
        updated_wanted.status,
        scryer_application::AcquisitionScopeStatus::Wanted
    );
}

#[tokio::test]
async fn automatic_import_series_pack_imports_every_episode_from_the_bounded_directory() {
    let ctx = TestContext::new().await;
    let app = app_with_real_imports(&ctx).await;
    let user = ctx.app.find_or_create_default_user().await.unwrap();
    let source_dir = tempfile::tempdir().expect("source tempdir");
    let source_file_1 = copy_fixture(
        source_dir.path(),
        "h264_aac.mkv",
        "Pack.Show.S01E01.1080p.WEB-DL.H264.mkv",
    );
    let source_file_2 = copy_fixture(
        source_dir.path(),
        "h264_aac.mkv",
        "Pack.Show.S01E02.1080p.WEB-DL.H264.mkv",
    );
    pad_file_past_series_sample_threshold(&source_file_1);
    pad_file_past_series_sample_threshold(&source_file_2);
    let dest_root = tempfile::tempdir().expect("dest tempdir");
    let title = add_short_form_series_title(
        &ctx,
        "title-automatic-series-pack",
        "Pack Show",
        dest_root.path().to_str().unwrap(),
    )
    .await;
    let episode_1 = seed_short_form_series_episode(&ctx, &title).await;
    let episode_2 = seed_second_short_form_series_episode(
        &ctx,
        &title,
        episode_1.collection_id.as_deref().expect("collection id"),
    )
    .await;
    let completed = scryer_completed(
        "dl-automatic-series-pack",
        source_dir.path().to_str().unwrap(),
        &title.id,
        "series",
    );

    let result = import_completed_download(&app, &user, &completed)
        .await
        .expect("import completed series pack");

    assert_eq!(
        result.decision,
        ImportDecision::Imported,
        "unexpected series-pack import result: {result:?}"
    );
    let media_files = ctx
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files");
    assert_eq!(media_files.len(), 2);
    let episode_ids = media_files
        .iter()
        .filter_map(|media_file| media_file.episode_id.as_deref())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        episode_ids,
        std::collections::BTreeSet::from([episode_1.id.as_str(), episode_2.id.as_str()])
    );
}

#[tokio::test]
async fn manual_import_series_pack_maps_each_file_within_the_bound_title() {
    let ctx = TestContext::new().await;
    let app = app_with_real_imports(&ctx).await;
    let user = ctx.app.find_or_create_default_user().await.unwrap();
    let source_dir = tempfile::tempdir().expect("source tempdir");
    let source_file_1 = copy_fixture(
        source_dir.path(),
        "h264_aac.mkv",
        "Manual.Pack.Show.S01E01.1080p.WEB-DL.H264.mkv",
    );
    let source_file_2 = copy_fixture(
        source_dir.path(),
        "h264_aac.mkv",
        "Manual.Pack.Show.S01E02.1080p.WEB-DL.H264.mkv",
    );
    pad_file_past_series_sample_threshold(&source_file_1);
    pad_file_past_series_sample_threshold(&source_file_2);
    let dest_root = tempfile::tempdir().expect("dest tempdir");
    let title = add_series_title(
        &ctx,
        "title-manual-series-pack",
        "Manual Pack Show",
        dest_root.path().to_str().unwrap(),
    )
    .await;
    let episode_1 = seed_series_episode(&ctx, &title).await;
    let episode_2 = seed_second_series_episode(
        &ctx,
        &title,
        episode_1.collection_id.as_deref().expect("collection id"),
    )
    .await;
    let completed = scryer_completed(
        "dl-manual-series-pack",
        source_dir.path().to_str().unwrap(),
        &title.id,
        "series",
    );
    let import_id = queue_import_record(&ctx, &completed).await;

    let results = scryer_application::execute_manual_import(
        &app,
        &user,
        &import_id,
        &title.id,
        Some(&completed),
        vec![
            scryer_application::ManualImportFileMapping {
                file_path: source_file_1.to_string_lossy().to_string(),
                episode_id: Some(episode_1.id.clone()),
                series_movie_link_id: None,
            },
            scryer_application::ManualImportFileMapping {
                file_path: source_file_2.to_string_lossy().to_string(),
                episode_id: Some(episode_2.id.clone()),
                series_movie_link_id: None,
            },
        ],
        Some(source_dir.path().to_path_buf()),
    )
    .await
    .expect("execute manual series-pack import");

    assert_eq!(results.len(), 2);
    assert!(
        results.iter().all(|result| result.success),
        "manual pack files should import successfully: {results:#?}"
    );
    let media_files = ctx
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files");
    assert_eq!(media_files.len(), 2);
    let episode_ids = media_files
        .iter()
        .filter_map(|media_file| media_file.episode_id.as_deref())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        episode_ids,
        std::collections::BTreeSet::from([episode_1.id.as_str(), episode_2.id.as_str()])
    );
}

#[tokio::test]
async fn manual_import_multi_episode_filename_keeps_the_explicit_single_episode_mapping() {
    let ctx = TestContext::new().await;
    let app = app_with_real_imports(&ctx).await;
    let user = ctx.app.find_or_create_default_user().await.unwrap();
    let source_dir = tempfile::tempdir().expect("source tempdir");
    let source_file = copy_fixture(
        source_dir.path(),
        "h264_aac.mkv",
        "Manual.Show.S01E01-E02.1080p.WEB-DL.H264.mkv",
    );
    pad_file_past_series_sample_threshold(&source_file);
    let dest_root = tempfile::tempdir().expect("dest tempdir");
    let title = add_series_title(
        &ctx,
        "title-manual-single-target",
        "Manual Show",
        dest_root.path().to_str().unwrap(),
    )
    .await;
    let episode_1 = seed_series_episode(&ctx, &title).await;
    let episode_2 = seed_second_series_episode(
        &ctx,
        &title,
        episode_1.collection_id.as_deref().expect("collection id"),
    )
    .await;
    let completed = scryer_completed(
        "dl-manual-single-target",
        source_dir.path().to_str().unwrap(),
        &title.id,
        "series",
    );
    let import_id = queue_import_record(&ctx, &completed).await;

    let results = scryer_application::execute_manual_import(
        &app,
        &user,
        &import_id,
        &title.id,
        Some(&completed),
        vec![scryer_application::ManualImportFileMapping {
            file_path: source_file.to_string_lossy().to_string(),
            episode_id: Some(episode_1.id.clone()),
            series_movie_link_id: None,
        }],
        Some(source_dir.path().to_path_buf()),
    )
    .await
    .expect("execute manual import");

    assert!(
        results.iter().all(|result| result.success),
        "manual import should succeed: {results:#?}"
    );
    let scoped_files = ctx
        .media_files
        .list_live_media_files_for_episode_ids(
            &title.id,
            &[episode_1.id.clone(), episode_2.id.clone()],
        )
        .await
        .expect("list episode-scoped media files");
    assert_eq!(scoped_files.len(), 1);
    assert_eq!(
        scoped_files[0].primary_episode_ids,
        vec![episode_1.id.clone()]
    );
    let linked_episode_ids = scoped_files
        .iter()
        .flat_map(|file| file.episode_ids.iter().cloned())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        linked_episode_ids,
        std::collections::BTreeSet::from([episode_1.id])
    );
}

#[tokio::test]
async fn manual_import_rejects_a_mixed_title_pack_before_moving_any_file() {
    let ctx = TestContext::new().await;
    let app = app_with_real_imports(&ctx).await;
    let user = ctx.app.find_or_create_default_user().await.unwrap();
    let source_dir = tempfile::tempdir().expect("source tempdir");
    let source_file_1 = copy_fixture(
        source_dir.path(),
        "h264_aac.mkv",
        "Bound.Show.S01E01.1080p.WEB-DL.H264.mkv",
    );
    let source_file_2 = copy_fixture(
        source_dir.path(),
        "h264_aac.mkv",
        "Bound.Show.S01E02.1080p.WEB-DL.H264.mkv",
    );
    pad_file_past_series_sample_threshold(&source_file_1);
    pad_file_past_series_sample_threshold(&source_file_2);
    let bound_dest_root = tempfile::tempdir().expect("bound dest tempdir");
    let bound_title = add_series_title(
        &ctx,
        "title-manual-pack-bound",
        "Bound Show",
        bound_dest_root.path().to_str().unwrap(),
    )
    .await;
    let foreign_title = add_series_title(
        &ctx,
        "title-manual-pack-foreign",
        "Foreign Show",
        bound_dest_root.path().to_str().unwrap(),
    )
    .await;
    let bound_episode = seed_series_episode(&ctx, &bound_title).await;
    let foreign_episode = seed_series_episode(&ctx, &foreign_title).await;
    let foreign_series_movie = seed_series_movie_link(&ctx, &foreign_title).await;
    let completed = scryer_completed(
        "dl-manual-pack-mixed-title",
        source_dir.path().to_str().unwrap(),
        &bound_title.id,
        "series",
    );

    let error = scryer_application::execute_manual_import(
        &app,
        &user,
        "manual-pack-mixed-title-import",
        &bound_title.id,
        Some(&completed),
        vec![
            scryer_application::ManualImportFileMapping {
                file_path: source_file_1.to_string_lossy().to_string(),
                episode_id: Some(bound_episode.id),
                series_movie_link_id: None,
            },
            scryer_application::ManualImportFileMapping {
                file_path: source_file_2.to_string_lossy().to_string(),
                episode_id: Some(foreign_episode.id),
                series_movie_link_id: None,
            },
        ],
        Some(source_dir.path().to_path_buf()),
    )
    .await
    .expect_err("mixed-title pack should be rejected before import");

    assert!(error.to_string().contains("does not belong to title"));
    assert!(source_file_1.exists());
    assert!(source_file_2.exists());

    let series_movie_error = scryer_application::execute_manual_import(
        &app,
        &user,
        "manual-pack-foreign-series-movie-import",
        &bound_title.id,
        Some(&completed),
        vec![scryer_application::ManualImportFileMapping {
            file_path: source_file_1.to_string_lossy().to_string(),
            episode_id: None,
            series_movie_link_id: Some(foreign_series_movie.id),
        }],
        Some(source_dir.path().to_path_buf()),
    )
    .await
    .expect_err("foreign series-movie target should be rejected before import");

    assert!(
        series_movie_error
            .to_string()
            .contains("does not belong to title")
    );
    assert!(source_file_1.exists());
    assert!(
        ctx.media_files
            .list_media_files_for_title(&bound_title.id)
            .await
            .expect("list bound-title media files")
            .is_empty()
    );
}

#[tokio::test]
async fn manual_import_series_persists_media_analysis_and_acquisition_score() {
    let ctx = TestContext::new().await;
    let app = app_with_real_imports(&ctx).await;
    let user = ctx.app.find_or_create_default_user().await.unwrap();
    let source_dir = tempfile::tempdir().expect("source tempdir");
    let source_file = copy_fixture(
        source_dir.path(),
        "h264_aac.mkv",
        "Manual.Show.S01E01.1080p.WEB-DL.H264.mkv",
    );
    pad_file_past_series_sample_threshold(&source_file);
    let dest_root = tempfile::tempdir().expect("dest tempdir");
    let title = add_series_title(
        &ctx,
        "title-manual-series-success",
        "Manual Show",
        dest_root.path().to_str().unwrap(),
    )
    .await;
    let episode = seed_series_episode(&ctx, &title).await;
    let completed = scryer_completed(
        "dl-manual-series-success",
        source_dir.path().to_str().unwrap(),
        &title.id,
        "series",
    );
    let import_id = queue_import_record(&ctx, &completed).await;

    let results = scryer_application::execute_manual_import(
        &app,
        &user,
        &import_id,
        &title.id,
        Some(&completed),
        vec![scryer_application::ManualImportFileMapping {
            file_path: source_file.to_string_lossy().to_string(),
            episode_id: Some(episode.id.clone()),
            series_movie_link_id: None,
        }],
        Some(source_dir.path().to_path_buf()),
    )
    .await
    .expect("execute manual import");

    assert_eq!(results.len(), 1);
    assert!(
        results[0].success,
        "manual import should succeed: {:#?}",
        results[0]
    );

    let media_files = ctx
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files");
    assert_eq!(media_files.len(), 1);
    let imported = &media_files[0];
    assert_eq!(imported.episode_id.as_deref(), Some(episode.id.as_str()));
    assert_eq!(imported.scan_status, "scanned");
    assert!(imported.acquisition_score.is_some());
    assert!(imported.duration_seconds.is_some());
    assert!(imported.video_codec.is_some());
}

#[tokio::test]
async fn manual_import_series_reuses_existing_title_folder_path_even_when_template_changes() {
    let ctx = TestContext::new().await;
    let app = app_with_real_imports(&ctx).await;
    let user = ctx.app.find_or_create_default_user().await.unwrap();
    let source_dir = tempfile::tempdir().expect("source tempdir");
    let source_file = copy_fixture(
        source_dir.path(),
        "h264_aac.mkv",
        "Template.Show.S01E01.1080p.WEB-DL.H264.mkv",
    );
    pad_file_past_series_sample_threshold(&source_file);
    let dest_root = tempfile::tempdir().expect("dest tempdir");
    let title = add_series_title(
        &ctx,
        "title-manual-series-existing-folder",
        "Template Show",
        dest_root.path().to_str().unwrap(),
    )
    .await;
    let existing_folder = dest_root.path().join("Template Show (2024)");
    std::fs::create_dir_all(existing_folder.join("Season 1")).expect("create existing folder");
    ctx.titles
        .set_folder_path(&title.id, existing_folder.to_string_lossy().as_ref())
        .await
        .expect("set folder path");
    set_folder_template(&ctx, MediaFacet::Series, "{title}").await;
    let episode = seed_series_episode(&ctx, &title).await;
    let completed = scryer_completed(
        "dl-manual-series-existing-folder",
        source_dir.path().to_str().unwrap(),
        &title.id,
        "series",
    );
    let import_id = queue_import_record(&ctx, &completed).await;

    let results = scryer_application::execute_manual_import(
        &app,
        &user,
        &import_id,
        &title.id,
        Some(&completed),
        vec![scryer_application::ManualImportFileMapping {
            file_path: source_file.to_string_lossy().to_string(),
            episode_id: Some(episode.id.clone()),
            series_movie_link_id: None,
        }],
        Some(source_dir.path().to_path_buf()),
    )
    .await
    .expect("execute manual import");

    assert_eq!(results.len(), 1);
    assert!(
        results[0].success,
        "manual import should succeed: {:#?}",
        results[0]
    );
    let dest_path = results[0].dest_path.clone().expect("dest path");
    assert!(dest_path.contains("Template Show (2024)/Season 1/"));
    assert!(!dest_path.contains("Template Show/Season 1/"));
}

#[tokio::test]
async fn manual_import_series_rejects_when_incumbent_covers_broader_episode_set() {
    let ctx = TestContext::new().await;
    let app = app_with_real_imports(&ctx).await;
    let user = ctx.app.find_or_create_default_user().await.unwrap();
    let source_dir = tempfile::tempdir().expect("source tempdir");
    let source_file = copy_fixture(
        source_dir.path(),
        "h264_aac.mkv",
        "Manual.Show.S01E01.1080p.WEB-DL.H264.mkv",
    );
    pad_file_past_series_sample_threshold(&source_file);
    let dest_root = tempfile::tempdir().expect("dest tempdir");
    let title = add_series_title(
        &ctx,
        "title-manual-series-broader-incumbent",
        "Manual Show",
        dest_root.path().to_str().unwrap(),
    )
    .await;
    let episode1 = seed_series_episode(&ctx, &title).await;
    let episode2 = seed_second_series_episode(
        &ctx,
        &title,
        episode1.collection_id.as_deref().expect("collection id"),
    )
    .await;
    let existing_file_id = ctx
        .media_files
        .insert_media_file(&scryer_application::InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: dest_root
                .path()
                .join("Manual Show")
                .join("Season 01")
                .join("Manual Show - S01E01-E02.mkv")
                .to_string_lossy()
                .to_string(),
            size_bytes: 52 * 1024 * 1024,
            quality_label: Some("1080P".to_string()),
            acquisition_score: Some(500),
            ..Default::default()
        })
        .await
        .expect("insert incumbent pack");
    ctx.media_files
        .link_file_to_episode(&existing_file_id, &episode1.id)
        .await
        .expect("link incumbent episode 1");
    ctx.media_files
        .link_file_to_episode(&existing_file_id, &episode2.id)
        .await
        .expect("link incumbent episode 2");
    for episode_id in [&episode1.id, &episode2.id] {
        ctx.media_files
            .set_media_file_roles_for_episode(&title.id, episode_id, &existing_file_id, &[])
            .await
            .expect("incumbent pack should be primary for linked episode");
    }

    let completed = scryer_completed(
        "dl-manual-series-broader-incumbent",
        source_dir.path().to_str().unwrap(),
        &title.id,
        "series",
    );
    let results = scryer_application::execute_manual_import(
        &app,
        &user,
        "manual-series-broader-incumbent-import",
        &title.id,
        Some(&completed),
        vec![scryer_application::ManualImportFileMapping {
            file_path: source_file.to_string_lossy().to_string(),
            episode_id: Some(episode1.id.clone()),
            series_movie_link_id: None,
        }],
        Some(source_dir.path().to_path_buf()),
    )
    .await
    .expect("execute manual import");

    assert_eq!(results.len(), 1);
    assert!(
        !results[0].success,
        "manual import should reject narrower replacement"
    );
    assert!(
        results[0]
            .error_message
            .as_deref()
            .is_some_and(|message| message.contains("broader episode set")),
        "unexpected manual import result: {:#?}",
        results[0]
    );

    let media_files = ctx
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files");
    let media_file_ids = media_files
        .iter()
        .map(|media_file| media_file.id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        media_file_ids,
        std::collections::BTreeSet::from([existing_file_id.as_str()]),
        "rejected manual import should not add a new file"
    );
}

#[tokio::test]
async fn import_movie_rule_eval_error_fails_open() {
    let ctx = TestContext::new().await;
    let app = app_with_real_imports(&ctx).await;
    let user = ctx.app.find_or_create_default_user().await.unwrap();
    let source_dir = tempfile::tempdir().expect("source tempdir");
    copy_fixture(
        source_dir.path(),
        "h264_aac.mkv",
        "Rule.Error.Movie.2024.1080p.WEB-DL.H264.mkv",
    );
    let dest_root = tempfile::tempdir().expect("dest tempdir");
    let title = add_movie_title(
        &ctx,
        "title-rule-error",
        "Rule Error Movie",
        dest_root.path().to_str().unwrap(),
    )
    .await;

    install_rule(
        &app,
        &user,
        r#"
import rego.v1

score_entry["bad_runtime"] := count(input.file.video_width) if {
    input.file != null
    input.file.num_chapters == 0
}
"#,
        vec![MediaFacet::Movie],
    )
    .await;

    let completed = scryer_completed(
        "dl-rule-error",
        source_dir.path().to_str().unwrap(),
        &title.id,
        "movie",
    );

    let result = import_completed_download(&app, &user, &completed)
        .await
        .expect("import completed download");

    assert_eq!(result.decision, ImportDecision::Imported);
    let media_files = ctx
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files");
    assert_eq!(media_files.len(), 1);
}

#[tokio::test]
async fn import_upgrade_rejected_by_post_download_rule_restores_prior_file() {
    let ctx = TestContext::new().await;
    let app = app_with_real_imports(&ctx).await;
    let user = ctx.app.find_or_create_default_user().await.unwrap();
    let dest_root = tempfile::tempdir().expect("dest tempdir");
    let title = add_movie_title(
        &ctx,
        "title-upgrade-rule-blocked",
        "Upgrade Movie",
        dest_root.path().to_str().unwrap(),
    )
    .await;
    let _wanted = seed_movie_wanted_item(
        &ctx,
        &title.id,
        scryer_application::AcquisitionScopeStatus::Grabbed,
    )
    .await;

    let old_path = dest_root
        .path()
        .join("Upgrade Movie (2024)")
        .join("Upgrade.Movie.2024.1080p.WEB-DL.H264.mkv");
    std::fs::create_dir_all(old_path.parent().expect("old path parent")).expect("create old dir");
    std::fs::copy(mediainfo_fixture("h264_aac.mkv"), &old_path).expect("seed old movie file");
    ctx.media_files
        .insert_media_file(&scryer_application::InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: old_path.to_string_lossy().to_string(),
            size_bytes: std::fs::metadata(&old_path).expect("old metadata").len() as i64,
            quality_label: Some("1080P".to_string()),
            acquisition_score: Some(100),
            ..Default::default()
        })
        .await
        .expect("insert old media file");

    install_rule(
        &app,
        &user,
        r#"
import rego.v1

score_entry["too_few_chapters"] := scryer.block_score() if {
    input.file != null
    input.file.num_chapters < 2
}
"#,
        vec![MediaFacet::Movie],
    )
    .await;

    let source_dir = tempfile::tempdir().expect("source tempdir");
    copy_fixture(
        source_dir.path(),
        "h264_aac.mkv",
        "Upgrade.Movie.2024.2160p.WEB-DL.H264.mkv",
    );
    let completed = scryer_completed(
        "dl-upgrade-rule-blocked",
        source_dir.path().to_str().unwrap(),
        &title.id,
        "movie",
    );
    // The grab-time indexer release title is the durable release evidence; the
    // blocklist must carry it (lowercased), never the client's display label.
    let grabbed_release = "Upgrade.Movie.2024.2160p.WEB-DL.H264-GRP";
    record_movie_grab_submission(&ctx, &completed, &title, grabbed_release).await;
    let blocklisted_title = grabbed_release.to_ascii_lowercase();

    let result = import_completed_download(&app, &user, &completed)
        .await
        .expect("import completed download");

    assert_eq!(result.decision, ImportDecision::Rejected);
    assert_eq!(
        result.skip_reason,
        Some(ImportSkipReason::PostDownloadRuleBlocked)
    );
    assert!(
        old_path.exists(),
        "old file should have been restored after rejected upgrade"
    );
    let media_files = ctx
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files");
    assert_eq!(media_files.len(), 1);
    assert_eq!(
        media_files[0].file_path,
        old_path.to_string_lossy().to_string()
    );

    let blocklist = ctx
        .library_state
        .list_for_title(&title.id, 10)
        .await
        .expect("list blocklist");
    assert!(
        blocklist.iter().any(|entry| {
            entry.normalized_release_name == blocklisted_title
                && entry
                    .reason
                    .as_deref()
                    .is_some_and(|reason| reason.contains("too_few_chapters"))
        }),
        "blocklist entry must carry the grabbed indexer title: {blocklist:#?}"
    );
}

// ---------------------------------------------------------------------------
// Manual movie import: primary-only semantics
// ---------------------------------------------------------------------------

fn movie_manual_mapping(path: &Path) -> scryer_application::ManualImportFileMapping {
    scryer_application::ManualImportFileMapping {
        file_path: path.to_string_lossy().to_string(),
        episode_id: None,
        series_movie_link_id: None,
    }
}

/// The tracked download the manual-import poller reconciles after a manual
/// import: same client identity as the completed download, movie facet.
fn tracked_movie_download(
    completed: &CompletedDownload,
    title_id: &str,
    download_id: scryer_domain::download_identity::DownloadId,
) -> scryer_application::tracked_downloads::TrackedDownload {
    scryer_application::tracked_downloads::TrackedDownload {
        download_id,
        id: format!(
            "{}:{}",
            completed.client_id, completed.download_client_item_id
        ),
        client_id: completed.client_id.clone(),
        client_type: completed.client_type.clone(),
        client_item: scryer_domain::DownloadQueueItem {
            id: Id::new().0,
            title_id: Some(title_id.to_string()),
            episode_id: None,
            title_name: completed.name.clone(),
            facet: Some("movie".to_string()),
            category: None,
            client_id: completed.client_id.clone(),
            client_name: completed.client_type.clone(),
            client_type: completed.client_type.clone(),
            state: scryer_domain::DownloadQueueState::Completed,
            progress_percent: 100,
            import_transfer_phase: None,
            import_transfer_bytes: None,
            import_transfer_total_bytes: None,
            import_transfer_started_at: None,
            import_transfer_updated_at: None,
            size_bytes: None,
            remaining_seconds: None,
            queued_at: None,
            last_updated_at: None,
            attention_required: false,
            attention_reason: None,
            download_client_item_id: completed.download_client_item_id.clone(),
            download_id: None,
            import_status: None,
            import_error_code: None,
            import_error_message: None,
            imported_at: None,
            delete_status: None,
            delete_error_message: None,
            source_provider: None,
            is_scryer_origin: true,
            tracked_state: None,
            tracked_status: None,
            tracked_status_messages: vec![],
            tracked_match_type: None,
            seeding: None,
        },
        completed_source: Some(completed.clone()),
        state: scryer_domain::TrackedDownloadState::ImportBlocked,
        status: scryer_domain::TrackedDownloadStatus::Ok,
        status_messages: vec![],
        title_id: Some(title_id.to_string()),
        facet: Some("movie".to_string()),
        source_title: None,
        indexer: None,
        added_at: None,
        notified_manual_interaction: false,
        match_type: scryer_domain::TitleMatchType::Submission,
        is_trackable: true,
        import_attempted: false,
        waiting_for_completed_history: false,
        path_missing_since: None,
        no_video_import_retry: None,
        import_execution_retry: None,
        import_hold: None,
        skip_reacquire_on_failure: false,
        burned_by_import_gate: false,
        snapshot_missing_since: None,
    }
}

#[tokio::test]
async fn manual_import_movie_imports_only_the_primary_and_skips_samples_and_extras() {
    let ctx = TestContext::new().await;
    let app = app_with_real_imports(&ctx).await;
    let user = ctx.app.find_or_create_default_user().await.unwrap();
    let source_dir = tempfile::tempdir().expect("source tempdir");
    // Mapped in the order the web client would send them (directory walk):
    // the sample first, so the primary is not simply "the first mapping".
    let sample = copy_fixture(
        source_dir.path(),
        "h264_aac.mkv",
        "Manual.Movie.2024.1080p.WEB-DL.H264-sample.mkv",
    );
    let movie = copy_fixture(
        source_dir.path(),
        "h264_aac.mkv",
        "Manual.Movie.2024.1080p.WEB-DL.H264.mkv",
    );
    pad_file_past_series_sample_threshold(&movie);
    let featurette = copy_fixture(
        source_dir.path(),
        "h264_aac.mkv",
        "Manual.Movie.2024.Making.Of.featurette.mkv",
    );
    let dest_root = tempfile::tempdir().expect("dest tempdir");
    let title = add_movie_title(
        &ctx,
        "title-manual-movie-primary",
        "Manual Movie",
        dest_root.path().to_str().unwrap(),
    )
    .await;
    let completed = scryer_completed(
        "dl-manual-movie-primary",
        source_dir.path().to_str().unwrap(),
        &title.id,
        "movie",
    );
    // Import artifacts reference the import record; queue one as the manual
    // import poller would before executing.
    let download_id = record_movie_grab_submission(&ctx, &completed, &title, &completed.name).await;
    let import_id = ImportStore::new(ctx.db.datastore())
        .queue_import_request_with_identity_for_download(
            ClientJobLocator::for_import_artifact(
                Some(&completed.client_id),
                &completed.client_type,
                &completed.download_client_item_id,
            ),
            "manual_import".to_string(),
            "{}".to_string(),
            None,
            Some(&download_id),
        )
        .await
        .expect("queue manual import record");

    let results = scryer_application::execute_manual_import(
        &app,
        &user,
        &import_id,
        &title.id,
        Some(&completed),
        vec![
            movie_manual_mapping(&sample),
            movie_manual_mapping(&movie),
            movie_manual_mapping(&featurette),
        ],
        Some(source_dir.path().to_path_buf()),
    )
    .await
    .expect("execute manual movie import");

    assert_eq!(results.len(), 3, "{results:#?}");
    let by_path = |path: &Path| {
        results
            .iter()
            .find(|result| result.file_path == path.to_string_lossy())
            .unwrap_or_else(|| panic!("result for {}: {results:#?}", path.display()))
    };
    let primary = by_path(&movie);
    assert!(primary.success, "primary should import: {primary:#?}");
    assert!(!primary.skipped);
    let dest_path = primary.dest_path.as_deref().expect("primary dest path");
    assert!(
        Path::new(dest_path).exists(),
        "primary should land at {dest_path}"
    );
    for extra in [by_path(&sample), by_path(&featurette)] {
        assert!(extra.skipped, "extra should be skipped: {extra:#?}");
        assert!(!extra.success);
        assert_eq!(extra.error_code, None);
        assert_eq!(
            extra.error_message.as_deref(),
            Some("skipped: not the primary movie file")
        );
        assert_eq!(extra.dest_path, None);
    }
    assert!(
        sample.exists(),
        "skipped sample must not be moved or deleted"
    );
    assert!(
        featurette.exists(),
        "skipped extra must not be moved or deleted"
    );

    let media_files = ctx
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files");
    assert_eq!(
        media_files.len(),
        1,
        "only the primary is imported: {media_files:#?}"
    );
    assert_eq!(media_files[0].file_path, dest_path);

    // What the manual-import poller reconciles: one attempted mapping (the
    // skipped extras are excluded from the expected count), one imported.
    let expected_mapping_count = Some(results.iter().filter(|result| !result.skipped).count());
    assert_eq!(expected_mapping_count, Some(1));
    let tracked = tracked_movie_download(&completed, &title.id, download_id);
    assert!(
        scryer_application::completed_download_handler::verify_manual_import(
            &app,
            &tracked,
            1,
            expected_mapping_count,
        )
        .await
        .expect("manual import verification should be available"),
        "the tracked download must verify as imported with the extras skipped"
    );
}

#[tokio::test]
async fn manual_import_movie_with_only_samples_fails_with_a_clear_message() {
    let ctx = TestContext::new().await;
    let app = app_with_real_imports(&ctx).await;
    let user = ctx.app.find_or_create_default_user().await.unwrap();
    let source_dir = tempfile::tempdir().expect("source tempdir");
    let named_sample = copy_fixture(
        source_dir.path(),
        "h264_aac.mkv",
        "Only.Samples.Movie.2024.1080p.WEB-DL.H264-sample.mkv",
    );
    let shouting_sample = copy_fixture(
        source_dir.path(),
        "h264_aac.mkv",
        "Only.Samples.Movie.2024.SAMPLE.Trailer.mkv",
    );
    let dest_root = tempfile::tempdir().expect("dest tempdir");
    let title = add_movie_title(
        &ctx,
        "title-manual-movie-only-samples",
        "Only Samples Movie",
        dest_root.path().to_str().unwrap(),
    )
    .await;
    let completed = scryer_completed(
        "dl-manual-movie-only-samples",
        source_dir.path().to_str().unwrap(),
        &title.id,
        "movie",
    );

    let results = scryer_application::execute_manual_import(
        &app,
        &user,
        "manual-movie-only-samples-import",
        &title.id,
        Some(&completed),
        vec![
            movie_manual_mapping(&named_sample),
            movie_manual_mapping(&shouting_sample),
        ],
        Some(source_dir.path().to_path_buf()),
    )
    .await
    .expect("execute manual movie import");

    assert_eq!(results.len(), 2, "{results:#?}");
    for result in &results {
        assert!(!result.success, "{result:#?}");
        assert!(
            !result.skipped,
            "a sample-only import fails, it is not skipped"
        );
        assert_eq!(
            result.error_code,
            Some(scryer_domain::ImportErrorCode::PolicyMismatch)
        );
        assert_eq!(
            result.error_message.as_deref(),
            Some("no primary movie file to import: every mapped video is named as a sample")
        );
    }
    let media_files = ctx
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files");
    assert!(
        media_files.is_empty(),
        "nothing may be imported: {media_files:#?}"
    );
    assert!(named_sample.exists());
    assert!(shouting_sample.exists());
}

#[tokio::test]
async fn manual_import_small_normally_named_movie_imports_as_the_primary() {
    // The automatic movie path never size-filters (a short film or old cartoon
    // under the 50 MB sample heuristic auto-imports), and manual import is the
    // user's escape hatch, so it must not be stricter: a normally named 1 MB
    // movie is the primary, while a sample-named file beside a bigger main file
    // is still skipped.
    let ctx = TestContext::new().await;
    let app = app_with_real_imports(&ctx).await;
    let user = ctx.app.find_or_create_default_user().await.unwrap();
    let source_dir = tempfile::tempdir().expect("source tempdir");
    let small_movie = copy_fixture(
        source_dir.path(),
        "h264_aac.mkv",
        "Short.Film.1998.480p.DVDRip.H264.mkv",
    );
    std::fs::OpenOptions::new()
        .write(true)
        .open(&small_movie)
        .expect("open small movie")
        .set_len(1024 * 1024)
        .expect("size small movie to 1 MB");
    let sample = copy_fixture(
        source_dir.path(),
        "h264_aac.mkv",
        "Short.Film.1998.480p.DVDRip.H264-sample.mkv",
    );
    assert!(std::fs::metadata(&sample).expect("sample metadata").len() < 1024 * 1024);
    let dest_root = tempfile::tempdir().expect("dest tempdir");
    let title = add_movie_title(
        &ctx,
        "title-manual-small-movie",
        "Short Film",
        dest_root.path().to_str().unwrap(),
    )
    .await;
    let completed = scryer_completed(
        "dl-manual-small-movie",
        source_dir.path().to_str().unwrap(),
        &title.id,
        "movie",
    );
    let import_id = queue_import_record(&ctx, &completed).await;

    let results = scryer_application::execute_manual_import(
        &app,
        &user,
        &import_id,
        &title.id,
        Some(&completed),
        vec![
            movie_manual_mapping(&sample),
            movie_manual_mapping(&small_movie),
        ],
        Some(source_dir.path().to_path_buf()),
    )
    .await
    .expect("execute manual small-movie import");

    assert_eq!(results.len(), 2, "{results:#?}");
    let primary = results
        .iter()
        .find(|result| result.file_path == small_movie.to_string_lossy())
        .expect("primary result");
    assert!(primary.success, "small movie should import: {primary:#?}");
    assert!(!primary.skipped);
    let skipped_sample = results
        .iter()
        .find(|result| result.file_path == sample.to_string_lossy())
        .expect("sample result");
    assert!(skipped_sample.skipped, "{skipped_sample:#?}");
    assert!(!skipped_sample.success);
    let media_files = ctx
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files");
    assert_eq!(media_files.len(), 1, "{media_files:#?}");
    assert_eq!(
        media_files[0].file_path,
        primary.dest_path.clone().expect("primary dest path")
    );
}

// ---------------------------------------------------------------------------
// Completed manual-import recovery query is time-bounded
// ---------------------------------------------------------------------------

#[tokio::test]
async fn completed_manual_import_recovery_query_only_returns_records_inside_the_window() {
    let ctx = TestContext::new().await;
    let workflow_store = ImportStore::new(ctx.db.datastore());

    let manual_import_id = workflow_store
        .queue_import_request(
            ClientJobLocator::new(Some("test-client"), "qbittorrent", "hash-recover"),
            "manual_import".to_string(),
            "{}".to_string(),
        )
        .await
        .expect("queue manual import");
    workflow_store
        .update_import_status(
            &manual_import_id,
            scryer_domain::ImportStatus::Completed,
            None,
        )
        .await
        .expect("complete manual import");
    let movie_import_id = workflow_store
        .queue_import_request(
            ClientJobLocator::new(Some("test-client"), "nzbget", "dl-auto"),
            "movie_download".to_string(),
            "{}".to_string(),
        )
        .await
        .expect("queue automatic import");
    workflow_store
        .update_import_status(
            &movie_import_id,
            scryer_domain::ImportStatus::Completed,
            None,
        )
        .await
        .expect("complete automatic import");

    let inside_window = workflow_store
        .list_completed_manual_imports(chrono::Utc::now() - chrono::Duration::hours(24), 500)
        .await
        .expect("list recent completed manual imports");
    assert_eq!(
        inside_window
            .iter()
            .map(|record| record.id.as_str())
            .collect::<Vec<_>>(),
        vec![manual_import_id.as_str()],
        "only completed manual imports updated inside the window are scanned"
    );

    // A cutoff after the record's last update excludes it: records older than
    // the recovery window are never re-scanned.
    let outside_window = workflow_store
        .list_completed_manual_imports(chrono::Utc::now() + chrono::Duration::hours(1), 500)
        .await
        .expect("list completed manual imports outside the window");
    assert!(outside_window.is_empty(), "{outside_window:#?}");
}

// ---------------------------------------------------------------------------
// srrdb filename recovery
// ---------------------------------------------------------------------------

/// A recorded srrdb port that answers by file size.
///
/// The real adapter keys on CRC-32 plus size; the CRC helper is crate private,
/// so these fixtures give every member a distinct byte length and the fake
/// answers on that. The CRC each call carries is still asserted to be the
/// 8-digit uppercase hex the API expects.
#[derive(Default)]
struct RecordingSrrdbLookup {
    calls: std::sync::Mutex<Vec<(String, u64)>>,
    names_by_size: std::collections::HashMap<u64, String>,
}

impl RecordingSrrdbLookup {
    fn new(names_by_size: &[(u64, &str)]) -> Arc<Self> {
        Arc::new(Self {
            calls: std::sync::Mutex::new(Vec::new()),
            names_by_size: names_by_size
                .iter()
                .map(|(size, name)| (*size, (*name).to_string()))
                .collect(),
        })
    }

    fn calls(&self) -> Vec<(String, u64)> {
        self.calls.lock().expect("srrdb call log").clone()
    }
}

#[async_trait::async_trait]
impl scryer_application::SrrdbFilenameLookup for RecordingSrrdbLookup {
    async fn recover_filename(
        &self,
        crc32_hex: &str,
        size_bytes: u64,
    ) -> Result<Option<String>, scryer_application::SrrdbOutage> {
        assert_eq!(
            crc32_hex.len(),
            8,
            "srrdb takes an 8 digit CRC: {crc32_hex:?}"
        );
        assert!(
            crc32_hex
                .chars()
                .all(|character| character.is_ascii_hexdigit() && !character.is_ascii_lowercase()),
            "srrdb takes uppercase hex: {crc32_hex:?}"
        );
        self.calls
            .lock()
            .expect("srrdb call log")
            .push((crc32_hex.to_string(), size_bytes));
        Ok(self.names_by_size.get(&size_bytes).cloned())
    }
}

/// `app_with_real_imports` plus the srrdb port, with the admin switch on.
async fn app_with_srrdb_recovery(
    ctx: &TestContext,
    lookup: Arc<RecordingSrrdbLookup>,
) -> scryer_application::AppUseCase {
    ctx.settings_store
        .batch_ensure_setting_definitions(vec![SettingDefinitionSeed {
            category: "general".into(),
            scope: "system".into(),
            key_name: scryer_application::SRRDB_FILENAME_RECOVERY_ENABLED_KEY.into(),
            data_type: "boolean".into(),
            default_value_json: "false".into(),
            is_sensitive: false,
            validation_json: None,
        }])
        .await
        .expect("seed srrdb filename recovery setting definition");
    scryer_application::SettingsRepository::upsert_setting_json(
        &*ctx.settings_store,
        scryer_application::SETTINGS_SCOPE_SYSTEM,
        scryer_application::SRRDB_FILENAME_RECOVERY_ENABLED_KEY,
        None,
        "true".to_string(),
        "integration_test",
        None,
    )
    .await
    .expect("enable srrdb filename recovery");

    let app = app_with_real_imports(ctx).await;
    app.with_test_overrides(|builder| {
        builder
            .with_srrdb_filename_lookup(lookup as Arc<dyn scryer_application::SrrdbFilenameLookup>)
    })
}

/// Extend `path` to exactly `target_len` bytes so each pack member has a size
/// of its own and survives the series sample-size filter.
fn pad_file_to(path: &Path, target_len: u64) -> u64 {
    use std::io::{Seek, SeekFrom, Write};

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .expect("open fixture for padding");
    file.seek(SeekFrom::Start(target_len - 1))
        .expect("seek fixture");
    file.write_all(&[0]).expect("extend fixture");
    drop(file);
    let len = std::fs::metadata(path).expect("fixture size").len();
    assert_eq!(len, target_len);
    len
}

/// An obfuscated pack: physical names carry no title signal at all.
const OBFUSCATED_PACK: [&str; 3] = [
    "a1b2c3d4e5f6a7b8c9d0.mkv",
    "b2c3d4e5f6a7b8c9d0e1.mkv",
    "c3d4e5f6a7b8c9d0e1f2.mkv",
];

#[tokio::test]
async fn automatic_import_places_a_fully_obfuscated_pack_from_srrdb_recovered_names() {
    let ctx = TestContext::new().await;
    // The release folder is obfuscated too, so nothing but the recovered names
    // can tell these three files apart.
    let source_root = tempfile::tempdir().expect("source tempdir");
    let source_dir = source_root.path().join("d4e5f6a7b8c9d0e1f2a3");
    std::fs::create_dir(&source_dir).expect("create obfuscated release folder");
    let sizes: Vec<u64> = OBFUSCATED_PACK
        .iter()
        .enumerate()
        .map(|(index, member)| {
            let path = copy_fixture(&source_dir, "h264_aac.mkv", member);
            pad_file_to(&path, (52 + index as u64) * 1024 * 1024)
        })
        .collect();
    let lookup = RecordingSrrdbLookup::new(&[
        (
            sizes[0],
            "Recovered.Pals.S01E01.1080p.WEB-DL.H264-LANTERNS.mkv",
        ),
        (
            sizes[1],
            "Recovered.Pals.S01E02.1080p.WEB-DL.H264-LANTERNS.mkv",
        ),
        (
            sizes[2],
            "Recovered.Pals.S01E03.1080p.WEB-DL.H264-LANTERNS.mkv",
        ),
    ]);
    let app = app_with_srrdb_recovery(&ctx, lookup.clone()).await;
    let user = ctx.app.find_or_create_default_user().await.unwrap();
    let dest_root = tempfile::tempdir().expect("dest tempdir");
    let title = add_short_form_series_title(
        &ctx,
        "title-srrdb-pack",
        "Recovered Pals",
        dest_root.path().to_str().unwrap(),
    )
    .await;
    let episode_1 = seed_short_form_series_episode(&ctx, &title).await;
    let collection_id = episode_1.collection_id.as_deref().expect("collection id");
    let episode_2 = seed_second_short_form_series_episode(&ctx, &title, collection_id).await;
    let episode_3 =
        seed_series_episode_in_collection(&ctx, &title, collection_id, 3, Some(60)).await;
    let completed = scryer_completed(
        "dl-srrdb-pack",
        source_dir.to_str().unwrap(),
        &title.id,
        "series",
    );

    let result = import_completed_download(&app, &user, &completed)
        .await
        .expect("import obfuscated series pack");

    assert_eq!(
        result.decision,
        ImportDecision::Imported,
        "unexpected obfuscated pack import result: {result:?}"
    );
    assert_eq!(
        lookup.calls().len(),
        3,
        "every obfuscated member must be recovered: {:?}",
        lookup.calls()
    );
    let media_files = ctx
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files");
    assert_eq!(media_files.len(), 3, "{media_files:#?}");
    let episode_ids = media_files
        .iter()
        .filter_map(|media_file| media_file.episode_id.as_deref())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        episode_ids,
        std::collections::BTreeSet::from([
            episode_1.id.as_str(),
            episode_2.id.as_str(),
            episode_3.id.as_str(),
        ]),
        "the recovered names are what place each member on its episode"
    );
}

#[tokio::test]
async fn automatic_import_places_an_obfuscated_pack_under_a_properly_named_release() {
    // The common real-world shape: the indexer's NZB carried the proper
    // release name, so the client unpacked into a well-named folder and
    // reports that name for the download, but the archive members inside are
    // obfuscated. The release name says which title and season; it never says
    // which member is which episode. Every member still has to be recovered.
    let ctx = TestContext::new().await;
    let release = "Harbor.Pals.S01.1080p.WEB-DL.H264-LANTERNS";
    let source_root = tempfile::tempdir().expect("source tempdir");
    let source_dir = source_root.path().join(release);
    std::fs::create_dir(&source_dir).expect("create named release folder");
    let members: Vec<PathBuf> = OBFUSCATED_PACK
        .iter()
        .map(|member| copy_fixture(&source_dir, "h264_aac.mkv", member))
        .collect();
    let sizes: Vec<u64> = members
        .iter()
        .enumerate()
        .map(|(index, path)| pad_file_to(path, (58 + index as u64) * 1024 * 1024))
        .collect();
    let lookup = RecordingSrrdbLookup::new(&[
        (
            sizes[0],
            "Harbor.Pals.S01E01.1080p.WEB-DL.H264-LANTERNS.mkv",
        ),
        (
            sizes[1],
            "Harbor.Pals.S01E02.1080p.WEB-DL.H264-LANTERNS.mkv",
        ),
        (
            sizes[2],
            "Harbor.Pals.S01E03.1080p.WEB-DL.H264-LANTERNS.mkv",
        ),
    ]);
    let app = app_with_srrdb_recovery(&ctx, lookup.clone()).await;
    let user = ctx.app.find_or_create_default_user().await.unwrap();
    let dest_root = tempfile::tempdir().expect("dest tempdir");
    let title = add_short_form_series_title(
        &ctx,
        "title-srrdb-named-folder",
        "Harbor Pals",
        dest_root.path().to_str().unwrap(),
    )
    .await;
    let episode_1 = seed_short_form_series_episode(&ctx, &title).await;
    let collection_id = episode_1.collection_id.as_deref().expect("collection id");
    let episode_2 = seed_second_short_form_series_episode(&ctx, &title, collection_id).await;
    let episode_3 =
        seed_series_episode_in_collection(&ctx, &title, collection_id, 3, Some(60)).await;
    let mut completed = scryer_completed(
        "dl-srrdb-named-folder",
        source_dir.to_str().unwrap(),
        &title.id,
        "series",
    );
    // What SABnzbd and NZBGet report for a download whose NZB was properly
    // named: the release, not an obfuscated job name.
    completed.release_name = Some(release.to_string());

    let result = import_completed_download(&app, &user, &completed)
        .await
        .expect("import obfuscated pack under a named release");

    assert_eq!(
        result.decision,
        ImportDecision::Imported,
        "unexpected named-release pack import result: {result:?}"
    );
    assert_eq!(
        lookup.calls().len(),
        3,
        "a well-named release does not tell a member which episode it is: {:?}",
        lookup.calls()
    );
    for member in &members {
        assert!(
            member.exists(),
            "no source file is ever renamed: {}",
            member.display()
        );
    }
    let media_files = ctx
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files");
    assert_eq!(media_files.len(), 3, "{media_files:#?}");
    let episode_ids = media_files
        .iter()
        .filter_map(|media_file| media_file.episode_id.as_deref())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        episode_ids,
        std::collections::BTreeSet::from([
            episode_1.id.as_str(),
            episode_2.id.as_str(),
            episode_3.id.as_str(),
        ]),
        "the recovered names are what place each member on its episode"
    );
}

#[tokio::test]
async fn automatic_import_parks_the_pack_member_srrdb_cannot_recover() {
    let ctx = TestContext::new().await;
    let source_root = tempfile::tempdir().expect("source tempdir");
    let source_dir = source_root.path().join("e5f6a7b8c9d0e1f2a3b4");
    std::fs::create_dir(&source_dir).expect("create obfuscated release folder");
    let sizes: Vec<u64> = OBFUSCATED_PACK
        .iter()
        .enumerate()
        .map(|(index, member)| {
            let path = copy_fixture(&source_dir, "h264_aac.mkv", member);
            pad_file_to(&path, (55 + index as u64) * 1024 * 1024)
        })
        .collect();
    // The third member is a miss: srrdb has nothing unambiguous for it.
    let lookup = RecordingSrrdbLookup::new(&[
        (
            sizes[0],
            "Parked.Pals.S01E01.1080p.WEB-DL.H264-LANTERNS.mkv",
        ),
        (
            sizes[1],
            "Parked.Pals.S01E02.1080p.WEB-DL.H264-LANTERNS.mkv",
        ),
    ]);
    let app = app_with_srrdb_recovery(&ctx, lookup.clone()).await;
    let user = ctx.app.find_or_create_default_user().await.unwrap();
    let dest_root = tempfile::tempdir().expect("dest tempdir");
    let title = add_short_form_series_title(
        &ctx,
        "title-srrdb-partial-pack",
        "Parked Pals",
        dest_root.path().to_str().unwrap(),
    )
    .await;
    let episode_1 = seed_short_form_series_episode(&ctx, &title).await;
    let collection_id = episode_1.collection_id.as_deref().expect("collection id");
    let episode_2 = seed_second_short_form_series_episode(&ctx, &title, collection_id).await;
    let episode_3 =
        seed_series_episode_in_collection(&ctx, &title, collection_id, 3, Some(60)).await;
    let completed = scryer_completed(
        "dl-srrdb-partial-pack",
        source_dir.to_str().unwrap(),
        &title.id,
        "series",
    );

    let result = import_completed_download(&app, &user, &completed)
        .await
        .expect("import partially recovered series pack");

    assert_eq!(lookup.calls().len(), 3, "{:?}", lookup.calls());
    let media_files = ctx
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files");
    let episode_ids = media_files
        .iter()
        .filter_map(|media_file| media_file.episode_id.as_deref())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        episode_ids,
        std::collections::BTreeSet::from([episode_1.id.as_str(), episode_2.id.as_str()]),
        "the recovered members import; the unrecovered one must not be guessed: {result:?}"
    );
    assert!(
        !episode_ids.contains(episode_3.id.as_str()),
        "an unrecovered member parks for manual import instead of landing somewhere"
    );
    assert!(
        source_dir.join(OBFUSCATED_PACK[2]).exists(),
        "the parked member stays on disk under its physical name"
    );
}

#[tokio::test]
async fn manual_import_never_asks_srrdb_for_a_filename() {
    let ctx = TestContext::new().await;
    let lookup = RecordingSrrdbLookup::new(&[]);
    let app = app_with_srrdb_recovery(&ctx, lookup.clone()).await;
    let user = ctx.app.find_or_create_default_user().await.unwrap();
    let source_root = tempfile::tempdir().expect("source tempdir");
    let source_dir = source_root.path().join("f6a7b8c9d0e1f2a3b4c5");
    std::fs::create_dir(&source_dir).expect("create obfuscated release folder");
    let source_file_1 = copy_fixture(&source_dir, "h264_aac.mkv", OBFUSCATED_PACK[0]);
    let source_file_2 = copy_fixture(&source_dir, "h264_aac.mkv", OBFUSCATED_PACK[1]);
    pad_file_to(&source_file_1, 58 * 1024 * 1024);
    pad_file_to(&source_file_2, 59 * 1024 * 1024);
    let dest_root = tempfile::tempdir().expect("dest tempdir");
    let title = add_series_title(
        &ctx,
        "title-srrdb-manual",
        "Manual Pals",
        dest_root.path().to_str().unwrap(),
    )
    .await;
    let episode_1 = seed_series_episode(&ctx, &title).await;
    let episode_2 = seed_second_series_episode(
        &ctx,
        &title,
        episode_1.collection_id.as_deref().expect("collection id"),
    )
    .await;
    let completed = scryer_completed(
        "dl-srrdb-manual",
        source_dir.to_str().unwrap(),
        &title.id,
        "series",
    );
    let import_id = queue_import_record(&ctx, &completed).await;

    let results = scryer_application::execute_manual_import(
        &app,
        &user,
        &import_id,
        &title.id,
        Some(&completed),
        vec![
            scryer_application::ManualImportFileMapping {
                file_path: source_file_1.to_string_lossy().to_string(),
                episode_id: Some(episode_1.id.clone()),
                series_movie_link_id: None,
            },
            scryer_application::ManualImportFileMapping {
                file_path: source_file_2.to_string_lossy().to_string(),
                episode_id: Some(episode_2.id.clone()),
                series_movie_link_id: None,
            },
        ],
        Some(source_dir.clone()),
    )
    .await
    .expect("execute manual import of obfuscated files");

    assert!(
        results.iter().all(|result| result.success),
        "manual mapping is explicit and must not need srrdb: {results:#?}"
    );
    assert!(
        lookup.calls().is_empty(),
        "manual import never asks a third party what a file is called: {:?}",
        lookup.calls()
    );
}

#[tokio::test]
async fn a_titleless_obfuscated_download_matches_and_imports_on_its_recovered_name() {
    let ctx = TestContext::new().await;
    // Nothing binds this download to a title: no Scryer parameters, an
    // obfuscated release folder and an obfuscated member. The recovered name
    // is the only title signal that exists.
    let source_root = tempfile::tempdir().expect("source tempdir");
    let source_dir = source_root.path().join("a7b8c9d0e1f2a3b4c5d6");
    std::fs::create_dir(&source_dir).expect("create obfuscated release folder");
    let source_file = copy_fixture(&source_dir, "h264_aac.mkv", OBFUSCATED_PACK[0]);
    let size = pad_file_to(&source_file, 61 * 1024 * 1024);
    let lookup = RecordingSrrdbLookup::new(&[(
        size,
        "Titleless.Pals.S01E01.1080p.WEB-DL.H264-LANTERNS.mkv",
    )]);
    let app = app_with_srrdb_recovery(&ctx, lookup.clone()).await;
    let user = ctx.app.find_or_create_default_user().await.unwrap();
    let dest_root = tempfile::tempdir().expect("dest tempdir");
    let title = add_short_form_series_title(
        &ctx,
        "title-srrdb-titleless",
        "Titleless Pals",
        dest_root.path().to_str().unwrap(),
    )
    .await;
    let episode = seed_short_form_series_episode(&ctx, &title).await;
    let completed = CompletedDownload {
        client_type: "sabnzbd".to_string(),
        client_id: "test-client".to_string(),
        download_client_item_id: "dl-srrdb-titleless".to_string(),
        download_id: None,
        name: "a7b8c9d0e1f2a3b4c5d6".to_string(),
        release_name: None,
        dest_dir: source_dir.to_str().unwrap().to_string(),
        category: None,
        size_bytes: None,
        completed_at: None,
        parameters: vec![],
    };

    let result = import_completed_download(&app, &user, &completed)
        .await
        .expect("import titleless obfuscated download");

    assert_eq!(
        result.decision,
        ImportDecision::Imported,
        "the recovered name must be enough to match and import: {result:?}"
    );
    assert_eq!(result.title_id.as_deref(), Some(title.id.as_str()));
    assert_eq!(result.episode_ids, vec![episode.id.clone()]);

    // The file on disk is never renamed: what the import recorded having read
    // is the physical name, not the name srrdb handed back.
    let artifacts = ImportStore::new(ctx.db.datastore())
        .list_by_source_identity(&ClientJobLocator::for_import_artifact(
            Some(&completed.client_id),
            &completed.client_type,
            &completed.download_client_item_id,
        ))
        .await
        .expect("list import artifacts");
    assert_eq!(artifacts.len(), 1, "{artifacts:#?}");
    assert_eq!(
        artifacts[0].normalized_file_name,
        OBFUSCATED_PACK[0].to_ascii_lowercase(),
        "the artifact records the physical file name: {artifacts:#?}"
    );
}

#![recursion_limit = "256"]

mod common;

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use async_trait::async_trait;
use common::TestContext;
use scryer_application::recycle_bin::{
    RECYCLE_STATUS_COMMITTED, RecycleBinConfig, RecycleManifest,
};
use scryer_application::testing::{
    AppUseCaseTestExt, UpgradeForTestInput, execute_upgrade_for_test,
    execute_upgrade_for_test_with_import_mode,
};
use scryer_application::upgrade::UpgradeResult;
use scryer_application::{
    ActivityKind, ActivitySeverity, AppError, AppResult, CollectionEpisodeProgressSummary,
    CutoffUnmetQualitySummary, EpisodeScopedMediaFile, FileImporter, InsertMediaFileInput,
    LibraryRootDraft, MediaFileAnalysis, MediaFileRepository, TitleEpisodeProgressSummary,
    TitleMediaFile, TitleMediaSizeSummary, TitleMovieMediaSummary, TitleQualitySummary,
    TitleRepository,
};
use scryer_domain::{
    AppPermission, AppPermissionMask, DomainEvent, DomainEventActorKind, DomainEventFilter,
    DomainEventPayload, DomainEventType, ImportMode, LibraryPermissionMask, MediaFacet,
    MediaFileDeletedReason, Title, User, UserAuthorization,
};
use scryer_infrastructure_library::media::search::media_file_store::MediaFileStore;
use scryer_infrastructure_workflow::workflow::file_importer::FsFileImporter;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn app_with_real_fs(ctx: &TestContext) -> scryer_application::AppUseCase {
    ctx.app.with_test_overrides(|builder| {
        builder
            .with_media_files(Arc::new(ctx.media_files.clone()))
            .with_file_importer(Arc::new(FsFileImporter::new()))
    })
}

fn app_with_cleanup_failing_importer(ctx: &TestContext) -> scryer_application::AppUseCase {
    ctx.app.with_test_overrides(|builder| {
        builder
            .with_media_files(Arc::new(ctx.media_files.clone()))
            .with_file_importer(Arc::new(CleanupFailingFileImporter))
    })
}

fn app_with_failing_media_path_update(
    ctx: &TestContext,
    fail_path: String,
) -> scryer_application::AppUseCase {
    ctx.app.with_test_overrides(|builder| {
        builder
            .with_media_files(Arc::new(FailingPathUpdateMediaFileRepo {
                inner: ctx.media_files.clone(),
                fail_insert: false,
                fail_path,
                rollback_occupant: None,
            }))
            .with_file_importer(Arc::new(FsFileImporter::new()))
    })
}

fn app_with_failing_media_path_update_and_rollback_occupant(
    ctx: &TestContext,
    fail_path: String,
    occupant_path: PathBuf,
    occupant_bytes: Vec<u8>,
) -> scryer_application::AppUseCase {
    ctx.app.with_test_overrides(|builder| {
        builder
            .with_media_files(Arc::new(FailingPathUpdateMediaFileRepo {
                inner: ctx.media_files.clone(),
                fail_insert: false,
                fail_path,
                rollback_occupant: Some((occupant_path, occupant_bytes)),
            }))
            .with_file_importer(Arc::new(FsFileImporter::new()))
    })
}

fn app_with_failing_media_insert(ctx: &TestContext) -> scryer_application::AppUseCase {
    ctx.app.with_test_overrides(|builder| {
        builder
            .with_media_files(Arc::new(FailingPathUpdateMediaFileRepo {
                inner: ctx.media_files.clone(),
                fail_insert: true,
                fail_path: String::new(),
                rollback_occupant: None,
            }))
            .with_file_importer(Arc::new(FsFileImporter::new()))
    })
}

struct CleanupFailingFileImporter;

#[async_trait]
impl FileImporter for CleanupFailingFileImporter {
    async fn snapshot_import_source(
        &self,
        source: &Path,
    ) -> AppResult<scryer_domain::ImportSourceSnapshot> {
        let importer = FsFileImporter::new();
        importer.snapshot_import_source(source).await
    }

    async fn import_file(
        &self,
        source: &Path,
        dest: &Path,
        mode: scryer_domain::ImportMode,
        expected_source: Option<&scryer_domain::ImportSourceSnapshot>,
    ) -> AppResult<scryer_domain::ImportFileResult> {
        let importer = FsFileImporter::new();
        importer
            .import_file(source, dest, mode, expected_source)
            .await
    }

    async fn remove_import_source_after_verified_import(
        &self,
        _guard: scryer_domain::ImportSourceCleanupGuard,
        _final_dest_path: &Path,
    ) -> AppResult<()> {
        Err(AppError::Repository(
            "forced post-commit source cleanup failure".to_string(),
        ))
    }
}

struct FailingPathUpdateMediaFileRepo {
    inner: MediaFileStore,
    fail_insert: bool,
    fail_path: String,
    rollback_occupant: Option<(PathBuf, Vec<u8>)>,
}

#[async_trait]
impl MediaFileRepository for FailingPathUpdateMediaFileRepo {
    async fn insert_media_file(&self, input: &InsertMediaFileInput) -> AppResult<String> {
        if self.fail_insert {
            return Err(AppError::Repository(
                "forced media-file insertion failure".to_string(),
            ));
        }
        self.inner.insert_media_file(input).await
    }

    async fn claim_import_destination(
        &self,
        input: &InsertMediaFileInput,
        associations: &scryer_application::MediaFileAssociations,
    ) -> AppResult<scryer_application::ClaimedMediaFile> {
        if self.fail_insert {
            return Err(AppError::Repository(
                "forced media-file insertion failure".to_string(),
            ));
        }
        self.inner
            .claim_import_destination(input, associations)
            .await
    }

    async fn link_file_to_episode(&self, file_id: &str, episode_id: &str) -> AppResult<()> {
        self.inner.link_file_to_episode(file_id, episode_id).await
    }

    async fn link_file_to_series_movie(
        &self,
        file_id: &str,
        series_movie_link_id: &str,
    ) -> AppResult<()> {
        self.inner
            .link_file_to_series_movie(file_id, series_movie_link_id)
            .await
    }

    async fn list_media_files_for_title(&self, title_id: &str) -> AppResult<Vec<TitleMediaFile>> {
        self.inner.list_media_files_for_title(title_id).await
    }

    async fn list_series_movie_link_ids_with_files_for_title(
        &self,
        title_id: &str,
    ) -> AppResult<Vec<String>> {
        self.inner
            .list_series_movie_link_ids_with_files_for_title(title_id)
            .await
    }

    async fn list_live_media_files_for_episode_ids(
        &self,
        title_id: &str,
        episode_ids: &[String],
    ) -> AppResult<Vec<EpisodeScopedMediaFile>> {
        self.inner
            .list_live_media_files_for_episode_ids(title_id, episode_ids)
            .await
    }

    async fn list_title_media_size_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleMediaSizeSummary>> {
        self.inner.list_title_media_size_summaries(title_ids).await
    }

    async fn collection_media_size_bytes(
        &self,
        title_id: &str,
        ordered_path: &str,
    ) -> AppResult<Option<i64>> {
        self.inner
            .collection_media_size_bytes(title_id, ordered_path)
            .await
    }

    async fn list_title_quality_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleQualitySummary>> {
        self.inner.list_title_quality_summaries(title_ids).await
    }

    async fn list_title_movie_media_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleMovieMediaSummary>> {
        self.inner.list_title_movie_media_summaries(title_ids).await
    }

    async fn list_cutoff_unmet_quality_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<CutoffUnmetQualitySummary>> {
        self.inner
            .list_cutoff_unmet_quality_summaries(title_ids)
            .await
    }

    async fn list_title_episode_progress_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleEpisodeProgressSummary>> {
        self.inner
            .list_title_episode_progress_summaries(title_ids)
            .await
    }

    async fn list_collection_episode_progress_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<CollectionEpisodeProgressSummary>> {
        self.inner
            .list_collection_episode_progress_summaries(title_ids)
            .await
    }

    async fn update_media_file_analysis(
        &self,
        file_id: &str,
        analysis: MediaFileAnalysis,
    ) -> AppResult<()> {
        self.inner
            .update_media_file_analysis(file_id, analysis)
            .await
    }

    async fn update_media_file_source_signature(
        &self,
        file_id: &str,
        size_bytes: i64,
        source_signature_scheme: Option<String>,
        source_signature_value: Option<String>,
    ) -> AppResult<()> {
        self.inner
            .update_media_file_source_signature(
                file_id,
                size_bytes,
                source_signature_scheme,
                source_signature_value,
            )
            .await
    }

    async fn update_media_file_path(&self, file_id: &str, file_path: &str) -> AppResult<()> {
        if file_path == self.fail_path {
            return Err(AppError::Repository(format!(
                "injected media file path failure for {file_id} -> {file_path}"
            )));
        }

        self.inner.update_media_file_path(file_id, file_path).await
    }

    async fn set_media_file_roles_for_title(
        &self,
        title_id: &str,
        primary_file_id: &str,
        additional_file_ids: &[String],
    ) -> AppResult<()> {
        self.inner
            .set_media_file_roles_for_title(title_id, primary_file_id, additional_file_ids)
            .await
    }

    async fn set_media_file_roles_for_episode(
        &self,
        title_id: &str,
        episode_id: &str,
        primary_file_id: &str,
        additional_file_ids: &[String],
    ) -> AppResult<()> {
        self.inner
            .set_media_file_roles_for_episode(
                title_id,
                episode_id,
                primary_file_id,
                additional_file_ids,
            )
            .await
    }

    async fn replace_media_file_for_upgrade(
        &self,
        old_file_id: &str,
        replacement_file_id: &str,
        replacement_file_path: &str,
    ) -> AppResult<()> {
        if replacement_file_path == self.fail_path {
            if let Some((path, bytes)) = &self.rollback_occupant {
                std::fs::write(path, bytes).map_err(|error| {
                    AppError::Repository(format!(
                        "failed to write injected rollback occupant {}: {}",
                        path.display(),
                        error
                    ))
                })?;
            }
            return Err(AppError::Repository(format!(
                "injected media file replacement failure for {old_file_id} -> {replacement_file_id} at {replacement_file_path}"
            )));
        }

        self.inner
            .replace_media_file_for_upgrade(old_file_id, replacement_file_id, replacement_file_path)
            .await
    }

    async fn mark_scan_failed(&self, file_id: &str, error: &str) -> AppResult<()> {
        self.inner.mark_scan_failed(file_id, error).await
    }

    async fn get_media_file_by_id(&self, file_id: &str) -> AppResult<Option<TitleMediaFile>> {
        self.inner.get_media_file_by_id(file_id).await
    }

    async fn get_media_file_by_path(&self, file_path: &str) -> AppResult<Option<TitleMediaFile>> {
        self.inner.get_media_file_by_path(file_path).await
    }

    async fn delete_media_file(&self, file_id: &str) -> AppResult<()> {
        self.inner.delete_media_file(file_id).await
    }
}

async fn seed_title(ctx: &TestContext, id: &str) -> Title {
    let title = Title {
        id: id.to_string(),
        name: "Test Movie".to_string(),
        facet: MediaFacet::Movie,
        library_id: scryer_domain::default_library_id_for_facet(&MediaFacet::Movie),
        monitored: true,
        tags: vec![],
        canonical_tags: vec![],
        external_ids: vec![],
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
        root_folder_id: scryer_domain::root_folder_id_for_path("/data/movies"),
        folder_path: None,
    };
    ctx.titles.create(title.clone()).await.expect("seed title");
    title
}

async fn seed_title_for_library(
    ctx: &TestContext,
    id: &str,
    name: &str,
    library_id: &str,
    root_path: &Path,
) -> Title {
    // Root ids are allocated, not derived from the path (FR-078), so the fixture
    // has to read the id the library actually stored for `root_path`.
    let normalized_root = scryer_domain::normalize_library_root_path(
        root_path.to_string_lossy().as_ref(),
    );
    let root_folder_id = scryer_application::LibraryRepository::get_by_id(&ctx.libraries, library_id)
        .await
        .expect("library should load")
        .expect("library should exist")
        .roots
        .iter()
        .find(|root| scryer_domain::normalize_library_root_path(&root.path) == normalized_root)
        .map(|root| root.id.clone())
        .expect("library should have a root at the seeded path");
    let title = Title {
        id: id.to_string(),
        name: name.to_string(),
        facet: MediaFacet::Movie,
        library_id: library_id.to_string(),
        monitored: true,
        tags: vec![],
        canonical_tags: vec![],
        external_ids: vec![],
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
        root_folder_id,
        folder_path: None,
    };
    ctx.titles.create(title.clone()).await.expect("seed title");
    title
}

fn make_recycle_config(base: &std::path::Path, source_root: &std::path::Path) -> RecycleBinConfig {
    make_recycle_config_with_roots(base, &[source_root])
}

fn make_recycle_config_with_roots(
    base: &std::path::Path,
    source_roots: &[&std::path::Path],
) -> RecycleBinConfig {
    RecycleBinConfig {
        enabled: true,
        base_path: base.to_path_buf(),
        retention_days: 7,
        cleanup_enabled: true,
        validation_error: None,
        source_roots: source_roots.iter().map(|root| root.to_path_buf()).collect(),
    }
}

#[cfg(unix)]
fn force_encoded_stored_path(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;

    let mut encoded = String::from("scryer-path-v1:u:");
    for &byte in path.as_os_str().as_bytes() {
        if (0x20..=0x7e).contains(&byte) && byte != b'%' {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

#[cfg(windows)]
fn force_encoded_stored_path(path: &Path) -> String {
    use std::os::windows::ffi::OsStrExt;

    let mut encoded = String::from("scryer-path-v1:w:");
    for unit in path.as_os_str().encode_wide() {
        if (0x20..=0x7e).contains(&unit) && unit != u16::from(b'%') {
            encoded.push(char::from_u32(unit as u32).unwrap_or_default());
        } else {
            encoded.push_str(&format!("%u{unit:04X}"));
        }
    }
    encoded
}

/// Insert a media file record in the DB and create the physical file.
async fn seed_media_file(
    ctx: &TestContext,
    title_id: &str,
    file_path: &std::path::Path,
    size: i64,
    score: i32,
) -> scryer_application::TitleMediaFile {
    let input = InsertMediaFileInput {
        title_id: title_id.to_string(),
        file_path: file_path.to_string_lossy().to_string(),
        size_bytes: size,
        announced_size_bytes: None,
        quality_label: Some("720p".to_string()),
        acquisition_score: Some(score),
        ..Default::default()
    };
    let file_id = ctx
        .media_files
        .insert_media_file(&input)
        .await
        .expect("insert");
    let files = ctx
        .media_files
        .list_media_files_for_title(title_id)
        .await
        .unwrap();
    files.into_iter().find(|f| f.id == file_id).unwrap()
}

fn last_upgrade_event(
    events: &[scryer_application::ActivityEvent],
) -> Option<&scryer_application::ActivityEvent> {
    events.iter().find(|e| e.kind == ActivityKind::FileUpgraded)
}

async fn upgrade_audit_events(
    app: &scryer_application::AppUseCase,
    actor: &User,
    title_id: &str,
) -> Vec<DomainEvent> {
    app.list_domain_events(
        actor,
        &DomainEventFilter {
            event_types: Some(vec![
                DomainEventType::MediaFileUpgraded,
                DomainEventType::MediaFileDeleted,
            ]),
            title_id: Some(title_id.to_string()),
            after_sequence: Some(0),
            limit: 10,
            ..DomainEventFilter::default()
        },
    )
    .await
    .expect("list upgrade audit events")
}

fn assert_backend_actor_metadata(event: &DomainEvent, actor: &User) {
    assert_eq!(event.actor_kind, DomainEventActorKind::User);
    assert_eq!(event.actor_user_id.as_deref(), Some(actor.id.as_str()));
    assert_eq!(event.actor_display_name, actor.username);
}

fn assert_upgrade_recycle_audit_trail(
    events: &[DomainEvent],
    actor: &User,
    previous_file_id: &str,
    current_file_id: Option<&str>,
) {
    assert_eq!(events.len(), 2, "upgrade should emit two audit events");
    assert!(
        events[0].sequence < events[1].sequence,
        "audit events should be returned in append order"
    );

    assert_backend_actor_metadata(&events[0], actor);
    assert_backend_actor_metadata(&events[1], actor);

    match &events[0].payload {
        DomainEventPayload::MediaFileUpgraded(data) => {
            assert_eq!(data.previous_file_id.as_deref(), Some(previous_file_id));
            if let Some(current_file_id) = current_file_id {
                assert_eq!(data.current_file_id.as_deref(), Some(current_file_id));
            }
        }
        other => panic!("expected MediaFileUpgraded first, got {other:?}"),
    }

    match &events[1].payload {
        DomainEventPayload::MediaFileDeleted(data) => {
            assert_eq!(data.file_id.as_deref(), Some(previous_file_id));
            assert_eq!(data.reason, MediaFileDeletedReason::UpgradeCleanup);
        }
        other => panic!("expected MediaFileDeleted second, got {other:?}"),
    }
}

fn test_actor() -> User {
    User {
        id: scryer_domain::Id::new().0,
        username: "admin".to_string(),
        password_hash: None,
        password_change_required: false,
        account_kind: Default::default(),
        authorization: UserAuthorization {
            actor_capabilities: scryer_domain::ActorCapabilityMask::MANAGE_OWN_ACCOUNT,
            loaded: true,
            default_library: LibraryPermissionMask::from_permissions([
                scryer_domain::LibraryPermission::View,
            ]),
            ..Default::default()
        },
    }
}

fn app_permission_actor(
    username: &str,
    permissions: impl IntoIterator<Item = AppPermission>,
) -> User {
    let mut user = User::new_admin(username);
    user.authorization = UserAuthorization {
        app: AppPermissionMask::from_permissions(permissions),
        actor_capabilities: scryer_domain::ActorCapabilityMask::MANAGE_OWN_ACCOUNT,
        loaded: true,
        ..Default::default()
    };
    user
}

// ---------------------------------------------------------------------------
// Happy path
// ---------------------------------------------------------------------------

#[tokio::test]
async fn upgrade_replaces_old_file_with_new() {
    let ctx = TestContext::new().await;
    let app = app_with_real_fs(&ctx);
    let title = seed_title(&ctx, "title-1").await;
    let mut actor = test_actor();
    actor.username = "Upgrade Auditor".to_string();

    // Set up directories
    let media_dir = tempfile::tempdir().expect("media dir");
    let recycle_dir = tempfile::tempdir().expect("recycle dir");
    let source_dir = tempfile::tempdir().expect("source dir");

    // Create "old" file in media library
    let old_path = media_dir.path().join("Movie.720p.mkv");
    std::fs::write(&old_path, b"old video content 720p").expect("write old");

    // Create "new" higher-quality source file
    let new_source = source_dir.path().join("Movie.1080p.mkv");
    std::fs::write(&new_source, b"new video content 1080p better quality").expect("write new");

    let new_dest = media_dir.path().join("Movie.1080p.mkv");

    // Seed old file in DB
    let existing = seed_media_file(&ctx, "title-1", &old_path, 22, 400).await;

    let parsed = scryer_application::parse_release_metadata("Movie.1080p.WEB-DL.x264");
    let recycle_config = make_recycle_config(recycle_dir.path(), media_dir.path());

    let outcome = execute_upgrade_for_test(
        &app,
        UpgradeForTestInput {
            actor: &actor,
            title: &title,
            existing_file: &existing,
            source_path: &new_source,
            dest_path: &new_dest,
            parsed,
            final_score: 650,
            target_episode_ids: &[],
            media_root: Some(media_dir.path().to_string_lossy().as_ref()),
            recycle_config: &recycle_config,
        },
    )
    .await
    .expect("execute_upgrade");

    let UpgradeResult::Upgraded(outcome) = outcome else {
        panic!("expected upgrade to succeed");
    };

    assert_eq!(outcome.old_score, 400);
    assert_eq!(outcome.new_score, 650);
    assert!(
        outcome.recycle_entry_committed,
        "successful upgrade should commit recycle proof"
    );

    // New file should exist at destination
    assert!(new_dest.exists(), "new file should exist");

    // Old file should be gone from original location (recycled)
    assert!(!old_path.exists(), "old file should be recycled");

    // Recycle dir should contain a committed entry for the replaced file.
    let recycle_entries: Vec<_> = std::fs::read_dir(recycle_dir.path())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .collect();
    assert_eq!(
        recycle_entries.len(),
        1,
        "recycle bin should have one entry"
    );
    let manifest_bytes = std::fs::read(recycle_entries[0].path().join("manifest.json")).unwrap();
    let manifest: RecycleManifest = serde_json::from_slice(&manifest_bytes).unwrap();
    assert!(
        manifest.entry_id.is_some(),
        "committed entry should have an id"
    );
    assert_eq!(manifest.status.as_deref(), Some(RECYCLE_STATUS_COMMITTED));
    assert_eq!(
        manifest.original_file_id.as_deref(),
        Some(existing.id.as_str())
    );
    assert_eq!(
        manifest.media_root.as_deref(),
        Some(media_dir.path().to_string_lossy().as_ref())
    );
    assert_eq!(
        manifest.replacement_file_id.as_deref(),
        Some(outcome.new_file_id.as_str())
    );
    assert_eq!(
        manifest.replacement_path.as_deref(),
        Some(new_dest.to_string_lossy().as_ref())
    );

    // DB should have the new file, not the old one
    let files = ctx
        .media_files
        .list_media_files_for_title("title-1")
        .await
        .unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].id, outcome.new_file_id);
    assert_eq!(files[0].acquisition_score, Some(650));

    // Activity event should be recorded
    let events = app
        .recent_activity(&actor, 10, 0)
        .await
        .expect("recent activity");
    let upgrade_event = last_upgrade_event(&events).expect("should have upgrade event");
    assert_eq!(upgrade_event.severity, ActivitySeverity::Success);
    assert!(upgrade_event.message.contains("400"));
    assert!(upgrade_event.message.contains("650"));
    assert!(upgrade_event.message.contains("Test Movie"));

    let audit_events = upgrade_audit_events(&app, &actor, &title.id).await;
    assert_upgrade_recycle_audit_trail(
        &audit_events,
        &actor,
        existing.id.as_str(),
        Some(outcome.new_file_id.as_str()),
    );
}

#[tokio::test]
async fn upgrade_after_root_change_recycles_old_file_from_old_root() {
    let ctx = TestContext::new().await;
    let app = app_with_real_fs(&ctx);
    let title = seed_title(&ctx, "title-root-change-upgrade").await;
    let actor = test_actor();

    let old_root = tempfile::tempdir().expect("old media root");
    let new_root = tempfile::tempdir().expect("new media root");
    let recycle_dir = tempfile::tempdir().expect("recycle dir");
    let source_dir = tempfile::tempdir().expect("source dir");

    let old_path = old_root.path().join("Movie.720p.mkv");
    std::fs::write(&old_path, b"old root video").expect("write old");
    let new_source = source_dir.path().join("Movie.1080p.mkv");
    std::fs::write(&new_source, b"new root replacement video").expect("write new");
    let new_dest = new_root.path().join("Movie.1080p.mkv");

    let existing = seed_media_file(&ctx, &title.id, &old_path, 14, 300).await;
    let parsed = scryer_application::parse_release_metadata("Movie.1080p.WEB-DL.x264");
    let recycle_config =
        make_recycle_config_with_roots(recycle_dir.path(), &[old_root.path(), new_root.path()]);

    let result = execute_upgrade_for_test(
        &app,
        UpgradeForTestInput {
            actor: &actor,
            title: &title,
            existing_file: &existing,
            source_path: &new_source,
            dest_path: &new_dest,
            parsed,
            final_score: 650,
            target_episode_ids: &[],
            media_root: Some(new_root.path().to_string_lossy().as_ref()),
            recycle_config: &recycle_config,
        },
    )
    .await
    .expect("upgrade should succeed with split old and replacement roots");

    let UpgradeResult::Upgraded(outcome) = result else {
        panic!("expected upgrade to succeed");
    };
    assert!(outcome.recycle_entry_committed);
    assert!(new_dest.exists(), "replacement should land on the new root");
    assert!(
        !old_path.exists(),
        "old file should be recycled from the old root"
    );

    let recycle_entries: Vec<_> = std::fs::read_dir(recycle_dir.path())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .collect();
    assert_eq!(recycle_entries.len(), 1);
    let manifest_bytes = std::fs::read(recycle_entries[0].path().join("manifest.json")).unwrap();
    let manifest: RecycleManifest = serde_json::from_slice(&manifest_bytes).unwrap();
    assert_eq!(manifest.status.as_deref(), Some(RECYCLE_STATUS_COMMITTED));
    assert_eq!(
        manifest.media_root.as_deref(),
        Some(old_root.path().to_string_lossy().as_ref())
    );
    assert_eq!(
        manifest.replacement_path.as_deref(),
        Some(new_dest.to_string_lossy().as_ref())
    );

    let files = ctx
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].id, outcome.new_file_id);
    assert_eq!(files[0].file_path, new_dest.to_string_lossy());
}

#[tokio::test]
async fn upgrade_audit_events_survive_move_source_cleanup_failure() {
    let ctx = TestContext::new().await;
    let app = app_with_cleanup_failing_importer(&ctx);
    let title = seed_title(&ctx, "title-cleanup-failure").await;
    let mut actor = test_actor();
    actor.username = "Cleanup Failure Auditor".to_string();

    let media_dir = tempfile::tempdir().expect("media dir");
    let recycle_dir = tempfile::tempdir().expect("recycle dir");
    let source_dir = tempfile::tempdir().expect("source dir");

    let old_path = media_dir.path().join("Movie.720p.mkv");
    std::fs::write(&old_path, b"old video content 720p").expect("write old");

    let new_source = source_dir.path().join("Movie.1080p.mkv");
    std::fs::write(&new_source, b"new video content 1080p better quality").expect("write new");

    let new_dest = media_dir.path().join("Movie.1080p.mkv");
    let existing = seed_media_file(&ctx, &title.id, &old_path, 22, 400).await;
    let parsed = scryer_application::parse_release_metadata("Movie.1080p.WEB-DL.x264");
    let recycle_config = make_recycle_config(recycle_dir.path(), media_dir.path());

    let result = execute_upgrade_for_test_with_import_mode(
        &app,
        UpgradeForTestInput {
            actor: &actor,
            title: &title,
            existing_file: &existing,
            source_path: &new_source,
            dest_path: &new_dest,
            parsed,
            final_score: 650,
            target_episode_ids: &[],
            media_root: Some(media_dir.path().to_string_lossy().as_ref()),
            recycle_config: &recycle_config,
        },
        ImportMode::Move,
    )
    .await;

    let Err(err) = result else {
        panic!("expected post-commit source cleanup failure");
    };
    assert!(
        format!("{err:?}").contains("forced post-commit source cleanup failure"),
        "unexpected cleanup error: {err:?}"
    );

    let audit_events = upgrade_audit_events(&app, &actor, &title.id).await;
    assert_upgrade_recycle_audit_trail(&audit_events, &actor, existing.id.as_str(), None);
}

// ---------------------------------------------------------------------------
// Rollback on import failure
// ---------------------------------------------------------------------------

#[tokio::test]
async fn upgrade_restores_old_file_on_import_failure() {
    let ctx = TestContext::new().await;
    let app = app_with_real_fs(&ctx);
    let title = seed_title(&ctx, "title-2").await;
    let actor = test_actor();

    let media_dir = tempfile::tempdir().expect("media dir");
    let recycle_dir = tempfile::tempdir().expect("recycle dir");

    // Create old file
    let old_path = media_dir.path().join("Movie.720p.mkv");
    std::fs::write(&old_path, b"old video content").expect("write old");

    // Source file does NOT exist — this will cause import to fail
    let bad_source = std::path::PathBuf::from("/nonexistent/path/does/not/exist.mkv");
    let new_dest = media_dir.path().join("Movie.1080p.mkv");

    let existing = seed_media_file(&ctx, "title-2", &old_path, 17, 400).await;
    let parsed = scryer_application::parse_release_metadata("Movie.1080p.WEB-DL");
    let recycle_config = make_recycle_config(recycle_dir.path(), media_dir.path());

    let result = execute_upgrade_for_test(
        &app,
        UpgradeForTestInput {
            actor: &actor,
            title: &title,
            existing_file: &existing,
            source_path: &bad_source,
            dest_path: &new_dest,
            parsed,
            final_score: 700,
            target_episode_ids: &[],
            media_root: Some(media_dir.path().to_string_lossy().as_ref()),
            recycle_config: &recycle_config,
        },
    )
    .await;

    // Should fail
    assert!(
        result.is_err(),
        "upgrade should fail when source is missing"
    );

    // Old file should be RESTORED (not lost)
    assert!(
        old_path.exists(),
        "old file should be restored after failed upgrade"
    );

    // Content should match original
    let content = std::fs::read_to_string(&old_path).unwrap();
    assert_eq!(content, "old video content");

    let recycle_entries: Vec<_> = std::fs::read_dir(recycle_dir.path())
        .ok()
        .into_iter()
        .flat_map(|entries| entries.filter_map(Result::ok))
        .filter(|entry| entry.path().is_dir())
        .collect();
    assert_eq!(
        recycle_entries.len(),
        0,
        "failed import should not recycle the old file before replacement validation"
    );
}

// ---------------------------------------------------------------------------
// Disabled recycle bin (safe refusal)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn upgrade_with_disabled_recycle_bin() {
    let ctx = TestContext::new().await;
    let app = app_with_real_fs(&ctx);
    let title = seed_title(&ctx, "title-3").await;
    let actor = test_actor();

    let media_dir = tempfile::tempdir().expect("media dir");
    let source_dir = tempfile::tempdir().expect("source dir");

    let old_path = media_dir.path().join("Movie.720p.mkv");
    std::fs::write(&old_path, b"old content").expect("write old");

    let new_source = source_dir.path().join("Movie.1080p.mkv");
    std::fs::write(&new_source, b"new content 1080p better").expect("write new");

    let new_dest = media_dir.path().join("Movie.1080p.mkv");

    let existing = seed_media_file(&ctx, "title-3", &old_path, 11, 300).await;
    let parsed = scryer_application::parse_release_metadata("Movie.1080p.WEB-DL");

    let disabled_config = RecycleBinConfig {
        enabled: false,
        base_path: std::path::PathBuf::from("/tmp/unused"),
        retention_days: 7,
        cleanup_enabled: true,
        validation_error: None,
        source_roots: vec![media_dir.path().to_path_buf()],
    };

    let result = execute_upgrade_for_test(
        &app,
        UpgradeForTestInput {
            actor: &actor,
            title: &title,
            existing_file: &existing,
            source_path: &new_source,
            dest_path: &new_dest,
            parsed,
            final_score: 600,
            target_episode_ids: &[],
            media_root: Some(media_dir.path().to_string_lossy().as_ref()),
            recycle_config: &disabled_config,
        },
    )
    .await;

    let outcome =
        result.expect("upgrade should direct-delete old file when recycle bin is disabled");
    let UpgradeResult::Upgraded(outcome) = outcome else {
        panic!("upgrade should be accepted");
    };
    assert!(
        !outcome.recycle_entry_committed,
        "disabled recycle bin should not report a committed recycle entry"
    );
    assert!(!old_path.exists(), "old file should be removed directly");
    assert!(new_dest.exists(), "new file should be imported");
}

#[tokio::test]
async fn upgrade_with_unknown_source_roots_keeps_old_record() {
    let ctx = TestContext::new().await;
    let app = app_with_real_fs(&ctx);
    let title = seed_title(&ctx, "title-rootless").await;
    let actor = test_actor();

    let media_dir = tempfile::tempdir().expect("media dir");
    let source_dir = tempfile::tempdir().expect("source dir");

    let old_path = media_dir.path().join("Movie.720p.mkv");
    std::fs::write(&old_path, b"old content").expect("write old");
    let new_source = source_dir.path().join("Movie.1080p.mkv");
    std::fs::write(&new_source, b"new content 1080p better").expect("write new");
    let new_dest = media_dir.path().join("Movie.1080p.mkv");

    let existing = seed_media_file(&ctx, "title-rootless", &old_path, 11, 300).await;
    let parsed = scryer_application::parse_release_metadata("Movie.1080p.WEB-DL");
    let rootless_config = RecycleBinConfig {
        enabled: false,
        base_path: std::path::PathBuf::from("/tmp/unused"),
        retention_days: 7,
        cleanup_enabled: true,
        validation_error: None,
        source_roots: Vec::new(),
    };

    let result = execute_upgrade_for_test(
        &app,
        UpgradeForTestInput {
            actor: &actor,
            title: &title,
            existing_file: &existing,
            source_path: &new_source,
            dest_path: &new_dest,
            parsed,
            final_score: 600,
            target_episode_ids: &[],
            media_root: None,
            recycle_config: &rootless_config,
        },
    )
    .await;

    let error = match result {
        Ok(_) => panic!("rootless recycle config should refuse old-file cleanup"),
        Err(error) => error,
    };
    assert!(
        error.to_string().contains("no configured media roots"),
        "unexpected error: {error}"
    );
    assert!(old_path.exists(), "old file should remain on disk");
    let existing_after = ctx
        .media_files
        .get_media_file_by_id(&existing.id)
        .await
        .expect("lookup existing media file")
        .expect("existing DB row should remain");
    assert_eq!(
        existing_after.file_path,
        old_path.to_string_lossy().to_string()
    );
}

#[tokio::test]
async fn upgrade_with_out_of_root_old_path_keeps_old_record_before_replacement() {
    let ctx = TestContext::new().await;
    let app = app_with_real_fs(&ctx);
    let title = seed_title(&ctx, "title-out-of-root").await;
    let actor = test_actor();

    let media_dir = tempfile::tempdir().expect("media dir");
    let outside_dir = tempfile::tempdir().expect("outside dir");
    let source_dir = tempfile::tempdir().expect("source dir");

    let old_path = outside_dir.path().join("Movie.720p.mkv");
    std::fs::write(&old_path, b"old content").expect("write old");
    let new_source = source_dir.path().join("Movie.1080p.mkv");
    std::fs::write(&new_source, b"new content 1080p better").expect("write new");
    let new_dest = media_dir.path().join("Movie.1080p.mkv");

    let existing = seed_media_file(&ctx, "title-out-of-root", &old_path, 11, 300).await;
    let parsed = scryer_application::parse_release_metadata("Movie.1080p.WEB-DL");
    let config = RecycleBinConfig {
        enabled: false,
        base_path: std::path::PathBuf::from("/tmp/unused"),
        retention_days: 7,
        cleanup_enabled: true,
        validation_error: None,
        source_roots: vec![media_dir.path().to_path_buf()],
    };

    let result = execute_upgrade_for_test(
        &app,
        UpgradeForTestInput {
            actor: &actor,
            title: &title,
            existing_file: &existing,
            source_path: &new_source,
            dest_path: &new_dest,
            parsed,
            final_score: 600,
            target_episode_ids: &[],
            media_root: Some(media_dir.path().to_string_lossy().as_ref()),
            recycle_config: &config,
        },
    )
    .await;

    let error = match result {
        Ok(_) => panic!("out-of-root old file should refuse old-file cleanup before replacement"),
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains("outside the configured media roots"),
        "unexpected error: {error}"
    );
    assert!(old_path.exists(), "old file should remain on disk");
    assert!(
        !new_dest.exists(),
        "replacement should not be imported after preflight refusal"
    );
    let existing_after = ctx
        .media_files
        .get_media_file_by_id(&existing.id)
        .await
        .expect("lookup existing media file")
        .expect("existing DB row should remain");
    assert_eq!(
        existing_after.file_path,
        old_path.to_string_lossy().to_string()
    );
}

#[tokio::test]
async fn disabled_recycle_bin_same_path_upgrade_keeps_backup_until_verified() {
    let ctx = TestContext::new().await;
    let app = app_with_real_fs(&ctx);
    let title = seed_title(&ctx, "title-4").await;
    let actor = test_actor();

    let media_dir = tempfile::tempdir().expect("media dir");
    let source_dir = tempfile::tempdir().expect("source dir");

    let old_path = media_dir.path().join("Movie.mkv");
    std::fs::write(&old_path, b"old same-path content").expect("write old");

    let new_source = source_dir.path().join("Movie.1080p.mkv");
    std::fs::write(&new_source, b"new same-path content").expect("write new");

    let existing = seed_media_file(&ctx, "title-4", &old_path, 21, 300).await;
    let parsed = scryer_application::parse_release_metadata("Movie.1080p.WEB-DL");
    let disabled_config = RecycleBinConfig {
        enabled: false,
        base_path: std::path::PathBuf::from("/tmp/unused"),
        retention_days: 7,
        cleanup_enabled: true,
        validation_error: None,
        source_roots: vec![media_dir.path().to_path_buf()],
    };

    let result = execute_upgrade_for_test(
        &app,
        UpgradeForTestInput {
            actor: &actor,
            title: &title,
            existing_file: &existing,
            source_path: &new_source,
            dest_path: &old_path,
            parsed,
            final_score: 650,
            target_episode_ids: &[],
            media_root: Some(media_dir.path().to_string_lossy().as_ref()),
            recycle_config: &disabled_config,
        },
    )
    .await
    .expect("same-path disabled recycle upgrade should succeed");

    let UpgradeResult::Upgraded(outcome) = result else {
        panic!("upgrade should be accepted");
    };
    assert!(!outcome.recycle_entry_committed);
    assert_eq!(
        std::fs::read(&old_path).expect("read final path"),
        b"new same-path content"
    );
    let leftovers = std::fs::read_dir(media_dir.path())
        .expect("read media dir")
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(".scryer-upgrade-")
        })
        .collect::<Vec<_>>();
    assert!(leftovers.is_empty(), "guard files should be cleaned up");

    let files = ctx
        .media_files
        .list_media_files_for_title("title-4")
        .await
        .expect("list media files");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].id, outcome.new_file_id);
    assert_eq!(files[0].file_path, old_path.to_string_lossy());
}

#[tokio::test]
async fn recycle_bin_same_path_upgrade_recycles_original_filename() {
    let ctx = TestContext::new().await;
    let app = app_with_real_fs(&ctx);
    let title = seed_title(&ctx, "title-4a").await;
    let actor = test_actor();

    let media_dir = tempfile::tempdir().expect("media dir");
    let recycle_dir = tempfile::tempdir().expect("recycle dir");
    let source_dir = tempfile::tempdir().expect("source dir");

    let old_path = media_dir.path().join("Movie.mkv");
    std::fs::write(&old_path, b"old same-path content").expect("write old");

    let new_source = source_dir.path().join("Movie.1080p.mkv");
    std::fs::write(&new_source, b"new same-path content").expect("write new");

    let existing = seed_media_file(&ctx, "title-4a", &old_path, 21, 300).await;
    let parsed = scryer_application::parse_release_metadata("Movie.1080p.WEB-DL");
    let recycle_config = make_recycle_config(recycle_dir.path(), media_dir.path());

    let result = execute_upgrade_for_test(
        &app,
        UpgradeForTestInput {
            actor: &actor,
            title: &title,
            existing_file: &existing,
            source_path: &new_source,
            dest_path: &old_path,
            parsed,
            final_score: 650,
            target_episode_ids: &[],
            media_root: Some(media_dir.path().to_string_lossy().as_ref()),
            recycle_config: &recycle_config,
        },
    )
    .await
    .expect("same-path recycle upgrade should succeed");

    let UpgradeResult::Upgraded(outcome) = result else {
        panic!("upgrade should be accepted");
    };
    assert!(outcome.recycle_entry_committed);
    assert_eq!(
        std::fs::read(&old_path).expect("read final path"),
        b"new same-path content"
    );

    let leftovers = std::fs::read_dir(media_dir.path())
        .expect("read media dir")
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(".scryer-upgrade-")
        })
        .collect::<Vec<_>>();
    assert!(leftovers.is_empty(), "guard files should be cleaned up");

    let recycle_entries: Vec<_> = std::fs::read_dir(recycle_dir.path())
        .expect("read recycle dir")
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .collect();
    assert_eq!(recycle_entries.len(), 1, "old file should be recycled once");
    let entry_dir = recycle_entries[0].path();
    assert!(
        entry_dir.join("Movie.mkv").exists(),
        "same-path recycle should store the original filename, not the guard filename"
    );

    let manifest_bytes = std::fs::read(entry_dir.join("manifest.json")).unwrap();
    let manifest: RecycleManifest = serde_json::from_slice(&manifest_bytes).unwrap();
    assert_eq!(manifest.status.as_deref(), Some(RECYCLE_STATUS_COMMITTED));
    assert_eq!(manifest.original_path, old_path.to_string_lossy());
    assert_eq!(
        manifest.replacement_file_id.as_deref(),
        Some(outcome.new_file_id.as_str())
    );
    assert_eq!(
        manifest.replacement_path.as_deref(),
        Some(old_path.to_string_lossy().as_ref())
    );

    let files = ctx
        .media_files
        .list_media_files_for_title("title-4a")
        .await
        .expect("list media files");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].id, outcome.new_file_id);
    assert_eq!(files[0].file_path, old_path.to_string_lossy());
}

#[tokio::test]
async fn disabled_recycle_bin_same_path_path_update_failure_preserves_old_file() {
    let ctx = TestContext::new().await;
    let title = seed_title(&ctx, "title-4b").await;
    let actor = test_actor();

    let media_dir = tempfile::tempdir().expect("media dir");
    let source_dir = tempfile::tempdir().expect("source dir");

    let old_path = media_dir.path().join("Movie.mkv");
    std::fs::write(&old_path, b"old same-path content").expect("write old");

    let new_source = source_dir.path().join("Movie.1080p.mkv");
    std::fs::write(&new_source, b"new same-path content").expect("write new");

    let existing = seed_media_file(&ctx, "title-4b", &old_path, 21, 300).await;
    let app = app_with_failing_media_path_update(&ctx, old_path.to_string_lossy().to_string());
    let parsed = scryer_application::parse_release_metadata("Movie.1080p.WEB-DL");
    let disabled_config = RecycleBinConfig {
        enabled: false,
        base_path: std::path::PathBuf::from("/tmp/unused"),
        retention_days: 7,
        cleanup_enabled: true,
        validation_error: None,
        source_roots: vec![media_dir.path().to_path_buf()],
    };

    let result = execute_upgrade_for_test(
        &app,
        UpgradeForTestInput {
            actor: &actor,
            title: &title,
            existing_file: &existing,
            source_path: &new_source,
            dest_path: &old_path,
            parsed,
            final_score: 650,
            target_episode_ids: &[],
            media_root: Some(media_dir.path().to_string_lossy().as_ref()),
            recycle_config: &disabled_config,
        },
    )
    .await;

    assert!(result.is_err(), "path update failure should abort upgrade");
    assert_eq!(
        std::fs::read(&old_path).expect("read original path"),
        b"old same-path content"
    );
    let leftovers = std::fs::read_dir(media_dir.path())
        .expect("read media dir")
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(".scryer-upgrade-")
        })
        .collect::<Vec<_>>();
    assert!(leftovers.is_empty(), "guard files should be cleaned up");

    let files = ctx
        .media_files
        .list_media_files_for_title("title-4b")
        .await
        .expect("list media files");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].id, existing.id);
    assert_eq!(files[0].file_path, old_path.to_string_lossy());
}

#[tokio::test]
async fn recycle_upgrade_db_replacement_failure_restores_old_file_from_pending_entry() {
    let ctx = TestContext::new().await;
    let title = seed_title(&ctx, "title-4c").await;
    let actor = test_actor();

    let media_dir = tempfile::tempdir().expect("media dir");
    let source_dir = tempfile::tempdir().expect("source dir");
    let recycle_dir = tempfile::tempdir().expect("recycle dir");

    let old_path = media_dir.path().join("Movie.720p.mkv");
    std::fs::write(&old_path, b"old distinct content").expect("write old");

    let new_source = source_dir.path().join("Movie.1080p.mkv");
    std::fs::write(&new_source, b"new distinct content").expect("write new");
    let new_dest = media_dir.path().join("Movie.1080p.mkv");

    let existing = seed_media_file(&ctx, "title-4c", &old_path, 20, 300).await;
    let app = app_with_failing_media_path_update(&ctx, new_dest.to_string_lossy().to_string());
    let parsed = scryer_application::parse_release_metadata("Movie.1080p.WEB-DL");
    let recycle_config = make_recycle_config(recycle_dir.path(), media_dir.path());

    let result = execute_upgrade_for_test(
        &app,
        UpgradeForTestInput {
            actor: &actor,
            title: &title,
            existing_file: &existing,
            source_path: &new_source,
            dest_path: &new_dest,
            parsed,
            final_score: 650,
            target_episode_ids: &[],
            media_root: Some(media_dir.path().to_string_lossy().as_ref()),
            recycle_config: &recycle_config,
        },
    )
    .await;

    assert!(
        result.is_err(),
        "DB replacement failure should abort upgrade"
    );
    assert_eq!(
        std::fs::read(&old_path).expect("old file restored"),
        b"old distinct content"
    );
    assert!(
        !new_dest.exists(),
        "replacement file should be rolled back after DB failure"
    );
    let recycle_entries = std::fs::read_dir(recycle_dir.path())
        .expect("read recycle dir")
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .collect::<Vec<_>>();
    assert!(
        recycle_entries.is_empty(),
        "pending recycle entry should be removed after rollback"
    );

    let files = ctx
        .media_files
        .list_media_files_for_title("title-4c")
        .await
        .expect("list media files");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].id, existing.id);
    assert_eq!(files[0].file_path, old_path.to_string_lossy());
}

#[tokio::test]
async fn disabled_recycle_distinct_path_rollback_refuses_occupied_original_path() {
    let ctx = TestContext::new().await;
    let title = seed_title(&ctx, "title-4d").await;
    let actor = test_actor();

    let media_dir = tempfile::tempdir().expect("media dir");
    let source_dir = tempfile::tempdir().expect("source dir");

    let old_path = media_dir.path().join("Movie.720p.mkv");
    std::fs::write(&old_path, b"old distinct content").expect("write old");

    let new_source = source_dir.path().join("Movie.1080p.mkv");
    std::fs::write(&new_source, b"new distinct content").expect("write new");
    let new_dest = media_dir.path().join("Movie.1080p.mkv");

    let existing = seed_media_file(&ctx, "title-4d", &old_path, 20, 300).await;
    let app = app_with_failing_media_path_update_and_rollback_occupant(
        &ctx,
        new_dest.to_string_lossy().to_string(),
        old_path.clone(),
        b"unexpected rollback occupant".to_vec(),
    );
    let parsed = scryer_application::parse_release_metadata("Movie.1080p.WEB-DL");
    let disabled_config = RecycleBinConfig {
        enabled: false,
        base_path: std::path::PathBuf::from("/tmp/unused"),
        retention_days: 7,
        cleanup_enabled: true,
        validation_error: None,
        source_roots: vec![media_dir.path().to_path_buf()],
    };

    let result = execute_upgrade_for_test(
        &app,
        UpgradeForTestInput {
            actor: &actor,
            title: &title,
            existing_file: &existing,
            source_path: &new_source,
            dest_path: &new_dest,
            parsed,
            final_score: 650,
            target_episode_ids: &[],
            media_root: Some(media_dir.path().to_string_lossy().as_ref()),
            recycle_config: &disabled_config,
        },
    )
    .await;
    let error = match result {
        Ok(_) => panic!("occupied rollback destination should fail closed"),
        Err(error) => error,
    };

    assert!(
        error.to_string().contains("destination is occupied"),
        "unexpected error: {error}"
    );
    assert_eq!(
        std::fs::read(&old_path).expect("read occupied original path"),
        b"unexpected rollback occupant"
    );
    assert!(
        !new_dest.exists(),
        "replacement file should be rolled back after DB failure"
    );
    let backups = std::fs::read_dir(media_dir.path())
        .expect("read media dir")
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(".scryer-upgrade-old-")
        })
        .collect::<Vec<_>>();
    assert_eq!(backups.len(), 1, "old backup should be preserved");
    assert_eq!(
        std::fs::read(backups[0].path()).expect("read preserved backup"),
        b"old distinct content"
    );

    let files = ctx
        .media_files
        .list_media_files_for_title("title-4d")
        .await
        .expect("list media files");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].id, existing.id);
}

#[tokio::test]
async fn housekeeping_reconciles_same_path_guard_before_db_swap() {
    let ctx = TestContext::new().await;
    let media_dir = tempfile::tempdir().expect("media dir");
    let catalog_actor = app_permission_actor("catalog", [AppPermission::ManageCatalogSettings]);
    let housekeeping_actor =
        app_permission_actor("housekeeping", [AppPermission::ManageSystemSettings]);
    let library = ctx
        .app
        .create_library(
            &catalog_actor,
            MediaFacet::Movie,
            "Recovery Library".to_string(),
            vec![LibraryRootDraft {
                path: media_dir.path().to_string_lossy().to_string(),
                is_default: true,
            }],
            None,
        )
        .await
        .expect("create library");

    let title = Title {
        id: "title-recovery".to_string(),
        name: "Recovery Movie".to_string(),
        facet: MediaFacet::Movie,
        library_id: library.id.clone(),
        monitored: true,
        tags: vec![],
        canonical_tags: vec![],
        external_ids: vec![],
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
        // Root ids are allocated, not derived from the path, so take the library's own.
        root_folder_id: library
            .roots
            .first()
            .map(|root| root.id.clone())
            .expect("created library should expose its root"),
        folder_path: None,
    };
    ctx.titles.create(title.clone()).await.expect("seed title");

    let final_path = media_dir.path().join("Movie.mkv");
    std::fs::write(&final_path, b"old content").expect("write old final");
    let old_file = seed_media_file(&ctx, &title.id, &final_path, 11, 300).await;

    let backup_path = media_dir
        .path()
        .join(".scryer-upgrade-old-recovery-Movie.mkv");
    std::fs::rename(&final_path, &backup_path).expect("move old to backup");
    std::fs::write(&final_path, b"new uncommitted content").expect("write replacement final");
    let staged_replacement_path = media_dir
        .path()
        .join(".scryer-upgrade-replacement-recovery-Movie.mkv");
    let replacement_id = ctx
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: staged_replacement_path.to_string_lossy().to_string(),
            size_bytes: 23,
            quality_label: Some("1080p".to_string()),
            acquisition_score: Some(650),
            ..Default::default()
        })
        .await
        .expect("insert replacement");

    let guard_dir = media_dir.path().join(".scryer-upgrade-guards");
    std::fs::create_dir_all(&guard_dir).expect("create guard dir");
    let guard_path = guard_dir.join(".scryer-upgrade-old-recovery-Movie.mkv.guard.json");
    let now = (chrono::Utc::now() - chrono::Duration::minutes(20)).to_rfc3339();
    std::fs::write(
        &guard_path,
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema": "scryer.same-path-upgrade-guard.v1",
            "phase": "replacement_moved",
            "title_id": title.id.clone(),
            "old_file_id": old_file.id.clone(),
            "old_size_bytes": 11,
            "replacement_file_id": replacement_id.clone(),
            "final_path": final_path.to_string_lossy(),
            "backup_path": backup_path.to_string_lossy(),
            "staged_replacement_path": staged_replacement_path.to_string_lossy(),
            "replacement_path": final_path.to_string_lossy(),
            "media_root": media_dir.path().to_string_lossy(),
            "created_at": now.clone(),
            "updated_at": now,
        }))
        .unwrap(),
    )
    .expect("write guard");

    ctx.app
        .run_housekeeping(&housekeeping_actor)
        .await
        .expect("housekeeping");

    assert_eq!(
        std::fs::read(&final_path).expect("restored final"),
        b"old content"
    );
    assert!(!backup_path.exists(), "backup should be restored");
    assert!(!guard_path.exists(), "guard should be removed");
    assert!(
        ctx.media_files
            .get_media_file_by_id(&replacement_id)
            .await
            .expect("load replacement")
            .is_none(),
        "uncommitted replacement row should be removed"
    );
    let files = ctx
        .media_files
        .list_media_files_for_title("title-recovery")
        .await
        .expect("list files");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].id, old_file.id);
}

#[tokio::test]
async fn housekeeping_old_moved_recovery_removes_staged_replacement_file() {
    let ctx = TestContext::new().await;
    let media_dir = tempfile::tempdir().expect("media dir");
    let catalog_actor =
        app_permission_actor("catalog-old-moved", [AppPermission::ManageCatalogSettings]);
    let housekeeping_actor = app_permission_actor(
        "housekeeping-old-moved",
        [AppPermission::ManageSystemSettings],
    );
    let library = ctx
        .app
        .create_library(
            &catalog_actor,
            MediaFacet::Movie,
            "Old Moved Recovery Library".to_string(),
            vec![LibraryRootDraft {
                path: media_dir.path().to_string_lossy().to_string(),
                is_default: true,
            }],
            None,
        )
        .await
        .expect("create library");

    let title = Title {
        id: "title-old-moved-recovery".to_string(),
        name: "Old Moved Recovery Movie".to_string(),
        facet: MediaFacet::Movie,
        library_id: library.id.clone(),
        monitored: true,
        tags: vec![],
        canonical_tags: vec![],
        external_ids: vec![],
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
        // Root ids are allocated, not derived from the path, so take the library's own.
        root_folder_id: library
            .roots
            .first()
            .map(|root| root.id.clone())
            .expect("created library should expose its root"),
        folder_path: None,
    };
    ctx.titles.create(title.clone()).await.expect("seed title");

    let final_path = media_dir.path().join("OldMoved.mkv");
    std::fs::write(&final_path, b"old content").expect("write old final");
    let old_file = seed_media_file(&ctx, &title.id, &final_path, 11, 300).await;

    let backup_path = media_dir
        .path()
        .join(".scryer-upgrade-old-old-moved-OldMoved.mkv");
    std::fs::rename(&final_path, &backup_path).expect("move old to backup");
    assert!(
        !final_path.exists(),
        "final path should be absent in old_moved"
    );

    let staged_replacement_path = media_dir
        .path()
        .join(".scryer-upgrade-replacement-old-moved-OldMoved.mkv");
    std::fs::write(&staged_replacement_path, b"staged replacement content")
        .expect("write staged replacement");
    let replacement_id = ctx
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: staged_replacement_path.to_string_lossy().to_string(),
            size_bytes: 26,
            quality_label: Some("1080p".to_string()),
            acquisition_score: Some(650),
            ..Default::default()
        })
        .await
        .expect("insert replacement");

    let guard_dir = media_dir.path().join(".scryer-upgrade-guards");
    std::fs::create_dir_all(&guard_dir).expect("create guard dir");
    let guard_path = guard_dir.join(".scryer-upgrade-old-old-moved-OldMoved.mkv.guard.json");
    let now = (chrono::Utc::now() - chrono::Duration::minutes(20)).to_rfc3339();
    std::fs::write(
        &guard_path,
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema": "scryer.same-path-upgrade-guard.v1",
            "phase": "old_moved",
            "title_id": title.id.clone(),
            "old_file_id": old_file.id.clone(),
            "old_size_bytes": 11,
            "replacement_file_id": replacement_id.clone(),
            "final_path": final_path.to_string_lossy(),
            "backup_path": backup_path.to_string_lossy(),
            "staged_replacement_path": staged_replacement_path.to_string_lossy(),
            "replacement_path": final_path.to_string_lossy(),
            "media_root": media_dir.path().to_string_lossy(),
            "created_at": now.clone(),
            "updated_at": now,
        }))
        .unwrap(),
    )
    .expect("write guard");

    ctx.app
        .run_housekeeping(&housekeeping_actor)
        .await
        .expect("housekeeping");

    assert_eq!(
        std::fs::read(&final_path).expect("restored final"),
        b"old content"
    );
    assert!(!backup_path.exists(), "backup should be restored");
    assert!(!guard_path.exists(), "guard should be removed");
    assert!(
        !staged_replacement_path.exists(),
        "staged replacement should be cleaned up"
    );
    assert!(
        ctx.media_files
            .get_media_file_by_id(&replacement_id)
            .await
            .expect("load replacement")
            .is_none(),
        "uncommitted replacement row should be removed"
    );
    let files = ctx
        .media_files
        .list_media_files_for_title("title-old-moved-recovery")
        .await
        .expect("list files");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].id, old_file.id);
}

#[tokio::test]
async fn housekeeping_skips_recent_same_path_guard() {
    let ctx = TestContext::new().await;
    let media_dir = tempfile::tempdir().expect("media dir");
    let catalog_actor = app_permission_actor("catalog", [AppPermission::ManageCatalogSettings]);
    let housekeeping_actor =
        app_permission_actor("housekeeping-recent", [AppPermission::ManageSystemSettings]);
    let library = ctx
        .app
        .create_library(
            &catalog_actor,
            MediaFacet::Movie,
            "Recent Guard Library".to_string(),
            vec![LibraryRootDraft {
                path: media_dir.path().to_string_lossy().to_string(),
                is_default: true,
            }],
            None,
        )
        .await
        .expect("create library");

    let title = Title {
        id: "title-recent-recovery".to_string(),
        name: "Recent Recovery Movie".to_string(),
        facet: MediaFacet::Movie,
        library_id: library.id.clone(),
        monitored: true,
        tags: vec![],
        canonical_tags: vec![],
        external_ids: vec![],
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
        // Root ids are allocated, not derived from the path, so take the library's own.
        root_folder_id: library
            .roots
            .first()
            .map(|root| root.id.clone())
            .expect("created library should expose its root"),
        folder_path: None,
    };
    ctx.titles.create(title.clone()).await.expect("seed title");

    let final_path = media_dir.path().join("Recent.mkv");
    std::fs::write(&final_path, b"old content").expect("write old final");
    let old_file = seed_media_file(&ctx, &title.id, &final_path, 11, 300).await;

    let backup_path = media_dir
        .path()
        .join(".scryer-upgrade-old-recent-Recent.mkv");
    std::fs::rename(&final_path, &backup_path).expect("move old to backup");
    std::fs::write(&final_path, b"new uncommitted content").expect("write replacement final");
    let staged_replacement_path = media_dir
        .path()
        .join(".scryer-upgrade-replacement-recent-Recent.mkv");
    let replacement_id = ctx
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: staged_replacement_path.to_string_lossy().to_string(),
            size_bytes: 23,
            quality_label: Some("1080p".to_string()),
            acquisition_score: Some(650),
            ..Default::default()
        })
        .await
        .expect("insert replacement");

    let guard_dir = media_dir.path().join(".scryer-upgrade-guards");
    std::fs::create_dir_all(&guard_dir).expect("create guard dir");
    let guard_path = guard_dir.join(".scryer-upgrade-old-recent-Recent.mkv.guard.json");
    let now = chrono::Utc::now().to_rfc3339();
    std::fs::write(
        &guard_path,
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema": "scryer.same-path-upgrade-guard.v1",
            "phase": "replacement_moved",
            "title_id": title.id.clone(),
            "old_file_id": old_file.id.clone(),
            "old_size_bytes": 11,
            "replacement_file_id": replacement_id.clone(),
            "final_path": final_path.to_string_lossy(),
            "backup_path": backup_path.to_string_lossy(),
            "staged_replacement_path": staged_replacement_path.to_string_lossy(),
            "replacement_path": final_path.to_string_lossy(),
            "media_root": media_dir.path().to_string_lossy(),
            "created_at": now.clone(),
            "updated_at": now,
        }))
        .unwrap(),
    )
    .expect("write guard");

    ctx.app
        .run_housekeeping(&housekeeping_actor)
        .await
        .expect("housekeeping");

    assert_eq!(
        std::fs::read(&final_path).expect("read final"),
        b"new uncommitted content"
    );
    assert!(backup_path.exists(), "recent backup should be left alone");
    assert!(guard_path.exists(), "recent guard should be left in place");
    assert!(
        ctx.media_files
            .get_media_file_by_id(&replacement_id)
            .await
            .expect("load replacement")
            .is_some(),
        "recent replacement row should be left alone"
    );
}

#[tokio::test]
async fn housekeeping_disposes_db_swapped_guard_with_encoded_media_root() {
    let ctx = TestContext::new().await;
    let media_dir = tempfile::tempdir().expect("media dir");
    let catalog_actor =
        app_permission_actor("catalog-encoded", [AppPermission::ManageCatalogSettings]);
    let housekeeping_actor = app_permission_actor(
        "housekeeping-encoded",
        [AppPermission::ManageSystemSettings],
    );
    let library = ctx
        .app
        .create_library(
            &catalog_actor,
            MediaFacet::Movie,
            "Encoded Guard Library".to_string(),
            vec![LibraryRootDraft {
                path: media_dir.path().to_string_lossy().to_string(),
                is_default: true,
            }],
            None,
        )
        .await
        .expect("create library");

    let title = Title {
        id: "title-encoded-recovery".to_string(),
        name: "Encoded Recovery Movie".to_string(),
        facet: MediaFacet::Movie,
        library_id: library.id.clone(),
        monitored: true,
        tags: vec![],
        canonical_tags: vec![],
        external_ids: vec![],
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
        // Root ids are allocated, not derived from the path, so take the library's own.
        root_folder_id: library
            .roots
            .first()
            .map(|root| root.id.clone())
            .expect("created library should expose its root"),
        folder_path: None,
    };
    ctx.titles.create(title.clone()).await.expect("seed title");

    let final_path = media_dir.path().join("Encoded.mkv");
    std::fs::write(&final_path, b"new committed content").expect("write final replacement");
    let backup_path = media_dir
        .path()
        .join(".scryer-upgrade-old-encoded-Encoded.mkv");
    std::fs::write(&backup_path, b"old encoded root content").expect("write backup");
    let staged_replacement_path = media_dir
        .path()
        .join(".scryer-upgrade-replacement-encoded-Encoded.mkv");
    let replacement_id = ctx
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: final_path.to_string_lossy().to_string(),
            size_bytes: 21,
            quality_label: Some("1080p".to_string()),
            acquisition_score: Some(650),
            ..Default::default()
        })
        .await
        .expect("insert replacement");

    let encoded_media_root = force_encoded_stored_path(media_dir.path());
    let guard_dir = media_dir.path().join(".scryer-upgrade-guards");
    std::fs::create_dir_all(&guard_dir).expect("create guard dir");
    let guard_path = guard_dir.join(".scryer-upgrade-old-encoded-Encoded.mkv.guard.json");
    let now = (chrono::Utc::now() - chrono::Duration::minutes(20)).to_rfc3339();
    std::fs::write(
        &guard_path,
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema": "scryer.same-path-upgrade-guard.v1",
            "phase": "db_swapped",
            "title_id": title.id.clone(),
            "old_file_id": "old-file-encoded-root",
            "old_size_bytes": 24,
            "replacement_file_id": replacement_id.clone(),
            "final_path": final_path.to_string_lossy(),
            "backup_path": backup_path.to_string_lossy(),
            "staged_replacement_path": staged_replacement_path.to_string_lossy(),
            "replacement_path": final_path.to_string_lossy(),
            "media_root": encoded_media_root.clone(),
            "created_at": now.clone(),
            "updated_at": now,
        }))
        .unwrap(),
    )
    .expect("write guard");

    ctx.app
        .run_housekeeping(&housekeeping_actor)
        .await
        .expect("housekeeping");

    assert!(!guard_path.exists(), "db-swapped guard should be removed");
    assert!(!backup_path.exists(), "old backup should be recycled");
    assert_eq!(
        std::fs::read(&final_path).expect("read final replacement"),
        b"new committed content"
    );
    let recycle_dir = media_dir.path().join(".scryer-recycle");
    let recycle_entries = std::fs::read_dir(&recycle_dir)
        .expect("read real recycle dir")
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .collect::<Vec<_>>();
    assert_eq!(
        recycle_entries.len(),
        1,
        "backup should be recycled under the decoded media root"
    );
    let manifest_bytes = std::fs::read(recycle_entries[0].path().join("manifest.json"))
        .expect("read recycle manifest");
    let manifest: RecycleManifest = serde_json::from_slice(&manifest_bytes).unwrap();
    assert_eq!(manifest.status.as_deref(), Some(RECYCLE_STATUS_COMMITTED));
    assert_eq!(
        manifest.media_root.as_deref(),
        Some(encoded_media_root.as_str())
    );
    assert!(
        !PathBuf::from(&encoded_media_root)
            .join(".scryer-recycle")
            .exists(),
        "encoded media root must not be treated as a literal filesystem path"
    );
}

#[tokio::test]
async fn housekeeping_decodes_stored_paths_before_orphan_cleanup() {
    let ctx = TestContext::new().await;
    let title = seed_title(&ctx, "title-stored-path-housekeeping").await;
    let housekeeping_actor = app_permission_actor(
        "housekeeping-stored-path",
        [AppPermission::ManageSystemSettings],
    );

    let media_dir = tempfile::tempdir().expect("media dir");
    let file_path = media_dir.path().join("Movie.mkv");
    std::fs::write(&file_path, b"encoded path content").expect("write encoded-path file");
    let stored_file_path = force_encoded_stored_path(&file_path);
    assert!(
        stored_file_path.starts_with("scryer-path-v1:"),
        "test path should be encoded to exercise stored-path decoding"
    );

    let file_id = ctx
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: stored_file_path.clone(),
            size_bytes: 20,
            quality_label: Some("720p".to_string()),
            acquisition_score: Some(300),
            ..Default::default()
        })
        .await
        .expect("insert stored-path media file");

    ctx.app
        .run_housekeeping(&housekeeping_actor)
        .await
        .expect("housekeeping");

    let row = ctx
        .media_files
        .get_media_file_by_id(&file_id)
        .await
        .expect("lookup media file");
    assert!(row.is_some(), "existing encoded-path row should survive");
}

#[tokio::test]
async fn housekeeping_skips_orphan_cleanup_when_root_is_empty() {
    let ctx = TestContext::new().await;
    let catalog_actor =
        app_permission_actor("catalog-empty-root", [AppPermission::ManageCatalogSettings]);
    let housekeeping_actor = app_permission_actor(
        "housekeeping-empty-root",
        [AppPermission::ManageSystemSettings],
    );
    let media_dir = tempfile::tempdir().expect("media dir");
    let library = ctx
        .app
        .create_library(
            &catalog_actor,
            MediaFacet::Movie,
            "Empty Root Housekeeping Library".to_string(),
            vec![LibraryRootDraft {
                path: media_dir.path().to_string_lossy().to_string(),
                is_default: true,
            }],
            None,
        )
        .await
        .expect("create library");
    let title = seed_title_for_library(
        &ctx,
        "title-empty-root-housekeeping",
        "Empty Root Movie",
        &library.id,
        media_dir.path(),
    )
    .await;

    let missing_path = media_dir.path().join("Missing.mkv");
    let file_id = ctx
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: missing_path.to_string_lossy().to_string(),
            size_bytes: 42,
            quality_label: Some("1080p".to_string()),
            acquisition_score: Some(400),
            ..Default::default()
        })
        .await
        .expect("insert missing media file");

    ctx.app
        .run_housekeeping(&housekeeping_actor)
        .await
        .expect("housekeeping");

    let row = ctx
        .media_files
        .get_media_file_by_id(&file_id)
        .await
        .expect("lookup media file");
    assert!(
        row.is_some(),
        "empty root should be treated as unavailable and keep catalog rows"
    );
}

#[tokio::test]
async fn housekeeping_removes_missing_rows_when_root_is_non_empty() {
    let ctx = TestContext::new().await;
    let catalog_actor = app_permission_actor(
        "catalog-non-empty-root",
        [AppPermission::ManageCatalogSettings],
    );
    let housekeeping_actor = app_permission_actor(
        "housekeeping-non-empty-root",
        [AppPermission::ManageSystemSettings],
    );
    let media_dir = tempfile::tempdir().expect("media dir");
    std::fs::write(media_dir.path().join(".mounted"), b"mounted").expect("write mount marker");
    let library = ctx
        .app
        .create_library(
            &catalog_actor,
            MediaFacet::Movie,
            "Non Empty Root Housekeeping Library".to_string(),
            vec![LibraryRootDraft {
                path: media_dir.path().to_string_lossy().to_string(),
                is_default: true,
            }],
            None,
        )
        .await
        .expect("create library");
    let title = seed_title_for_library(
        &ctx,
        "title-non-empty-root-housekeeping",
        "Non Empty Root Movie",
        &library.id,
        media_dir.path(),
    )
    .await;

    let missing_path = media_dir.path().join("Missing.mkv");
    let file_id = ctx
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: missing_path.to_string_lossy().to_string(),
            size_bytes: 42,
            quality_label: Some("1080p".to_string()),
            acquisition_score: Some(400),
            ..Default::default()
        })
        .await
        .expect("insert missing media file");

    ctx.app
        .run_housekeeping(&housekeeping_actor)
        .await
        .expect("housekeeping");

    let row = ctx
        .media_files
        .get_media_file_by_id(&file_id)
        .await
        .expect("lookup media file");
    assert!(
        row.is_none(),
        "available non-empty root should allow DB-only orphan cleanup"
    );
}

#[tokio::test]
async fn disabled_recycle_bin_upgrade_validation_failure_preserves_old_file() {
    let ctx = TestContext::new().await;
    let app = app_with_real_fs(&ctx);
    let title = seed_title(&ctx, "title-5").await;
    let actor = test_actor();

    let media_dir = tempfile::tempdir().expect("media dir");
    let source_dir = tempfile::tempdir().expect("source dir");
    let wrong_root = tempfile::tempdir().expect("wrong root");

    let old_path = media_dir.path().join("Movie.720p.mkv");
    std::fs::write(&old_path, b"old content guarded").expect("write old");

    let new_source = source_dir.path().join("Movie.1080p.mkv");
    std::fs::write(&new_source, b"new content guarded").expect("write new");
    let new_dest = media_dir.path().join("Movie.1080p.mkv");

    let existing = seed_media_file(&ctx, "title-5", &old_path, 19, 300).await;
    let parsed = scryer_application::parse_release_metadata("Movie.1080p.WEB-DL");
    let disabled_config = RecycleBinConfig {
        enabled: false,
        base_path: std::path::PathBuf::from("/tmp/unused"),
        retention_days: 7,
        cleanup_enabled: true,
        validation_error: None,
        source_roots: vec![media_dir.path().to_path_buf()],
    };

    let result = execute_upgrade_for_test(
        &app,
        UpgradeForTestInput {
            actor: &actor,
            title: &title,
            existing_file: &existing,
            source_path: &new_source,
            dest_path: &new_dest,
            parsed,
            final_score: 650,
            target_episode_ids: &[],
            media_root: Some(wrong_root.path().to_string_lossy().as_ref()),
            recycle_config: &disabled_config,
        },
    )
    .await;

    assert!(result.is_err(), "replacement validation should fail");
    assert_eq!(
        std::fs::read(&old_path).expect("old file still exists"),
        b"old content guarded"
    );
    assert!(
        !new_dest.exists(),
        "unverified replacement should be rolled back before old deletion"
    );
    let files = ctx
        .media_files
        .list_media_files_for_title("title-5")
        .await
        .expect("list media files");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].id, existing.id);
}

#[tokio::test]
async fn reused_upgrade_destination_survives_pre_swap_validation_failure() {
    let ctx = TestContext::new().await;
    let app = app_with_real_fs(&ctx);
    let title = seed_title(&ctx, "title-reused-upgrade").await;
    let actor = test_actor();
    let media_dir = tempfile::tempdir().expect("media dir");
    let source_dir = tempfile::tempdir().expect("source dir");
    let wrong_root = tempfile::tempdir().expect("wrong root");
    let old_path = media_dir.path().join("Movie.720p.mkv");
    let new_source = source_dir.path().join("Movie.1080p.mkv");
    let new_dest = media_dir.path().join("Movie.1080p.mkv");
    std::fs::write(&old_path, b"old content guarded").expect("write old");
    std::fs::write(&new_source, b"recovered replacement content").expect("write source");
    std::fs::write(&new_dest, b"recovered replacement content").expect("write destination");
    let existing = seed_media_file(
        &ctx,
        "title-reused-upgrade",
        &old_path,
        b"old content guarded".len() as i64,
        300,
    )
    .await;
    let recovered = seed_media_file(
        &ctx,
        "title-reused-upgrade",
        &new_dest,
        b"recovered replacement content".len() as i64,
        650,
    )
    .await;
    let disabled_config = RecycleBinConfig {
        enabled: false,
        base_path: std::path::PathBuf::from("/tmp/unused"),
        retention_days: 7,
        cleanup_enabled: true,
        validation_error: None,
        source_roots: vec![media_dir.path().to_path_buf()],
    };

    let result = execute_upgrade_for_test(
        &app,
        UpgradeForTestInput {
            actor: &actor,
            title: &title,
            existing_file: &existing,
            source_path: &new_source,
            dest_path: &new_dest,
            parsed: scryer_application::parse_release_metadata("Movie.1080p.WEB-DL"),
            final_score: 650,
            target_episode_ids: &[],
            media_root: Some(wrong_root.path().to_string_lossy().as_ref()),
            recycle_config: &disabled_config,
        },
    )
    .await;

    assert!(result.is_err(), "replacement validation should fail");
    assert_eq!(
        std::fs::read(&new_dest).expect("recovered destination should remain"),
        b"recovered replacement content"
    );
    assert!(
        ctx.media_files
            .get_media_file_by_id(&recovered.id)
            .await
            .expect("load recovered row")
            .is_some(),
        "rollback must not delete a row from an earlier finalization attempt"
    );
    assert!(
        ctx.media_files
            .get_media_file_by_id(&existing.id)
            .await
            .expect("load old row")
            .is_some(),
        "old row must remain before the guarded swap"
    );
}

#[tokio::test]
async fn uncataloged_existing_upgrade_destination_survives_insert_failure() {
    let ctx = TestContext::new().await;
    let app = app_with_failing_media_insert(&ctx);
    let title = seed_title(&ctx, "title-existing-destination").await;
    let actor = test_actor();
    let media_dir = tempfile::tempdir().expect("media dir");
    let source_dir = tempfile::tempdir().expect("source dir");
    let old_path = media_dir.path().join("Movie.720p.mkv");
    let new_source = source_dir.path().join("Movie.1080p.mkv");
    let new_dest = media_dir.path().join("Movie.1080p.mkv");
    std::fs::write(&old_path, b"old content guarded").expect("write old");
    std::fs::write(&new_source, b"existing destination content").expect("write source");
    std::fs::write(&new_dest, b"existing destination content").expect("write destination");
    let existing = seed_media_file(
        &ctx,
        "title-existing-destination",
        &old_path,
        b"old content guarded".len() as i64,
        300,
    )
    .await;
    let disabled_config = RecycleBinConfig {
        enabled: false,
        base_path: std::path::PathBuf::from("/tmp/unused"),
        retention_days: 7,
        cleanup_enabled: true,
        validation_error: None,
        source_roots: vec![media_dir.path().to_path_buf()],
    };

    let result = execute_upgrade_for_test(
        &app,
        UpgradeForTestInput {
            actor: &actor,
            title: &title,
            existing_file: &existing,
            source_path: &new_source,
            dest_path: &new_dest,
            parsed: scryer_application::parse_release_metadata("Movie.1080p.WEB-DL"),
            final_score: 650,
            target_episode_ids: &[],
            media_root: Some(media_dir.path().to_string_lossy().as_ref()),
            recycle_config: &disabled_config,
        },
    )
    .await;
    let Err(error) = result else {
        panic!("forced media-row insertion should fail the upgrade");
    };

    assert!(
        error
            .to_string()
            .contains("forced media-file insertion failure")
    );
    assert_eq!(
        std::fs::read(&new_dest).expect("existing destination should remain"),
        b"existing destination content"
    );
    assert_eq!(
        std::fs::read(&new_source).expect("source should remain"),
        b"existing destination content"
    );
    assert!(
        ctx.media_files
            .get_media_file_by_path(new_dest.to_string_lossy().as_ref())
            .await
            .expect("destination lookup should succeed")
            .is_none(),
        "the failed insert must not leave a catalog row"
    );
    assert!(
        ctx.media_files
            .get_media_file_by_id(&existing.id)
            .await
            .expect("load old row")
            .is_some(),
        "the incumbent must remain before a completed swap"
    );
}

//! Viewer privacy for library data: an actor with only View on a library sees
//! the catalog, but not where its media lives on disk, how it was acquired, or
//! its acquisition and download history.
//!
//! Privileged access to a library means Manage Titles or Manage Library on it,
//! or the Manage System Settings / Manage Catalog Settings app permission.
//! Every case runs against two libraries so a grant on one cannot leak into the
//! other.

use super::*;

use crate::error_reasons;
use scryer_domain::{AppPermissionMask, LibraryPermissionMask};

const MOVIE_FILE_NAME: &str = "Lumen.Drift.2001.1080p.mkv";
const SERIES_FILE_NAME: &str = "Harbor.Lantern.S01E01.1080p.mkv";
const EPISODE_ID: &str = "episode-privacy-1";

// ── Actors ────────────────────────────────────────────────────────────────

fn library_actor(
    username: &str,
    app_permissions: AppPermissionMask,
    grants: &[(&str, LibraryPermissionMask)],
) -> User {
    let mut user = User {
        id: Id::new().0,
        username: username.to_string(),
        password_hash: None,
        password_change_required: false,
        account_kind: Default::default(),
        authorization: Default::default(),
    };
    user.authorization.app = app_permissions;
    user.authorization.libraries = grants
        .iter()
        .map(|(library_id, mask)| (library_id.to_string(), mask.normalized_for_storage()))
        .collect();
    user.authorization.loaded = true;
    user
}

fn mask_with(extra: LibraryPermissionMask) -> LibraryPermissionMask {
    let mut mask = LibraryPermissionMask::VIEW;
    mask.insert(extra);
    mask
}

fn viewer_for(library_id: &str) -> User {
    library_actor(
        "privacy-viewer",
        AppPermissionMask::NONE,
        &[(library_id, LibraryPermissionMask::VIEW)],
    )
}

fn subtitle_manager_for(library_id: &str) -> User {
    library_actor(
        "privacy-subtitle-manager",
        AppPermissionMask::NONE,
        &[(
            library_id,
            mask_with(LibraryPermissionMask::MANAGE_SUBTITLES),
        )],
    )
}

fn title_manager_for(library_id: &str) -> User {
    library_actor(
        "privacy-title-manager",
        AppPermissionMask::NONE,
        &[(library_id, mask_with(LibraryPermissionMask::MANAGE_TITLES))],
    )
}

fn library_manager_for(library_id: &str) -> User {
    library_actor(
        "privacy-library-manager",
        AppPermissionMask::NONE,
        &[(library_id, mask_with(LibraryPermissionMask::MANAGE_LIBRARY))],
    )
}

fn app_admin_viewer_for(library_id: &str, app_permission: AppPermissionMask) -> User {
    library_actor(
        "privacy-app-admin",
        app_permission,
        &[(library_id, LibraryPermissionMask::VIEW)],
    )
}

/// Manage Titles on the movie library, View only on the series library.
fn mixed_actor(movie_library_id: &str, series_library_id: &str) -> User {
    library_actor(
        "privacy-mixed",
        AppPermissionMask::NONE,
        &[
            (
                movie_library_id,
                mask_with(LibraryPermissionMask::MANAGE_TITLES),
            ),
            (series_library_id, LibraryPermissionMask::VIEW),
        ],
    )
}

fn assert_refused_with(result: AppResult<impl std::fmt::Debug>, reason: &str, what: &str) {
    match result {
        Err(AppError::UnauthorizedReason { reason: actual, .. }) => {
            assert_eq!(actual, reason, "{what}: wrong refusal reason");
        }
        Err(error) => panic!("{what}: expected refusal {reason}, got {error}"),
        Ok(value) => panic!("{what}: expected refusal {reason}, got Ok({value:?})"),
    }
}

// ── Fixture ───────────────────────────────────────────────────────────────

struct PrivacyFixture {
    app: AppUseCase,
    admin: User,
    movie_library_id: String,
    movie_title: Title,
    movie_file_id: String,
    series_library_id: String,
    series_title: Title,
    _tempdir: tempfile::TempDir,
}

fn provenance_file(title_id: &str, file_path: String, episode_id: Option<&str>) -> TitleMediaFile {
    TitleMediaFile {
        id: Id::new().0,
        title_id: title_id.to_string(),
        episode_id: episode_id.map(str::to_string),
        file_path,
        size_bytes: 1_024,
        role: MediaFileRole::Primary,
        scan_status: "scanned".to_string(),
        quality_label: Some("1080p".to_string()),
        resolution: Some("1080p".to_string()),
        scene_name: Some("Sample.Release.1080p-PRIVGRP".to_string()),
        release_group: Some("PRIVGRP".to_string()),
        grabbed_release_title: Some("Sample.Release.1080p-PRIVGRP".to_string()),
        indexer_source: Some("privacy-indexer".to_string()),
        scoring_log: Some("[]".to_string()),
        acquisition_score: Some(77),
        release_hash: Some("privacy-hash".to_string()),
        release_listing_json: Some("{}".to_string()),
        grabbed_at: Some("2026-01-01T00:00:00Z".to_string()),
        original_file_path: Some("/downloads/complete/privacy/sample.mkv".to_string()),
        ..TitleMediaFile::default()
    }
}

async fn seed_fixture() -> PrivacyFixture {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let movie_root = tempdir.path().join("movies");
    let series_root = tempdir.path().join("series");
    std::fs::create_dir_all(&movie_root).expect("create movie root");
    std::fs::create_dir_all(&series_root).expect("create series root");

    let media_files = Arc::new(MockMediaFileRepo::default());
    let (app, admin, _titles) = bootstrap_with_cutoff_projection_state(
        Arc::new(StoredSettingsRepo::default()),
        Arc::new(StoredQualityProfileRepo::default()),
        media_files.clone(),
    );

    for (facet, root) in [
        (MediaFacet::Movie, &movie_root),
        (MediaFacet::Series, &series_root),
    ] {
        update_default_library_roots(
            &app,
            &admin,
            facet,
            vec![build_root_folder_entry(root, true)],
        )
        .await
        .expect("save roots");
    }

    let mut titles = Vec::new();
    for (facet, name) in [
        (MediaFacet::Movie, "Lumen Drift"),
        (MediaFacet::Series, "Harbor Lantern"),
    ] {
        titles.push(
            app.add_title(
                &admin,
                NewTitle {
                    name: name.to_string(),
                    facet,
                    monitored: true,
                    tags: vec![],
                    external_ids: vec![],
                    min_availability: None,
                    ..Default::default()
                },
            )
            .await
            .expect("create title"),
        );
    }
    let series_title = titles.pop().expect("series title");
    let movie_title = titles.pop().expect("movie title");

    let movie_file = provenance_file(
        &movie_title.id,
        movie_root
            .join("Lumen Drift")
            .join(MOVIE_FILE_NAME)
            .to_string_lossy()
            .to_string(),
        None,
    );
    let movie_file_id = movie_file.id.clone();
    let series_file = provenance_file(
        &series_title.id,
        series_root
            .join("Harbor Lantern")
            .join("Season 01")
            .join(SERIES_FILE_NAME)
            .to_string_lossy()
            .to_string(),
        Some(EPISODE_ID),
    );
    {
        let mut store = media_files.store.lock().await;
        store.push(movie_file);
        store.push(series_file);
    }

    PrivacyFixture {
        app,
        admin,
        movie_library_id: movie_title.library_id.clone(),
        movie_title,
        movie_file_id,
        series_library_id: series_title.library_id.clone(),
        series_title,
        _tempdir: tempdir,
    }
}

fn assert_redacted(file: &TitleMediaFile, expected_name: &str, what: &str) {
    assert_eq!(file.file_path, expected_name, "{what}: file_path");
    assert_eq!(file.original_file_path, None, "{what}: original_file_path");
    assert_eq!(file.release_group, None, "{what}: release_group");
    assert_eq!(file.scene_name, None, "{what}: scene_name");
    assert_eq!(
        file.grabbed_release_title, None,
        "{what}: grabbed_release_title"
    );
    assert_eq!(file.indexer_source, None, "{what}: indexer_source");
    assert_eq!(file.scoring_log, None, "{what}: scoring_log");
    assert_eq!(file.acquisition_score, None, "{what}: acquisition_score");
    assert_eq!(file.release_hash, None, "{what}: release_hash");
    assert_eq!(
        file.release_listing_json, None,
        "{what}: release_listing_json"
    );
    assert_eq!(file.grabbed_at, None, "{what}: grabbed_at");
    assert_eq!(
        file.quality_label.as_deref(),
        Some("1080p"),
        "{what}: quality kept"
    );
}

fn assert_unredacted(file: &TitleMediaFile, expected_name: &str, what: &str) {
    assert!(
        file.file_path.len() > expected_name.len() && file.file_path.ends_with(expected_name),
        "{what}: expected the full path, got {}",
        file.file_path
    );
    assert_eq!(
        file.release_group.as_deref(),
        Some("PRIVGRP"),
        "{what}: release_group"
    );
    assert_eq!(
        file.indexer_source.as_deref(),
        Some("privacy-indexer"),
        "{what}: indexer"
    );
    assert_eq!(
        file.acquisition_score,
        Some(77),
        "{what}: acquisition_score"
    );
    assert!(
        file.original_file_path.is_some(),
        "{what}: original_file_path"
    );
    assert!(file.grabbed_at.is_some(), "{what}: grabbed_at");
}

// ── Predicate ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn privileged_access_follows_management_grants_only() {
    let fixture = seed_fixture().await;
    let movie = fixture.movie_library_id.as_str();
    let series = fixture.series_library_id.as_str();

    let cases: Vec<(&str, User, bool, bool)> = vec![
        ("viewer", viewer_for(movie), false, false),
        (
            "subtitle manager",
            subtitle_manager_for(movie),
            false,
            false,
        ),
        ("title manager", title_manager_for(movie), true, false),
        ("library manager", library_manager_for(movie), true, false),
        (
            "catalog settings admin",
            app_admin_viewer_for(movie, AppPermissionMask::MANAGE_CATALOG_SETTINGS),
            true,
            true,
        ),
        (
            "system settings admin",
            app_admin_viewer_for(movie, AppPermissionMask::MANAGE_SYSTEM_SETTINGS),
            true,
            true,
        ),
        ("full admin", fixture.admin.clone(), true, true),
        ("mixed grants", mixed_actor(movie, series), true, false),
    ];

    for (label, actor, on_movie, on_series) in cases {
        assert_eq!(
            fixture
                .app
                .has_privileged_library_access(&actor, movie)
                .await
                .expect("movie predicate"),
            on_movie,
            "{label}: movie library"
        );
        assert_eq!(
            fixture
                .app
                .has_privileged_library_access(&actor, series)
                .await
                .expect("series predicate"),
            on_series,
            "{label}: series library"
        );
    }

    let mixed_ids = fixture
        .app
        .privileged_library_ids(&mixed_actor(movie, series), None)
        .await
        .expect("mixed privileged ids");
    assert_eq!(mixed_ids, vec![movie.to_string()]);
}

// ── Part A: media file redaction ──────────────────────────────────────────

#[tokio::test]
async fn title_media_files_are_redacted_for_viewers_only() {
    let fixture = seed_fixture().await;
    let movie = fixture.movie_library_id.as_str();
    let title_id = fixture.movie_title.id.as_str();

    let viewer_files = fixture
        .app
        .list_title_media_files(&viewer_for(movie), title_id)
        .await
        .expect("viewer lists media files");
    assert_eq!(viewer_files.len(), 1);
    assert_redacted(&viewer_files[0], MOVIE_FILE_NAME, "viewer");

    for (label, actor) in [
        ("title manager", title_manager_for(movie)),
        ("library manager", library_manager_for(movie)),
        (
            "catalog settings admin",
            app_admin_viewer_for(movie, AppPermissionMask::MANAGE_CATALOG_SETTINGS),
        ),
        ("full admin", fixture.admin.clone()),
    ] {
        let files = fixture
            .app
            .list_title_media_files(&actor, title_id)
            .await
            .expect("privileged actor lists media files");
        assert_eq!(files.len(), 1, "{label}");
        assert_unredacted(&files[0], MOVIE_FILE_NAME, label);
    }
}

#[tokio::test]
async fn episode_media_files_are_redacted_for_viewers_only() {
    let fixture = seed_fixture().await;
    let series = fixture.series_library_id.as_str();
    let title_id = fixture.series_title.id.as_str();
    let episode_ids = vec![EPISODE_ID.to_string()];

    let viewer = viewer_for(series);
    let single = fixture
        .app
        .list_episode_media_files(&viewer, title_id, EPISODE_ID)
        .await
        .expect("viewer lists episode files");
    assert_eq!(single.len(), 1);
    assert_redacted(&single[0], SERIES_FILE_NAME, "viewer single episode");

    let batch = fixture
        .app
        .list_episode_media_files_for_title(&viewer, title_id, &episode_ids)
        .await
        .expect("viewer lists episode files in batch");
    assert_redacted(
        &batch[EPISODE_ID][0],
        SERIES_FILE_NAME,
        "viewer episode batch",
    );

    let manager = title_manager_for(series);
    let single = fixture
        .app
        .list_episode_media_files(&manager, title_id, EPISODE_ID)
        .await
        .expect("manager lists episode files");
    assert_unredacted(&single[0], SERIES_FILE_NAME, "manager single episode");
    let batch = fixture
        .app
        .list_episode_media_files_for_title(&manager, title_id, &episode_ids)
        .await
        .expect("manager lists episode files in batch");
    assert_unredacted(
        &batch[EPISODE_ID][0],
        SERIES_FILE_NAME,
        "manager episode batch",
    );
}

#[tokio::test]
async fn batch_media_files_redact_per_library_for_mixed_grants() {
    let fixture = seed_fixture().await;
    let actor = mixed_actor(&fixture.movie_library_id, &fixture.series_library_id);
    let title_ids = vec![
        fixture.movie_title.id.clone(),
        fixture.series_title.id.clone(),
    ];

    let files = fixture
        .app
        .list_media_files_for_titles(&actor, &title_ids)
        .await
        .expect("mixed actor lists media files");

    assert_unredacted(
        &files[&fixture.movie_title.id][0],
        MOVIE_FILE_NAME,
        "managed movie library",
    );
    assert_redacted(
        &files[&fixture.series_title.id][0],
        SERIES_FILE_NAME,
        "view-only series library",
    );
}

#[tokio::test]
async fn score_evaluation_is_null_for_viewers() {
    let fixture = seed_fixture().await;
    let evaluation = fixture
        .app
        .media_file_score_evaluation(
            &viewer_for(&fixture.movie_library_id),
            &fixture.movie_file_id,
        )
        .await
        .expect("a viewer is not refused");
    assert!(evaluation.is_none());
}

// ── Part B: privileged-only root queries refuse viewers ───────────────────

#[tokio::test]
async fn privileged_root_queries_refuse_viewers_with_library_access_denied() {
    let fixture = seed_fixture().await;
    let viewer = viewer_for(&fixture.movie_library_id);
    let title_id = fixture.movie_title.id.as_str();
    let denied = error_reasons::authorization::LIBRARY_ACCESS_DENIED;
    let app = &fixture.app;

    assert_refused_with(
        app.list_import_history(&viewer, 10).await,
        denied,
        "import history",
    );
    assert_refused_with(
        app.list_title_history(&viewer, &TitleHistoryFilter::default())
            .await,
        denied,
        "title history",
    );
    assert_refused_with(
        app.list_title_history_for_title(&viewer, title_id, None, 10, 0)
            .await,
        denied,
        "title history for title",
    );
    assert_refused_with(
        app.dashboard_recent_imports(&viewer, 10).await,
        denied,
        "dashboard recent imports",
    );
    assert_refused_with(
        app.list_active_import_streams(&viewer).await,
        denied,
        "active import streams",
    );
    assert_refused_with(
        app.list_pending_releases(&viewer).await,
        denied,
        "pending releases",
    );
    assert_refused_with(
        app.list_cutoff_unmet_titles(&viewer, None, None).await,
        denied,
        "cutoff unmet titles",
    );
    assert_refused_with(
        app.storage_root_usage(&viewer).await,
        denied,
        "storage roots",
    );
    assert_refused_with(app.disk_space(&viewer).await, denied, "disk space");
    assert_refused_with(
        app.get_title_wanted_item(&viewer, title_id, None).await,
        denied,
        "title wanted item",
    );
    assert_refused_with(
        app.list_title_release_blocklist(&viewer, title_id, 10)
            .await,
        denied,
        "title release blocklist",
    );
    assert_refused_with(
        app.list_download_queue_for_title(
            &viewer,
            title_id,
            true,
            false,
            true,
            DownloadActivityFilter::All,
        )
        .await,
        denied,
        "download queue for title",
    );

    let queue_denied = error_reasons::integration::LIBRARY_ACCESS_DENIED;
    assert_refused_with(
        app.list_download_queue(&viewer, true, false, true, DownloadActivityFilter::All)
            .await,
        queue_denied,
        "download queue",
    );
    assert_refused_with(
        app.subscribe_download_queue(&viewer).await.map(|_| ()),
        queue_denied,
        "download queue subscription",
    );
}

#[tokio::test]
async fn privileged_root_queries_admit_management_grants() {
    let fixture = seed_fixture().await;
    let movie = fixture.movie_library_id.as_str();

    for (label, actor) in [
        ("title manager", title_manager_for(movie)),
        ("library manager", library_manager_for(movie)),
        (
            "catalog settings admin",
            app_admin_viewer_for(movie, AppPermissionMask::MANAGE_CATALOG_SETTINGS),
        ),
    ] {
        fixture
            .app
            .list_import_history(&actor, 10)
            .await
            .unwrap_or_else(|error| panic!("{label}: import history refused: {error}"));
        fixture
            .app
            .list_title_history(&actor, &TitleHistoryFilter::default())
            .await
            .unwrap_or_else(|error| panic!("{label}: title history refused: {error}"));
        fixture
            .app
            .list_pending_releases(&actor)
            .await
            .unwrap_or_else(|error| panic!("{label}: pending releases refused: {error}"));
    }
}

#[tokio::test]
async fn storage_roots_list_only_privileged_libraries_for_mixed_grants() {
    let fixture = seed_fixture().await;
    let actor = mixed_actor(&fixture.movie_library_id, &fixture.series_library_id);

    let roots = fixture
        .app
        .storage_root_usage(&actor)
        .await
        .expect("mixed actor lists storage roots");

    assert!(!roots.is_empty());
    assert!(
        roots
            .iter()
            .all(|root| root.library_id == fixture.movie_library_id),
        "only the managed library's roots are listed"
    );
}

// ── Nested fields: silent empty or null ───────────────────────────────────

#[tokio::test]
async fn nested_wanted_items_are_silently_empty_for_viewers() {
    let fixture = seed_fixture().await;
    let wanted = fixture
        .app
        .get_title_wanted_items_for_titles(
            &viewer_for(&fixture.movie_library_id),
            std::slice::from_ref(&fixture.movie_title.id),
        )
        .await
        .expect("a viewer is not refused");
    assert!(wanted.is_empty());
}

#[tokio::test]
async fn title_root_folder_path_is_null_for_viewers() {
    let fixture = seed_fixture().await;
    let title = &fixture.movie_title;
    let movie = fixture.movie_library_id.as_str();

    let viewer_path = fixture
        .app
        .title_root_folder_path_for_viewer(
            &viewer_for(movie),
            &title.root_folder_id,
            &title.library_id,
            &title.facet,
        )
        .await
        .expect("a viewer is not refused");
    assert_eq!(viewer_path, None);

    for (label, actor) in [
        ("title manager", title_manager_for(movie)),
        ("library manager", library_manager_for(movie)),
        (
            "catalog settings admin",
            app_admin_viewer_for(movie, AppPermissionMask::MANAGE_CATALOG_SETTINGS),
        ),
        ("full admin", fixture.admin.clone()),
    ] {
        let path = fixture
            .app
            .title_root_folder_path_for_viewer(
                &actor,
                &title.root_folder_id,
                &title.library_id,
                &title.facet,
            )
            .await
            .expect("privileged actor reads the root path")
            .unwrap_or_else(|| panic!("{label}: root path should be visible"));
        assert!(path.ends_with("movies"), "{label}: unexpected root {path}");
    }

    // Without View on the library the read is still refused outright.
    let outsider = viewer_for(&fixture.series_library_id);
    assert!(
        fixture
            .app
            .title_root_folder_path_for_viewer(
                &outsider,
                &title.root_folder_id,
                &title.library_id,
                &title.facet,
            )
            .await
            .is_err()
    );
}

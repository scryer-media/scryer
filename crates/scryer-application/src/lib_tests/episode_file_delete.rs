use super::*;

/// Seed a series title whose media files live under a real temp root, so the
/// delete manifests can be built from actual files on disk.
struct EpisodeDeleteFixture {
    app: AppUseCase,
    admin: User,
    title_id: String,
    root: PathBuf,
    media_files: Arc<MockMediaFileRepo>,
    _tempdir: tempfile::TempDir,
}

async fn seed_episode_delete_fixture() -> EpisodeDeleteFixture {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root = tempdir.path().join("series");
    std::fs::create_dir_all(root.join("Emberfall/Season 01")).expect("create season folder");

    let media_files = Arc::new(MockMediaFileRepo::default());
    let (app, admin, _titles) = bootstrap_with_cutoff_projection_state(
        Arc::new(StoredSettingsRepo::default()),
        Arc::new(StoredQualityProfileRepo::default()),
        media_files.clone(),
    );

    app.update_media_settings(
        &admin,
        MediaFacet::Series,
        empty_update_media_settings_with_roots(vec![build_root_folder_entry(&root, true)]),
    )
    .await
    .expect("save series roots");

    let title = app
        .add_title(
            &admin,
            NewTitle {
                name: "Emberfall".into(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create series title");

    EpisodeDeleteFixture {
        app,
        admin,
        title_id: title.id,
        root,
        media_files,
        _tempdir: tempdir,
    }
}

impl EpisodeDeleteFixture {
    /// Create the file on disk and insert a media-file row linked to `episode_id`.
    async fn add_episode_file(&self, relative_path: &str, episode_id: &str) -> String {
        let path = self.root.join(relative_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent folder");
        }
        std::fs::write(&path, b"video").expect("write media file");
        self.insert_media_file_row(&path.to_string_lossy(), Some(episode_id))
            .await
    }

    /// Insert a media-file row without creating anything on disk.
    async fn insert_media_file_row(&self, file_path: &str, episode_id: Option<&str>) -> String {
        let file_id = self
            .app
            .services
            .library
            .media_files
            .insert_media_file(&InsertMediaFileInput {
                title_id: self.title_id.clone(),
                file_path: file_path.to_string(),
                size_bytes: 5,
                role: MediaFileRole::Primary,
                ..Default::default()
            })
            .await
            .expect("insert media file");
        if let Some(episode_id) = episode_id {
            let mut store = self.media_files.store.lock().await;
            let row = store
                .iter_mut()
                .find(|row| row.id == file_id)
                .expect("seeded media file row");
            row.episode_id = Some(episode_id.to_string());
        }
        file_id
    }

    async fn remaining_file_ids(&self) -> Vec<String> {
        self.app
            .services
            .library
            .media_files
            .list_media_files_for_title(&self.title_id)
            .await
            .expect("list media files")
            .into_iter()
            .map(|file| file.id)
            .collect()
    }
}

#[tokio::test]
async fn preview_delete_episode_files_covers_only_requested_episodes() {
    let fixture = seed_episode_delete_fixture().await;
    let selected = fixture
        .add_episode_file("Emberfall/Season 01/Emberfall - S01E01.mkv", "episode-1")
        .await;
    let other_episode = fixture
        .add_episode_file("Emberfall/Season 01/Emberfall - S01E02.mkv", "episode-2")
        .await;
    let unlinked = fixture
        .insert_media_file_row(
            &fixture
                .root
                .join("Emberfall/Season 01/Emberfall - extras.mkv")
                .to_string_lossy(),
            None,
        )
        .await;

    let preview = fixture
        .app
        .preview_delete_episode_files(
            &fixture.admin,
            &fixture.title_id,
            &["episode-1".to_string()],
        )
        .await
        .expect("preview episode file delete");

    assert_eq!(preview.file_count, 1);
    let previewed_ids = preview
        .items
        .iter()
        .map(|item| item.file_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(previewed_ids, vec![selected.as_str()]);
    assert!(!previewed_ids.contains(&other_episode.as_str()));
    assert!(!previewed_ids.contains(&unlinked.as_str()));
    assert_eq!(preview.items[0].episode_id, "episode-1");
    assert_eq!(preview.preview.media_count, 1);
    assert!(!preview.preview.fingerprint.is_empty());
}

#[tokio::test]
async fn preview_delete_episode_files_deduplicates_and_ignores_episode_id_order() {
    let fixture = seed_episode_delete_fixture().await;
    fixture
        .add_episode_file("Emberfall/Season 01/Emberfall - S01E01.mkv", "episode-1")
        .await;
    fixture
        .add_episode_file("Emberfall/Season 01/Emberfall - S01E02.mkv", "episode-2")
        .await;

    let forward = fixture
        .app
        .preview_delete_episode_files(
            &fixture.admin,
            &fixture.title_id,
            &["episode-1".to_string(), "episode-2".to_string()],
        )
        .await
        .expect("forward preview");
    let reversed = fixture
        .app
        .preview_delete_episode_files(
            &fixture.admin,
            &fixture.title_id,
            &[
                "episode-2".to_string(),
                "episode-1".to_string(),
                "episode-2".to_string(),
            ],
        )
        .await
        .expect("reversed preview");

    assert_eq!(forward.file_count, 2);
    assert_eq!(reversed.file_count, 2);
    assert_eq!(
        forward.preview.fingerprint, reversed.preview.fingerprint,
        "aggregate fingerprint must not depend on episode id order"
    );
}

#[tokio::test]
async fn preview_delete_episode_files_returns_empty_preview_when_nothing_matches() {
    let fixture = seed_episode_delete_fixture().await;
    fixture
        .add_episode_file("Emberfall/Season 01/Emberfall - S01E01.mkv", "episode-1")
        .await;

    let preview = fixture
        .app
        .preview_delete_episode_files(
            &fixture.admin,
            &fixture.title_id,
            &["episode-missing".to_string()],
        )
        .await
        .expect("empty preview should not be an error");

    assert_eq!(preview.file_count, 0);
    assert!(preview.items.is_empty());
    assert_eq!(preview.preview.total_file_count, 0);
    assert!(!preview.preview.requires_typed_confirmation);
}

#[tokio::test]
async fn preview_delete_episode_files_rejects_an_empty_selection() {
    let fixture = seed_episode_delete_fixture().await;

    let error = fixture
        .app
        .preview_delete_episode_files(&fixture.admin, &fixture.title_id, &[])
        .await
        .expect_err("empty selection must be rejected");
    assert!(matches!(error, AppError::Validation(_)), "{error:?}");

    let error = fixture
        .app
        .delete_episode_files(&fixture.admin, &fixture.title_id, &[], false, None)
        .await
        .expect_err("empty selection must be rejected on execute");
    assert!(matches!(error, AppError::Validation(_)), "{error:?}");
}

#[tokio::test]
async fn preview_delete_episode_files_requires_manage_titles() {
    let fixture = seed_episode_delete_fixture().await;
    fixture
        .add_episode_file("Emberfall/Season 01/Emberfall - S01E01.mkv", "episode-1")
        .await;
    let viewer = create_user_with_permissions(
        &fixture.app,
        &fixture.admin,
        "viewer",
        "password123",
        vec![TestPermissionPreset::CatalogView],
    )
    .await
    .expect("create viewer");

    let error = fixture
        .app
        .preview_delete_episode_files(&viewer, &fixture.title_id, &["episode-1".to_string()])
        .await
        .expect_err("viewer must not preview episode file deletes");

    assert!(
        matches!(error, AppError::Unauthorized(_)),
        "expected Unauthorized, got {error:?}"
    );
}

#[tokio::test]
async fn delete_episode_files_rejects_a_stale_aggregate_fingerprint() {
    let fixture = seed_episode_delete_fixture().await;
    let file_id = fixture
        .add_episode_file("Emberfall/Season 01/Emberfall - S01E01.mkv", "episode-1")
        .await;

    let error = fixture
        .app
        .delete_episode_files(
            &fixture.admin,
            &fixture.title_id,
            &["episode-1".to_string()],
            true,
            Some(DeleteExecutionConfirmation {
                preview_fingerprint: "not-the-current-fingerprint".to_string(),
                typed_confirmation: None,
            }),
        )
        .await
        .expect_err("stale fingerprint must be rejected");

    assert!(
        matches!(&error, AppError::Validation(message) if message.contains("stale")),
        "expected a stale-preview validation error, got {error:?}"
    );
    assert_eq!(fixture.remaining_file_ids().await, vec![file_id]);
}

#[tokio::test]
async fn delete_episode_files_requires_confirmation_for_disk_deletes() {
    let fixture = seed_episode_delete_fixture().await;
    fixture
        .add_episode_file("Emberfall/Season 01/Emberfall - S01E01.mkv", "episode-1")
        .await;

    let error = fixture
        .app
        .delete_episode_files(
            &fixture.admin,
            &fixture.title_id,
            &["episode-1".to_string()],
            true,
            None,
        )
        .await
        .expect_err("disk delete without confirmation must be rejected");

    assert!(
        matches!(&error, AppError::Validation(message) if message.contains("confirmation")),
        "expected a confirmation validation error, got {error:?}"
    );
}

#[tokio::test]
async fn delete_episode_files_removes_selected_files_from_disk_and_catalog() {
    let fixture = seed_episode_delete_fixture().await;
    let first = fixture
        .add_episode_file("Emberfall/Season 01/Emberfall - S01E01.mkv", "episode-1")
        .await;
    let second = fixture
        .add_episode_file("Emberfall/Season 01/Emberfall - S01E02.mkv", "episode-2")
        .await;
    let kept = fixture
        .add_episode_file("Emberfall/Season 01/Emberfall - S01E03.mkv", "episode-3")
        .await;

    let episode_ids = vec!["episode-1".to_string(), "episode-2".to_string()];
    let preview = fixture
        .app
        .preview_delete_episode_files(&fixture.admin, &fixture.title_id, &episode_ids)
        .await
        .expect("preview");

    let outcome = fixture
        .app
        .delete_episode_files(
            &fixture.admin,
            &fixture.title_id,
            &episode_ids,
            true,
            Some(DeleteExecutionConfirmation {
                preview_fingerprint: preview.preview.fingerprint.clone(),
                typed_confirmation: None,
            }),
        )
        .await
        .expect("delete episode files");

    let mut deleted = outcome.deleted_file_ids.clone();
    deleted.sort();
    let mut expected = vec![first, second];
    expected.sort();
    assert_eq!(deleted, expected);
    assert!(outcome.failed.is_empty(), "unexpected failures: {:?}", outcome.failed);

    assert_eq!(fixture.remaining_file_ids().await, vec![kept]);
    assert!(
        !fixture
            .root
            .join("Emberfall/Season 01/Emberfall - S01E01.mkv")
            .exists()
    );
    assert!(
        !fixture
            .root
            .join("Emberfall/Season 01/Emberfall - S01E02.mkv")
            .exists()
    );
    assert!(
        fixture
            .root
            .join("Emberfall/Season 01/Emberfall - S01E03.mkv")
            .exists(),
        "unselected episode file must survive"
    );
}

#[tokio::test]
async fn delete_episode_files_records_per_file_failures_and_keeps_going() {
    let fixture = seed_episode_delete_fixture().await;
    let deletable = fixture
        .add_episode_file("Emberfall/Season 01/Emberfall - S01E01.mkv", "episode-1")
        .await;
    // A row whose path sits outside every configured root: its preview fails, so
    // it must be reported as a failure rather than aborting the whole batch.
    let outside_root = fixture
        .insert_media_file_row("/definitely/not/a/root/Emberfall - S01E02.mkv", Some("episode-2"))
        .await;

    let episode_ids = vec!["episode-1".to_string(), "episode-2".to_string()];
    let preview = fixture
        .app
        .preview_delete_episode_files(&fixture.admin, &fixture.title_id, &episode_ids)
        .await
        .expect("preview");
    assert_eq!(preview.file_count, 2);
    assert_eq!(
        preview
            .items
            .iter()
            .filter(|item| item.error.is_some())
            .count(),
        1
    );

    let outcome = fixture
        .app
        .delete_episode_files(
            &fixture.admin,
            &fixture.title_id,
            &episode_ids,
            true,
            Some(DeleteExecutionConfirmation {
                preview_fingerprint: preview.preview.fingerprint.clone(),
                typed_confirmation: None,
            }),
        )
        .await
        .expect("batch delete should not abort on a per-file failure");

    assert_eq!(outcome.deleted_file_ids, vec![deletable]);
    assert_eq!(outcome.failed.len(), 1);
    assert_eq!(outcome.failed[0].file_id, outside_root);
    assert_eq!(fixture.remaining_file_ids().await, vec![outside_root]);
}

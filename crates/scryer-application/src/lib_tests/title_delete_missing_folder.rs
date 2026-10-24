use super::*;

/// A library root under a temp folder and one title with no recorded folder.
struct MissingFolderFixture {
    app: AppUseCase,
    admin: User,
    title_id: String,
    root: PathBuf,
    tempdir: tempfile::TempDir,
    titles: Arc<MockTitleRepo>,
}

async fn seed(facet: MediaFacet) -> MissingFolderFixture {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root = tempdir.path().join("library");
    std::fs::create_dir_all(&root).expect("create library root");
    let (app, admin, titles) = bootstrap_with_cutoff_projection_state(
        Arc::new(StoredSettingsRepo::default()),
        Arc::new(StoredQualityProfileRepo::default()),
        Arc::new(MockMediaFileRepo::default()),
    );
    app.update_media_settings(
        &admin,
        facet.clone(),
        empty_update_media_settings_with_roots(vec![build_root_folder_entry(&root, true)]),
    )
    .await
    .expect("save library roots");
    let title_id = add_title(&app, &admin, "Quillmere", facet).await;
    let fixture = MissingFolderFixture {
        app,
        admin,
        title_id,
        root,
        tempdir,
        titles,
    };
    assert_eq!(fixture.folder_path(&fixture.title_id).await, None);
    fixture
}

async fn add_title(app: &AppUseCase, admin: &User, name: &str, facet: MediaFacet) -> String {
    app.add_title(
        admin,
        NewTitle {
            name: name.into(),
            facet,
            monitored: false,
            ..Default::default()
        },
    )
    .await
    .expect("create title")
    .id
}

impl MissingFolderFixture {
    /// Write a file and record it for `title_id`.
    async fn record_file(&self, title_id: &str, path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).expect("create file folder");
        std::fs::write(path, b"synthetic video").expect("write media file");
        self.app
            .services
            .library
            .media_files
            .insert_media_file(&InsertMediaFileInput {
                title_id: title_id.to_string(),
                file_path: path.to_string_lossy().into_owned(),
                size_bytes: 15,
                role: MediaFileRole::Primary,
                ..Default::default()
            })
            .await
            .expect("record media file");
    }

    async fn folder_path(&self, title_id: &str) -> Option<String> {
        self.app
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await
            .expect("load title")
            .expect("title exists")
            .folder_path
            .filter(|path| !path.is_empty())
    }

    async fn title_exists(&self) -> bool {
        self.app
            .services
            .catalog
            .titles
            .get_by_id(&self.title_id)
            .await
            .expect("load title")
            .is_some()
    }

    async fn delete_with_files(&self, fingerprint: String) -> AppResult<()> {
        self.app
            .delete_title(
                &self.admin,
                &self.title_id,
                true,
                Some(DeleteExecutionConfirmation {
                    preview_fingerprint: fingerprint,
                    typed_confirmation: None,
                }),
            )
            .await
    }

    /// The preview and the delete both refuse, and nothing changes.
    async fn assert_refused(&self, kept: &[&Path]) {
        let preview = self
            .app
            .preview_delete_title_files(&self.admin, &self.title_id)
            .await;
        assert!(
            matches!(
                preview,
                Err(AppError::ValidationRefused {
                    reason: "title_folder_unknown",
                    ..
                })
            ),
            "preview should refuse: {preview:?}"
        );
        let bulk = self
            .app
            .preview_delete_titles_files(&self.admin, std::slice::from_ref(&self.title_id))
            .await
            .expect("bulk preview");
        assert!(bulk.items[0].error.is_some(), "bulk preview should refuse");
        let deleted = self.delete_with_files("stale-or-forged".into()).await;
        assert!(
            matches!(
                deleted,
                Err(AppError::ValidationRefused {
                    reason: "title_folder_unknown",
                    ..
                })
            ),
            "delete should refuse: {deleted:?}"
        );
        assert!(self.title_exists().await);
        assert_eq!(self.folder_path(&self.title_id).await, None);
        for path in kept {
            assert_eq!(std::fs::read(path).unwrap(), b"synthetic video");
        }
    }
}

#[tokio::test]
async fn a_title_with_no_folder_and_no_files_deletes_with_nothing_removed_on_disk() {
    let fixture = seed(MediaFacet::Movie).await;
    let bystander = fixture.root.join("Bystander (2026)/Bystander (2026).mkv");
    std::fs::create_dir_all(bystander.parent().unwrap()).unwrap();
    std::fs::write(&bystander, b"synthetic video").unwrap();

    let preview = fixture
        .app
        .preview_delete_title_files(&fixture.admin, &fixture.title_id)
        .await
        .expect("preview a title with nothing on disk");
    assert_eq!(preview.media_count, 0);
    assert_eq!(preview.total_file_count, 0);

    fixture
        .delete_with_files(preview.fingerprint)
        .await
        .expect("delete with files ticked");
    assert!(!fixture.title_exists().await);
    assert_eq!(std::fs::read(&bystander).unwrap(), b"synthetic video");
}

#[tokio::test]
async fn a_title_with_files_and_no_folder_records_the_implied_folder_and_deletes_it() {
    let fixture = seed(MediaFacet::Series).await;
    let title_folder = fixture.root.join("Quillmere");
    fixture
        .record_file(
            &fixture.title_id,
            &title_folder.join("Season 01/Quillmere - S01E01.mkv"),
        )
        .await;
    fixture
        .record_file(
            &fixture.title_id,
            &title_folder.join("Season 02/Quillmere - S02E01.mkv"),
        )
        .await;
    let neighbour = add_title(&fixture.app, &fixture.admin, "Vellicor", MediaFacet::Series).await;
    let neighbour_folder = fixture.root.join("Vellicor");
    fixture
        .app
        .services
        .catalog
        .titles
        .set_folder_path(&neighbour, &neighbour_folder.to_string_lossy())
        .await
        .unwrap();
    let neighbour_file = neighbour_folder.join("Season 01/Vellicor - S01E01.mkv");
    fixture.record_file(&neighbour, &neighbour_file).await;

    let preview = fixture
        .app
        .preview_delete_title_files(&fixture.admin, &fixture.title_id)
        .await
        .expect("preview from the implied folder");
    assert_eq!(preview.media_count, 2);
    assert_eq!(
        fixture.folder_path(&fixture.title_id).await,
        None,
        "a preview records nothing"
    );

    fixture
        .delete_with_files(preview.fingerprint)
        .await
        .expect("delete the implied folder");
    assert!(!fixture.title_exists().await);
    assert!(!title_folder.exists());
    assert_eq!(std::fs::read(&neighbour_file).unwrap(), b"synthetic video");
    assert_eq!(
        fixture.folder_path(&neighbour).await.as_deref(),
        Some(neighbour_folder.to_string_lossy().as_ref())
    );
}

#[tokio::test]
async fn files_split_across_folders_leave_the_title_folder_unknown() {
    let fixture = seed(MediaFacet::Movie).await;
    let first = fixture.root.join("Quillmere (2026)/Quillmere (2026).mkv");
    let second = fixture.root.join("Quillmere Copy/Quillmere (2026).mkv");
    fixture.record_file(&fixture.title_id, &first).await;
    fixture.record_file(&fixture.title_id, &second).await;
    fixture.assert_refused(&[&first, &second]).await;
}

#[tokio::test]
async fn files_directly_in_a_library_root_are_never_claimed() {
    let fixture = seed(MediaFacet::Movie).await;
    let loose = fixture.root.join("Quillmere (2026).mkv");
    let bystander = fixture.root.join("Bystander (2026).mkv");
    std::fs::write(&bystander, b"synthetic video").unwrap();
    fixture.record_file(&fixture.title_id, &loose).await;
    fixture.assert_refused(&[&loose, &bystander]).await;
}

#[tokio::test]
async fn a_folder_another_title_owns_is_never_claimed() {
    let fixture = seed(MediaFacet::Movie).await;
    let folder = fixture.root.join("Shared (2026)");
    let owner = add_title(&fixture.app, &fixture.admin, "Shared", MediaFacet::Movie).await;
    fixture
        .app
        .services
        .catalog
        .titles
        .set_folder_path(&owner, &folder.to_string_lossy())
        .await
        .unwrap();
    let owner_file = folder.join("Shared (2026).mkv");
    fixture.record_file(&owner, &owner_file).await;
    let ours = folder.join("Quillmere (2026).mkv");
    fixture.record_file(&fixture.title_id, &ours).await;
    fixture.assert_refused(&[&owner_file, &ours]).await;
}

#[tokio::test]
async fn a_folder_outside_the_library_is_never_claimed() {
    let fixture = seed(MediaFacet::Movie).await;
    let outside = fixture
        .tempdir
        .path()
        .join("elsewhere/Quillmere (2026)/Quillmere (2026).mkv");
    fixture.record_file(&fixture.title_id, &outside).await;
    fixture.assert_refused(&[&outside]).await;
}

#[tokio::test]
async fn title_delete_lost_folder_claim_preserves_files_and_catalog() {
    let fixture = seed(MediaFacet::Movie).await;
    let file = fixture.root.join("Quillmere (2026)/Quillmere (2026).mkv");
    fixture.record_file(&fixture.title_id, &file).await;
    let preview = fixture
        .app
        .preview_delete_title_files(&fixture.admin, &fixture.title_id)
        .await
        .unwrap();
    fixture
        .titles
        .lose_folder_claim
        .store(true, Ordering::SeqCst);
    let error = fixture
        .delete_with_files(preview.fingerprint)
        .await
        .unwrap_err();
    assert_eq!(
        error.validation_reason(),
        Some(crate::title_delete::FOLDER_CLAIM_LOST)
    );
    assert_eq!(std::fs::read(&file).unwrap(), b"synthetic video");
    assert!(fixture.title_exists().await);
    assert_eq!(fixture.folder_path(&fixture.title_id).await, None);
}

#[tokio::test]
async fn title_delete_bulk_enforces_aggregate_confirmation() {
    let fixture = seed(MediaFacet::Series).await;
    let second = add_title(&fixture.app, &fixture.admin, "Other", MediaFacet::Series).await;
    for (id, name) in [(&fixture.title_id, "Quillmere"), (&second, "Other")] {
        let folder = fixture.root.join(name);
        fixture
            .app
            .services
            .catalog
            .titles
            .set_folder_path(id, &folder.to_string_lossy())
            .await
            .unwrap();
        for episode in 1..=26 {
            fixture
                .record_file(
                    id,
                    &folder.join(format!("Season 01/{name}.S01E{episode:02}.mkv")),
                )
                .await;
        }
    }
    let ids = vec![fixture.title_id.clone(), second];
    let mut items = Vec::new();
    for id in &ids {
        let preview = fixture
            .app
            .preview_delete_title_files(&fixture.admin, id)
            .await
            .unwrap();
        assert!(!preview.requires_typed_confirmation);
        items.push(DeleteTitlesJobItem {
            title_id: id.clone(),
            preview_fingerprint: Some(preview.fingerprint),
        });
    }
    let error = fixture
        .app
        .start_delete_titles_job(
            &fixture.admin,
            DeleteTitlesJobRequest {
                items,
                delete_files_on_disk: true,
                typed_confirmation: None,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(
        error.validation_reason(),
        Some(crate::title_delete::TYPED_CONFIRMATION_REQUIRED)
    );
    assert!(fixture.title_exists().await);
    assert!(
        fixture
            .root
            .join("Other/Season 01/Other.S01E26.mkv")
            .exists()
    );
}

#[tokio::test]
async fn title_delete_bulk_sees_owner_created_during_earlier_item() {
    crate::test_wait::within_deadline("late owner blocks the later batch item", async {
        let fixture = seed(MediaFacet::Movie).await;
        let later = add_title(
            &fixture.app,
            &fixture.admin,
            "Later Feature",
            MediaFacet::Movie,
        )
        .await;
        let first_file = fixture.root.join("Quillmere/feature.mkv");
        let later_folder = fixture.root.join("Later Feature");
        let later_file = later_folder.join("feature.mkv");
        fixture.record_file(&fixture.title_id, &first_file).await;
        fixture.record_file(&later, &later_file).await;
        fixture
            .titles
            .set_folder_path(&later, &later_folder.to_string_lossy())
            .await
            .unwrap();
        let ids = vec![fixture.title_id.clone(), later.clone()];
        let preview = fixture
            .app
            .preview_delete_titles_files(&fixture.admin, &ids)
            .await
            .unwrap();
        let jobs = Arc::new(RecordingJobRunRepo::default());
        let app = fixture
            .app
            .with_test_overrides(|services| services.with_job_runs(jobs.clone()));
        let operations = Arc::new(Mutex::new(Vec::new()));
        fixture
            .titles
            .set_delete_operation_log(operations.clone())
            .await;
        let barrier = operations.lock().await;
        let accepted = app
            .start_delete_titles_job(
                &fixture.admin,
                DeleteTitlesJobRequest {
                    items: preview
                        .items
                        .into_iter()
                        .map(|item| DeleteTitlesJobItem {
                            title_id: item.title_id,
                            preview_fingerprint: Some(item.preview.unwrap().fingerprint),
                        })
                        .collect(),
                    delete_files_on_disk: true,
                    typed_confirmation: None,
                },
            )
            .await
            .unwrap();
        crate::test_wait::wait_until("first batch item reaches catalog deletion", || async {
            fixture
                .titles
                .delete_started
                .lock()
                .await
                .contains(&fixture.title_id)
        })
        .await;
        let owner = add_title(&app, &fixture.admin, "Late Folder Owner", MediaFacet::Movie).await;
        fixture
            .titles
            .set_folder_path(&owner, &later_folder.to_string_lossy())
            .await
            .unwrap();
        drop(barrier);
        let run = crate::test_wait::wait_for("specific late-owner batch completes", || async {
            jobs.runs
                .lock()
                .await
                .iter()
                .find(|run| run.id == accepted.job_run.id && run.status.is_terminal())
                .cloned()
        })
        .await;
        let summary: serde_json::Value =
            serde_json::from_str(run.summary_json.as_deref().unwrap()).unwrap();
        assert_eq!(
            summary["succeededTitleIds"],
            serde_json::json!([fixture.title_id])
        );
        assert_eq!(summary["failures"][0]["titleId"], later);
        assert_eq!(
            summary["failures"][0]["reason"],
            crate::title_delete::FOLDER_OVERLAPS_TITLE
        );
        assert_eq!(std::fs::read(&later_file).unwrap(), b"synthetic video");
        assert!(
            app.services
                .catalog
                .titles
                .get_by_id(&later)
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            app.services
                .catalog
                .titles
                .get_by_id(&owner)
                .await
                .unwrap()
                .is_some()
        );
    })
    .await;
}

#[tokio::test]
async fn title_delete_catalog_failure_retains_media_and_folder_for_fresh_preview_retry() {
    let mut fixture = seed(MediaFacet::Movie).await;
    let pending = Arc::new(TrackingPendingReleaseRepo::default());
    pending
        .fail_delete_for_title("synthetic cleanup failure")
        .await;
    fixture.app = fixture
        .app
        .with_test_overrides(|services| services.with_pending_releases(pending.clone()));
    let folder = fixture.root.join("Quillmere (2026)");
    let file = folder.join("Quillmere (2026).mkv");
    fixture.record_file(&fixture.title_id, &file).await;
    let bystander = fixture.root.join("unrelated.txt");
    std::fs::write(&bystander, b"preserve").unwrap();
    let preview = fixture
        .app
        .preview_delete_title_files(&fixture.admin, &fixture.title_id)
        .await
        .unwrap();
    let error = fixture
        .delete_with_files(preview.fingerprint)
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::RepositoryFailed { .. }));
    assert_eq!(
        error.reason(),
        Some(crate::title_delete::CATALOG_CLEANUP_FAILED)
    );
    assert!(!file.exists());
    assert!(fixture.title_exists().await);
    assert_eq!(
        fixture.folder_path(&fixture.title_id).await.as_deref(),
        Some(folder.to_str().unwrap())
    );
    assert_eq!(
        fixture
            .app
            .services
            .library
            .media_files
            .list_media_files_for_title(&fixture.title_id)
            .await
            .unwrap()
            .len(),
        1
    );
    let retry = fixture
        .app
        .preview_delete_title_files(&fixture.admin, &fixture.title_id)
        .await
        .unwrap();
    assert_eq!(retry.total_file_count, 0);
    *pending.delete_error.lock().await = None;
    fixture.delete_with_files(retry.fingerprint).await.unwrap();
    assert!(!fixture.title_exists().await);
    assert_eq!(std::fs::read(bystander).unwrap(), b"preserve");
}

#[tokio::test]
async fn title_delete_bulk_read_failure_is_not_catalog_cleanup_failure() {
    crate::test_wait::within_deadline("bulk delete classifies failure by step", async {
        let fixture = seed(MediaFacet::Movie).await;
        let later = add_title(
            &fixture.app,
            &fixture.admin,
            "Unreadable Later Title",
            MediaFacet::Movie,
        )
        .await;
        let jobs = Arc::new(RecordingJobRunRepo::default());
        let app = fixture
            .app
            .with_test_overrides(|services| services.with_job_runs(jobs.clone()));
        let operations = Arc::new(Mutex::new(Vec::new()));
        fixture
            .titles
            .set_delete_operation_log(operations.clone())
            .await;
        let barrier = operations.lock().await;
        let accepted = app
            .start_delete_titles_job(
                &fixture.admin,
                DeleteTitlesJobRequest {
                    items: [&fixture.title_id, &later]
                        .into_iter()
                        .map(|id| DeleteTitlesJobItem {
                            title_id: id.clone(),
                            preview_fingerprint: None,
                        })
                        .collect(),
                    delete_files_on_disk: false,
                    typed_confirmation: None,
                },
            )
            .await
            .unwrap();
        crate::test_wait::wait_until("first item reaches catalog delete", || async {
            fixture
                .titles
                .delete_started
                .lock()
                .await
                .contains(&fixture.title_id)
        })
        .await;
        fixture.titles.fail_get_by_id.store(true, Ordering::SeqCst);
        drop(barrier);
        let run = crate::test_wait::wait_for("specific read-failure batch completes", || async {
            jobs.runs
                .lock()
                .await
                .iter()
                .find(|run| run.id == accepted.job_run.id && run.status.is_terminal())
                .cloned()
        })
        .await;
        let summary: serde_json::Value =
            serde_json::from_str(run.summary_json.as_deref().unwrap()).unwrap();
        assert_eq!(
            summary["succeededTitleIds"],
            serde_json::json!([fixture.title_id])
        );
        assert_eq!(summary["failures"][0]["titleId"], later);
        assert!(summary["failures"][0]["reason"].is_null());
        fixture.titles.fail_get_by_id.store(false, Ordering::SeqCst);
        assert!(
            app.services
                .catalog
                .titles
                .get_by_id(&later)
                .await
                .unwrap()
                .is_some()
        );
        assert!(!app.title_is_being_deleted(&later));
    })
    .await;
}

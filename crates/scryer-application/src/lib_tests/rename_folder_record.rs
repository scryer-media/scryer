use super::*;

/// Moves each planned file the way the library renamer does, without the
/// collision handling a real one carries.
struct MovingRenamer;

#[async_trait]
impl LibraryRenamer for MovingRenamer {
    async fn validate_targets(&self, _plan: &RenamePlan) -> AppResult<()> {
        Ok(())
    }

    async fn apply_plan(
        &self,
        plan: &RenamePlan,
        _permissions: &crate::ImportFilePermissions,
    ) -> AppResult<Vec<RenameApplyItemResult>> {
        let mut results = Vec::new();
        for item in &plan.items {
            let target = item.proposed_path.clone().expect("planned target");
            std::fs::create_dir_all(Path::new(&target).parent().unwrap())
                .expect("create target folder");
            std::fs::rename(&item.current_path, &target).expect("move file");
            results.push(RenameApplyItemResult {
                collection_id: item.collection_id.clone(),
                media_file_id: item.media_file_id.clone(),
                series_movie_link_ids: item.series_movie_link_ids.clone(),
                current_path: item.current_path.clone(),
                proposed_path: item.proposed_path.clone(),
                final_path: Some(target),
                write_action: item.write_action.clone(),
                status: RenameApplyStatus::Applied,
                reason_code: "renamed".into(),
                error_message: None,
            });
        }
        Ok(results)
    }

    async fn rollback(
        &self,
        _applied_items: &[RenameApplyItemResult],
    ) -> AppResult<Vec<RenameApplyItemResult>> {
        Ok(Vec::new())
    }
}

/// One movie with one file in `<root>/Emberfall`, and a plan that moves the
/// file into `<root>/Emberfall (2026)`.
struct RenameFixture {
    app: AppUseCase,
    admin: User,
    titles: Arc<MockTitleRepo>,
    title_id: String,
    old_folder: PathBuf,
    old_folder_path: String,
    old_file: PathBuf,
    new_file: PathBuf,
    plan: RenamePlan,
    _tempdir: tempfile::TempDir,
}

async fn rename_fixture() -> RenameFixture {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root = tempdir.path().join("library");
    std::fs::create_dir_all(&root).expect("create library root");
    let (mut app, admin, titles) = bootstrap_with_cutoff_projection_state(
        Arc::new(StoredSettingsRepo::default()),
        Arc::new(StoredQualityProfileRepo::default()),
        Arc::new(MockMediaFileRepo::default()),
    );
    app.services.library.library_renamer = Arc::new(MovingRenamer);
    update_default_library_roots(
        &app,
        &admin,
        MediaFacet::Movie,
        vec![build_root_folder_entry(&root, true)],
    )
    .await
    .expect("save library roots");
    let title_id = app
        .add_title(
            &admin,
            NewTitle {
                name: "Emberfall".into(),
                facet: MediaFacet::Movie,
                monitored: false,
                ..Default::default()
            },
        )
        .await
        .expect("create title")
        .id;

    let old_folder = root.join("Emberfall");
    let new_folder = root.join("Emberfall (2026)");
    let old_file = old_folder.join("Emberfall.mkv");
    let new_file = new_folder.join("Emberfall (2026).mkv");
    std::fs::create_dir_all(&old_folder).expect("create title folder");
    std::fs::write(&old_file, b"synthetic video").expect("write media file");
    let old_folder_path = old_folder.to_string_lossy().into_owned();
    app.services
        .catalog
        .titles
        .set_folder_path(&title_id, &old_folder_path)
        .await
        .expect("record title folder");
    let file_id = app
        .services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title_id.clone(),
            file_path: old_file.to_string_lossy().into_owned(),
            size_bytes: 15,
            role: MediaFileRole::Primary,
            ..Default::default()
        })
        .await
        .expect("record media file");

    let mut plan = crate::library::rename::build_rename_plan_from_items(
        MediaFacet::Movie,
        Some(title_id.clone()),
        "{title}".into(),
        RenameCollisionPolicy::Skip,
        RenameMissingMetadataPolicy::FallbackTitle,
        vec![RenamePlanItem {
            collection_id: None,
            media_file_id: Some(file_id),
            series_movie_link_ids: Vec::new(),
            current_path: old_file.to_string_lossy().into_owned(),
            proposed_path: Some(new_file.to_string_lossy().into_owned()),
            normalized_filename: None,
            collision: false,
            reason_code: "rename".into(),
            write_action: RenameWriteAction::Move,
            source_size_bytes: None,
            source_mtime_unix_ms: None,
            companion_of: None,
        }],
    );
    plan.title_folders = vec![RenamePlanTitleFolder {
        title_id: title_id.clone(),
        folder_path: new_folder.to_string_lossy().into_owned(),
    }];
    RenameFixture {
        app,
        admin,
        titles,
        title_id,
        old_folder,
        old_folder_path,
        old_file,
        new_file,
        plan,
        _tempdir: tempdir,
    }
}

impl RenameFixture {
    async fn recorded_folder(&self) -> Option<String> {
        self.app
            .services
            .catalog
            .titles
            .get_by_id(&self.title_id)
            .await
            .expect("load title")
            .expect("title exists")
            .folder_path
    }
}

#[tokio::test]
async fn a_rename_whose_folder_record_is_refused_keeps_the_previous_folder() {
    let fixture = rename_fixture().await;
    // The file moves; the folder record is then lost to a concurrent change.
    fixture
        .titles
        .lose_folder_claim
        .store(true, Ordering::SeqCst);
    let result = fixture
        .app
        .apply_rename_plan(&fixture.admin, fixture.plan.clone())
        .await
        .expect("apply rename");

    assert_eq!(result.failed, 1, "{:?}", result.items);
    assert_eq!(
        result.items[0].reason_code,
        crate::library::rename::RENAME_FOLDER_RECORD_FAILED
    );
    assert_eq!(
        std::fs::read(&fixture.new_file).unwrap(),
        b"synthetic video"
    );
    assert!(
        fixture.old_folder.is_dir(),
        "the folder the title still records must not be removed"
    );
    assert_eq!(
        fixture.recorded_folder().await.as_deref(),
        Some(fixture.old_folder_path.as_str())
    );
}

#[tokio::test]
async fn a_rename_into_a_folder_the_title_cannot_own_moves_nothing() {
    let fixture = rename_fixture().await;
    fixture
        .titles
        .refuse_folder_checks
        .store(true, Ordering::SeqCst);
    let error = fixture
        .app
        .apply_rename_plan(&fixture.admin, fixture.plan.clone())
        .await
        .expect_err("the folder check refuses the rename");

    assert_eq!(
        error.validation_reason(),
        Some(crate::error_reasons::folder_ownership::FOLDER_ALREADY_OWNED_TITLE)
    );
    assert_eq!(
        std::fs::read(&fixture.old_file).unwrap(),
        b"synthetic video"
    );
    assert!(!fixture.new_file.exists());
    assert_eq!(
        fixture.recorded_folder().await.as_deref(),
        Some(fixture.old_folder_path.as_str())
    );
    let files = fixture
        .app
        .services
        .library
        .media_files
        .list_media_files_for_title(&fixture.title_id)
        .await
        .expect("list media files");
    assert_eq!(files[0].file_path, fixture.old_file.to_string_lossy());
}

/// Applies a plan moving three files of one movie into a new folder, and
/// returns what a caller and the catalog see afterwards, with the temp root
/// replaced so two runs compare.
async fn apply_three_file_rename(
    refuse_grouped_write: bool,
) -> (Vec<String>, Vec<String>, Option<String>, usize) {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root = tempdir.path().join("library");
    std::fs::create_dir_all(&root).expect("create library root");
    let media_files = Arc::new(MockMediaFileRepo::default());
    if refuse_grouped_write {
        *media_files.grouped_path_write_error.lock().await = Some("grouped write refused".into());
    }
    let (mut app, admin, _titles) = bootstrap_with_cutoff_projection_state(
        Arc::new(StoredSettingsRepo::default()),
        Arc::new(StoredQualityProfileRepo::default()),
        media_files.clone(),
    );
    app.services.library.library_renamer = Arc::new(MovingRenamer);
    update_default_library_roots(
        &app,
        &admin,
        MediaFacet::Movie,
        vec![build_root_folder_entry(&root, true)],
    )
    .await
    .expect("save library roots");
    let title_id = app
        .add_title(
            &admin,
            NewTitle {
                name: "Emberfall".into(),
                facet: MediaFacet::Movie,
                monitored: false,
                ..Default::default()
            },
        )
        .await
        .expect("create title")
        .id;
    let old_folder = root.join("Emberfall");
    let new_folder = root.join("Emberfall (2026)");
    std::fs::create_dir_all(&old_folder).expect("create title folder");
    app.services
        .catalog
        .titles
        .set_folder_path(&title_id, &old_folder.to_string_lossy())
        .await
        .expect("record title folder");
    let mut items = Vec::new();
    for name in ["Emberfall", "Emberfall.part2", "Emberfall.part3"] {
        let old_file = old_folder.join(format!("{name}.mkv"));
        let new_file = new_folder.join(format!("{name} (2026).mkv"));
        std::fs::write(&old_file, b"synthetic video").expect("write media file");
        let file_id = app
            .services
            .library
            .media_files
            .insert_media_file(&InsertMediaFileInput {
                title_id: title_id.clone(),
                file_path: old_file.to_string_lossy().into_owned(),
                size_bytes: 15,
                role: MediaFileRole::Primary,
                ..Default::default()
            })
            .await
            .expect("record media file");
        items.push(RenamePlanItem {
            collection_id: None,
            media_file_id: Some(file_id),
            series_movie_link_ids: Vec::new(),
            current_path: old_file.to_string_lossy().into_owned(),
            proposed_path: Some(new_file.to_string_lossy().into_owned()),
            normalized_filename: None,
            collision: false,
            reason_code: "rename".into(),
            write_action: RenameWriteAction::Move,
            source_size_bytes: None,
            source_mtime_unix_ms: None,
            companion_of: None,
        });
    }
    let mut plan = crate::library::rename::build_rename_plan_from_items(
        MediaFacet::Movie,
        Some(title_id.clone()),
        "{title}".into(),
        RenameCollisionPolicy::Skip,
        RenameMissingMetadataPolicy::FallbackTitle,
        items,
    );
    plan.title_folders = vec![RenamePlanTitleFolder {
        title_id: title_id.clone(),
        folder_path: new_folder.to_string_lossy().into_owned(),
    }];

    let result = app
        .apply_rename_plan(&admin, plan)
        .await
        .expect("apply rename");

    let root_text = tempdir.path().to_string_lossy().into_owned();
    let strip = |value: &str| value.replace(&root_text, "<tmp>");
    let outcome = result
        .items
        .iter()
        .map(|item| {
            strip(&format!(
                "{:?}|{}|{:?}",
                item.status, item.reason_code, item.final_path
            ))
        })
        .chain([format!(
            "applied={} skipped={} failed={}",
            result.applied, result.skipped, result.failed
        )])
        .collect();
    let mut paths: Vec<String> = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title_id)
        .await
        .expect("list media files")
        .iter()
        .map(|file| strip(&file.file_path))
        .collect();
    paths.sort();
    let folder = app
        .services
        .catalog
        .titles
        .get_by_id(&title_id)
        .await
        .expect("load title")
        .expect("title exists")
        .folder_path
        .map(|folder| strip(&folder));
    let grouped = media_files.grouped_path_writes.load(Ordering::SeqCst);
    (outcome, paths, folder, grouped)
}

/// Recording a rename's new file paths in one grouped write leaves the same
/// results and catalog as writing each path in turn, which is what a refused
/// grouped write falls back to.
#[tokio::test]
async fn a_grouped_path_write_records_the_same_rename_as_one_write_per_file() {
    let grouped = apply_three_file_rename(false).await;
    let replayed = apply_three_file_rename(true).await;

    assert_eq!(grouped.0, replayed.0, "item results");
    assert_eq!(grouped.1, replayed.1, "recorded file paths");
    assert_eq!(grouped.2, replayed.2, "recorded title folder");
    assert_eq!(grouped.3, 1, "the three paths go out as one write");
    assert_eq!(replayed.3, 1, "the refused write is attempted once");
    assert!(
        grouped
            .1
            .iter()
            .all(|path| path.contains("Emberfall (2026)/")),
        "{:?}",
        grouped.1
    );
    assert!(
        grouped.0.last().unwrap().contains("applied=3"),
        "{:?}",
        grouped.0
    );
}

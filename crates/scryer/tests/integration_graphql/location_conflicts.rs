use super::*;
use scryer_application::location::conflicts::{
    ConflictLeftovers, ConflictPolicy, ConflictRequest, ConflictTask, ResolveConflictRequest,
    refusal,
};
use scryer_application::{LibraryScanUnmatchedItemRepository, UpdateRecycleBinSettings};

struct ConflictFixture {
    ctx: TestContext,
    actor: User,
    _temp: tempfile::TempDir,
    root: std::path::PathBuf,
    title: Title,
    named: std::path::PathBuf,
    junk: std::path::PathBuf,
    pending_id: String,
}

impl ConflictFixture {
    async fn new() -> Self {
        let ctx = TestContext::new().await;
        ctx.settings_store
            .batch_ensure_setting_definitions(vec![SettingDefinitionSeed {
                category: "media".into(),
                scope: scryer_application::SETTINGS_SCOPE_MEDIA.into(),
                key_name: scryer_application::RECYCLE_BIN_ENABLED_KEY.into(),
                data_type: "boolean".into(),
                default_value_json: "true".into(),
                is_sensitive: false,
                validation_json: None,
            }])
            .await
            .unwrap();
        let actor = ctx.app.find_or_create_default_user().await.unwrap();
        let temp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(temp.path()).unwrap();
        configure_default_library_root(&ctx, MediaFacet::Movie, &root).await;
        let title = create_catalog_title(
            &ctx,
            "Emberfall",
            MediaFacet::Movie,
            vec![],
            vec!["scryer:quality-profile:4k".into()],
            true,
        )
        .await;
        let named = root.join("Emberfall (2024)");
        let junk = root.join("ADEG78");
        std::fs::create_dir_all(&named).unwrap();
        std::fs::create_dir_all(&junk).unwrap();
        let inferior = named.join("Emberfall.2024.720p.WEB-DL.mkv");
        std::fs::write(&inferior, b"inferior movie").unwrap();
        std::fs::write(
            junk.join("Emberfall.2024.2160p.BluRay.mkv"),
            b"best movie contents",
        )
        .unwrap();
        std::fs::write(
            junk.join("Emberfall.2024.2160p.BluRay.en.srt"),
            b"subtitle contents",
        )
        .unwrap();
        std::fs::write(junk.join("poster.jpg"), b"artwork contents").unwrap();
        set_title_folder_path(&ctx, &title.id, &named).await;
        ctx.media_files
            .insert_media_file(&InsertMediaFileInput {
                title_id: title.id.clone(),
                file_path: inferior.to_string_lossy().into_owned(),
                size_bytes: 14,
                role: MediaFileRole::Primary,
                quality_label: Some("720p".into()),
                resolution: Some("720p".into()),
                source_type: Some("WEB-DL".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        let pending_id = seed_conflict(&ctx, &title, &junk, "duplicate-a").await;
        ctx.shows
            .create_collection(Collection {
                id: Id::new().0,
                title_id: title.id.clone(),
                collection_type: scryer_domain::CollectionType::Movie,
                collection_index: "1".into(),
                label: Some("720p".into()),
                ordered_path: Some(inferior.to_string_lossy().into_owned()),
                narrative_order: None,
                first_episode_number: None,
                last_episode_number: None,
                monitored: true,
                created_at: Utc::now(),
            })
            .await
            .unwrap();
        Self {
            ctx,
            actor,
            _temp: temp,
            root,
            title,
            named,
            junk,
            pending_id,
        }
    }

    fn request(&self, policy: ConflictPolicy, leftovers: ConflictLeftovers) -> ConflictRequest {
        ConflictRequest {
            pending_import_ids: vec![self.pending_id.clone()],
            policy,
            leftovers,
        }
    }

    async fn settle(&self, request: ConflictRequest, confirm: bool) -> ConflictTask {
        let preview = self
            .ctx
            .app
            .preview_pending_import_conflicts(&self.actor, request.clone())
            .await
            .unwrap();
        assert!(
            preview.rows.iter().all(|row| row.error_reason.is_none()),
            "{preview:?}"
        );
        let task = self
            .ctx
            .app
            .resolve_pending_import_conflicts(
                &self.actor,
                ResolveConflictRequest {
                    request,
                    plan_fingerprint: preview.plan_fingerprint,
                    confirm_permanent_delete: confirm,
                },
            )
            .await
            .unwrap();
        self.wait_task(&task.job_run_id).await
    }

    async fn wait_task(&self, id: &str) -> ConflictTask {
        use scryer_application::{JobRunRepository, JobRunStatus};
        use scryer_infrastructure_workflow::workflow::stores::workflow_operation_store::WorkflowOperationStore;
        let store = WorkflowOperationStore::new(self.ctx.db.datastore());
        let mut changes = self.ctx.app.subscribe_location_transfers();
        tokio::time::timeout(std::time::Duration::from_secs(120), async {
            loop {
                let persisted = store.get_job_run(id).await.unwrap().unwrap();
                if persisted.status != JobRunStatus::Running {
                    let state = self
                        .ctx
                        .app
                        .pending_import_conflict_task(&self.actor, id)
                        .await
                        .unwrap()
                        .unwrap();
                    if state.completed {
                        return state;
                    }
                }
                changes.changed().await.unwrap();
            }
        })
        .await
        .expect("the exact conflict task must settle")
    }
}

async fn seed_conflict(
    ctx: &TestContext,
    title: &Title,
    folder: &std::path::Path,
    id: &str,
) -> String {
    let now = Utc::now().to_rfc3339();
    let item = scryer_application::LibraryScanUnmatchedItem {
        id: id.into(),
        library_id: title.library_id.clone(),
        facet: MediaFacet::Movie,
        status: scryer_application::PendingImportStatus::Pending,
        title_id: Some(title.id.clone()),
        scan_session_id: "conflict-fixture".into(),
        scan_root: scryer_application::stored_paths::path_to_stored_string(
            folder.parent().unwrap(),
        ),
        item_path: scryer_application::stored_paths::path_to_stored_string(folder),
        display_name: "Emberfall".into(),
        query: "Emberfall".into(),
        year_hint: Some(2024),
        reason_code: scryer_application::location::conflicts::PENDING_REASON.into(),
        error_message: None,
        search_attempts: vec![],
        size_bytes: None,
        created_at: now.clone(),
        updated_at: now,
    };
    ctx.library_scan_unmatched
        .upsert_library_scan_unmatched_item(&item)
        .await
        .unwrap();
    id.into()
}

#[tokio::test]
async fn conflict_file_imports_normalize_and_deduplicate_without_losing_files() {
    for policy in [ConflictPolicy::AdoptAll, ConflictPolicy::KeepOne] {
        let fixture = ConflictFixture::new().await;
        let best = fixture.junk.join("Emberfall.2024.2160p.BluRay.mkv");
        let second = fixture.junk.join("Emberfall.2024.1080p.WEB-DL.mkv");
        let third_folder = fixture.root.join("third-copy");
        std::fs::create_dir_all(&third_folder).unwrap();
        let third = third_folder.join("Emberfall.2024.480p.WEB-DL.mkv");
        let unrelated = fixture.root.join("unrelated.mkv");
        std::fs::write(&second, b"second movie").unwrap();
        std::fs::write(&third, b"third movie").unwrap();
        std::fs::write(&unrelated, b"unrelated movie").unwrap();
        let mut ids = Vec::new();
        for (path, id) in [(&best, "file-a"), (&second, "file-b"), (&third, "file-c")] {
            ids.push(seed_conflict(&fixture.ctx, &fixture.title, path, id).await);
        }
        let request = ConflictRequest {
            pending_import_ids: ids.clone(),
            policy,
            leftovers: ConflictLeftovers::PreserveAll,
        };
        let preview = fixture
            .ctx
            .app
            .preview_pending_import_conflicts(&fixture.actor, request.clone())
            .await
            .unwrap();
        assert_eq!(
            (
                preview.total_count,
                preview.actionable_count,
                preview.failed_count
            ),
            (3, 3, 0)
        );
        assert!(
            preview.rows.iter().all(|row| row.error_reason.is_none()),
            "{preview:?}"
        );
        // Preview is read-only, including for inferior media and neighboring files.
        for (path, contents) in [
            (&best, b"best movie contents".as_slice()),
            (&second, b"second movie"),
            (&third, b"third movie"),
            (&unrelated, b"unrelated movie"),
        ] {
            assert_eq!(std::fs::read(path).unwrap(), contents);
        }
        let task = fixture.settle(request, false).await;
        assert_eq!(
            (
                task.settled_count,
                task.failed_count,
                task.operation_ids.len()
            ),
            (3, 0, 1),
            "{task:?}"
        );
        let media = fixture
            .ctx
            .media_files
            .list_media_files_for_title(&fixture.title.id)
            .await
            .unwrap();
        assert_eq!(
            media.len(),
            if policy == ConflictPolicy::AdoptAll {
                4
            } else {
                1
            }
        );
        assert_eq!(
            std::fs::read(fixture.named.join("Emberfall.2024.2160p.BluRay.mkv")).unwrap(),
            b"best movie contents"
        );
        assert_eq!(
            std::fs::read(fixture.named.join("poster.jpg")).unwrap(),
            b"artwork contents"
        );
        assert_eq!(
            std::fs::read(fixture.named.join("Emberfall.2024.2160p.BluRay.en.srt")).unwrap(),
            b"subtitle contents"
        );
        assert_eq!(std::fs::read(&unrelated).unwrap(), b"unrelated movie");
        for id in ids.iter().chain(std::iter::once(&fixture.pending_id)) {
            assert!(
                fixture
                    .ctx
                    .library_scan_unmatched
                    .get_library_scan_unmatched_item(id)
                    .await
                    .unwrap()
                    .is_none()
            );
        }
        if policy == ConflictPolicy::KeepOne {
            let recycled = fixture
                .ctx
                .app
                .list_recycled_items(&fixture.actor, Some(vec![fixture.title.library_id.clone()]))
                .await
                .unwrap();
            assert_eq!(recycled.len(), 3, "inferior media is recycled, not lost");
        }
    }
}

#[tokio::test]
async fn conflict_preview_exposes_counts_but_not_unauthorized_title_identity() {
    let fixture = ConflictFixture::new().await;
    let query = format!(
        r#"mutation {{ previewPendingImportConflicts(input: {{pendingImportIds: ["{}"], policy: ADOPT_ALL, leftovers: PRESERVE_ALL}}) {{ totalCount actionableCount failedCount rows {{ titleId titleName errorReason }} }} }}"#,
        fixture.pending_id
    );
    let response = schema_exec(&fixture.ctx, &query, Some(fixture.actor.clone())).await;
    assert_no_errors(&response);
    let preview = &response["data"]["previewPendingImportConflicts"];
    assert_eq!(preview["totalCount"], 1);
    assert_eq!(preview["actionableCount"], 1);
    assert_eq!(preview["failedCount"], 0);
    assert_eq!(preview["rows"][0]["titleId"], fixture.title.id);

    let mut denied_actor = fixture.actor.clone();
    denied_actor.id = Id::new().0;
    denied_actor.username = "conflict-observer".into();
    denied_actor.authorization.app = scryer_domain::AppPermissionMask::from_permissions([]);
    denied_actor.authorization.libraries.clear();
    denied_actor.authorization.default_library = scryer_domain::LibraryPermissionMask::NONE;
    denied_actor.authorization.loaded = true;
    let denied = schema_exec(&fixture.ctx, &query, Some(denied_actor)).await;
    assert_graphql_field_denied(&denied, "previewPendingImportConflicts");
    assert!(!denied.to_string().contains(&fixture.title.name));
}

#[tokio::test]
async fn conflict_owned_folder_pointing_to_file_is_not_normalized_to_a_broader_folder() {
    let fixture = ConflictFixture::new().await;
    let file = fixture.named.join("Emberfall.2024.720p.WEB-DL.mkv");
    set_title_folder_path(&fixture.ctx, &fixture.title.id, &file).await;
    let preview = fixture
        .ctx
        .app
        .preview_pending_import_conflicts(
            &fixture.actor,
            fixture.request(ConflictPolicy::KeepOne, ConflictLeftovers::PreserveAll),
        )
        .await
        .unwrap();
    assert_eq!(
        preview.rows[0].error_reason.as_deref(),
        Some(refusal::PATH_NOT_DIRECTORY)
    );
    assert_eq!(preview.actionable_count, 0);
    assert_eq!(std::fs::read(file).unwrap(), b"inferior movie");
}

#[tokio::test]
async fn conflict_missing_file_retains_title_and_cannot_be_applied() {
    let fixture = ConflictFixture::new().await;
    let missing = fixture.junk.join("missing.mkv");
    let id = seed_conflict(&fixture.ctx, &fixture.title, &missing, "missing-file").await;
    let request = ConflictRequest {
        pending_import_ids: vec![id],
        policy: ConflictPolicy::KeepOne,
        leftovers: ConflictLeftovers::PreserveAll,
    };
    let preview = fixture
        .ctx
        .app
        .preview_pending_import_conflicts(&fixture.actor, request.clone())
        .await
        .unwrap();
    assert_eq!((preview.actionable_count, preview.failed_count), (0, 1));
    assert_eq!(
        preview.rows[0].error_reason.as_deref(),
        Some(refusal::PATH_MISSING)
    );
    assert_eq!(preview.rows[0].title_id, fixture.title.id);
    assert_eq!(preview.rows[0].title_name, fixture.title.name);
    let error = fixture
        .ctx
        .app
        .resolve_pending_import_conflicts(
            &fixture.actor,
            ResolveConflictRequest {
                request,
                plan_fingerprint: preview.plan_fingerprint,
                confirm_permanent_delete: false,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(error.validation_reason(), Some(refusal::UNSUPPORTED));
    assert_eq!(
        std::fs::read(fixture.junk.join("Emberfall.2024.2160p.BluRay.mkv")).unwrap(),
        b"best movie contents"
    );
}

#[tokio::test]
async fn conflict_complete_counts_include_actionable_rows_beyond_the_sample() {
    let fixture = ConflictFixture::new().await;
    let mut request = fixture.request(ConflictPolicy::AdoptAll, ConflictLeftovers::PreserveAll);
    request
        .pending_import_ids
        .extend((0..100).map(|i| format!("absent-{i:03}")));
    let preview = fixture
        .ctx
        .app
        .preview_pending_import_conflicts(&fixture.actor, request.clone())
        .await
        .unwrap();
    assert_eq!(
        (
            preview.total_count,
            preview.actionable_count,
            preview.failed_count
        ),
        (101, 1, 100)
    );
    assert!(preview.sampled);
    assert_eq!(preview.rows.len(), 100);
    assert!(
        preview
            .rows
            .iter()
            .all(|row| row.error_reason.as_deref() == Some(refusal::NOT_FOUND))
    );
    let task = fixture
        .ctx
        .app
        .resolve_pending_import_conflicts(
            &fixture.actor,
            ResolveConflictRequest {
                request,
                plan_fingerprint: preview.plan_fingerprint,
                confirm_permanent_delete: false,
            },
        )
        .await
        .unwrap();
    let task = fixture.wait_task(&task.job_run_id).await;
    assert_eq!(
        (task.total_count, task.settled_count, task.failed_count),
        (101, 1, 100)
    );
    assert_eq!(task.operation_ids.len(), 1);
}

#[tokio::test]
async fn conflict_file_at_library_root_is_not_a_folder_resolution_target() {
    let fixture = ConflictFixture::new().await;
    let file = fixture.root.join("loose.mkv");
    std::fs::write(&file, b"unrelated contents").unwrap();
    let id = seed_conflict(&fixture.ctx, &fixture.title, &file, "root-file").await;
    let mut request = fixture.request(ConflictPolicy::KeepOne, ConflictLeftovers::PreserveAll);
    request.pending_import_ids = vec![id];
    let preview = fixture
        .ctx
        .app
        .preview_pending_import_conflicts(&fixture.actor, request)
        .await
        .unwrap();
    assert_eq!(
        preview.rows[0].error_reason.as_deref(),
        Some(refusal::FOLDER_UNSAFE)
    );
    assert_eq!(std::fs::read(file).unwrap(), b"unrelated contents");
}

#[cfg(unix)]
#[tokio::test]
async fn conflict_non_regular_import_is_refused_without_removing_it() {
    let fixture = ConflictFixture::new().await;
    let path = fixture.junk.join("socket");
    let _listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
    let id = seed_conflict(&fixture.ctx, &fixture.title, &path, "socket-import").await;
    let mut request = fixture.request(ConflictPolicy::KeepOne, ConflictLeftovers::PreserveAll);
    request.pending_import_ids = vec![id];
    let preview = fixture
        .ctx
        .app
        .preview_pending_import_conflicts(&fixture.actor, request)
        .await
        .unwrap();
    assert_eq!(
        preview.rows[0].error_reason.as_deref(),
        Some(refusal::FOLDER_UNSAFE)
    );
    use std::os::unix::fs::FileTypeExt;
    assert!(
        std::fs::symlink_metadata(path)
            .unwrap()
            .file_type()
            .is_socket()
    );
}

#[tokio::test]
async fn conflict_file_parent_owned_by_another_title_is_preserved() {
    let fixture = ConflictFixture::new().await;
    let other = create_catalog_title(
        &fixture.ctx,
        "Other Movie",
        MediaFacet::Movie,
        vec![],
        vec![],
        true,
    )
    .await;
    set_title_folder_path(&fixture.ctx, &other.id, &fixture.junk).await;
    let file = fixture.junk.join("Emberfall.2024.2160p.BluRay.mkv");
    let id = seed_conflict(&fixture.ctx, &fixture.title, &file, "claimed-file").await;
    let mut request = fixture.request(ConflictPolicy::KeepOne, ConflictLeftovers::PreserveAll);
    request.pending_import_ids = vec![id];
    let preview = fixture
        .ctx
        .app
        .preview_pending_import_conflicts(&fixture.actor, request)
        .await
        .unwrap();
    assert_eq!(
        preview.rows[0].error_reason.as_deref(),
        Some(refusal::OWNERSHIP_CHANGED)
    );
    assert_eq!(std::fs::read(file).unwrap(), b"best movie contents");
}

#[cfg(unix)]
#[tokio::test]
async fn conflict_file_import_refuses_symlink_and_symlink_parent() {
    for link_parent in [false, true] {
        let fixture = ConflictFixture::new().await;
        let best = fixture.junk.join("Emberfall.2024.2160p.BluRay.mkv");
        let link = fixture.root.join("alias");
        std::os::unix::fs::symlink(if link_parent { &fixture.junk } else { &best }, &link).unwrap();
        let source = if link_parent {
            link.join(best.file_name().unwrap())
        } else {
            link.clone()
        };
        let id = seed_conflict(&fixture.ctx, &fixture.title, &source, "linked-file").await;
        let mut request = fixture.request(ConflictPolicy::KeepOne, ConflictLeftovers::PreserveAll);
        request.pending_import_ids = vec![id];
        let preview = fixture
            .ctx
            .app
            .preview_pending_import_conflicts(&fixture.actor, request)
            .await
            .unwrap();
        let reason = preview.rows[0].error_reason.as_deref().unwrap();
        assert_eq!(
            reason,
            if link_parent {
                scryer_application::error_reasons::location::RECOVERY_SYMLINK_FORBIDDEN
            } else {
                refusal::FOLDER_UNSAFE
            }
        );
        assert_eq!(preview.actionable_count, 0);
        assert_eq!(std::fs::read(best).unwrap(), b"best movie contents");
        assert!(
            std::fs::symlink_metadata(link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }
}

#[tokio::test]
async fn conflict_adopt_all_consolidates_three_folders_and_roles_without_losing_leftovers() {
    let fixture = ConflictFixture::new().await;
    let third = fixture.root.join("XYZZ98");
    std::fs::create_dir_all(&third).unwrap();
    std::fs::write(
        third.join("Emberfall.2024.1080p.WEB-DL.mkv"),
        b"additional movie",
    )
    .unwrap();
    std::fs::write(third.join("sample.mkv"), b"sample contents").unwrap();
    let second_id = seed_conflict(&fixture.ctx, &fixture.title, &third, "duplicate-b").await;
    let mut request = fixture.request(ConflictPolicy::AdoptAll, ConflictLeftovers::PreserveAll);
    request.pending_import_ids.push(second_id);
    let preview = fixture
        .ctx
        .app
        .preview_pending_import_conflicts(&fixture.actor, request.clone())
        .await
        .unwrap();
    assert_eq!(preview.total_count, 2);
    assert_eq!(
        preview.rows[0].survivor_path,
        fixture.named.to_string_lossy()
    );
    assert_eq!(
        preview.rows[0].leftover_count, 2,
        "artwork and sample remain after media and its subtitle companion"
    );
    let task = fixture.settle(request, false).await;
    assert_eq!((task.settled_count, task.failed_count), (2, 0), "{task:?}");
    assert_eq!(
        task.operation_ids.len(),
        1,
        "one title, one checkpointed operation"
    );
    let files = fixture
        .ctx
        .media_files
        .list_media_files_for_title(&fixture.title.id)
        .await
        .unwrap();
    assert_eq!(files.len(), 3);
    assert_eq!(
        files.iter().filter(|file| file.role.is_primary()).count(),
        1
    );
    assert!(
        files
            .iter()
            .find(|file| file.role.is_primary())
            .unwrap()
            .file_path
            .contains("2160p")
    );
    let collections = fixture
        .ctx
        .shows
        .list_collections_for_title(&fixture.title.id)
        .await
        .unwrap();
    assert_eq!(collections.len(), 2);
    let primary_path = &files
        .iter()
        .find(|file| file.role.is_primary())
        .unwrap()
        .file_path;
    assert!(
        collections
            .iter()
            .any(|collection| collection.ordered_path.as_ref() == Some(primary_path))
    );
    assert!(collections.iter().any(|collection| {
        collection
            .ordered_path
            .as_deref()
            .is_some_and(|path| path.contains("720p"))
    }));
    assert!(
        files
            .iter()
            .all(|file| std::path::Path::new(&file.file_path).starts_with(&fixture.named))
    );
    assert_eq!(
        std::fs::read(fixture.named.join("poster.jpg")).unwrap(),
        b"artwork contents"
    );
    assert_eq!(
        std::fs::read(fixture.named.join("sample.mkv")).unwrap(),
        b"sample contents"
    );
    assert_eq!(
        std::fs::read(fixture.named.join("Emberfall.2024.2160p.BluRay.en.srt")).unwrap(),
        b"subtitle contents"
    );
    assert!(!fixture.junk.exists());
    assert!(!third.exists());
}

#[tokio::test]
async fn conflict_keep_one_moves_only_best_movie_and_requires_explicit_permanent_consent() {
    let fixture = ConflictFixture::new().await;
    fixture
        .ctx
        .app
        .update_recycle_bin_settings(
            &fixture.actor,
            UpdateRecycleBinSettings {
                enabled: Some(false),
                path: None,
                retention_days: None,
            },
        )
        .await
        .unwrap();
    let unrelated = fixture.root.join("unrelated.txt");
    std::fs::write(&unrelated, b"unrelated contents").unwrap();
    let request = fixture.request(ConflictPolicy::KeepOne, ConflictLeftovers::MediaOnly);
    let preview = fixture
        .ctx
        .app
        .preview_pending_import_conflicts(&fixture.actor, request.clone())
        .await
        .unwrap();
    assert!(preview.requires_permanent_delete_confirmation);
    assert_eq!(
        preview.rows[0].move_bytes, 36,
        "only the best media and its subtitle companion move"
    );
    assert_eq!(preview.permanent_delete_count, 2);
    let error = fixture
        .ctx
        .app
        .resolve_pending_import_conflicts(
            &fixture.actor,
            ResolveConflictRequest {
                request: request.clone(),
                plan_fingerprint: preview.plan_fingerprint,
                confirm_permanent_delete: false,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(
        error.validation_reason(),
        Some(refusal::PERMANENT_CONFIRMATION_REQUIRED)
    );
    assert!(
        fixture
            .named
            .join("Emberfall.2024.720p.WEB-DL.mkv")
            .exists()
    );
    assert!(fixture.junk.join("poster.jpg").exists());
    let task = fixture.settle(request, true).await;
    assert_eq!((task.settled_count, task.failed_count), (1, 0), "{task:?}");
    let files = fixture
        .ctx
        .media_files
        .list_media_files_for_title(&fixture.title.id)
        .await
        .unwrap();
    assert_eq!(files.len(), 1);
    assert!(files[0].role.is_primary());
    assert!(files[0].file_path.contains("2160p"));
    assert_eq!(
        std::fs::read(&files[0].file_path).unwrap(),
        b"best movie contents"
    );
    assert_eq!(std::fs::read(unrelated).unwrap(), b"unrelated contents");
    assert!(!fixture.junk.exists());
    assert!(
        !fixture
            .named
            .join("Emberfall.2024.720p.WEB-DL.mkv")
            .exists()
    );
}

#[tokio::test]
async fn conflict_confirmation_refuses_changed_files_and_preserves_all_sources() {
    let fixture = ConflictFixture::new().await;
    let request = fixture.request(ConflictPolicy::KeepOne, ConflictLeftovers::MediaOnly);
    let preview = fixture
        .ctx
        .app
        .preview_pending_import_conflicts(&fixture.actor, request.clone())
        .await
        .unwrap();
    std::fs::write(
        fixture.junk.join("new-unpreviewed.txt"),
        b"newly arrived contents",
    )
    .unwrap();
    let error = fixture
        .ctx
        .app
        .resolve_pending_import_conflicts(
            &fixture.actor,
            ResolveConflictRequest {
                request,
                plan_fingerprint: preview.plan_fingerprint,
                confirm_permanent_delete: true,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(error.validation_reason(), Some(refusal::PLAN_CHANGED));
    assert_eq!(
        std::fs::read(fixture.junk.join("new-unpreviewed.txt")).unwrap(),
        b"newly arrived contents"
    );
    assert!(
        fixture
            .named
            .join("Emberfall.2024.720p.WEB-DL.mkv")
            .exists()
    );
}

#[tokio::test]
async fn conflict_keep_one_recycles_media_and_restores_its_catalog_provenance() {
    let fixture = ConflictFixture::new().await;
    let original = fixture
        .ctx
        .media_files
        .list_media_files_for_title(&fixture.title.id)
        .await
        .unwrap()
        .remove(0);
    let snapshot = scryer_application::recycle_bin::RecycledMediaRowSnapshot {
        acquisition_score: Some(42),
        scene_name: Some("Emberfall.2024.720p.WEB-DL-GROUP".into()),
        release_group: Some("GROUP".into()),
        indexer_source: Some("synthetic-indexer".into()),
        ..Default::default()
    };
    fixture
        .ctx
        .media_files
        .restore_media_file_acquisition_metadata(&original.id, &snapshot)
        .await
        .unwrap();
    let original = fixture
        .ctx
        .media_files
        .get_media_file_by_id(&original.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(original.acquisition_score, Some(42));
    assert_eq!(original.scene_name, snapshot.scene_name);
    fixture
        .ctx
        .app
        .update_recycle_bin_settings(
            &fixture.actor,
            UpdateRecycleBinSettings {
                enabled: Some(true),
                path: None,
                retention_days: None,
            },
        )
        .await
        .unwrap();
    let task = fixture
        .settle(
            fixture.request(ConflictPolicy::KeepOne, ConflictLeftovers::PreserveAll),
            false,
        )
        .await;
    assert_eq!((task.settled_count, task.failed_count), (1, 0), "{task:?}");
    let entries = fixture
        .ctx
        .app
        .list_recycled_items(&fixture.actor, Some(vec![fixture.title.library_id.clone()]))
        .await
        .unwrap();
    let entry = entries
        .iter()
        .find(|entry| entry.reason == "folder_conflict_resolution")
        .expect("inferior movie in recycle bin");
    assert!(entry.original_path.ends_with("720p.WEB-DL.mkv"));
    let config = fixture
        .ctx
        .app
        .recycle_bin_config_for_media_root(Some(fixture.root.to_str().unwrap()))
        .await;
    let committed = scryer_application::recycle_bin::list_committed_entries(&config)
        .await
        .unwrap();
    let manifest = &committed
        .iter()
        .find(|item| item.manifest.original_path == entry.original_path)
        .unwrap()
        .manifest;
    assert_eq!(
        manifest.media_row.as_ref(),
        Some(
            &scryer_application::recycle_bin::RecycledMediaRowSnapshot::from_media_file(&original)
        )
    );
    assert!(
        fixture
            .ctx
            .app
            .restore_recycled_item(&fixture.actor, &entry.id)
            .await
            .unwrap()
    );
    let files = fixture
        .ctx
        .media_files
        .list_media_files_for_title(&fixture.title.id)
        .await
        .unwrap();
    let restored = files
        .iter()
        .find(|file| file.file_path.ends_with("720p.WEB-DL.mkv"))
        .expect("restored catalog row");
    assert_eq!(restored.source_type.as_deref(), Some("WEB-DL"));
    assert_eq!(restored.acquisition_score, original.acquisition_score);
    assert_eq!(
        restored.scene_name.as_deref(),
        Some("Emberfall.2024.720p.WEB-DL")
    );
    assert_eq!(restored.release_group, original.release_group);
    assert_eq!(restored.indexer_source, original.indexer_source);
    assert_eq!(
        std::fs::read(&restored.file_path).unwrap(),
        b"inferior movie"
    );
    assert_eq!(
        std::fs::read(fixture.named.join("poster.jpg")).unwrap(),
        b"artwork contents"
    );
}

#[tokio::test]
async fn conflict_reconciliation_preserves_each_retained_movie_collection() {
    for policy in [ConflictPolicy::AdoptAll, ConflictPolicy::KeepOne] {
        let fixture = ConflictFixture::new().await;
        let old = fixture
            .ctx
            .shows
            .list_collections_for_title(&fixture.title.id)
            .await
            .unwrap()
            .remove(0);
        let best_path = fixture
            .junk
            .join("Emberfall.2024.2160p.BluRay.mkv")
            .to_string_lossy()
            .into_owned();
        fixture
            .ctx
            .media_files
            .insert_media_file(&InsertMediaFileInput {
                title_id: fixture.title.id.clone(),
                file_path: best_path.clone(),
                size_bytes: 19,
                role: MediaFileRole::Additional,
                quality_label: Some("2160p".into()),
                resolution: Some("2160p".into()),
                source_type: Some("BluRay".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        let mut best_collection = old.clone();
        best_collection.id = Id::new().0;
        best_collection.collection_index = "2".into();
        best_collection.ordered_path = Some(best_path);
        best_collection.label = Some("2160p".into());
        fixture
            .ctx
            .shows
            .create_collection(best_collection.clone())
            .await
            .unwrap();
        let task = fixture
            .settle(
                fixture.request(policy, ConflictLeftovers::PreserveAll),
                false,
            )
            .await;
        assert_eq!((task.settled_count, task.failed_count), (1, 0), "{task:?}");
        let collections = fixture
            .ctx
            .shows
            .list_collections_for_title(&fixture.title.id)
            .await
            .unwrap();
        let files = fixture
            .ctx
            .media_files
            .list_media_files_for_title(&fixture.title.id)
            .await
            .unwrap();
        let primary = files.iter().find(|file| file.role.is_primary()).unwrap();
        let mapped_best = collections
            .iter()
            .find(|collection| collection.id == best_collection.id)
            .unwrap();
        assert_eq!(mapped_best.ordered_path.as_ref(), Some(&primary.file_path));
        assert!(
            primary
                .file_path
                .starts_with(fixture.named.to_str().unwrap())
        );
        if policy == ConflictPolicy::AdoptAll {
            assert_eq!(collections.len(), 2);
            let retained_old = collections
                .iter()
                .find(|collection| collection.id == old.id)
                .unwrap();
            assert_eq!(retained_old.ordered_path, old.ordered_path);
            assert_ne!(retained_old.ordered_path, mapped_best.ordered_path);
        } else {
            assert_eq!(collections.len(), 1);
            assert!(!collections.iter().any(|collection| collection.id == old.id));
        }
        for collection in collections {
            let path = collection.ordered_path.unwrap();
            assert!(files.iter().any(|file| file.file_path == path));
            assert!(std::path::Path::new(&path).is_file());
        }
    }
}

#[tokio::test]
async fn conflict_bulk_continues_after_an_invalid_item() {
    let fixture = ConflictFixture::new().await;
    let mut request = fixture.request(ConflictPolicy::AdoptAll, ConflictLeftovers::PreserveAll);
    request.pending_import_ids.push("missing-conflict".into());
    let preview = fixture
        .ctx
        .app
        .preview_pending_import_conflicts(&fixture.actor, request.clone())
        .await
        .unwrap();
    assert_eq!(preview.total_count, 2);
    assert!(
        preview
            .rows
            .iter()
            .any(|row| row.error_reason.as_deref() == Some(refusal::NOT_FOUND))
    );
    let task = fixture
        .ctx
        .app
        .resolve_pending_import_conflicts(
            &fixture.actor,
            ResolveConflictRequest {
                request,
                plan_fingerprint: preview.plan_fingerprint,
                confirm_permanent_delete: false,
            },
        )
        .await
        .unwrap();
    let task = fixture.wait_task(&task.job_run_id).await;
    assert_eq!((task.settled_count, task.failed_count), (1, 1), "{task:?}");
    assert_eq!(task.operation_ids.len(), 1);
}

#[cfg(unix)]
#[tokio::test]
async fn conflict_disposal_hardlinks_preserve_external_aliases_and_recover_owned_ctime() {
    use scryer_application::LocationOperationRepository;
    use scryer_application::location::resolution::ResolutionDisposition;
    use scryer_infrastructure_library::media::libraries::location_operation_store::LocationOperationStore;
    use std::os::unix::fs::MetadataExt;
    for enabled in [false, true] {
        for shared_survivor in [false, true] {
            let fixture = ConflictFixture::new().await;
            fixture
                .ctx
                .app
                .update_recycle_bin_settings(
                    &fixture.actor,
                    UpdateRecycleBinSettings {
                        enabled: Some(enabled),
                        path: None,
                        retention_days: None,
                    },
                )
                .await
                .unwrap();
            let inferior = fixture.named.join("Emberfall.2024.720p.WEB-DL.mkv");
            let alias = fixture.junk.join("Emberfall.2024.480p.WEB-DL.mkv");
            let external = fixture.root.join("unrelated-hardlink.mkv");
            std::fs::hard_link(&inferior, &alias).unwrap();
            std::fs::hard_link(&inferior, &external).unwrap();
            if shared_survivor {
                let best = fixture.junk.join("Emberfall.2024.2160p.BluRay.mkv");
                std::fs::remove_file(&best).unwrap();
                std::fs::hard_link(&inferior, best).unwrap();
            }
            let before = std::fs::metadata(&external).unwrap();
            let task = fixture
                .settle(
                    fixture.request(ConflictPolicy::KeepOne, ConflictLeftovers::PreserveAll),
                    true,
                )
                .await;
            let diagnostics = if task.failed_count > 0 {
                let store = LocationOperationStore::new(fixture.ctx.db.datastore());
                let mut diagnostics = Vec::new();
                for id in &task.operation_ids {
                    diagnostics.push(format!(
                        "operation={:?}; checkpoints={:?}; resolutions={:?}",
                        store.get_location_operation(id).await.unwrap(),
                        store.list_location_title_checkpoints(id).await.unwrap(),
                        store.file_resolutions(id, &fixture.title.id).await.unwrap()
                    ));
                }
                diagnostics
            } else {
                Vec::new()
            };
            assert_eq!(
                (task.settled_count, task.failed_count),
                (1, 0),
                "enabled={enabled}, shared={shared_survivor}: {task:?}; {diagnostics:?}"
            );
            assert!(!inferior.exists());
            assert!(!alias.exists());
            assert_eq!(std::fs::read(&external).unwrap(), b"inferior movie");
            assert_eq!(
                std::fs::metadata(&external).unwrap().modified().unwrap(),
                before.modified().unwrap()
            );
            let expected_links = if enabled {
                if shared_survivor { 4 } else { 3 }
            } else if shared_survivor {
                2
            } else {
                1
            };
            assert_eq!(
                std::fs::metadata(&external).unwrap().nlink(),
                expected_links
            );
            let catalog = fixture
                .ctx
                .media_files
                .list_media_files_for_title(&fixture.title.id)
                .await
                .unwrap();
            let primary = catalog.iter().find(|file| file.role.is_primary()).unwrap();
            assert_eq!(
                std::fs::read(&primary.file_path).unwrap(),
                if shared_survivor {
                    b"inferior movie".as_slice()
                } else {
                    b"best movie contents".as_slice()
                }
            );
            let store = LocationOperationStore::new(fixture.ctx.db.datastore());
            let mut rows = store
                .file_resolutions(&task.operation_ids[0], &fixture.title.id)
                .await
                .unwrap();
            assert_eq!(rows.iter().filter(|row| row.is_disposal()).count(), 2);
            let latest = rows
                .iter_mut()
                .filter(|row| row.is_disposal())
                .max_by_key(|row| {
                    let ResolutionDisposition::Disposal(proof) = &row.disposition else {
                        unreachable!()
                    };
                    proof.sequence
                })
                .unwrap();
            let ResolutionDisposition::Disposal(proof) = &mut latest.disposition else {
                unreachable!()
            };
            // Recreate the durable intent immediately after its pathname effect,
            // before either the bin manifest or resolution completion was saved.
            proof.after_versions.clear();
            proof.after_link_count = None;
            latest.completed = false;
            if let Some(recycle) = &proof.recycle {
                let config = fixture
                    .ctx
                    .app
                    .recycle_bin_config_for_media_root(Some(fixture.root.to_str().unwrap()))
                    .await;
                let path = config
                    .base_path
                    .join(&recycle.entry_id)
                    .join("manifest.json");
                let mut manifest: Value =
                    serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
                manifest["status"] =
                    json!(scryer_application::recycle_bin::RECYCLE_STATUS_CONFLICT_PENDING);
                std::fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
            }
            store.save_file_resolution(latest).await.unwrap();
            retry_cleanup(&fixture, &task.operation_ids[0]).await;
            retry_cleanup(&fixture, &task.operation_ids[0]).await;
            assert_eq!(
                store
                    .get_location_operation(&task.operation_ids[0])
                    .await
                    .unwrap()
                    .unwrap()
                    .counters
                    .renames,
                0,
                "recycled entries never count as collision renames"
            );
            assert_eq!(std::fs::read(&external).unwrap(), b"inferior movie");
            if enabled {
                let entries = fixture
                    .ctx
                    .app
                    .list_recycled_items(
                        &fixture.actor,
                        Some(vec![fixture.title.library_id.clone()]),
                    )
                    .await
                    .unwrap();
                assert_eq!(
                    entries
                        .iter()
                        .filter(|entry| entry.reason == "folder_conflict_resolution")
                        .count(),
                    2
                );
                let entry = entries
                    .iter()
                    .find(|entry| entry.original_path == inferior.to_string_lossy())
                    .unwrap();
                fixture
                    .ctx
                    .app
                    .restore_recycled_item(&fixture.actor, &entry.id)
                    .await
                    .unwrap();
                assert_eq!(std::fs::read(&inferior).unwrap(), b"inferior movie");
                assert_eq!(std::fs::read(&external).unwrap(), b"inferior movie");
            }
        }
    }
}

#[cfg(unix)]
#[tokio::test]
async fn conflict_preview_refuses_links_without_following_or_moving_them() {
    let fixture = ConflictFixture::new().await;
    let unrelated = fixture.root.join("unrelated.txt");
    std::fs::write(&unrelated, b"unrelated contents").unwrap();
    let link = fixture.junk.join("linked.txt");
    std::os::unix::fs::symlink(&unrelated, &link).unwrap();
    let preview = fixture
        .ctx
        .app
        .preview_pending_import_conflicts(
            &fixture.actor,
            fixture.request(ConflictPolicy::AdoptAll, ConflictLeftovers::PreserveAll),
        )
        .await
        .unwrap();
    assert!(preview.rows[0].error_reason.is_some());
    assert_eq!(std::fs::read(&unrelated).unwrap(), b"unrelated contents");
    assert!(
        std::fs::symlink_metadata(link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn conflict_native_paths_preserve_distinct_names_collisions_companions_and_resume() {
    use scryer_application::stored_paths::{path_to_stored_string, stored_path_to_path_buf};
    use std::{ffi::OsString, os::unix::ffi::OsStringExt, path::PathBuf};
    let mut fixture = ConflictFixture::new().await;
    let folder = fixture
        .root
        .join(OsString::from_vec(b"ADEG78\xff".to_vec()));
    std::fs::rename(&fixture.junk, &folder).unwrap();
    fixture
        .ctx
        .library_scan_unmatched
        .delete_library_scan_unmatched_item(
            &fixture.title.library_id,
            MediaFacet::Movie,
            &path_to_stored_string(&fixture.junk),
        )
        .await
        .unwrap();
    fixture.junk = folder;
    seed_conflict(
        &fixture.ctx,
        &fixture.title,
        &fixture.junk,
        &fixture.pending_id,
    )
    .await;
    let first = PathBuf::from(OsString::from_vec(b"movie\xff.mkv".to_vec()));
    let second = PathBuf::from(OsString::from_vec(b"movie\xfe.mkv".to_vec()));
    assert_eq!(first.to_string_lossy(), second.to_string_lossy());
    let occupied = PathBuf::from(OsString::from_vec(
        b"movie\xff (from duplicate).mkv".to_vec(),
    ));
    let renamed = PathBuf::from(OsString::from_vec(
        b"movie\xff (from duplicate) (2).mkv".to_vec(),
    ));
    let subtitle = PathBuf::from(OsString::from_vec(b"movie\xff.en\xfd.srt".to_vec()));
    let renamed_subtitle = PathBuf::from(OsString::from_vec(
        b"movie\xff (from duplicate) (2).en\xfd.srt".to_vec(),
    ));
    let frame = PathBuf::from(OsString::from_vec(
        b"movie\xff.trickplay/frame\xfc.jpg".to_vec(),
    ));
    let renamed_frame = PathBuf::from(OsString::from_vec(
        b"movie\xff (from duplicate) (2).trickplay/frame\xfc.jpg".to_vec(),
    ));
    let literal = "scryer-path-v1:u:poster.jpg";
    std::fs::write(fixture.named.join(&first), b"destination movie").unwrap();
    std::fs::write(fixture.named.join(&occupied), b"occupied suffix").unwrap();
    for (relative, contents) in [
        (
            first.as_path(),
            b"incoming movie with different bytes".as_slice(),
        ),
        (second.as_path(), b"distinct native name".as_slice()),
        (
            std::path::Path::new("movie\u{fffd}.mkv"),
            b"literal replacement character".as_slice(),
        ),
        (subtitle.as_path(), b"native subtitle".as_slice()),
        (frame.as_path(), b"native frame".as_slice()),
        (
            std::path::Path::new(literal),
            b"literal codec prefix".as_slice(),
        ),
    ] {
        let path = fixture.junk.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }
    let request = fixture.request(ConflictPolicy::AdoptAll, ConflictLeftovers::PreserveAll);
    let preview = fixture
        .ctx
        .app
        .preview_pending_import_conflicts(&fixture.actor, request.clone())
        .await
        .unwrap();
    assert!(preview.rows[0].error_reason.is_none(), "{preview:?}");
    assert_eq!(preview.rows[0].collision_renames, 3, "{preview:?}");
    assert!(
        preview.rows[0]
            .collision_rename_samples
            .iter()
            .any(|rename| {
                stored_path_to_path_buf(&rename.source_path) == fixture.junk.join(&frame)
                    && stored_path_to_path_buf(&rename.destination_path)
                        == fixture.named.join(&renamed_frame)
            }),
        "the frame must follow its renamed trickplay directory: {preview:?}"
    );
    let task = fixture.settle(request, false).await;
    assert_eq!((task.settled_count, task.failed_count), (1, 0), "{task:?}");
    for (relative, contents) in [
        (first.as_path(), b"destination movie".as_slice()),
        (occupied.as_path(), b"occupied suffix".as_slice()),
        (
            renamed.as_path(),
            b"incoming movie with different bytes".as_slice(),
        ),
        (second.as_path(), b"distinct native name".as_slice()),
        (
            std::path::Path::new("movie\u{fffd}.mkv"),
            b"literal replacement character".as_slice(),
        ),
        (renamed_subtitle.as_path(), b"native subtitle".as_slice()),
        (renamed_frame.as_path(), b"native frame".as_slice()),
        (
            std::path::Path::new(literal),
            b"literal codec prefix".as_slice(),
        ),
    ] {
        assert_eq!(
            std::fs::read(fixture.named.join(relative)).unwrap(),
            contents
        );
    }
    let catalog = fixture
        .ctx
        .media_files
        .list_media_files_for_title(&fixture.title.id)
        .await
        .unwrap();
    for relative in [&first, &occupied, &renamed, &second] {
        let expected = fixture.named.join(relative);
        assert!(
            catalog
                .iter()
                .any(|row| stored_path_to_path_buf(&row.file_path) == expected),
            "missing {}",
            expected.display()
        );
    }
    assert_eq!(
        catalog.iter().filter(|row| row.role.is_primary()).count(),
        1
    );
    retry_cleanup(&fixture, &task.operation_ids[0]).await;
    assert_eq!(
        std::fs::read(fixture.named.join(&renamed)).unwrap(),
        b"incoming movie with different bytes"
    );
    assert_eq!(
        fixture
            .ctx
            .media_files
            .list_media_files_for_title(&fixture.title.id)
            .await
            .unwrap()
            .len(),
        catalog.len()
    );
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn conflict_native_paths_recycle_and_restore_exact_catalog_paths_after_resume() {
    use scryer_application::stored_paths::{path_to_stored_string, stored_path_to_path_buf};
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let fixture = ConflictFixture::new().await;
    let original = fixture
        .ctx
        .media_files
        .list_media_files_for_title(&fixture.title.id)
        .await
        .unwrap()
        .remove(0);
    let native = fixture.named.join(OsString::from_vec(
        b"Emberfall.2024.720p.WEB-DL\xff.mkv".to_vec(),
    ));
    std::fs::rename(stored_path_to_path_buf(&original.file_path), &native).unwrap();
    fixture
        .ctx
        .media_files
        .update_media_file_path(&original.id, &path_to_stored_string(&native))
        .await
        .unwrap();
    fixture
        .ctx
        .app
        .update_recycle_bin_settings(
            &fixture.actor,
            UpdateRecycleBinSettings {
                enabled: Some(true),
                path: None,
                retention_days: None,
            },
        )
        .await
        .unwrap();
    let task = fixture
        .settle(
            fixture.request(ConflictPolicy::KeepOne, ConflictLeftovers::PreserveAll),
            false,
        )
        .await;
    assert_eq!((task.settled_count, task.failed_count), (1, 0), "{task:?}");
    assert!(!native.exists());
    retry_cleanup(&fixture, &task.operation_ids[0]).await;
    let config = fixture
        .ctx
        .app
        .recycle_bin_config_for_media_root(Some(&path_to_stored_string(&fixture.root)))
        .await;
    let entries = scryer_application::recycle_bin::list_committed_entries(&config)
        .await
        .unwrap();
    let recycled = entries
        .iter()
        .find(|entry| stored_path_to_path_buf(&entry.manifest.original_path) == native)
        .expect("native path persisted in recycle manifest");
    assert_eq!(
        recycled.manifest.original_file_id.as_deref(),
        Some(original.id.as_str())
    );
    assert!(
        fixture
            .ctx
            .app
            .restore_recycled_item(
                &fixture.actor,
                recycled.manifest.entry_id.as_deref().unwrap()
            )
            .await
            .unwrap()
    );
    assert_eq!(std::fs::read(&native).unwrap(), b"inferior movie");
    let catalog = fixture
        .ctx
        .media_files
        .list_media_files_for_title(&fixture.title.id)
        .await
        .unwrap();
    assert!(
        catalog
            .iter()
            .any(|row| stored_path_to_path_buf(&row.file_path) == native)
    );
    assert_eq!(
        std::fs::read(fixture.named.join("Emberfall.2024.2160p.BluRay.mkv")).unwrap(),
        b"best movie contents"
    );
}

#[tokio::test]
async fn conflict_keep_one_identical_collision_always_retains_the_resolved_primary() {
    for enabled in [false, true] {
        let fixture = ConflictFixture::new().await;
        fixture
            .ctx
            .app
            .update_recycle_bin_settings(
                &fixture.actor,
                UpdateRecycleBinSettings {
                    enabled: Some(enabled),
                    path: None,
                    retention_days: None,
                },
            )
            .await
            .unwrap();
        for file in fixture
            .ctx
            .media_files
            .list_media_files_for_title(&fixture.title.id)
            .await
            .unwrap()
        {
            fixture
                .ctx
                .media_files
                .delete_media_file(&file.id)
                .await
                .unwrap();
        }
        std::fs::remove_file(fixture.named.join("Emberfall.2024.720p.WEB-DL.mkv")).unwrap();
        std::fs::remove_file(fixture.junk.join("Emberfall.2024.2160p.BluRay.mkv")).unwrap();
        let retained = fixture.named.join("movie.mkv");
        std::fs::write(&retained, b"identical movie contents").unwrap();
        std::fs::write(fixture.junk.join("movie.mkv"), b"identical movie contents").unwrap();
        fixture
            .ctx
            .media_files
            .insert_media_file(&InsertMediaFileInput {
                title_id: fixture.title.id.clone(),
                file_path: retained.to_string_lossy().into_owned(),
                size_bytes: 24,
                role: MediaFileRole::Primary,
                ..Default::default()
            })
            .await
            .unwrap();
        let request = fixture.request(ConflictPolicy::KeepOne, ConflictLeftovers::PreserveAll);
        let preview = fixture
            .ctx
            .app
            .preview_pending_import_conflicts(&fixture.actor, request.clone())
            .await
            .unwrap();
        assert!(
            preview.rows[0]
                .recycle_paths
                .contains(&retained.to_string_lossy().into_owned()),
            "the preview originally designates the destination inferior"
        );
        let task = fixture.settle(request, true).await;
        assert_eq!(
            (task.settled_count, task.failed_count),
            (1, 0),
            "bin enabled={enabled}: {task:?}"
        );
        let files = fixture
            .ctx
            .media_files
            .list_media_files_for_title(&fixture.title.id)
            .await
            .unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].role.is_primary());
        assert_eq!(files[0].file_path, retained.to_string_lossy());
        assert_eq!(
            std::fs::read(&retained).unwrap(),
            b"identical movie contents"
        );
        assert!(!fixture.junk.join("movie.mkv").exists());
        retry_cleanup(&fixture, &task.operation_ids[0]).await;
        assert_eq!(
            std::fs::read(&retained).unwrap(),
            b"identical movie contents"
        );
        assert_eq!(
            fixture
                .ctx
                .media_files
                .list_media_files_for_title(&fixture.title.id)
                .await
                .unwrap()
                .len(),
            1
        );
    }
}

async fn retry_cleanup(fixture: &ConflictFixture, operation_id: &str) {
    use scryer_application::location::model::LocationOperationState;
    let state = retry_cleanup_state(fixture, operation_id).await;
    assert!(
        matches!(
            state,
            LocationOperationState::Completed | LocationOperationState::CompletedWithWarnings
        ),
        "{state:?}"
    );
}

async fn retry_cleanup_state(
    fixture: &ConflictFixture,
    operation_id: &str,
) -> scryer_application::location::model::LocationOperationState {
    use scryer_application::LocationOperationRepository;
    use scryer_application::location::model::{
        LocationOperationState, LocationReasonCode, TitleCheckpointState,
    };
    use scryer_application::location::operations::LocationResumeDecision;
    use scryer_infrastructure_library::media::libraries::location_operation_store::LocationOperationStore;
    let store = LocationOperationStore::new(fixture.ctx.db.datastore());
    let mut checkpoint = store
        .list_location_title_checkpoints(operation_id)
        .await
        .unwrap()
        .remove(0);
    // Recreate the exact crash window: cleanup committed, its checkpoint did not.
    checkpoint.state = TitleCheckpointState::CleaningUp;
    checkpoint.completed_at = None;
    store
        .upsert_location_title_checkpoint(&checkpoint)
        .await
        .unwrap();
    let operation = store
        .get_location_operation(operation_id)
        .await
        .unwrap()
        .unwrap();
    store
        .update_location_operation_progress(&scryer_application::LocationOperationProgress {
            operation_id: operation_id.into(),
            state: LocationOperationState::Failed,
            counters: operation.counters,
            verification_fallback_count: operation.verification_fallback_count,
            detail: Some("synthetic interruption after source cleanup".into()),
            clear_detail: false,
            reason_code: Some(LocationReasonCode::SourceCleanupFailed),
            started_at: operation.started_at,
            completed_at: Some(Utc::now()),
        })
        .await
        .unwrap();
    let decision = fixture
        .ctx
        .app
        .resume_location_operation(operation_id)
        .await
        .unwrap();
    let LocationResumeDecision::Resume(plan) = decision else {
        panic!("cleanup must remain resumable: {decision:?}");
    };
    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(120),
        fixture.ctx.app.run_root_move(operation_id, &plan),
    )
    .await
    .unwrap()
    .unwrap();
    outcome.state
}

#[cfg(unix)]
#[tokio::test]
async fn conflict_adopt_hardlink_dedup_resumes_after_its_own_cleanup_changed_ctime() {
    let fixture = ConflictFixture::new().await;
    let primary = fixture.named.join("Emberfall.2024.720p.WEB-DL.mkv");
    std::fs::remove_file(fixture.junk.join("Emberfall.2024.2160p.BluRay.mkv")).unwrap();
    std::fs::hard_link(
        &primary,
        fixture.junk.join("Emberfall.2024.720p.WEB-DL.mkv"),
    )
    .unwrap();
    let task = fixture
        .settle(
            fixture.request(ConflictPolicy::AdoptAll, ConflictLeftovers::PreserveAll),
            false,
        )
        .await;
    assert_eq!((task.settled_count, task.failed_count), (1, 0), "{task:?}");
    retry_cleanup(&fixture, &task.operation_ids[0]).await;
    assert_eq!(std::fs::read(&primary).unwrap(), b"inferior movie");
    assert_eq!(
        fixture
            .ctx
            .media_files
            .list_media_files_for_title(&fixture.title.id)
            .await
            .unwrap()
            .iter()
            .filter(|file| file.role.is_primary())
            .count(),
        1
    );
}

#[tokio::test]
async fn conflict_survivor_never_uses_a_folder_identifying_a_different_work() {
    for name in [
        "Different.Feature.2160p.BluRay",
        "Different Feature (2024)",
        "Different Feature {imdb-tt9999999}",
    ] {
        let fixture = ConflictFixture::new().await;
        let owner = fixture.root.join("ZZZ000");
        std::fs::rename(&fixture.named, &owner).unwrap();
        set_title_folder_path(&fixture.ctx, &fixture.title.id, &owner).await;
        for media in fixture
            .ctx
            .media_files
            .list_media_files_for_title(&fixture.title.id)
            .await
            .unwrap()
        {
            fixture
                .ctx
                .media_files
                .update_media_file_path(
                    &media.id,
                    owner
                        .join("Emberfall.2024.720p.WEB-DL.mkv")
                        .to_string_lossy()
                        .as_ref(),
                )
                .await
                .unwrap();
        }
        let different = fixture.root.join(name);
        std::fs::rename(&fixture.junk, &different).unwrap();
        let mut item = fixture
            .ctx
            .library_scan_unmatched
            .get_library_scan_unmatched_item(&fixture.pending_id)
            .await
            .unwrap()
            .unwrap();
        fixture
            .ctx
            .library_scan_unmatched
            .delete_library_scan_unmatched_item(
                &item.library_id,
                item.facet.clone(),
                &item.item_path,
            )
            .await
            .unwrap();
        item.item_path = different.to_string_lossy().into_owned();
        fixture
            .ctx
            .library_scan_unmatched
            .upsert_library_scan_unmatched_item(&item)
            .await
            .unwrap();
        let preview = fixture
            .ctx
            .app
            .preview_pending_import_conflicts(
                &fixture.actor,
                fixture.request(ConflictPolicy::AdoptAll, ConflictLeftovers::PreserveAll),
            )
            .await
            .unwrap();
        assert!(preview.rows[0].error_reason.is_none(), "{preview:?}");
        assert_eq!(
            preview.rows[0].survivor_path,
            owner.to_string_lossy(),
            "{name}"
        );
        assert!(different.join("Emberfall.2024.2160p.BluRay.mkv").exists());
    }
}

#[tokio::test]
async fn conflict_bucket_holding_another_work_is_refused_for_every_policy() {
    let fixture = ConflictFixture::new().await;
    let other = fixture.junk.join("Other.Picture.2011.1080p.BluRay.mkv");
    std::fs::write(&other, b"another film").unwrap();
    for policy in [ConflictPolicy::KeepOne, ConflictPolicy::AdoptAll] {
        let preview = fixture
            .ctx
            .app
            .preview_pending_import_conflicts(
                &fixture.actor,
                fixture.request(policy, ConflictLeftovers::MediaOnly),
            )
            .await
            .unwrap();
        assert_eq!(
            preview.rows[0].error_reason.as_deref(),
            Some(refusal::FOLDER_HOLDS_OTHER_WORKS),
            "{preview:?}"
        );
        assert_eq!(preview.actionable_count, 0);
    }
    assert_eq!(std::fs::read(&other).unwrap(), b"another film");
    assert!(
        fixture
            .junk
            .join("Emberfall.2024.2160p.BluRay.mkv")
            .exists()
    );
}

#[tokio::test]
async fn conflict_keep_one_refuses_a_stacked_multi_part_title() {
    let fixture = ConflictFixture::new().await;
    std::fs::rename(
        fixture.junk.join("Emberfall.2024.2160p.BluRay.mkv"),
        fixture.junk.join("Emberfall.2024.2160p.BluRay.cd1.mkv"),
    )
    .unwrap();
    std::fs::write(
        fixture.junk.join("Emberfall.2024.2160p.BluRay.cd2.mkv"),
        b"second part",
    )
    .unwrap();
    let preview = fixture
        .ctx
        .app
        .preview_pending_import_conflicts(
            &fixture.actor,
            fixture.request(ConflictPolicy::KeepOne, ConflictLeftovers::MediaOnly),
        )
        .await
        .unwrap();
    assert_eq!(
        preview.rows[0].error_reason.as_deref(),
        Some(refusal::MULTI_PART_MEDIA),
        "{preview:?}"
    );
    // Adopting every part keeps them all, so it stays available.
    let preview = fixture
        .ctx
        .app
        .preview_pending_import_conflicts(
            &fixture.actor,
            fixture.request(ConflictPolicy::AdoptAll, ConflictLeftovers::PreserveAll),
        )
        .await
        .unwrap();
    assert!(preview.rows[0].error_reason.is_none(), "{preview:?}");
    assert!(
        fixture
            .junk
            .join("Emberfall.2024.2160p.BluRay.cd1.mkv")
            .exists()
    );
    assert!(
        fixture
            .junk
            .join("Emberfall.2024.2160p.BluRay.cd2.mkv")
            .exists()
    );
}

#[tokio::test]
async fn conflict_preview_refuses_a_folder_that_became_another_librarys_root() {
    let fixture = ConflictFixture::new().await;
    // Added after the pending row was written.
    configure_default_library_root(&fixture.ctx, MediaFacet::Series, &fixture.junk).await;
    let preview = fixture
        .ctx
        .app
        .preview_pending_import_conflicts(
            &fixture.actor,
            fixture.request(ConflictPolicy::KeepOne, ConflictLeftovers::MediaOnly),
        )
        .await
        .unwrap();
    assert_eq!(
        preview.rows[0].error_reason.as_deref(),
        Some(refusal::FOLDER_UNSAFE),
        "{preview:?}"
    );
    assert!(
        fixture
            .junk
            .join("Emberfall.2024.2160p.BluRay.mkv")
            .exists()
    );
}

#[tokio::test]
async fn conflict_keep_one_refuses_a_title_with_part_folders() {
    let fixture = ConflictFixture::new().await;
    let part = fixture.named.join("CD2");
    std::fs::create_dir(&part).unwrap();
    std::fs::write(part.join("Emberfall.2024.720p.WEB-DL.mkv"), b"second part").unwrap();
    let preview = fixture
        .ctx
        .app
        .preview_pending_import_conflicts(
            &fixture.actor,
            fixture.request(ConflictPolicy::KeepOne, ConflictLeftovers::MediaOnly),
        )
        .await
        .unwrap();
    assert_eq!(
        preview.rows[0].error_reason.as_deref(),
        Some(refusal::MULTI_PART_MEDIA),
        "{preview:?}"
    );
    assert!(part.join("Emberfall.2024.720p.WEB-DL.mkv").exists());
}

#[tokio::test]
async fn conflict_keep_one_uses_each_of_three_roots_for_its_disposal_manifest() {
    let mut fixture = ConflictFixture::new().await;
    let second_temp = tempfile::tempdir().unwrap();
    let third_temp = tempfile::tempdir().unwrap();
    let second_root = std::fs::canonicalize(second_temp.path()).unwrap();
    let third_root = std::fs::canonicalize(third_temp.path()).unwrap();
    let library = fixture
        .ctx
        .libraries
        .get_by_id(&fixture.title.library_id)
        .await
        .unwrap()
        .unwrap();
    fixture
        .ctx
        .libraries
        .update(
            &library.id,
            library.name,
            library.slug,
            vec![
                LibraryRootDraft {
                    id: Some(fixture.title.root_folder_id.clone()),
                    path: fixture.root.to_string_lossy().into_owned(),
                    is_default: true,
                },
                LibraryRootDraft {
                    id: None,
                    path: second_root.to_string_lossy().into_owned(),
                    is_default: false,
                },
                LibraryRootDraft {
                    id: None,
                    path: third_root.to_string_lossy().into_owned(),
                    is_default: false,
                },
            ],
            None,
        )
        .await
        .unwrap();
    let moved_junk = second_root.join("ADEG78");
    std::fs::rename(&fixture.junk, &moved_junk).unwrap();
    fixture.junk = moved_junk;
    let mut item = fixture
        .ctx
        .library_scan_unmatched
        .get_library_scan_unmatched_item(&fixture.pending_id)
        .await
        .unwrap()
        .unwrap();
    fixture
        .ctx
        .library_scan_unmatched
        .delete_library_scan_unmatched_item(&item.library_id, item.facet.clone(), &item.item_path)
        .await
        .unwrap();
    item.item_path = fixture.junk.to_string_lossy().into_owned();
    item.scan_root = second_root.to_string_lossy().into_owned();
    fixture
        .ctx
        .library_scan_unmatched
        .upsert_library_scan_unmatched_item(&item)
        .await
        .unwrap();
    let third_folder = third_root.join("XYZZ98");
    std::fs::create_dir_all(&third_folder).unwrap();
    std::fs::write(
        third_folder.join("Emberfall.2024.1080p.WEB-DL.mkv"),
        b"third root movie",
    )
    .unwrap();
    seed_conflict(
        &fixture.ctx,
        &fixture.title,
        &third_folder,
        "duplicate-third-root",
    )
    .await;
    let task = fixture
        .settle(
            fixture.request(ConflictPolicy::KeepOne, ConflictLeftovers::PreserveAll),
            false,
        )
        .await;
    assert_eq!((task.settled_count, task.failed_count), (1, 0), "{task:?}");
    let recycled = fixture
        .ctx
        .app
        .list_recycled_items(&fixture.actor, Some(vec![fixture.title.library_id.clone()]))
        .await
        .unwrap();
    assert!(
        recycled
            .iter()
            .any(|entry| entry.media_root == third_root.to_string_lossy())
    );
    assert!(
        recycled
            .iter()
            .any(|entry| entry.media_root == fixture.root.to_string_lossy())
    );
    assert!(!third_folder.exists());
    assert!(!fixture.junk.exists());
    assert_eq!(
        std::fs::read(fixture.named.join("Emberfall.2024.2160p.BluRay.mkv")).unwrap(),
        b"best movie contents"
    );
}

#[test]
fn conflict_refusal_codes_are_distinct_stable_identifiers() {
    let unique = refusal::ALL
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(unique.len(), refusal::ALL.len());
    assert!(unique.iter().all(|reason| {
        reason.starts_with("LOCATION_CONFLICT_")
            && reason
                .chars()
                .all(|character| character.is_ascii_uppercase() || character == '_')
    }));
}

#[tokio::test]
async fn conflict_task_restart_finds_older_unfinished_journal_behind_completed_history() {
    use scryer_application::{JobRunRepository, JobRunStatus};
    use scryer_infrastructure_workflow::workflow::stores::workflow_operation_store::WorkflowOperationStore;
    let fixture = ConflictFixture::new().await;
    let finished = fixture
        .settle(
            fixture.request(ConflictPolicy::AdoptAll, ConflictLeftovers::PreserveAll),
            false,
        )
        .await;
    assert_eq!(finished.settled_count, 1);
    let store = WorkflowOperationStore::new(fixture.ctx.db.datastore());
    let mut original = store
        .get_job_run(&finished.job_run_id)
        .await
        .unwrap()
        .unwrap();
    for sequence in 1..=17 {
        let mut newer = original.clone();
        newer.id = Id::new().0;
        newer.started_at = original.started_at + chrono::Duration::seconds(sequence);
        newer.created_at = newer.started_at;
        newer.updated_at = newer.started_at;
        store.create_job_run(&newer).await.unwrap();
    }
    let mut journal: Value =
        serde_json::from_str(original.progress_json.as_deref().unwrap()).unwrap();
    journal["task"]["completed"] = json!(false);
    journal["task"]["settled_count"] = json!(0);
    journal["task"]["failed_count"] = json!(0);
    journal["next"] = json!(0);
    original.progress_json = Some(journal.to_string());
    original.status = JobRunStatus::Running;
    original.completed_at = None;
    store.update_job_run(&original).await.unwrap();
    let mut abandoned = original.clone();
    abandoned.id = Id::new().0;
    abandoned.operation_type = "synthetic_nonresumable".into();
    abandoned.progress_json = Some("synthetic progress".into());
    let abandoned = store.create_job_run(&abandoned).await.unwrap();
    let mut future = original.clone();
    future.id = Id::new().0;
    future.progress_json = Some(r#"{"futureFormat":true,"task":{"completed":false}}"#.into());
    let future = store.create_job_run(&future).await.unwrap();
    // Use the real boot order: reserve resumable work before generic job cleanup.
    let reserved = fixture
        .ctx
        .app
        .resumable_location_operation_job_run_ids()
        .await
        .unwrap();
    assert!(reserved.contains(&original.id));
    assert!(
        reserved.contains(&future.id),
        "an unreadable future journal must never be erased by generic reconciliation"
    );
    fixture
        .ctx
        .app
        .reconcile_interrupted_job_runs(&reserved)
        .await
        .unwrap();
    let preserved = store.get_job_run(&original.id).await.unwrap().unwrap();
    assert_eq!(preserved.progress_json, original.progress_json);
    assert_eq!(preserved.status, JobRunStatus::Running);
    assert_eq!(
        store
            .get_job_run(&abandoned.id)
            .await
            .unwrap()
            .unwrap()
            .status,
        JobRunStatus::Failed
    );
    fixture
        .ctx
        .app
        .resume_interrupted_location_operations()
        .await
        .unwrap();
    let recovered = fixture.wait_task(&finished.job_run_id).await;
    assert_eq!((recovered.settled_count, recovered.failed_count), (1, 0));
    assert_eq!(
        recovered.operation_ids, finished.operation_ids,
        "restart rejoins its existing child operation instead of enrolling another one"
    );
    let future_after = store.get_job_run(&future.id).await.unwrap().unwrap();
    assert_eq!(future_after.progress_json, future.progress_json);
    assert_eq!(future_after.status, JobRunStatus::Running);
    assert_eq!(
        std::fs::read(fixture.named.join("Emberfall.2024.2160p.BluRay.mkv")).unwrap(),
        b"best movie contents"
    );
}

#[tokio::test]
async fn conflict_private_journal_never_enters_generic_job_history_or_live_projection() {
    use scryer_application::{JobRun, JobRunRepository, JobRunStatus, JobRunTracker};
    use scryer_infrastructure_workflow::workflow::stores::workflow_operation_store::WorkflowOperationStore;
    let fixture = ConflictFixture::new().await;
    let task = fixture
        .settle(
            fixture.request(ConflictPolicy::AdoptAll, ConflictLeftovers::PreserveAll),
            false,
        )
        .await;
    let store = WorkflowOperationStore::new(fixture.ctx.db.datastore());
    let record = store.get_job_run(&task.job_run_id).await.unwrap().unwrap();
    let private = record
        .progress_json
        .clone()
        .expect("terminal task retains its private recovery journal");
    assert!(private.contains(fixture.root.to_str().unwrap()));
    assert!(private.contains(&task.operation_ids[0]));
    let authorized = fixture
        .ctx
        .app
        .pending_import_conflict_task(&fixture.actor, &task.job_run_id)
        .await
        .unwrap()
        .unwrap();
    assert!(authorized.completed);
    assert_eq!(authorized.operation_ids, task.operation_ids);

    let mut system_only = fixture.actor.clone();
    system_only.id = Id::new().0;
    system_only.username = "system-job-observer".into();
    system_only.authorization.app = scryer_domain::AppPermissionMask::from_permissions([
        scryer_domain::AppPermission::ManageSystemSettings,
    ]);
    system_only.authorization.libraries.clear();
    system_only.authorization.default_library = scryer_domain::LibraryPermissionMask::NONE;
    system_only.authorization.loaded = true;
    let denied = schema_exec(
        &fixture.ctx,
        &format!(
            r#"{{ pendingImportConflictTask(jobRunId: "{}") {{ jobRunId operationIds }} }}"#,
            task.job_run_id
        ),
        Some(system_only.clone()),
    )
    .await;
    assert_graphql_field_denied(&denied, "pendingImportConflictTask");
    let history = schema_exec(
        &fixture.ctx,
        "{ jobRuns(jobKey: LOCATION_OPERATION, limit: 100) { id progressJson } }",
        Some(system_only),
    )
    .await;
    assert_no_errors(&history);
    let row = history["data"]["jobRuns"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == task.job_run_id)
        .expect("system job history includes the task summary");
    assert!(row["progressJson"].is_null());
    assert!(!history.to_string().contains(&task.operation_ids[0]));

    let tracker = JobRunTracker::new();
    let mut events = tracker.subscribe();
    let mut running = record.clone();
    running.status = JobRunStatus::Running;
    running.completed_at = None;
    tracker
        .upsert_active_run(JobRun::from_record(&running, None))
        .await;
    let initial = tracker.list_active().await;
    assert!(
        initial
            .iter()
            .find(|run| run.id == task.job_run_id)
            .unwrap()
            .progress_json
            .is_none()
    );
    let live = tokio::time::timeout(std::time::Duration::from_secs(60), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(live.id, task.job_run_id);
    assert!(live.progress_json.is_none());
    tracker
        .upsert_active_run(JobRun::from_record(&record, None))
        .await;
    let completed = tokio::time::timeout(std::time::Duration::from_secs(60), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(completed.id, task.job_run_id);
    assert!(completed.progress_json.is_none());
    assert_eq!(
        record.progress_json.as_deref(),
        Some(private.as_str()),
        "projection never destroys the stored journal"
    );
}

fn folder_entries(folder: &std::path::Path) -> Vec<String> {
    let mut names = std::fs::read_dir(folder)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    names.sort();
    names
}

async fn recycled_original_paths(fixture: &ConflictFixture) -> Vec<String> {
    fixture
        .ctx
        .app
        .list_recycled_items(&fixture.actor, Some(vec![fixture.title.library_id.clone()]))
        .await
        .unwrap()
        .into_iter()
        .map(|entry| entry.original_path)
        .collect()
}

#[cfg(unix)]
#[tokio::test]
async fn conflict_preview_refuses_a_case_variant_alias_of_the_title_folder() {
    use std::os::unix::fs::MetadataExt;
    let fixture = ConflictFixture::new().await;
    let alias = fixture.root.join("emberfall (2024)");
    let aliased = std::fs::metadata(&alias)
        .ok()
        .zip(std::fs::metadata(&fixture.named).ok())
        .is_some_and(|(alias, named)| alias.dev() == named.dev() && alias.ino() == named.ino());
    if !aliased {
        eprintln!("skipping: this filesystem is case-sensitive, so no case alias exists");
        return;
    }
    let alias_id = seed_conflict(&fixture.ctx, &fixture.title, &alias, "case-alias").await;
    let request = ConflictRequest {
        pending_import_ids: vec![alias_id],
        policy: ConflictPolicy::KeepOne,
        leftovers: ConflictLeftovers::MediaOnly,
    };
    let preview = fixture
        .ctx
        .app
        .preview_pending_import_conflicts(&fixture.actor, request)
        .await
        .unwrap();
    assert_eq!(
        preview.rows[0].error_reason.as_deref(),
        Some(refusal::FOLDER_UNSAFE),
        "{preview:?}"
    );
    assert_eq!(
        std::fs::read(fixture.named.join("Emberfall.2024.720p.WEB-DL.mkv")).unwrap(),
        b"inferior movie"
    );
}

#[tokio::test]
async fn conflict_tracked_media_is_media_even_when_its_name_says_sample() {
    let fixture = ConflictFixture::new().await;
    let tracked = fixture.junk.join("Emberfall.2024.1080p.sample.mkv");
    std::fs::write(&tracked, b"tracked additional").unwrap();
    let row = fixture
        .ctx
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: fixture.title.id.clone(),
            file_path: tracked.to_string_lossy().into_owned(),
            size_bytes: 18,
            role: MediaFileRole::Additional,
            quality_label: Some("1080p".into()),
            resolution: Some("1080p".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    let request = fixture.request(ConflictPolicy::AdoptAll, ConflictLeftovers::PreserveAll);
    let preview = fixture
        .ctx
        .app
        .preview_pending_import_conflicts(&fixture.actor, request.clone())
        .await
        .unwrap();
    assert_eq!(
        preview.rows[0].leftover_count, 1,
        "only the artwork is a leftover; the tracked file is media: {preview:?}"
    );
    let task = fixture.settle(request, false).await;
    assert_eq!((task.settled_count, task.failed_count), (1, 0), "{task:?}");
    let moved = fixture
        .ctx
        .media_files
        .get_media_file_by_id(&row)
        .await
        .unwrap()
        .expect("the tracked row survives adoption");
    assert!(
        std::path::Path::new(&moved.file_path).starts_with(&fixture.named),
        "{moved:?}"
    );
    assert_eq!(
        std::fs::read(&moved.file_path).unwrap(),
        b"tracked additional"
    );
}

#[tokio::test]
async fn conflict_retry_resumes_a_keep_one_that_failed_part_way() {
    use scryer_application::location::model::LocationOperationState;
    use scryer_application::location::operations::LocationResumeDecision;
    let fixture = ConflictFixture::new().await;
    // A file where the bin directory belongs makes every recycle fail.
    let blocker = fixture.root.join(".scryer-recycle");
    std::fs::write(&blocker, b"not a directory").unwrap();
    let task = fixture
        .settle(
            fixture.request(ConflictPolicy::KeepOne, ConflictLeftovers::PreserveAll),
            false,
        )
        .await;
    assert_eq!((task.settled_count, task.failed_count), (0, 1), "{task:?}");
    let operation_id = task.operation_ids[0].clone();
    let inferior = fixture.named.join("Emberfall.2024.720p.WEB-DL.mkv");
    assert_eq!(std::fs::read(&inferior).unwrap(), b"inferior movie");
    std::fs::remove_file(&blocker).unwrap();
    let decision = fixture
        .ctx
        .app
        .resume_location_operation(&operation_id)
        .await
        .unwrap();
    let LocationResumeDecision::Resume(plan) = decision else {
        panic!("a failed conflict resolution must stay resumable: {decision:?}");
    };
    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(120),
        fixture.ctx.app.run_root_move(&operation_id, &plan),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        matches!(
            outcome.state,
            LocationOperationState::Completed | LocationOperationState::CompletedWithWarnings
        ),
        "{outcome:?}"
    );
    let files = fixture
        .ctx
        .media_files
        .list_media_files_for_title(&fixture.title.id)
        .await
        .unwrap();
    assert_eq!(files.len(), 1, "{files:?}");
    assert!(files[0].role.is_primary());
    assert!(std::path::Path::new(&files[0].file_path).starts_with(&fixture.named));
    assert_eq!(
        std::fs::read(&files[0].file_path).unwrap(),
        b"best movie contents"
    );
    assert!(!inferior.exists());
    assert!(
        recycled_original_paths(&fixture)
            .await
            .contains(&inferior.to_string_lossy().into_owned())
    );
    assert_eq!(
        std::fs::read(fixture.named.join("poster.jpg")).unwrap(),
        b"artwork contents"
    );
    assert!(!fixture.junk.exists());
    assert!(
        fixture
            .ctx
            .library_scan_unmatched
            .get_library_scan_unmatched_item(&fixture.pending_id)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn conflict_adopt_all_media_only_recycles_only_the_loser_remainder() {
    let fixture = ConflictFixture::new().await;
    std::fs::write(fixture.named.join("folder.jpg"), b"survivor artwork").unwrap();
    let task = fixture
        .settle(
            fixture.request(ConflictPolicy::AdoptAll, ConflictLeftovers::MediaOnly),
            false,
        )
        .await;
    assert_eq!((task.settled_count, task.failed_count), (1, 0), "{task:?}");
    assert_eq!(
        folder_entries(&fixture.named),
        [
            "Emberfall.2024.2160p.BluRay.en.srt",
            "Emberfall.2024.2160p.BluRay.mkv",
            "Emberfall.2024.720p.WEB-DL.mkv",
            "folder.jpg",
        ]
    );
    assert_eq!(
        std::fs::read(fixture.named.join("folder.jpg")).unwrap(),
        b"survivor artwork"
    );
    assert_eq!(
        std::fs::read(fixture.named.join("Emberfall.2024.2160p.BluRay.en.srt")).unwrap(),
        b"subtitle contents"
    );
    let recycled = recycled_original_paths(&fixture).await;
    assert_eq!(
        recycled,
        [fixture
            .junk
            .join("poster.jpg")
            .to_string_lossy()
            .into_owned()],
        "only the loser's non-media remainder is recycled"
    );
    assert!(!fixture.junk.exists());
    assert_eq!(
        fixture
            .ctx
            .media_files
            .list_media_files_for_title(&fixture.title.id)
            .await
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test]
async fn conflict_media_only_preserves_unrelated_non_media_inside_the_survivor() {
    let fixture = ConflictFixture::new().await;
    std::fs::write(fixture.named.join("notes.txt"), b"operator notes").unwrap();
    let task = fixture
        .settle(
            fixture.request(ConflictPolicy::KeepOne, ConflictLeftovers::MediaOnly),
            false,
        )
        .await;
    assert_eq!((task.settled_count, task.failed_count), (1, 0), "{task:?}");
    assert_eq!(
        std::fs::read(fixture.named.join("notes.txt")).unwrap(),
        b"operator notes"
    );
    assert!(
        !recycled_original_paths(&fixture)
            .await
            .iter()
            .any(|path| path.ends_with("notes.txt"))
    );
}

#[tokio::test]
async fn conflict_keep_one_moves_nothing_when_the_survivor_already_holds_the_best_file() {
    let fixture = ConflictFixture::new().await;
    for file in fixture
        .ctx
        .media_files
        .list_media_files_for_title(&fixture.title.id)
        .await
        .unwrap()
    {
        fixture
            .ctx
            .media_files
            .delete_media_file(&file.id)
            .await
            .unwrap();
    }
    std::fs::remove_file(fixture.named.join("Emberfall.2024.720p.WEB-DL.mkv")).unwrap();
    std::fs::remove_file(fixture.junk.join("Emberfall.2024.2160p.BluRay.mkv")).unwrap();
    std::fs::remove_file(fixture.junk.join("Emberfall.2024.2160p.BluRay.en.srt")).unwrap();
    let best = fixture.named.join("Emberfall.2024.2160p.BluRay.mkv");
    std::fs::write(&best, b"best movie contents").unwrap();
    let loser = fixture.junk.join("Emberfall.2024.720p.WEB-DL.mkv");
    std::fs::write(&loser, b"inferior movie").unwrap();
    fixture
        .ctx
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: fixture.title.id.clone(),
            file_path: best.to_string_lossy().into_owned(),
            size_bytes: 19,
            role: MediaFileRole::Primary,
            quality_label: Some("2160p".into()),
            resolution: Some("2160p".into()),
            source_type: Some("BluRay".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    let request = fixture.request(ConflictPolicy::KeepOne, ConflictLeftovers::MediaOnly);
    let preview = fixture
        .ctx
        .app
        .preview_pending_import_conflicts(&fixture.actor, request.clone())
        .await
        .unwrap();
    assert_eq!(
        preview.rows[0].survivor_path,
        fixture.named.to_string_lossy()
    );
    assert_eq!(preview.rows[0].move_bytes, 0, "{preview:?}");
    let task = fixture.settle(request, false).await;
    assert_eq!((task.settled_count, task.failed_count), (1, 0), "{task:?}");
    assert_eq!(
        folder_entries(&fixture.named),
        ["Emberfall.2024.2160p.BluRay.mkv"]
    );
    assert_eq!(std::fs::read(&best).unwrap(), b"best movie contents");
    let files = fixture
        .ctx
        .media_files
        .list_media_files_for_title(&fixture.title.id)
        .await
        .unwrap();
    assert_eq!(files.len(), 1);
    assert!(files[0].role.is_primary());
    assert_eq!(files[0].file_path, best.to_string_lossy());
    assert!(
        recycled_original_paths(&fixture)
            .await
            .contains(&loser.to_string_lossy().into_owned())
    );
    assert!(!fixture.junk.exists());
}

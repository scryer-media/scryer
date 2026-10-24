use super::*;
use scryer_application::LocationOperationRepository;
use scryer_application::location::model::LocationOperationState;
use scryer_application::location::title_folder_recovery::RECOVERY_VERSION;
use scryer_infrastructure_library::media::libraries::location_operation_store::LocationOperationStore;

async fn configure_recovery_recycle_path(ctx: &TestContext, path: &std::path::Path) {
    ctx.settings_store
        .batch_ensure_setting_definitions(vec![SettingDefinitionSeed {
            category: "media".into(),
            scope: "media".into(),
            key_name: "recycle_bin.path".into(),
            data_type: "string".into(),
            default_value_json: "null".into(),
            is_sensitive: false,
            validation_json: None,
        }])
        .await
        .unwrap();
    ctx.settings_store
        .upsert_setting_value(
            "media",
            "recycle_bin.path",
            None,
            serde_json::to_string(&path.to_string_lossy()).unwrap(),
            "test",
            None,
        )
        .await
        .unwrap();
}

async fn await_recovery(
    ctx: &TestContext,
    title_id: &str,
) -> scryer_application::location::model::LocationOperation {
    let store = LocationOperationStore::new(ctx.db.datastore());
    let id = format!("{RECOVERY_VERSION}:{title_id}");
    tokio::time::timeout(std::time::Duration::from_secs(60), async {
        loop {
            if let Some(operation) = store.get_location_operation(&id).await.unwrap()
                && matches!(
                    operation.state,
                    LocationOperationState::Completed
                        | LocationOperationState::CompletedWithWarnings
                        | LocationOperationState::Failed
                        | LocationOperationState::Canceled
                )
            {
                return operation;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("recovery must reach an observable terminal state")
}

#[tokio::test]
async fn title_folder_recovery_preserves_seventeen_originals_and_catalog_identity() {
    let ctx = TestContext::new().await;
    let temp = tempfile::tempdir().unwrap();
    let ancestor = temp.path().canonicalize().unwrap();
    let root = ancestor.join("Videos/Series");
    std::fs::create_dir_all(&root).unwrap();
    configure_default_library_root(&ctx, MediaFacet::Series, &root).await;
    let title = create_catalog_title(
        &ctx,
        "Recovered Series",
        MediaFacet::Series,
        vec![],
        vec!["keep-tag".into()],
        true,
    )
    .await;
    set_title_folder_path(&ctx, &title.id, &ancestor).await;
    let healthy = create_catalog_title(
        &ctx,
        "Neighbor Series",
        MediaFacet::Series,
        vec![],
        vec![],
        true,
    )
    .await;
    let healthy_folder = root.join("Neighbor Series");
    std::fs::create_dir_all(&healthy_folder).unwrap();
    std::fs::write(healthy_folder.join("neighbor.mkv"), b"neighbor").unwrap();
    set_title_folder_path(&ctx, &healthy.id, &healthy_folder).await;
    ctx.media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: healthy.id.clone(),
            file_path: healthy_folder
                .join("neighbor.mkv")
                .to_string_lossy()
                .into_owned(),
            size_bytes: 8,
            role: MediaFileRole::Primary,
            ..Default::default()
        })
        .await
        .unwrap();
    let mut originals = Vec::new();
    for index in 1..=17 {
        let path = ancestor.join(format!("Recovered.Series.S01E{index:02}.mkv"));
        let content = format!("synthetic media {index}").into_bytes();
        std::fs::write(&path, &content).unwrap();
        let media_id = ctx
            .media_files
            .insert_media_file(&InsertMediaFileInput {
                title_id: title.id.clone(),
                file_path: path.to_string_lossy().into_owned(),
                size_bytes: content.len() as i64,
                role: MediaFileRole::Primary,
                original_file_path: Some(format!("/download/original-{index}.mkv")),
                quality_label: Some("1080p".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        let episode = ctx
            .shows
            .create_episode(Episode {
                id: Id::new().0,
                title_id: title.id.clone(),
                collection_id: None,
                episode_type: EpisodeType::Standard,
                episode_number: Some(index.to_string()),
                season_number: Some("1".into()),
                episode_label: None,
                title: None,
                air_date: None,
                duration_seconds: None,
                has_multi_audio: false,
                has_subtitle: false,
                is_filler: false,
                is_recap: false,
                absolute_number: None,
                contiguous_absolute_number: None,
                overview: None,
                tvdb_id: None,
                tmdb_id: None,
                image_url: None,
                monitored: true,
                created_at: Utc::now(),
            })
            .await
            .unwrap();
        ctx.media_files
            .link_file_to_episode(&media_id, &episode.id)
            .await
            .unwrap();
        originals.push((path, content));
    }
    let before = ctx
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .unwrap();
    let links_before: Vec<(String, String)> = sqlx::query_as(
        "SELECT file_id, episode_id FROM file_episode_map ORDER BY file_id, episode_id",
    )
    .fetch_all(ctx.db.pool())
    .await
    .unwrap();
    assert_eq!(links_before.len(), 17);
    let actor = User::system_execution_actor();
    let blocked = ctx
        .app
        .preview_delete_title_files(&actor, &healthy.id)
        .await
        .expect_err("ancestor ownership blocks the neighbor");
    assert!(
        blocked
            .to_string()
            .contains("overlaps another title's recorded folder")
    );
    ctx.app.enroll_title_folder_recovery().await.unwrap();
    let operation = await_recovery(&ctx, &title.id).await;
    assert!(
        matches!(
            operation.state,
            LocationOperationState::Completed | LocationOperationState::CompletedWithWarnings
        ),
        "{operation:?}"
    );
    ctx.app
        .preview_delete_title_files(&actor, &healthy.id)
        .await
        .expect("recovery unblocks the healthy neighbor's preview");
    let after = ctx
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .unwrap();
    assert_eq!(before.len(), after.len());
    for old in &before {
        let new = after
            .iter()
            .find(|new| new.id == old.id)
            .expect("same media ID");
        assert_eq!(new.title_id, old.title_id);
        assert_eq!(new.role, old.role);
        assert_eq!(new.original_file_path, old.original_file_path);
        assert!(std::path::Path::new(&new.file_path).starts_with(&root));
        assert_eq!(
            std::fs::read(&new.file_path).unwrap(),
            std::fs::read(&old.file_path).unwrap()
        );
    }
    let links_after: Vec<(String, String)> = sqlx::query_as(
        "SELECT file_id, episode_id FROM file_episode_map ORDER BY file_id, episode_id",
    )
    .fetch_all(ctx.db.pool())
    .await
    .unwrap();
    assert_eq!(links_after, links_before);
    for (path, content) in originals {
        assert_eq!(std::fs::read(path).unwrap(), content);
    }
    let repaired = ctx.titles.get_by_id(&title.id).await.unwrap().unwrap();
    assert_eq!(repaired.id, title.id);
    assert_eq!(repaired.tags, title.tags);
    assert_eq!(repaired.monitored, title.monitored);
    assert_eq!(
        std::fs::read(healthy_folder.join("neighbor.mkv")).unwrap(),
        b"neighbor"
    );
    assert_eq!(
        ctx.titles
            .get_by_id(&healthy.id)
            .await
            .unwrap()
            .unwrap()
            .folder_path,
        Some(healthy_folder.to_string_lossy().into_owned())
    );
    ctx.app.enroll_title_folder_recovery().await.unwrap();
    let store = LocationOperationStore::new(ctx.db.datastore());
    assert!(store.recovery_committed(&operation.id).await.unwrap());
    assert_eq!(
        ctx.media_files
            .list_media_files_for_title(&title.id)
            .await
            .unwrap()
            .len(),
        17
    );
}

#[tokio::test]
async fn title_folder_recovery_missing_sources_stays_unresolved() {
    let ctx = TestContext::new().await;
    let temp = tempfile::tempdir().unwrap();
    let ancestor = temp.path().canonicalize().unwrap();
    let root = ancestor.join("Library");
    std::fs::create_dir_all(&root).unwrap();
    configure_default_library_root(&ctx, MediaFacet::Movie, &root).await;
    let title = create_catalog_title(
        &ctx,
        "Missing Film",
        MediaFacet::Movie,
        vec![],
        vec![],
        true,
    )
    .await;
    set_title_folder_path(&ctx, &title.id, &ancestor).await;
    let source = ancestor.join("missing.mkv").to_string_lossy().into_owned();
    ctx.media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: source.clone(),
            size_bytes: 17,
            role: MediaFileRole::Primary,
            ..Default::default()
        })
        .await
        .unwrap();
    ctx.app.enroll_title_folder_recovery().await.unwrap();
    let operation = await_recovery(&ctx, &title.id).await;
    assert_eq!(
        operation.state,
        LocationOperationState::Failed,
        "{operation:?}"
    );
    assert_eq!(
        ctx.titles
            .get_by_id(&title.id)
            .await
            .unwrap()
            .unwrap()
            .folder_path,
        Some(ancestor.to_string_lossy().into_owned())
    );
    assert_eq!(
        ctx.media_files
            .list_media_files_for_title(&title.id)
            .await
            .unwrap()[0]
            .file_path,
        source
    );
    assert_eq!(std::fs::read_dir(root).unwrap().count(), 0);
}

#[tokio::test]
async fn title_folder_recovery_rolls_back_all_paths_and_resumes_verified_copies() {
    let ctx = TestContext::new().await;
    let temp = tempfile::tempdir().unwrap();
    let ancestor = temp.path().canonicalize().unwrap();
    let root = ancestor.join("Library");
    std::fs::create_dir_all(&root).unwrap();
    configure_default_library_root(&ctx, MediaFacet::Movie, &root).await;
    let title = create_catalog_title(
        &ctx,
        "Rollback Film",
        MediaFacet::Movie,
        vec![],
        vec![],
        true,
    )
    .await;
    set_title_folder_path(&ctx, &title.id, &ancestor).await;
    let source = ancestor.join("film.mkv");
    std::fs::write(&source, b"synthetic movie").unwrap();
    ctx.media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: source.to_string_lossy().into_owned(),
            size_bytes: 15,
            role: MediaFileRole::Primary,
            ..Default::default()
        })
        .await
        .unwrap();
    sqlx::raw_sql("CREATE TRIGGER reject_recovery_title_write BEFORE UPDATE OF folder_path ON titles BEGIN SELECT RAISE(ABORT, 'synthetic catalog failure'); END")
        .execute(ctx.db.pool()).await.unwrap();
    ctx.app.enroll_title_folder_recovery().await.unwrap();
    let operation = await_recovery(&ctx, &title.id).await;
    assert_eq!(
        operation.state,
        LocationOperationState::Failed,
        "{operation:?}"
    );
    let store = LocationOperationStore::new(ctx.db.datastore());
    assert!(!store.recovery_committed(&operation.id).await.unwrap());
    assert_eq!(
        ctx.media_files
            .list_media_files_for_title(&title.id)
            .await
            .unwrap()[0]
            .file_path,
        source.to_string_lossy()
    );
    assert_eq!(
        ctx.titles
            .get_by_id(&title.id)
            .await
            .unwrap()
            .unwrap()
            .folder_path,
        Some(ancestor.to_string_lossy().into_owned())
    );
    let receipts = store
        .list_location_file_verifications(&operation.id, Some(&title.id))
        .await
        .unwrap();
    assert_eq!(receipts.len(), 1);
    let destination = receipts[0].destination_path.clone();
    assert_eq!(std::fs::read(&destination).unwrap(), b"synthetic movie");
    assert_eq!(std::fs::read(&source).unwrap(), b"synthetic movie");
    // A re-created catalog row must replace the old receipt even though its
    // source and destination filenames have not changed.
    let replacement_id = Id::new().0;
    sqlx::query("UPDATE media_files SET id = ? WHERE title_id = ?")
        .bind(&replacement_id)
        .bind(&title.id)
        .execute(ctx.db.pool())
        .await
        .unwrap();
    sqlx::raw_sql("DROP TRIGGER reject_recovery_title_write")
        .execute(ctx.db.pool())
        .await
        .unwrap();
    // Wait for the runner guard as well as its durable terminal checkpoint.
    let plan = tokio::time::timeout(std::time::Duration::from_secs(60), async {
        loop {
            match ctx.app.resume_title_folder_recovery(&operation).await {
                Ok(plan) => break plan,
                Err(error) if error.to_string().contains("already running") => {
                    tokio::task::yield_now().await
                }
                Err(error) => panic!("resume failed: {error}"),
            }
        }
    })
    .await
    .unwrap();
    ctx.app
        .spawn_title_folder_recovery(operation.id.clone(), plan);
    let resumed = await_recovery(&ctx, &title.id).await;
    assert!(
        matches!(
            resumed.state,
            LocationOperationState::Completed | LocationOperationState::CompletedWithWarnings
        ),
        "{resumed:?}"
    );
    assert!(store.recovery_committed(&operation.id).await.unwrap());
    let refreshed = store
        .list_location_file_verifications(&operation.id, Some(&title.id))
        .await
        .unwrap();
    assert_eq!(
        refreshed[0].media_file_id.as_deref(),
        Some(replacement_id.as_str())
    );
    assert_eq!(
        ctx.media_files
            .list_media_files_for_title(&title.id)
            .await
            .unwrap()[0]
            .file_path,
        destination
    );
    assert_eq!(std::fs::read(&source).unwrap(), b"synthetic movie");
    assert_eq!(
        store
            .list_location_file_verifications(&operation.id, Some(&title.id))
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn title_folder_recovery_adopts_unique_external_id_folders_without_recreating_titles() {
    for spelling in [
        "Example (2014) - [tvdbid-456789]",
        "Example (2014) {tvdb-456789}",
    ] {
        let ctx = TestContext::new().await;
        let temp = tempfile::tempdir().unwrap();
        let ancestor = temp.path().canonicalize().unwrap();
        let root = ancestor.join("Anime");
        let folder = root.join(spelling);
        std::fs::create_dir_all(&folder).unwrap();
        configure_default_library_root(&ctx, MediaFacet::Anime, &root).await;
        let title = create_catalog_title(
            &ctx,
            "Example",
            MediaFacet::Anime,
            vec![ExternalId::new("tvdb", "456789")],
            vec![],
            false,
        )
        .await;
        set_title_folder_path(&ctx, &title.id, &ancestor).await;
        ctx.app.enroll_title_folder_recovery().await.unwrap();
        let operation = await_recovery(&ctx, &title.id).await;
        assert!(
            matches!(
                operation.state,
                LocationOperationState::Completed | LocationOperationState::CompletedWithWarnings
            ),
            "{spelling}: {operation:?}"
        );
        let repaired = ctx.titles.get_by_id(&title.id).await.unwrap().unwrap();
        assert_eq!(
            repaired.folder_path,
            Some(folder.to_string_lossy().into_owned())
        );
        assert_eq!(repaired.id, title.id);
        assert!(!repaired.monitored);
        assert_eq!(repaired.external_ids, title.external_ids);
        assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 0);
    }
}

#[tokio::test]
async fn title_folder_recovery_duplicate_external_id_folders_are_unresolved() {
    let ctx = TestContext::new().await;
    let temp = tempfile::tempdir().unwrap();
    let ancestor = temp.path().canonicalize().unwrap();
    let root = ancestor.join("Anime");
    for spelling in [
        "Example (2014) - [tvdbid-456789]",
        "Example (2014) {tvdb-456789}",
    ] {
        std::fs::create_dir_all(root.join(spelling)).unwrap();
    }
    configure_default_library_root(&ctx, MediaFacet::Anime, &root).await;
    let title = create_catalog_title(
        &ctx,
        "Example",
        MediaFacet::Anime,
        vec![ExternalId::new("tvdb", "456789")],
        vec![],
        true,
    )
    .await;
    set_title_folder_path(&ctx, &title.id, &ancestor).await;
    ctx.app.enroll_title_folder_recovery().await.unwrap();
    let operation = await_recovery(&ctx, &title.id).await;
    assert_eq!(
        operation.state,
        LocationOperationState::Failed,
        "{operation:?}"
    );
    let checkpoints = LocationOperationStore::new(ctx.db.datastore())
        .list_location_title_checkpoints(&operation.id)
        .await
        .unwrap();
    assert!(
        operation
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("Multiple folders"))
            || checkpoints.iter().any(|checkpoint| checkpoint
                .detail
                .as_deref()
                .is_some_and(|detail| detail.contains("Multiple folders"))),
        "{operation:?} {checkpoints:?}"
    );
    assert_eq!(
        ctx.titles
            .get_by_id(&title.id)
            .await
            .unwrap()
            .unwrap()
            .folder_path,
        Some(ancestor.to_string_lossy().into_owned())
    );
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 2);
}

#[tokio::test]
async fn title_folder_recovery_leaves_healthy_libraries_and_nested_layouts_unchanged() {
    let ctx = TestContext::new().await;
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().canonicalize().unwrap();
    let mut titles = Vec::new();
    for (facet, name) in [
        (MediaFacet::Movie, "Movies"),
        (MediaFacet::Series, "Series"),
        (MediaFacet::Anime, "Anime"),
    ] {
        let root = base.join(name);
        std::fs::create_dir_all(&root).unwrap();
        configure_default_library_root(&ctx, facet.clone(), &root).await;
        for layout in ["Flat", "Seasons/Season 01", "Group/Nested"] {
            let folder = root.join(layout);
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::write(folder.join("media.mkv"), b"keep").unwrap();
            let title =
                create_catalog_title(&ctx, layout, facet.clone(), vec![], vec![], true).await;
            set_title_folder_path(&ctx, &title.id, &folder).await;
            titles.push((title.id, folder));
        }
    }
    ctx.app.enroll_title_folder_recovery().await.unwrap();
    let store = LocationOperationStore::new(ctx.db.datastore());
    for (id, folder) in titles {
        assert!(
            store
                .get_location_operation(&format!("{RECOVERY_VERSION}:{id}"))
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(
            ctx.titles
                .get_by_id(&id)
                .await
                .unwrap()
                .unwrap()
                .folder_path,
            Some(folder.to_string_lossy().into_owned())
        );
        assert_eq!(std::fs::read(folder.join("media.mkv")).unwrap(), b"keep");
    }
}

#[tokio::test]
async fn title_folder_recovery_uses_tracked_folder_in_another_root_of_the_same_library() {
    let ctx = TestContext::new().await;
    let temp = tempfile::tempdir().unwrap();
    let ancestor = temp.path().canonicalize().unwrap();
    let first = ancestor.join("First");
    let second = ancestor.join("Second");
    let folder = second.join("Existing Series");
    std::fs::create_dir_all(&first).unwrap();
    std::fs::create_dir_all(&folder).unwrap();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Series);
    let unrelated = second.join("Unrelated (2014) - [tvdbid-999999]");
    std::fs::create_dir(&unrelated).unwrap();
    std::fs::write(
        unrelated.join("tvshow.nfo"),
        "<tvshow><uniqueid type=\"tvdb\">888888</uniqueid></tvshow>",
    )
    .unwrap();
    let library = ctx.libraries.get_by_id(&library_id).await.unwrap().unwrap();
    let library = ctx
        .libraries
        .update(
            &library_id,
            library.name,
            library.slug,
            vec![
                LibraryRootDraft {
                    id: None,
                    path: first.to_string_lossy().into_owned(),
                    is_default: true,
                },
                LibraryRootDraft {
                    id: None,
                    path: second.to_string_lossy().into_owned(),
                    is_default: false,
                },
            ],
        )
        .await
        .unwrap();
    let destination_root = library
        .roots
        .iter()
        .find(|root| root.path == second.to_string_lossy())
        .unwrap();
    let title = create_catalog_title(
        &ctx,
        "Existing Series",
        MediaFacet::Series,
        vec![],
        vec![],
        true,
    )
    .await;
    set_title_folder_path(&ctx, &title.id, &ancestor).await;
    let path = folder.join("episode.mkv");
    std::fs::write(&path, b"media").unwrap();
    ctx.media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: path.to_string_lossy().into_owned(),
            size_bytes: 5,
            role: MediaFileRole::Primary,
            ..Default::default()
        })
        .await
        .unwrap();
    let before = ctx
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .unwrap();
    ctx.app.enroll_title_folder_recovery().await.unwrap();
    let operation = await_recovery(&ctx, &title.id).await;
    assert!(
        matches!(
            operation.state,
            LocationOperationState::Completed | LocationOperationState::CompletedWithWarnings
        ),
        "{operation:?}"
    );
    let repaired = ctx.titles.get_by_id(&title.id).await.unwrap().unwrap();
    assert_eq!(repaired.root_folder_id, destination_root.id);
    assert_eq!(
        repaired.folder_path,
        Some(folder.to_string_lossy().into_owned())
    );
    let after = ctx
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .unwrap();
    assert_eq!(before[0].id, after[0].id);
    assert_eq!(before[0].file_path, after[0].file_path);
    assert_eq!(std::fs::read(&path).unwrap(), b"media");
    assert_eq!(std::fs::read_dir(&first).unwrap().count(), 0);
}

#[tokio::test]
async fn title_folder_recovery_refuses_foreign_generated_destination() {
    for reserve_recycle in [false, true] {
        let ctx = TestContext::new().await;
        let temp = tempfile::tempdir().unwrap();
        let ancestor = temp.path().canonicalize().unwrap();
        let root = ancestor.join("Library");
        std::fs::create_dir(&root).unwrap();
        configure_default_library_root(&ctx, MediaFacet::Movie, &root).await;
        let title = create_catalog_title(
            &ctx,
            "Generated Film",
            MediaFacet::Movie,
            vec![],
            vec![],
            true,
        )
        .await;
        set_title_folder_path(&ctx, &title.id, &ancestor).await;
        let source = ancestor.join("film.mkv");
        std::fs::write(&source, b"media").unwrap();
        ctx.media_files
            .insert_media_file(&InsertMediaFileInput {
                title_id: title.id.clone(),
                file_path: source.to_string_lossy().into_owned(),
                size_bytes: 5,
                role: MediaFileRole::Primary,
                ..Default::default()
            })
            .await
            .unwrap();
        let current = ctx.titles.get_by_id(&title.id).await.unwrap().unwrap();
        let destination = root.join(format!("{} ({})", current.name, current.year.unwrap()));
        let foreign = destination.join("foreign.mkv");
        if reserve_recycle {
            configure_recovery_recycle_path(&ctx, &destination).await;
        } else {
            std::fs::create_dir_all(&destination).unwrap();
            std::fs::write(&foreign, b"unrelated content").unwrap();
        }
        ctx.app.enroll_title_folder_recovery().await.unwrap();
        let operation = await_recovery(&ctx, &title.id).await;
        assert_eq!(operation.state, LocationOperationState::Failed);
        assert!(operation.detail.unwrap().contains(if reserve_recycle {
            "configured recycle storage"
        } else {
            "without ownership evidence"
        }));
        assert_eq!(
            ctx.titles
                .get_by_id(&title.id)
                .await
                .unwrap()
                .unwrap()
                .folder_path,
            current.folder_path
        );
        assert_eq!(std::fs::read(source).unwrap(), b"media");
        if reserve_recycle {
            assert!(!destination.exists());
        } else {
            assert_eq!(std::fs::read(foreign).unwrap(), b"unrelated content");
            assert_eq!(std::fs::read_dir(destination).unwrap().count(), 1);
        }
    }
}

#[tokio::test]
async fn title_folder_recovery_never_adopts_exact_id_recycle_storage() {
    let ctx = TestContext::new().await;
    let temp = tempfile::tempdir().unwrap();
    let ancestor = temp.path().canonicalize().unwrap();
    let root = ancestor.join("Anime");
    let recycle = root.join("Example (2014) - [tvdbid-456789]");
    std::fs::create_dir_all(&recycle).unwrap();
    std::fs::write(recycle.join("retained.mkv"), b"retained original").unwrap();
    configure_default_library_root(&ctx, MediaFacet::Anime, &root).await;
    configure_recovery_recycle_path(&ctx, &recycle).await;
    let title = create_catalog_title(
        &ctx,
        "Example",
        MediaFacet::Anime,
        vec![ExternalId::new("tvdb", "456789")],
        vec![],
        true,
    )
    .await;
    set_title_folder_path(&ctx, &title.id, &ancestor).await;
    ctx.app.enroll_title_folder_recovery().await.unwrap();
    let operation = await_recovery(&ctx, &title.id).await;
    assert_eq!(operation.state, LocationOperationState::Failed);
    assert_eq!(
        ctx.titles
            .get_by_id(&title.id)
            .await
            .unwrap()
            .unwrap()
            .folder_path,
        Some(ancestor.to_string_lossy().into_owned())
    );
    assert_eq!(
        std::fs::read(recycle.join("retained.mkv")).unwrap(),
        b"retained original"
    );
    assert!(
        ctx.media_files
            .list_media_files_for_title(&title.id)
            .await
            .unwrap()
            .is_empty()
    );
}

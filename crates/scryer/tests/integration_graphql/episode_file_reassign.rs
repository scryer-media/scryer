use super::*;

#[tokio::test]
async fn media_issue_graphql_permissions_and_stale_identity() {
    use scryer_infrastructure_library::media::libraries::state_store::BlocklistStore;

    let ctx = TestContext::new().await;
    let fixture = reassign_fixture(&ctx, "Bad Media Fixture", "97108").await;
    let (file_id, path) =
        seed_primary_file(&ctx, &fixture, "synthetic.mkv", &fixture.episodes[0]).await;
    let store = std::sync::Arc::new(BlocklistStore::new(ctx.db.datastore()));
    let app = ctx
        .app
        .with_test_overrides(|builder| builder.with_blocklist_repo(store));
    let schema = scryer_interface::build_schema(app.clone(), ctx.auth_runtime.clone());
    let admin = app.find_or_create_default_user().await.expect("operator");
    let mut requester = admin.clone();
    requester.authorization = scryer_domain::UserAuthorization {
        app: scryer_domain::AppPermissionMask::NONE,
        libraries: std::collections::HashMap::from([(
            fixture.title.library_id.clone(),
            scryer_domain::LibraryPermissionMask::from_permissions([
                LibraryPermission::View,
                LibraryPermission::Request,
            ]),
        )]),
        loaded: true,
        ..Default::default()
    };
    let execute = |query, variables, actor: User| {
        schema.execute(
            async_graphql::Request::new(query)
                .variables(async_graphql::Variables::from_json(variables))
                .data(actor),
        )
    };
    let report = execute(
        "mutation($input: SubmitMediaIssueInput!) {
            submitMediaIssue(input: $input) { id status affectedEpisodeIds authorizedRemediation }
        }",
        json!({"input": {"mediaFileId": file_id, "reasonCode": "corrupted", "remediation": "MARK_BAD"}}),
        requester.clone(),
    ).await;
    let report = serde_json::to_value(report).unwrap();
    assert_no_errors(&report);
    let issue = &report["data"]["submitMediaIssue"];
    assert_eq!(issue["status"], "PENDING");
    assert_eq!(issue["affectedEpisodeIds"], json!([fixture.episodes[0].id]));
    assert!(issue["authorizedRemediation"].is_null());

    let approve = "mutation($id: ID!) { approveMediaIssue(issueId: $id) { status } }";
    let variables = json!({"id": issue["id"]});
    assert!(
        !execute(approve, variables.clone(), requester)
            .await
            .errors
            .is_empty()
    );

    let len = std::fs::metadata(&path).unwrap().len() as usize;
    std::fs::write(&path, vec![b'x'; len]).unwrap();
    let stale = serde_json::to_value(execute(approve, variables, admin).await).unwrap();
    assert_eq!(
        stale["errors"][0]["extensions"]["reason"], "MEDIA_SOURCE_CHANGED_DURING_INSPECTION",
        "{stale}"
    );
    assert_eq!(std::fs::read(path).unwrap(), vec![b'x'; len]);
}

/// A file marked bad stays the scope's file: the wanted pass does not count
/// the scope as missing, and an upgrade import for the scope is held rather
/// than recycling the marked file.
#[tokio::test]
async fn a_file_marked_bad_survives_the_wanted_pass_and_an_upgrade_import() {
    use scryer_application::media_issues::SubmitMediaIssueInput;
    use scryer_application::testing::{UpgradeForTestInput, execute_upgrade_for_test};
    use scryer_application::upgrade::UpgradeResult;
    use scryer_infrastructure_library::media::libraries::state_store::BlocklistStore;

    let ctx = TestContext::new().await;
    let fixture = reassign_fixture(&ctx, "Marked Bad Fixture", "97109").await;
    let episode = &fixture.episodes[0];
    let (file_id, path) = seed_primary_file(&ctx, &fixture, "synthetic.mkv", episode).await;
    let store = std::sync::Arc::new(BlocklistStore::new(ctx.db.datastore()));
    let app = ctx.app.with_test_overrides(|builder| {
        builder
            .with_blocklist_repo(store.clone())
            .with_media_files(std::sync::Arc::new(ctx.media_files.clone()))
            .with_file_importer(std::sync::Arc::new(
                scryer_infrastructure_workflow::workflow::file_importer::FsFileImporter::new(),
            ))
    });
    let operator = app.find_or_create_default_user().await.expect("operator");
    let with_library_permissions = |permissions: &[LibraryPermission]| {
        let mut actor = operator.clone();
        actor.authorization = scryer_domain::UserAuthorization {
            libraries: std::collections::HashMap::from([(
                fixture.title.library_id.clone(),
                scryer_domain::LibraryPermissionMask::from_permissions(permissions.iter().copied()),
            )]),
            loaded: true,
            ..Default::default()
        };
        actor
    };
    let requester =
        with_library_permissions(&[LibraryPermission::View, LibraryPermission::Request]);
    let admin =
        with_library_permissions(&[LibraryPermission::View, LibraryPermission::ManageTitles]);
    let issue = app
        .submit_media_issue(
            &requester,
            SubmitMediaIssueInput {
                media_file_id: file_id.clone(),
                reason: scryer_domain::MediaIssueReason::Corrupted,
                details: None,
                remediation: scryer_domain::MediaIssueRemediation::MarkBad,
            },
        )
        .await
        .expect("submit report");
    if issue.status == scryer_domain::MediaIssueStatus::Pending {
        app.approve_media_issue(&admin, &issue.id)
            .await
            .expect("approve report");
    }
    let marked = scryer_application::BlocklistRepository::marked_bad_file_ids(
        store.as_ref(),
        std::slice::from_ref(&file_id),
    )
    .await
    .expect("read marked files");
    assert!(marked.contains(&file_id), "the file is marked bad");

    let missing = ctx
        .media_files
        .list_missing_scope_candidates_for_title(Some(&fixture.title.id))
        .await
        .expect("wanted pass candidates");
    let missing_episodes: Vec<&str> = missing
        .episodes
        .iter()
        .map(|candidate| candidate.episode_id.as_str())
        .collect();
    assert!(
        missing_episodes.contains(&fixture.episodes[1].id.as_str()),
        "an episode without a file is missing: {missing_episodes:?}"
    );
    assert!(
        !missing_episodes.contains(&episode.id.as_str()),
        "the marked file still covers its episode"
    );

    let existing = ctx
        .media_files
        .list_media_files_for_title(&fixture.title.id)
        .await
        .expect("list files")
        .into_iter()
        .find(|file| file.id == file_id)
        .expect("marked file row");
    let source_dir = tempfile::tempdir().expect("source dir");
    let source = source_dir.path().join("synthetic.upgrade.mkv");
    std::fs::write(&source, b"synthetic upgrade content").expect("write source");
    let recycle_dir = tempfile::tempdir().expect("recycle dir");
    let media_root = fixture._media_root.path().to_string_lossy().into_owned();
    let recycle_config = scryer_application::recycle_bin::RecycleBinConfig {
        enabled: true,
        base_path: recycle_dir.path().to_path_buf(),
        retention_days: 7,
        cleanup_enabled: true,
        validation_error: None,
        source_roots: vec![fixture._media_root.path().to_path_buf()],
    };
    let result = execute_upgrade_for_test(
        &app,
        UpgradeForTestInput {
            actor: &admin,
            title: &fixture.title,
            existing_file: &existing,
            source_path: &source,
            dest_path: &fixture.season_dir.join("synthetic.upgrade.mkv"),
            parsed: scryer_application::parse_release_metadata("Synthetic.S01E01.2160p.WEB-DL"),
            final_score: 900,
            target_episode_ids: std::slice::from_ref(&episode.id),
            media_root: Some(&media_root),
            recycle_config: &recycle_config,
        },
    )
    .await
    .expect("upgrade attempt");
    let UpgradeResult::Rejected(rejection) = result else {
        panic!("an upgrade over a marked file is held");
    };
    assert_eq!(rejection.recycle_reason, "MEDIA_UPGRADE_REPLACES_BAD_FILE");
    assert!(path.exists(), "the marked file stays on disk");
    assert!(source.exists(), "the upgrade source is left in place");
    assert!(
        ctx.media_files
            .list_media_files_for_title(&fixture.title.id)
            .await
            .expect("list files")
            .iter()
            .any(|file| file.id == file_id),
        "the marked file stays in the catalog"
    );
}

/// A wrong-content report marks only the reported file and blocks only its
/// release. The same content in a sibling file stays unmarked, and the import
/// gate lets it through, so it can import for the episode it belongs to. A
/// corrupted report on that content then marks it and gates it title-wide.
#[tokio::test]
async fn a_wrong_content_report_marks_only_its_file_and_blocks_only_its_release() {
    use scryer_application::BlocklistRepository;
    use scryer_application::media_issues::SubmitMediaIssueInput;
    use scryer_application::testing::bad_file_gate_refuses_for_test;
    use scryer_infrastructure_library::media::libraries::state_store::BlocklistStore;

    let ctx = TestContext::new().await;
    let fixture = reassign_fixture(&ctx, "Wrong Content Fixture", "97110").await;
    let bytes = b"synthetic shared episode content".to_vec();
    let seed = |name: &str, release: &str, episode: usize| {
        let path = fixture.season_dir.join(name);
        std::fs::write(&path, &bytes).expect("write synthetic file");
        let input = InsertMediaFileInput {
            title_id: fixture.title.id.clone(),
            file_path: path.to_string_lossy().into_owned(),
            size_bytes: bytes.len() as i64,
            quality_label: Some("1080p".to_string()),
            indexer_source: Some("synthetic-indexer".to_string()),
            grabbed_release_title: Some(release.to_string()),
            release_listing_json: Some(
                r#"{"v":1,"captured_at":"2024-02-01T12:00:00Z"}"#.to_string(),
            ),
            ..Default::default()
        };
        (input, path, fixture.episodes[episode].id.clone())
    };
    let mut files = Vec::new();
    for (name, release, episode) in [
        ("synthetic.e01.mkv", "Synthetic.S01E01.1080p.WEB-DL-GRP", 0),
        ("synthetic.e02.mkv", "Synthetic.S01E02.1080p.WEB-DL-GRP", 1),
    ] {
        let (input, path, episode_id) = seed(name, release, episode);
        let file_id = ctx
            .media_files
            .insert_media_file(&input)
            .await
            .expect("insert media file");
        ctx.link_primary_file_to_episode(&fixture.title.id, &file_id, &episode_id)
            .await
            .expect("link primary file");
        files.push((file_id, path));
    }
    let (reported_id, reported_path) = files[0].clone();
    let (sibling_id, _) = files[1].clone();

    let store = std::sync::Arc::new(BlocklistStore::new(ctx.db.datastore()));
    let app = ctx.app.with_test_overrides(|builder| {
        builder
            .with_blocklist_repo(store.clone())
            .with_media_files(std::sync::Arc::new(ctx.media_files.clone()))
    });
    let operator = app.find_or_create_default_user().await.expect("operator");
    let with_library_permissions = |permissions: &[LibraryPermission]| {
        let mut actor = operator.clone();
        actor.authorization = scryer_domain::UserAuthorization {
            libraries: std::collections::HashMap::from([(
                fixture.title.library_id.clone(),
                scryer_domain::LibraryPermissionMask::from_permissions(permissions.iter().copied()),
            )]),
            loaded: true,
            ..Default::default()
        };
        actor
    };
    let requester =
        with_library_permissions(&[LibraryPermission::View, LibraryPermission::Request]);
    let admin =
        with_library_permissions(&[LibraryPermission::View, LibraryPermission::ManageTitles]);
    let report = |file_id: String, reason: scryer_domain::MediaIssueReason| {
        let app = &app;
        let requester = &requester;
        let admin = &admin;
        async move {
            let issue = app
                .submit_media_issue(
                    requester,
                    SubmitMediaIssueInput {
                        media_file_id: file_id,
                        reason,
                        details: None,
                        remediation: scryer_domain::MediaIssueRemediation::MarkBad,
                    },
                )
                .await
                .expect("submit report");
            if issue.status == scryer_domain::MediaIssueStatus::Pending {
                app.approve_media_issue(admin, &issue.id)
                    .await
                    .expect("approve report");
            }
        }
    };
    let both = vec![reported_id.clone(), sibling_id.clone()];

    report(
        reported_id.clone(),
        scryer_domain::MediaIssueReason::WrongContent,
    )
    .await;
    let marked = store
        .marked_bad_file_ids(&both)
        .await
        .expect("marked files");
    assert!(marked.contains(&reported_id), "the reported file is marked");
    assert!(
        !marked.contains(&sibling_id),
        "a sibling holding the same content is not marked"
    );
    assert!(
        !bad_file_gate_refuses_for_test(&app, &fixture.title.id, &reported_path)
            .await
            .expect("gate check"),
        "the same content still imports for another episode"
    );
    assert!(
        store
            .is_blocked(
                &fixture.title.id,
                "synthetic-indexer",
                "Synthetic.S01E01.1080p.WEB-DL-GRP",
                None,
            )
            .await
            .expect("reported release"),
        "the reported release is blocked"
    );
    assert!(
        !store
            .is_blocked(
                &fixture.title.id,
                "synthetic-indexer",
                "Synthetic.S01E02.1080p.WEB-DL-GRP",
                None,
            )
            .await
            .expect("other release"),
        "a different release of the title is not blocked"
    );

    report(
        sibling_id.clone(),
        scryer_domain::MediaIssueReason::Corrupted,
    )
    .await;
    let marked = store
        .marked_bad_file_ids(&both)
        .await
        .expect("marked files");
    assert!(marked.contains(&sibling_id), "the corrupted file is marked");
    assert!(
        bad_file_gate_refuses_for_test(&app, &fixture.title.id, &reported_path)
            .await
            .expect("gate check"),
        "corrupted content is refused anywhere in the title"
    );
}

/// One synthetic series with three episodes in season one, a recorded title
/// folder under a temp library root, and the season folder that holds files.
struct ReassignFixture {
    _media_root: tempfile::TempDir,
    title: Title,
    episodes: Vec<Episode>,
    season_dir: std::path::PathBuf,
}

/// A synthetic series title with three season-one episodes, in the default
/// series library.
async fn series_with_three_episodes(
    ctx: &TestContext,
    name: &str,
    tvdb_id: &str,
) -> (Title, Vec<Episode>) {
    let title = create_catalog_title(
        ctx,
        name,
        MediaFacet::Series,
        vec![ExternalId::new("tvdb".to_string(), tvdb_id.to_string())],
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
            last_episode_number: Some("3".to_string()),
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create season collection");
    let mut episodes = Vec::new();
    for number in 1..=3 {
        episodes.push(
            create_series_scan_episode(
                ctx,
                &title,
                &collection,
                "1",
                &number.to_string(),
                &format!("S01E{number:02}"),
            )
            .await,
        );
    }
    (title, episodes)
}

async fn reassign_fixture(ctx: &TestContext, name: &str, tvdb_id: &str) -> ReassignFixture {
    seed_typed_settings_definitions(ctx).await;
    let media_root = tempfile::tempdir().expect("media root tempdir");
    configure_default_library_root(ctx, MediaFacet::Series, media_root.path()).await;
    let (title, episodes) = series_with_three_episodes(ctx, name, tvdb_id).await;
    let title_dir = media_root.path().join(format!("{name} (2024)"));
    let season_dir = title_dir.join("Season 1");
    std::fs::create_dir_all(&season_dir).expect("create season dir");
    set_title_folder_path(ctx, &title.id, &title_dir).await;
    ReassignFixture {
        _media_root: media_root,
        title,
        episodes,
        season_dir,
    }
}

/// Write a synthetic file into the season folder and link it as the primary
/// file of `episode`.
async fn seed_primary_file(
    ctx: &TestContext,
    fixture: &ReassignFixture,
    file_name: &str,
    episode: &Episode,
) -> (String, std::path::PathBuf) {
    let path = fixture.season_dir.join(file_name);
    let bytes = format!("synthetic {file_name}").into_bytes();
    std::fs::write(&path, &bytes).expect("write synthetic file");
    let file_id = ctx
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: fixture.title.id.clone(),
            file_path: path.to_string_lossy().into_owned(),
            size_bytes: bytes.len() as i64,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert media file");
    ctx.link_primary_file_to_episode(&fixture.title.id, &file_id, &episode.id)
        .await
        .expect("link primary file");
    (file_id, path)
}

/// Every (episode, role) link row of one file, sorted.
async fn file_links(ctx: &TestContext, title_id: &str, file_id: &str) -> Vec<(String, String)> {
    let mut links = ctx
        .media_files
        .list_media_files_for_title(title_id)
        .await
        .expect("list title media files")
        .into_iter()
        .filter(|file| file.id == file_id)
        .filter_map(|file| {
            file.episode_id
                .map(|episode_id| (episode_id, file.role.as_str().to_string()))
        })
        .collect::<Vec<_>>();
    links.sort();
    links
}

fn app_with_filesystem_renamer(ctx: &TestContext) -> scryer_application::AppUseCase {
    ctx.app.with_test_overrides(|builder| {
        builder.with_library_renamer(std::sync::Arc::new(FileSystemLibraryRenamer::new()))
    })
}

const REASSIGN_MUTATION: &str = r#"
    mutation Reassign($input: ReassignEpisodeFileInput!) {
      reassignEpisodeFile(input: $input) {
        previousPath
        episodeIds
        file { id filePath }
      }
    }
"#;

struct GatedReassignRenamer {
    title_id: String,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}

#[async_trait::async_trait]
impl scryer_application::LibraryRenamer for GatedReassignRenamer {
    async fn validate_targets(
        &self,
        plan: &scryer_application::RenamePlan,
    ) -> scryer_application::AppResult<()> {
        if plan.title_id.as_deref() == Some(self.title_id.as_str()) {
            self.entered.notify_one();
            tokio::time::timeout(std::time::Duration::from_secs(60), self.release.notified())
                .await
                .expect("test releases the exact blocked title");
        }
        Err(scryer_application::AppError::Repository(
            "synthetic rename refusal".into(),
        ))
    }

    async fn apply_plan(
        &self,
        _: &scryer_application::RenamePlan,
        _: &scryer_application::ImportFilePermissions,
    ) -> scryer_application::AppResult<Vec<scryer_application::RenameApplyItemResult>> {
        panic!("validation refuses before any file is moved")
    }

    async fn rollback(
        &self,
        _: &[scryer_application::RenameApplyItemResult],
    ) -> scryer_application::AppResult<Vec<scryer_application::RenameApplyItemResult>> {
        Ok(vec![])
    }
}

// Sonarr 465b5192, EpisodeImport/Manual/ManualImportService.cs preserves the
// operator's chosen episode assignment. A failed rename cannot undo a peer's
// successful assignment: serialize the title through compensation.
#[tokio::test]
async fn reassign_episode_file_serializes_different_files_through_compensation() {
    tokio::time::timeout(std::time::Duration::from_secs(120), async {
        let ctx = TestContext::new().await;
        let fixture = reassign_fixture(&ctx, "Concurrent Fixture", "97107").await;
        let (first, first_path) = seed_primary_file(
            &ctx,
            &fixture,
            "Concurrent.Fixture.S01E01.1080p.WEB-DL.mkv",
            &fixture.episodes[0],
        )
        .await;
        let (second, second_path) = seed_primary_file(
            &ctx,
            &fixture,
            "Concurrent.Fixture.S01E02.1080p.WEB-DL.mkv",
            &fixture.episodes[1],
        )
        .await;
        let (other_title, other_episodes) =
            series_with_three_episodes(&ctx, "Independent Fixture", "97108").await;
        let other_root = tempfile::tempdir_in(fixture._media_root.path()).unwrap();
        let other_season = other_root.path().join("Season 1");
        std::fs::create_dir_all(&other_season).unwrap();
        set_title_folder_path(&ctx, &other_title.id, other_root.path()).await;
        let other = ReassignFixture {
            _media_root: other_root,
            title: other_title,
            episodes: other_episodes,
            season_dir: other_season,
        };
        let (other_file, other_path) = seed_primary_file(
            &ctx,
            &other,
            "Independent.Fixture.S01E01.1080p.WEB-DL.mkv",
            &other.episodes[0],
        )
        .await;
        let renamer = std::sync::Arc::new(GatedReassignRenamer {
            title_id: fixture.title.id.clone(),
            entered: tokio::sync::Notify::new(),
            release: tokio::sync::Notify::new(),
        });
        let app = ctx
            .app
            .with_test_overrides(|builder| builder.with_library_renamer(renamer.clone()));
        let actor = app.find_or_create_default_user().await.unwrap();
        let target = vec![fixture.episodes[1].id.clone()];
        let first_request = app.reassign_episode_file(&actor, &first, &target, true);
        tokio::pin!(first_request);
        tokio::select! {
            result = &mut first_request => panic!("rename was not gated: {result:?}"),
            () = renamer.entered.notified() => {}
        }
        let error = app
            .reassign_episode_file(
                &actor,
                &second,
                std::slice::from_ref(&fixture.episodes[2].id),
                false,
            )
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            scryer_application::AppError::LocationOperationBusy(_)
        ));
        let independent = app
            .reassign_episode_file(
                &actor,
                &other_file,
                std::slice::from_ref(&other.episodes[1].id),
                false,
            )
            .await
            .unwrap_err();
        assert_eq!(
            independent.validation_reason(),
            Some("EPISODE_FILE_REASSIGN_RENAME_FAILED"),
            "another title reaches rename validation while the first is guarded"
        );
        assert!(other_path.exists());
        assert_eq!(
            file_links(&ctx, &other.title.id, &other_file).await,
            vec![(other.episodes[0].id.clone(), "primary".into())]
        );
        renamer.release.notify_one();
        let error = first_request.await.unwrap_err();
        assert_eq!(
            error.validation_reason(),
            Some("EPISODE_FILE_REASSIGN_RENAME_FAILED")
        );
        assert_eq!(
            file_links(&ctx, &fixture.title.id, &first).await,
            vec![(fixture.episodes[0].id.clone(), "primary".into())]
        );
        assert_eq!(
            file_links(&ctx, &fixture.title.id, &second).await,
            vec![(fixture.episodes[1].id.clone(), "primary".into())]
        );
        assert!(first_path.exists() && second_path.exists());
    })
    .await
    .expect("reassignment and compensation finish");
}

#[tokio::test]
async fn reassign_episode_file_renames_the_file_on_disk_and_records_history() {
    let ctx = TestContext::new().await;
    let fixture = reassign_fixture(&ctx, "Reassign Fixture", "97101").await;
    let (file_id, old_path) = seed_primary_file(
        &ctx,
        &fixture,
        "Reassign.Fixture.S01E01.1080p.WEB-DL.mkv",
        &fixture.episodes[0],
    )
    .await;
    let bytes = std::fs::read(&old_path).expect("read seeded file");
    let app = app_with_filesystem_renamer(&ctx);
    let actor = app.find_or_create_default_user().await.expect("actor");

    let result = app
        .reassign_episode_file(
            &actor,
            &file_id,
            std::slice::from_ref(&fixture.episodes[1].id),
            false,
        )
        .await
        .expect("reassign file");

    let new_path = std::path::PathBuf::from(&result.file.file_path);
    assert_eq!(result.previous_path, old_path.to_string_lossy());
    assert_ne!(new_path, old_path);
    assert!(!old_path.exists(), "the old path no longer exists");
    assert_eq!(std::fs::read(&new_path).expect("read renamed file"), bytes);
    let file_name = new_path
        .file_name()
        .and_then(|name| name.to_str())
        .expect("file name");
    assert!(
        file_name.contains("S01E02"),
        "renamed for the new episode: {file_name}"
    );
    assert!(
        !file_name.contains("S01E01"),
        "old episode gone from the name: {file_name}"
    );
    assert!(new_path.starts_with(fixture.season_dir.parent().expect("title folder")));
    assert_eq!(
        result.previous_episode_ids,
        vec![fixture.episodes[0].id.clone()]
    );
    assert_eq!(result.episode_ids, vec![fixture.episodes[1].id.clone()]);

    // The stored path is the one a fresh rename of the new episode renders.
    let plan = app
        .preview_rename_for_title(&actor, &fixture.title.id, MediaFacet::Series)
        .await
        .expect("rename preview");
    let item = plan
        .items
        .iter()
        .find(|item| item.media_file_id.as_deref() == Some(file_id.as_str()))
        .expect("plan item for the file");
    assert_eq!(item.current_path, result.file.file_path);
    assert_eq!(
        item.write_action,
        scryer_application::RenameWriteAction::Noop,
        "the file already sits at its rendered name: {:?}",
        item.proposed_path
    );

    let stored = ctx
        .media_files
        .get_media_file_by_id(&file_id)
        .await
        .expect("read file")
        .expect("file row");
    assert_eq!(stored.file_path, result.file.file_path);
    assert_eq!(stored.size_bytes, bytes.len() as i64);
    assert_eq!(
        file_links(&ctx, &fixture.title.id, &file_id).await,
        vec![(fixture.episodes[1].id.clone(), "primary".to_string())]
    );

    let history = gql(
        &ctx,
        r#"
        query TitleHistory($titleId: ID!) {
          titleHistory(filter: { titleIds: [$titleId], limit: 10 }) {
            items { eventType destPath }
          }
        }
        "#,
        json!({ "titleId": fixture.title.id }),
    )
    .await;
    assert_no_errors(&history);
    let items = history["data"]["titleHistory"]["items"]
        .as_array()
        .expect("history items");
    assert!(
        items.iter().any(|item| item["eventType"] == "rematched"
            && item["destPath"] == json!(result.file.file_path)),
        "reassignment recorded in title history: {items:?}"
    );

    let files = gql(
        &ctx,
        r#"
        query Title($id: ID!) {
          title(id: $id) { mediaFiles { id episodeId } }
        }
        "#,
        json!({ "id": fixture.title.id }),
    )
    .await;
    assert_no_errors(&files);
    let media_files = files["data"]["title"]["mediaFiles"]
        .as_array()
        .expect("media files");
    assert!(
        media_files.iter().any(|file| file["id"] == json!(file_id)
            && file["episodeId"] == json!(fixture.episodes[1].id))
    );

    // The reassignment leaves nothing behind that holds the links: a later
    // re-link pass may still re-derive them.
    let replaced = ctx
        .media_files
        .replace_file_episode_links(
            &file_id,
            std::slice::from_ref(&fixture.episodes[1].id),
            std::slice::from_ref(&fixture.episodes[0].id),
        )
        .await
        .expect("replace links");
    assert_eq!(
        replaced,
        scryer_application::EpisodeLinkReplacement::Replaced
    );
}

#[tokio::test]
async fn reassign_episode_file_with_replace_demotes_the_existing_primary() {
    let ctx = TestContext::new().await;
    let fixture = reassign_fixture(&ctx, "Replace Fixture", "97102").await;
    let (moving_id, moving_path) = seed_primary_file(
        &ctx,
        &fixture,
        "Replace.Fixture.S01E01.1080p.WEB-DL.mkv",
        &fixture.episodes[0],
    )
    .await;
    let (holder_id, holder_path) = seed_primary_file(
        &ctx,
        &fixture,
        "Replace.Fixture.S01E02.720p.HDTV.mkv",
        &fixture.episodes[1],
    )
    .await;
    let app = app_with_filesystem_renamer(&ctx);
    let actor = app.find_or_create_default_user().await.expect("actor");

    let result = app
        .reassign_episode_file(
            &actor,
            &moving_id,
            std::slice::from_ref(&fixture.episodes[1].id),
            true,
        )
        .await
        .expect("reassign with replace");

    assert_eq!(result.demoted_file_ids, vec![holder_id.clone()]);
    assert!(!moving_path.exists());
    assert!(std::path::Path::new(&result.file.file_path).exists());
    assert!(holder_path.exists(), "the demoted file is never deleted");
    assert_eq!(
        file_links(&ctx, &fixture.title.id, &moving_id).await,
        vec![(fixture.episodes[1].id.clone(), "primary".to_string())]
    );
    assert_eq!(
        file_links(&ctx, &fixture.title.id, &holder_id).await,
        vec![(fixture.episodes[1].id.clone(), "additional".to_string())]
    );
    let first_episode_files = ctx
        .media_files
        .list_media_files_for_title(&fixture.title.id)
        .await
        .expect("list files")
        .into_iter()
        .filter(|file| file.episode_id.as_deref() == Some(fixture.episodes[0].id.as_str()))
        .count();
    assert_eq!(
        first_episode_files, 0,
        "the episode the file left is now missing"
    );
}

#[tokio::test]
async fn graphql_reassign_episode_file_refuses_a_primary_conflict_with_typed_facts() {
    let ctx = TestContext::new().await;
    let fixture = reassign_fixture(&ctx, "Conflict Fixture", "97103").await;
    let (moving_id, moving_path) = seed_primary_file(
        &ctx,
        &fixture,
        "Conflict.Fixture.S01E01.1080p.WEB-DL.mkv",
        &fixture.episodes[0],
    )
    .await;
    let (holder_id, _) = seed_primary_file(
        &ctx,
        &fixture,
        "Conflict.Fixture.S01E02.1080p.WEB-DL.mkv",
        &fixture.episodes[1],
    )
    .await;

    let body = gql(
        &ctx,
        REASSIGN_MUTATION,
        json!({ "input": { "fileId": moving_id, "episodeIds": [fixture.episodes[1].id] } }),
    )
    .await;
    let extensions = &body["errors"][0]["extensions"];
    assert_eq!(extensions["code"], "VALIDATION_ERROR", "{body}");
    assert_eq!(
        extensions["reason"],
        "EPISODE_FILE_REASSIGN_PRIMARY_CONFLICT"
    );
    assert_eq!(extensions["conflictingFileId"], json!(holder_id));
    assert_eq!(extensions["episodeId"], json!(fixture.episodes[1].id));
    assert!(moving_path.exists());
    assert_eq!(
        file_links(&ctx, &fixture.title.id, &moving_id).await,
        vec![(fixture.episodes[0].id.clone(), "primary".to_string())]
    );
}

#[tokio::test]
async fn graphql_reassign_episode_file_refuses_an_episode_of_another_title() {
    let ctx = TestContext::new().await;
    let fixture = reassign_fixture(&ctx, "Owner Fixture", "97104").await;
    let (_, other_episodes) = series_with_three_episodes(&ctx, "Other Fixture", "97105").await;
    let (file_id, file_path) = seed_primary_file(
        &ctx,
        &fixture,
        "Owner.Fixture.S01E01.1080p.WEB-DL.mkv",
        &fixture.episodes[0],
    )
    .await;

    let body = gql(
        &ctx,
        REASSIGN_MUTATION,
        json!({ "input": { "fileId": file_id, "episodeIds": [other_episodes[1].id] } }),
    )
    .await;
    let extensions = &body["errors"][0]["extensions"];
    assert_eq!(
        extensions["reason"], "EPISODE_FILE_REASSIGN_EPISODE_OUTSIDE_TITLE",
        "{body}"
    );
    assert_eq!(extensions["episodeId"], json!(other_episodes[1].id));
    assert!(file_path.exists());
    assert_eq!(
        file_links(&ctx, &fixture.title.id, &file_id).await,
        vec![(fixture.episodes[0].id.clone(), "primary".to_string())]
    );
}

#[tokio::test]
async fn graphql_reassign_episode_file_puts_the_links_back_when_the_rename_fails() {
    // The served app has no library renamer configured, so the move fails
    // after the links have moved; the links must return to the old episode.
    let ctx = TestContext::new().await;
    let fixture = reassign_fixture(&ctx, "Rollback Fixture", "97106").await;
    let (file_id, file_path) = seed_primary_file(
        &ctx,
        &fixture,
        "Rollback.Fixture.S01E01.1080p.WEB-DL.mkv",
        &fixture.episodes[0],
    )
    .await;

    let body = gql(
        &ctx,
        REASSIGN_MUTATION,
        json!({ "input": { "fileId": file_id, "episodeIds": [fixture.episodes[2].id] } }),
    )
    .await;
    assert_eq!(
        body["errors"][0]["extensions"]["reason"], "EPISODE_FILE_REASSIGN_RENAME_FAILED",
        "{body}"
    );
    assert!(file_path.exists(), "the file stays where it was");
    let stored = ctx
        .media_files
        .get_media_file_by_id(&file_id)
        .await
        .expect("read file")
        .expect("file row");
    assert_eq!(stored.file_path, file_path.to_string_lossy());
    assert_eq!(
        file_links(&ctx, &fixture.title.id, &file_id).await,
        vec![(fixture.episodes[0].id.clone(), "primary".to_string())]
    );
}

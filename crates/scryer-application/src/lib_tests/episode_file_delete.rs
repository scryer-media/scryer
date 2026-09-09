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
    seed_episode_delete_fixture_for(MediaFacet::Series).await
}

async fn seed_episode_delete_fixture_for(facet: MediaFacet) -> EpisodeDeleteFixture {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root = tempdir.path().join("series");
    std::fs::create_dir_all(root.join("Emberfall/Season 01")).expect("create season folder");

    let media_files = Arc::new(MockMediaFileRepo::default());
    let (base_app, admin, _titles) = bootstrap_with_cutoff_projection_state(
        Arc::new(StoredSettingsRepo::default()),
        Arc::new(StoredQualityProfileRepo::default()),
        media_files.clone(),
    );
    // The deletion runs are read back through `list_job_runs`, so the fixture
    // needs a job-run repo that actually persists what the job writes.
    let job_runs = Arc::new(RecordingJobRunRepo::default());
    let app = base_app.with_test_overrides(|services| services.with_job_runs(job_runs.clone()));

    app.update_media_settings(
        &admin,
        facet.clone(),
        empty_update_media_settings_with_roots(vec![build_root_folder_entry(&root, true)]),
    )
    .await
    .expect("save series roots");

    let title = app
        .add_title(
            &admin,
            NewTitle {
                name: "Emberfall".into(),
                facet,
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

    fn run_summary(&self, run: &JobRun) -> serde_json::Value {
        serde_json::from_str(
            run.summary_json
                .as_deref()
                .expect("finished run should carry summary json"),
        )
        .expect("summary json should parse")
    }

    async fn deletion_runs(&self) -> Vec<JobRun> {
        self.app
            .list_job_runs(&self.admin, JobKey::MediaFileDeletion, 10)
            .await
            .expect("list media file deletion runs")
    }

    /// Poll the media-file deletion runs until `run_id` reaches a terminal
    /// status. Bounded so a job that never finishes fails fast instead of
    /// hanging the suite.
    async fn wait_for_run(&self, run_id: &str) -> JobRun {
        timeout(Duration::from_secs(5), async {
            loop {
                let run = self
                    .app
                    .list_job_runs(&self.admin, JobKey::MediaFileDeletion, 10)
                    .await
                    .expect("list media file deletion runs")
                    .into_iter()
                    .find(|run| run.id == run_id);
                if let Some(run) = run
                    && run.status.is_terminal()
                {
                    return run;
                }
                sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("episode file deletion job should reach a terminal status")
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
        .start_delete_episode_files_job(&fixture.admin, &fixture.title_id, &[], false, None)
        .await
        .expect_err("empty selection must be rejected before a run is created");
    assert!(matches!(error, AppError::Validation(_)), "{error:?}");
    assert!(fixture.deletion_runs().await.is_empty());
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
        .start_delete_episode_files_job(
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
    assert!(
        fixture.deletion_runs().await.is_empty(),
        "a rejected request must not create a job run"
    );
}

#[tokio::test]
async fn delete_episode_files_requires_confirmation_for_disk_deletes() {
    let fixture = seed_episode_delete_fixture().await;
    fixture
        .add_episode_file("Emberfall/Season 01/Emberfall - S01E01.mkv", "episode-1")
        .await;

    let error = fixture
        .app
        .start_delete_episode_files_job(
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
    assert!(
        fixture.deletion_runs().await.is_empty(),
        "a rejected request must not create a job run"
    );
}

#[tokio::test]
async fn delete_episode_files_rejects_a_busy_media_file_guard() {
    let fixture = seed_episode_delete_fixture().await;
    let first = fixture
        .add_episode_file("Emberfall/Season 01/Emberfall - S01E01.mkv", "episode-1")
        .await;
    let second = fixture
        .add_episode_file("Emberfall/Season 01/Emberfall - S01E02.mkv", "episode-2")
        .await;
    // Targets are ordered by file id, so holding the greater id guarantees the
    // batch takes the other file's guard before it hits the busy one — which is
    // what makes the release assertion below meaningful.
    let (other, busy) = if first < second {
        (first, second)
    } else {
        (second, first)
    };

    // Stand in for an in-flight single-file deletion job over the same file.
    let _held = fixture
        .app
        .runtime
        .jobs
        .interactive_operation_guards
        .try_acquire(&format!("media-file:{busy}"))
        .await
        .expect("hold the media file guard");

    let episode_ids = vec!["episode-1".to_string(), "episode-2".to_string()];
    let preview = fixture
        .app
        .preview_delete_episode_files(&fixture.admin, &fixture.title_id, &episode_ids)
        .await
        .expect("preview");

    let error = fixture
        .app
        .start_delete_episode_files_job(
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
        .expect_err("a busy media file must abort the whole batch");

    assert!(
        matches!(&error, AppError::Validation(message) if message.contains(&busy)),
        "expected a busy-file validation error, got {error:?}"
    );
    assert!(
        fixture.deletion_runs().await.is_empty(),
        "a rejected request must not create a job run"
    );
    // The guards taken before the busy one are released, so the other file is
    // free for a later request.
    assert!(
        fixture
            .app
            .runtime
            .jobs
            .interactive_operation_guards
            .try_acquire(&format!("media-file:{other}"))
            .await
            .is_some(),
        "guards taken before the rejection must be released"
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

    let accepted = fixture
        .app
        .start_delete_episode_files_job(
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
        .expect("start episode file deletion job");

    let mut accepted_ids = accepted.accepted_file_ids.clone();
    accepted_ids.sort();
    let mut expected = vec![first.clone(), second.clone()];
    expected.sort();
    assert_eq!(accepted_ids, expected);

    let run = fixture.wait_for_run(&accepted.job_run.id).await;
    assert_eq!(run.status, JobRunStatus::Completed, "run: {run:?}");
    assert_eq!(
        run.summary_text.as_deref(),
        Some("Deleted 2 of 2 episode files")
    );
    let summary = fixture.run_summary(&run);
    let mut summary_deleted = summary["deletedFileIds"]
        .as_array()
        .expect("deletedFileIds")
        .iter()
        .map(|value| value.as_str().expect("file id").to_string())
        .collect::<Vec<_>>();
    summary_deleted.sort();
    assert_eq!(summary_deleted, expected);
    assert!(
        summary["failed"].as_array().expect("failed").is_empty(),
        "unexpected failures: {summary:?}"
    );

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
        .insert_media_file_row(
            "/definitely/not/a/root/Emberfall - S01E02.mkv",
            Some("episode-2"),
        )
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

    let accepted = fixture
        .app
        .start_delete_episode_files_job(
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
        .expect("a file whose preview failed is still accepted into the run");
    assert_eq!(accepted.accepted_file_ids.len(), 2);

    let run = fixture.wait_for_run(&accepted.job_run.id).await;
    assert_eq!(run.status, JobRunStatus::Warning, "run: {run:?}");
    assert_eq!(
        run.summary_text.as_deref(),
        Some("Deleted 1 of 2 episode files")
    );
    assert!(
        run.error_text
            .as_deref()
            .is_some_and(|text| text.contains(&outside_root)),
        "error text should name the failed file: {:?}",
        run.error_text
    );
    let summary = fixture.run_summary(&run);
    assert_eq!(
        summary["deletedFileIds"]
            .as_array()
            .expect("deletedFileIds")
            .iter()
            .map(|value| value.as_str().expect("file id"))
            .collect::<Vec<_>>(),
        vec![deletable.as_str()]
    );
    let failed = summary["failed"].as_array().expect("failed");
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0]["fileId"].as_str(), Some(outside_root.as_str()));

    assert_eq!(fixture.remaining_file_ids().await, vec![outside_root]);
}

// Maintainerr af02e66c: explicit scope, fresh reads, confirmed unmonitoring,
// independent files, and partial failure evidence.
struct ScopedMaintenanceFixture {
    fixture: EpisodeDeleteFixture,
    evaluation: Arc<crate::lib_tests::maintenance_evaluation::InMemoryMaintenanceEvaluationRepo>,
    shows: Arc<MockShowRepo>,
    seasons: Vec<Collection>,
    episodes: Vec<Episode>,
}

async fn scoped_maintenance_fixture() -> ScopedMaintenanceFixture {
    scoped_maintenance_fixture_for(MediaFacet::Series).await
}

async fn scoped_maintenance_fixture_for(facet: MediaFacet) -> ScopedMaintenanceFixture {
    use crate::lib_tests::maintenance_evaluation::InMemoryMaintenanceEvaluationRepo;
    use crate::lib_tests::maintenance_rules::InMemoryMaintenanceRuleRepo;
    let mut fixture = seed_episode_delete_fixture_for(facet).await;
    let evaluation = Arc::new(InMemoryMaintenanceEvaluationRepo::default());
    let shows = Arc::new(MockShowRepo::default());
    fixture.app = fixture.app.with_test_overrides(|services| {
        services
            .with_shows(shows.clone())
            .with_maintenance_rule_set_store(Arc::new(InMemoryMaintenanceRuleRepo::default()))
            .with_maintenance_evaluation_store(evaluation.clone())
    });
    let mut seasons = Vec::new();
    let mut episodes = Vec::new();
    for season in 0..=2 {
        let collection = fixture
            .app
            .create_collection(
                &fixture.admin,
                fixture.title_id.clone(),
                if season == 0 { "specials" } else { "season" }.into(),
                season.to_string(),
                None,
                None,
                None,
                None,
            )
            .await
            .expect("season");
        for number in 1..=2 {
            let episode = fixture
                .app
                .create_episode(
                    &fixture.admin,
                    fixture.title_id.clone(),
                    Some(collection.id.clone()),
                    "standard".into(),
                    Some(number.to_string()),
                    Some(season.to_string()),
                    None,
                    Some(format!("Episode {number}")),
                    Some("2020-01-01".into()),
                    None,
                    false,
                    false,
                )
                .await
                .expect("episode");
            fixture
                .add_episode_file(
                    &format!(
                        "Emberfall/Season {season:02}/Emberfall - S{season:02}E{number:02}.mkv"
                    ),
                    &episode.id,
                )
                .await;
            episodes.push(episode);
        }
        seasons.push(collection);
    }
    ScopedMaintenanceFixture {
        fixture,
        evaluation,
        shows,
        seasons,
        episodes,
    }
}

impl ScopedMaintenanceFixture {
    async fn rule(
        &self,
        scope: scryer_domain::MaintenanceRuleSubjectKind,
        matcher: String,
        grace: i64,
    ) -> String {
        use crate::maintenance_rules::*;
        let app = &self.fixture.app;
        let actor = &self.fixture.admin;
        let rule = app
            .create_maintenance_rule_set(
                actor,
                MaintenanceRuleDraft {
                    subject_kind: scope,
                    name: "Scoped deletion".into(),
                    description: String::new(),
                    rego_source: matcher,
                    action_definition:
                        crate::maintenance_rules::MaintenanceActionDefinition::Legacy(
                            MaintenanceActionSpec::new(
                                MaintenanceActionKind::UnmonitorScopeDeleteFiles,
                            ),
                        ),
                    grace_days: grace,
                    storage_root_id: None,
                    library_ids: vec![],
                    evaluation_mode: None,
                },
            )
            .await
            .expect("create scoped rule");
        assert!(!rule.rule_set.enabled);
        assert_eq!(
            rule.rule_set.effect_arming,
            scryer_domain::MaintenanceEffectArming::None
        );
        app.set_maintenance_rule_evaluation_mode(
            actor,
            &rule.rule_set.id,
            scryer_domain::MaintenanceEvaluationMode::Observe,
        )
        .await
        .expect("observe");
        app.set_maintenance_instance_gates(
            actor,
            MaintenanceGatesUpdate {
                evaluation_enabled: Some(true),
                destructive_effects_enabled: Some(true),
                result_display_enabled: Some(true),
                ..Default::default()
            },
        )
        .await
        .expect("gates");
        app.run_maintenance_rule_evaluation_job()
            .await
            .expect("evaluate");
        let count = self
            .evaluation
            .all_candidates()
            .await
            .iter()
            .filter(|candidate| {
                candidate.rule_set_id == rule.rule_set.id && !candidate.state.is_terminal()
            })
            .count();
        app.set_maintenance_rule_arming(
            actor,
            &rule.rule_set.id,
            scryer_domain::MaintenanceEffectArming::Destructive,
            Some(count as i64),
        )
        .await
        .expect("arm");
        rule.rule_set.id
    }
}

#[tokio::test]
async fn maintenance_scope_episode_deletion_preserves_siblings_and_records() {
    use crate::maintenance_rules::{
        MaintenancePreviewMatcher, MaintenancePreviewRequest, MaintenancePreviewSelection,
    };
    let setup = scoped_maintenance_fixture().await;
    let f = &setup.fixture;
    let target = &setup.episodes[3]; // S01E02, never S02E02.
    let extra = f
        .add_episode_file("Emberfall/Season 01/Emberfall - S01E02.v2.mkv", &target.id)
        .await;
    let rule = setup.rule(scryer_domain::MaintenanceRuleSubjectKind::Episode,
        format!("match if {{ input.subject.episode_id == {:?}; input.facts.monitored; input.facts.has_file }}", target.id), 0).await;
    let preview = f
        .app
        .preview_maintenance_rule(
            &f.admin,
            MaintenancePreviewRequest {
                matcher: MaintenancePreviewMatcher::Stored { rule_set_id: rule },
                selection: MaintenancePreviewSelection::Titles(vec![f.title_id.clone()]),
            },
        )
        .await
        .expect("preview");
    let matched = preview
        .titles
        .iter()
        .find(|row| row.subject_id == target.id)
        .expect("target preview");
    assert_eq!(matched.file_count, 2);
    assert_eq!(matched.title_id, f.title_id);
    let report = f
        .app
        .run_lifecycle_action_handling_job()
        .await
        .expect("execute");
    assert_eq!(report.executed, 1, "{report:?}");
    assert!(!f.remaining_file_ids().await.contains(&extra));
    assert_eq!(f.remaining_file_ids().await.len(), 5);
    assert!(
        f.root
            .join("Emberfall/Season 02/Emberfall - S02E02.mkv")
            .exists()
    );
    assert!(
        f.root
            .join("Emberfall/Season 01/Emberfall - S01E01.mkv")
            .exists()
    );
    assert!(
        f.app
            .services
            .catalog
            .titles
            .get_by_id(&f.title_id)
            .await
            .unwrap()
            .unwrap()
            .monitored
    );
    assert_eq!(setup.shows.episodes.lock().await.len(), 6);
    assert_eq!(setup.shows.collections.lock().await.len(), 3);
    assert!(
        !f.app
            .services
            .catalog
            .shows
            .get_episode_by_id(&target.id)
            .await
            .unwrap()
            .unwrap()
            .monitored
    );
    let runs = setup.evaluation.all_action_runs().await;
    assert_eq!(runs.len(), 1);
    let detail: serde_json::Value = serde_json::from_str(&runs[0].detail).unwrap();
    assert_eq!(detail["files"].as_array().unwrap().len(), 2);
    assert!(
        detail["files"]
            .as_array()
            .unwrap()
            .iter()
            .all(|file| file["completed"] == true)
    );
}

#[tokio::test]
async fn maintenance_scope_season_deletion_preserves_other_seasons_and_unmonitors_future() {
    let setup = scoped_maintenance_fixture().await;
    let f = &setup.fixture;
    setup.shows.episodes.lock().await[3].air_date = Some("2099-01-01".into());
    setup
        .rule(
            scryer_domain::MaintenanceRuleSubjectKind::Season,
            "match if { input.subject.season_number == 1; input.facts.monitored }".into(),
            0,
        )
        .await;
    let report = f.app.run_lifecycle_action_handling_job().await.unwrap();
    assert_eq!(report.executed, 1, "{report:?}");
    assert_eq!(f.remaining_file_ids().await.len(), 4);
    let episodes = setup.shows.episodes.lock().await;
    assert!(
        episodes
            .iter()
            .filter(|episode| episode.season_number.as_deref() != Some("1"))
            .all(|episode| episode.monitored)
    );
    assert!(
        episodes
            .iter()
            .filter(|episode| episode.season_number.as_deref() == Some("1"))
            .all(|episode| !episode.monitored)
    );
    assert!(
        f.root
            .join("Emberfall/Season 02/Emberfall - S02E02.mkv")
            .exists()
    );
    assert!(f.root.join("Emberfall/Season 01").is_dir());
    assert_eq!(setup.shows.collections.lock().await.len(), 3);
}

#[tokio::test]
async fn maintenance_scope_specials_zero_and_inherited_exclusions() {
    let setup = scoped_maintenance_fixture().await;
    let f = &setup.fixture;
    let rule = setup
        .rule(
            scryer_domain::MaintenanceRuleSubjectKind::Episode,
            "match := true".into(),
            0,
        )
        .await;
    f.app
        .exclude_maintenance_scoped_subject(
            &f.admin,
            &f.title_id,
            scryer_domain::MaintenanceRuleSubjectKind::Season,
            &setup.seasons[0].id,
            Some(rule),
            None,
        )
        .await
        .unwrap();
    let report = f.app.run_lifecycle_action_handling_job().await.unwrap();
    assert_eq!(report.executed, 4, "{report:?}");
    assert_eq!(f.remaining_file_ids().await.len(), 2);
    assert!(
        setup.shows.episodes.lock().await[..2]
            .iter()
            .all(|episode| episode.monitored)
    );
}

#[tokio::test]
async fn maintenance_scope_shared_episode_file_is_held_before_unmonitoring() {
    let setup = scoped_maintenance_fixture().await;
    let f = &setup.fixture;
    let mut shared = f.media_files.store.lock().await[2].clone();
    shared.episode_id = Some(setup.episodes[4].id.clone());
    f.media_files.store.lock().await.push(shared);
    setup
        .rule(
            scryer_domain::MaintenanceRuleSubjectKind::Episode,
            format!(
                "match if {{ input.subject.episode_id == {:?} }}",
                setup.episodes[2].id
            ),
            0,
        )
        .await;
    let report = f.app.run_lifecycle_action_handling_job().await.unwrap();
    assert_eq!(report.executed, 0);
    assert_eq!(report.held, 1, "{report:?}");
    assert!(setup.shows.episodes.lock().await[2].monitored);
    assert!(
        f.root
            .join("Emberfall/Season 01/Emberfall - S01E01.mkv")
            .exists()
    );
}

#[tokio::test]
async fn maintenance_scope_missing_episode_never_falls_back_to_show() {
    let setup = scoped_maintenance_fixture().await;
    let f = &setup.fixture;
    setup
        .rule(
            scryer_domain::MaintenanceRuleSubjectKind::Episode,
            format!(
                "match if {{ input.subject.episode_id == {:?} }}",
                setup.episodes[3].id
            ),
            0,
        )
        .await;
    setup
        .shows
        .episodes
        .lock()
        .await
        .retain(|episode| episode.id != setup.episodes[3].id);
    let report = f.app.run_lifecycle_action_handling_job().await.unwrap();
    assert_eq!(report.executed, 0);
    assert_eq!(f.remaining_file_ids().await.len(), 6);
    assert!(
        f.app
            .services
            .catalog
            .titles
            .get_by_id(&f.title_id)
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn maintenance_scope_failed_unmonitor_preserves_files_and_retry_rechecks_them() {
    let setup = scoped_maintenance_fixture().await;
    let f = &setup.fixture;
    setup
        .rule(
            scryer_domain::MaintenanceRuleSubjectKind::Episode,
            format!(
                "match if {{ input.subject.episode_id == {:?}; input.facts.monitored }}",
                setup.episodes[3].id
            ),
            0,
        )
        .await;
    *setup.shows.fail_monitoring.lock().await = true;
    let first = f.app.run_lifecycle_action_handling_job().await.unwrap();
    assert_eq!(first.executed, 0);
    assert_eq!(f.remaining_file_ids().await.len(), 6);
    assert!(setup.shows.episodes.lock().await[3].monitored);
    let runs = setup.evaluation.all_action_runs().await;
    assert!(
        runs[0]
            .error
            .as_deref()
            .unwrap()
            .contains("monitoring failure")
    );
    *setup.shows.fail_monitoring.lock().await = false;
    let report = f.app.run_lifecycle_action_handling_job().await.unwrap();
    assert_eq!(report.executed, 1, "{report:?}");
    assert_eq!(f.remaining_file_ids().await.len(), 5);
}

#[tokio::test]
async fn maintenance_scope_changed_file_is_preserved_on_retry() {
    let setup = scoped_maintenance_fixture().await;
    let f = &setup.fixture;
    setup
        .rule(
            scryer_domain::MaintenanceRuleSubjectKind::Episode,
            format!(
                "match if {{ input.subject.episode_id == {:?} }}",
                setup.episodes[3].id
            ),
            0,
        )
        .await;
    *setup.shows.fail_monitoring.lock().await = true;
    f.app.run_lifecycle_action_handling_job().await.unwrap();
    *setup.shows.fail_monitoring.lock().await = false;
    append_maintenance_safety_holds(&setup).await;
    let path = f.root.join("Emberfall/Season 01/Emberfall - S01E02.mkv");
    std::fs::write(&path, b"new replacement version").unwrap();
    let report = f.app.run_lifecycle_action_handling_job().await.unwrap();
    assert_eq!(report.executed, 0);
    assert_eq!(std::fs::read(path).unwrap(), b"new replacement version");
    let runs = setup.evaluation.all_action_runs().await;
    assert!(runs.iter().any(|run| run.detail.contains("file changed")));
}

async fn append_maintenance_safety_holds(setup: &ScopedMaintenanceFixture) {
    use crate::LifecycleActionRunRepository;
    let checkpoint_run = setup.evaluation.all_action_runs().await.remove(0);
    for index in 0..101 {
        let mut held = checkpoint_run.clone();
        held.id = format!("later-hold-{index}");
        held.idempotency_key = format!("hold:{}", held.id);
        held.status = scryer_domain::LifecycleActionRunStatus::Held;
        held.hold_reason = Some("active_playback".into());
        held.error = None;
        held.detail = "{}".into();
        held.started_at = checkpoint_run.started_at + chrono::Duration::microseconds(index + 1);
        held.finished_at = Some(held.started_at);
        setup.evaluation.start_action_run(&held).await.unwrap();
    }
    assert_eq!(
        setup.evaluation.all_candidates().await[0].action_attempts,
        1,
        "safety holds do not spend mutation attempts"
    );
}

#[tokio::test]
async fn maintenance_scope_partial_preview_failure_records_each_file_and_preserves_shared_sidecars()
{
    let setup = scoped_maintenance_fixture().await;
    let f = &setup.fixture;
    let outside = tempfile::tempdir().unwrap();
    let outside_path = outside.path().join("outside.mkv");
    std::fs::write(&outside_path, b"outside").unwrap();
    let rejected = f
        .insert_media_file_row(&outside_path.to_string_lossy(), Some(&setup.episodes[3].id))
        .await;
    let sidecar = f.root.join("Emberfall/Season 01/season.nfo");
    std::fs::write(&sidecar, b"shared season metadata").unwrap();
    setup
        .rule(
            scryer_domain::MaintenanceRuleSubjectKind::Episode,
            format!(
                "match if {{ input.subject.episode_id == {:?} }}",
                setup.episodes[3].id
            ),
            0,
        )
        .await;
    let report = f.app.run_lifecycle_action_handling_job().await.unwrap();
    assert_eq!(report.executed, 0);
    assert!(outside_path.exists());
    assert!(sidecar.exists());
    assert!(
        !f.root
            .join("Emberfall/Season 01/Emberfall - S01E02.mkv")
            .exists()
    );
    assert!(f.remaining_file_ids().await.contains(&rejected));
    let runs = setup.evaluation.all_action_runs().await;
    let detail: serde_json::Value = serde_json::from_str(&runs[0].detail).unwrap();
    let outcomes = detail["files"].as_array().unwrap();
    assert_eq!(
        outcomes
            .iter()
            .filter(|file| file["completed"] == true)
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|file| file["error"].is_string())
            .count(),
        1
    );
}

#[tokio::test]
async fn maintenance_scope_crash_after_unmonitor_and_disk_removal_resumes_catalog_cleanup() {
    let setup = scoped_maintenance_fixture().await;
    let f = &setup.fixture;
    setup.rule(scryer_domain::MaintenanceRuleSubjectKind::Episode,
        format!("match if {{ input.subject.episode_id == {:?}; input.facts.monitored; input.facts.has_file }}", setup.episodes[3].id), 0).await;
    *setup.shows.fail_monitoring.lock().await = true;
    f.app.run_lifecycle_action_handling_job().await.unwrap();
    *setup.shows.fail_monitoring.lock().await = false;
    f.app
        .set_episode_monitored(&f.admin, &setup.episodes[3].id, false)
        .await
        .unwrap();
    // Crash window: the approved video was removed, but its catalog row and
    // completion flag were not persisted. The next lease uses the saved plan.
    std::fs::remove_file(f.root.join("Emberfall/Season 01/Emberfall - S01E02.mkv")).unwrap();
    let candidate = setup.evaluation.all_candidates().await.remove(0);
    append_maintenance_safety_holds(&setup).await;
    setup
        .evaluation
        .strand_as_executing(&candidate.id, Utc::now() - chrono::Duration::hours(2))
        .await;
    f.app.run_maintenance_rule_evaluation_job().await.unwrap();
    let report = f.app.run_lifecycle_action_handling_job().await.unwrap();
    assert_eq!(report.executed, 1, "{report:?}");
    assert_eq!(f.remaining_file_ids().await.len(), 5);
    assert_eq!(setup.evaluation.all_candidates().await.len(), 1);
}

#[tokio::test]
async fn maintenance_scope_preview_targets_are_exact_and_sibling_grace_is_independent() {
    use crate::maintenance_rules::{
        MaintenancePreviewMatcher, MaintenancePreviewRequest, MaintenancePreviewSelection,
    };
    let setup = scoped_maintenance_fixture().await;
    let f = &setup.fixture;
    setup.shows.episodes.lock().await[3].monitored = false;
    let rule = setup
        .rule(
            scryer_domain::MaintenanceRuleSubjectKind::Episode,
            "match if { input.subject.season_number == 1; input.facts.monitored }".into(),
            7,
        )
        .await;
    let first = setup.evaluation.all_candidates().await.remove(0);
    setup.shows.episodes.lock().await[3].monitored = true;
    f.app.run_maintenance_rule_evaluation_job().await.unwrap();
    let rows = setup.evaluation.all_candidates().await;
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows.iter()
            .find(|row| row.id == first.id)
            .unwrap()
            .first_matched_at,
        first.first_matched_at
    );
    assert_eq!(
        rows.iter().find(|row| row.id == first.id).unwrap().due_at,
        first.due_at
    );
    let sibling = rows.iter().find(|row| row.id != first.id).unwrap();
    assert_eq!(sibling.match_generation, 1);
    assert!(sibling.first_matched_at > first.first_matched_at);
    let preview = f
        .app
        .preview_maintenance_rule(
            &f.admin,
            MaintenancePreviewRequest {
                matcher: MaintenancePreviewMatcher::Stored {
                    rule_set_id: rule.clone(),
                },
                selection: MaintenancePreviewSelection::Subjects {
                    title_ids: vec![f.title_id.clone()],
                    subject_ids: vec![setup.episodes[3].id.clone()],
                },
            },
        )
        .await
        .unwrap();
    assert_eq!(preview.titles.len(), 1);
    assert_eq!(preview.titles[0].subject_id, setup.episodes[3].id);
    assert_eq!(preview.titles[0].due_at, Some(sibling.due_at));
    assert!(!preview.titles[0].due_at_is_estimate);
    f.app
        .exclude_maintenance_scoped_subject(
            &f.admin,
            &f.title_id,
            scryer_domain::MaintenanceRuleSubjectKind::Season,
            &setup.seasons[1].id,
            Some(rule.clone()),
            None,
        )
        .await
        .unwrap();
    let excluded = f
        .app
        .preview_maintenance_rule(
            &f.admin,
            MaintenancePreviewRequest {
                matcher: MaintenancePreviewMatcher::Stored {
                    rule_set_id: rule.clone(),
                },
                selection: MaintenancePreviewSelection::Subjects {
                    title_ids: vec![f.title_id.clone()],
                    subject_ids: vec![setup.episodes[3].id.clone()],
                },
            },
        )
        .await
        .unwrap();
    assert!(excluded.titles[0].excluded);
    assert!(excluded.titles[0].due_at.is_none());
    let missing = f
        .app
        .preview_maintenance_rule(
            &f.admin,
            MaintenancePreviewRequest {
                matcher: MaintenancePreviewMatcher::Stored { rule_set_id: rule },
                selection: MaintenancePreviewSelection::Subjects {
                    title_ids: vec![f.title_id.clone()],
                    subject_ids: vec!["missing".into()],
                },
            },
        )
        .await;
    assert!(missing.is_err());
}

#[tokio::test]
async fn maintenance_scope_anime_specials_and_missing_files_keep_parent_records() {
    let setup = scoped_maintenance_fixture_for(MediaFacet::Anime).await;
    let f = &setup.fixture;
    setup.shows.episodes.lock().await[0].absolute_number = Some("100".into());
    std::fs::remove_file(f.root.join("Emberfall/Season 00/Emberfall - S00E01.mkv")).unwrap();
    setup
        .rule(
            scryer_domain::MaintenanceRuleSubjectKind::Season,
            "match if { input.subject.season_number == 0 }".into(),
            0,
        )
        .await;
    let report = f.app.run_lifecycle_action_handling_job().await.unwrap();
    assert_eq!(report.executed, 1, "{report:?}");
    assert_eq!(f.remaining_file_ids().await.len(), 4);
    assert_eq!(setup.shows.episodes.lock().await.len(), 6);
    assert_eq!(setup.shows.collections.lock().await.len(), 3);
    assert!(
        f.app
            .services
            .catalog
            .titles
            .get_by_id(&f.title_id)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        f.root
            .join("Emberfall/Season 01/Emberfall - S01E02.mkv")
            .exists()
    );
}

#[tokio::test]
async fn maintenance_scope_season_failed_propagation_preserves_every_file() {
    let setup = scoped_maintenance_fixture().await;
    let f = &setup.fixture;
    setup
        .rule(
            scryer_domain::MaintenanceRuleSubjectKind::Season,
            "match if { input.subject.season_number == 1 }".into(),
            0,
        )
        .await;
    *setup.shows.fail_monitoring.lock().await = true;
    let report = f.app.run_lifecycle_action_handling_job().await.unwrap();
    assert_eq!(report.executed, 0);
    assert_eq!(f.remaining_file_ids().await.len(), 6);
    assert!(
        setup.shows.episodes.lock().await[2..4]
            .iter()
            .all(|episode| episode.monitored)
    );
    assert!(
        f.root
            .join("Emberfall/Season 01/Emberfall - S01E02.mkv")
            .exists()
    );
}

#[tokio::test]
async fn maintenance_scope_overlapping_episode_and_season_rules_preserve_other_seasons() {
    let setup = scoped_maintenance_fixture().await;
    let f = &setup.fixture;
    setup
        .rule(
            scryer_domain::MaintenanceRuleSubjectKind::Episode,
            format!(
                "match if {{ input.subject.episode_id == {:?}; input.facts.monitored }}",
                setup.episodes[3].id
            ),
            0,
        )
        .await;
    setup
        .rule(
            scryer_domain::MaintenanceRuleSubjectKind::Season,
            "match if { input.subject.season_number == 1; input.facts.monitored }".into(),
            0,
        )
        .await;
    f.app.run_lifecycle_action_handling_job().await.unwrap();
    assert_eq!(f.remaining_file_ids().await.len(), 4);
    assert!(
        f.root
            .join("Emberfall/Season 02/Emberfall - S02E02.mkv")
            .exists()
    );
    assert!(
        f.app
            .services
            .catalog
            .titles
            .get_by_id(&f.title_id)
            .await
            .unwrap()
            .is_some()
    );
    let completed: Vec<String> = setup
        .evaluation
        .all_action_runs()
        .await
        .into_iter()
        .flat_map(|run| {
            let detail: serde_json::Value = serde_json::from_str(&run.detail).unwrap();
            detail["files"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|file| file["completed"] == true)
                .map(|file| file["file_id"].as_str().unwrap().to_owned())
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(completed.len(), 2);
    assert_eq!(completed.iter().collect::<HashSet<_>>().len(), 2);
}

#[tokio::test]
async fn maintenance_scope_newly_discovered_future_episodes_inherit_unmonitored_season() {
    let setup = scoped_maintenance_fixture().await;
    let f = &setup.fixture;
    setup
        .rule(
            scryer_domain::MaintenanceRuleSubjectKind::Season,
            "match if { input.subject.season_number == 1 }".into(),
            0,
        )
        .await;
    assert_eq!(
        f.app
            .run_lifecycle_action_handling_job()
            .await
            .unwrap()
            .executed,
        1
    );
    let title = f
        .app
        .services
        .catalog
        .titles
        .get_by_id(&f.title_id)
        .await
        .unwrap()
        .unwrap();
    let seasons = vec![SeasonMetadata {
        tvdb_id: 101,
        number: 1,
        label: "Season 1".into(),
        episode_type: "official".into(),
    }];
    let episodes = vec![EpisodeMetadata {
        tvdb_id: 10103,
        episode_number: 3,
        name: "Future episode".into(),
        aired: "2099-01-01".into(),
        runtime_minutes: 24,
        is_filler: false,
        is_recap: false,
        overview: String::new(),
        absolute_number: "3".into(),
        season_number: 1,
        image_url: String::new(),
    }];
    f.app
        .create_series_seasons_and_episodes(&title, &seasons, &episodes, &[], &[])
        .await;
    let episodes = setup.shows.episodes.lock().await;
    let future = episodes
        .iter()
        .find(|episode| episode.tvdb_id.as_deref() == Some("10103"))
        .expect("new episode");
    assert!(!future.monitored);
    assert_eq!(future.collection_id.as_ref(), Some(&setup.seasons[1].id));
    assert!(title.monitored);
}

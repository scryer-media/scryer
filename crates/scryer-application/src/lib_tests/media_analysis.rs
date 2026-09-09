use super::*;

struct SelectionAnalyzer {
    seen: Arc<Mutex<Vec<scryer_media_types::DiscSelection>>>,
    started: Arc<tokio::sync::Notify>,
    resume: Option<Arc<tokio::sync::Notify>>,
}

#[async_trait]
impl MediaAnalyzer for SelectionAnalyzer {
    async fn analyze_file(&self, path: PathBuf) -> AppResult<MediaAnalysisOutcome> {
        self.analyze_file_with_selection(path, Default::default())
            .await
    }
    async fn analyze_file_with_selection(
        &self,
        _path: PathBuf,
        selection: scryer_media_types::DiscSelection,
    ) -> AppResult<MediaAnalysisOutcome> {
        self.seen.lock().await.push(selection.clone());
        self.started.notify_one();
        if let Some(resume) = &self.resume {
            resume.notified().await;
        }
        let selected = selection.title_id.clone().unwrap_or_else(|| "00001".into());
        let mut analysis = MediaFileAnalysis::default();
        analysis.details.revision = scryer_media_types::ANALYSIS_REVISION;
        analysis.details.disc = Some(scryer_media_types::DiscMetadata {
            disc_type: "bluray".into(),
            selected_title_id: Some(selected),
            titles: [("00001", 7200.0), ("00002", 1800.0)]
                .into_iter()
                .map(|(id, duration)| scryer_media_types::DiscTitle {
                    id: id.into(),
                    duration_seconds: Some(duration),
                    report: scryer_media_types::ProbeReport {
                        status: scryer_media_types::ProbeStatus::Complete,
                        ..Default::default()
                    },
                    ..Default::default()
                })
                .collect(),
            automatic_selection: selection.title_id.is_none(),
            selection,
            ..Default::default()
        });
        analysis.duration_seconds = Some(7200);
        Ok(MediaAnalysisOutcome::Valid(Box::new(analysis)))
    }
}

async fn selection_fixture(
    analyzer: Arc<dyn MediaAnalyzer>,
) -> (
    AppUseCase,
    User,
    Arc<MockMediaFileRepo>,
    String,
    tempfile::TempDir,
) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Movie.iso");
    std::fs::write(&path, b"disc fixture").unwrap();
    let files = Arc::new(MockMediaFileRepo::default());
    let (base, admin, _) = bootstrap_with_cutoff_projection_state(
        Arc::new(StoredSettingsRepo::default()),
        Arc::new(StoredQualityProfileRepo::default()),
        files.clone(),
    );
    let app = base.with_test_overrides(|services| services.with_media_analyzer(analyzer));
    let title = app
        .add_title(
            &admin,
            NewTitle {
                name: "Disc Selection".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let id = files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id,
            file_path: path.to_string_lossy().into_owned(),
            size_bytes: 12,
            ..Default::default()
        })
        .await
        .unwrap();
    (app, admin, files, id, dir)
}

#[tokio::test]
async fn disc_review_does_not_supply_an_incumbent_or_displayed_landed_score() {
    let analyzer = Arc::new(SelectionAnalyzer {
        seen: Arc::new(Mutex::new(Vec::new())),
        started: Arc::new(tokio::sync::Notify::new()),
        resume: None,
    });
    let (app, _admin, files, id, _dir) = selection_fixture(analyzer).await;
    let file = files.get_media_file_by_id(&id).await.unwrap().unwrap();
    let title = app
        .services
        .catalog
        .titles
        .get_by_id(&file.title_id)
        .await
        .unwrap()
        .unwrap();
    let profile = app.resolve_quality_profile_for_title(&title).await.unwrap();
    let context = app
        .resolve_canonical_scoring_context(&title, &profile)
        .await;
    for (status, count) in [("scanned", 1), ("review_required", 0), ("scanned", 1)] {
        files
            .store
            .lock()
            .await
            .iter_mut()
            .find(|row| row.id == id)
            .unwrap()
            .scan_status = status.into();
        let subject = app
            .admission_subject_for_scope(
                &title,
                &crate::SubmissionScope::Title,
                &context,
                None,
                crate::quality::canonical_context::SubjectIntent::Import,
            )
            .await;
        assert_eq!(subject.incumbents().len(), count, "{status}");
        if status == "review_required" {
            let bars = app
                .landed_bars_for_scopes(&[crate::acquisition_workflow::LandedBarScope {
                    title_id: title.id.clone(),
                    episode_id: None,
                    collection_id: None,
                    series_movie_link_id: None,
                }])
                .await;
            assert_eq!(bars, vec![None]);
        }
    }
}

#[tokio::test]
async fn disc_episode_mapping_validates_each_authored_runtime_and_episode_owner() {
    let analyzer = Arc::new(SelectionAnalyzer {
        seen: Arc::new(Mutex::new(Vec::new())),
        started: Arc::new(tokio::sync::Notify::new()),
        resume: None,
    });
    let (app, admin, files, id, _dir) = selection_fixture(analyzer).await;
    let series = app
        .add_title(
            &admin,
            NewTitle {
                name: "Disc Episodes".into(),
                facet: MediaFacet::Series,
                monitored: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    files
        .store
        .lock()
        .await
        .iter_mut()
        .find(|row| row.id == id)
        .unwrap()
        .title_id = series.id.clone();
    app.services
        .catalog
        .shows
        .create_episode(Episode {
            id: "mapped-episode".into(),
            title_id: series.id.clone(),
            collection_id: None,
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("1".into()),
            season_number: Some("1".into()),
            episode_label: None,
            title: None,
            air_date: None,
            duration_seconds: Some(1800),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            image_url: None,
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .unwrap();
    let mapping = |disc_id: &str, episode_id: &str| scryer_media_types::DiscEpisodeMapping {
        disc_title_id: disc_id.into(),
        episode_ids: vec![episode_id.into()],
    };
    assert!(
        app.map_media_file_disc_episodes(&admin, &id, vec![mapping("00001", "mapped-episode")])
            .await
            .is_err(),
        "longest title cannot satisfy a short episode"
    );
    assert!(
        app.map_media_file_disc_episodes(&admin, &id, vec![mapping("00002", "unknown-episode")])
            .await
            .is_err()
    );
    assert!(
        files
            .get_media_file_by_id(&id)
            .await
            .unwrap()
            .unwrap()
            .analysis_details
            .disc
            .is_none()
    );
    let saved = app
        .map_media_file_disc_episodes(&admin, &id, vec![mapping("00002", "mapped-episode")])
        .await
        .unwrap();
    assert_eq!(
        saved
            .analysis_details
            .disc
            .as_ref()
            .unwrap()
            .selection
            .episode_mappings,
        vec![mapping("00002", "mapped-episode")]
    );
    assert_eq!(saved.size_bytes, 12);
    let reprobe = app
        .analyze_catalogued_media_file(Some(&id), PathBuf::from(saved.file_path))
        .await
        .unwrap();
    let MediaAnalysisOutcome::Valid(reprobed) = reprobe.outcome else {
        panic!("expected saved mapping to survive refresh")
    };
    assert_eq!(
        reprobed.details.disc.unwrap().selection.episode_mappings,
        vec![mapping("00002", "mapped-episode")]
    );
    let mut alias_disc = saved.analysis_details.disc.clone().unwrap();
    alias_disc
        .titles
        .iter_mut()
        .find(|item| item.id == "00002")
        .unwrap()
        .aliases
        .push("00099".into());
    alias_disc.selection.episode_mappings = vec![
        mapping("00099", "mapped-episode"),
        mapping("00003", "missing-episode"),
    ];
    let pending_selection = alias_disc.selection.clone();
    assert!(
        app.validate_disc_episode_mappings(&series, &mut alias_disc)
            .await
            .is_err()
    );
    assert_eq!(
        alias_disc.selection, pending_selection,
        "failed validation must not partially canonicalize the saved choice"
    );
    app.services
        .catalog
        .shows
        .update_episode(
            "mapped-episode",
            EpisodeUpdate {
                duration_seconds: Some(7200),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let before = files.get_media_file_by_id(&id).await.unwrap().unwrap();
    let error = app
        .select_media_file_disc_title(&admin, &id, Some("00001".into()))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("implausible"), "{error}");
    let attempts = files.analysis_attempts.lock().await;
    let (_, attempt) = attempts
        .last()
        .expect("failed mapping validation is recorded");
    assert_eq!(
        attempt.details.report.status,
        scryer_media_types::ProbeStatus::Incomplete
    );
    assert!(
        attempt
            .details
            .report
            .warnings
            .iter()
            .any(|warning| warning.code == "disc_episode_mapping_review")
    );
    assert_eq!(
        attempt
            .details
            .disc
            .as_ref()
            .unwrap()
            .selection
            .episode_mappings,
        before
            .analysis_details
            .disc
            .as_ref()
            .unwrap()
            .selection
            .episode_mappings
    );
    drop(attempts);
    let after = files.get_media_file_by_id(&id).await.unwrap().unwrap();
    assert_eq!(
        after.analysis_details, before.analysis_details,
        "changing the selected title must not publish stale episode runtime facts"
    );
}

#[tokio::test]
async fn stale_analysis_respects_batch_bounds_and_duplicate_candidates() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let analyzer = Arc::new(SelectionAnalyzer {
        seen: seen.clone(),
        started: Arc::new(tokio::sync::Notify::new()),
        resume: None,
    });
    let (app, _admin, files, id, dir) = selection_fixture(analyzer).await;
    let original = files.get_media_file_by_id(&id).await.unwrap().unwrap();
    let mut ids = vec![id.clone(), id];
    for index in 1..25 {
        let path = dir.path().join(format!("disc-{index}.iso"));
        std::fs::write(&path, b"disc fixture").unwrap();
        let mut row = original.clone();
        row.id = format!("stale-{index}");
        row.file_path = path.to_string_lossy().into_owned();
        ids.push(row.id.clone());
        files.store.lock().await.push(row);
    }
    *files.pending_analysis_ids.lock().await = ids;
    app.refresh_stale_media_analysis("fixture-library")
        .await
        .unwrap();
    assert_eq!(
        seen.lock().await.len(),
        19,
        "20 candidates include one duplicate"
    );
    assert_eq!(
        files
            .store
            .lock()
            .await
            .iter()
            .filter(|row| row.analysis_details.revision > 0)
            .count(),
        19
    );
    app.refresh_stale_media_analysis("fixture-library")
        .await
        .unwrap();
    assert_eq!(seen.lock().await.len(), 25);
}

#[tokio::test]
async fn stale_analysis_skips_busy_destinations_and_yields_to_foreground_imports() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let analyzer = Arc::new(SelectionAnalyzer {
        seen: seen.clone(),
        started: Arc::new(tokio::sync::Notify::new()),
        resume: None,
    });
    let (app, _admin, files, id, dir) = selection_fixture(analyzer).await;
    files.pending_analysis_ids.lock().await.push(id);
    let destination = app
        .runtime
        .imports
        .execution_coordinator
        .acquire_destination(&dir.path().join("Movie.iso"))
        .await;
    timeout(
        Duration::from_secs(1),
        app.refresh_stale_media_analysis("fixture-library"),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(seen.lock().await.is_empty());
    drop(destination);
    let foreground = app
        .runtime
        .imports
        .execution_coordinator
        .acquire_preparation()
        .await;
    app.refresh_stale_media_analysis("fixture-library")
        .await
        .unwrap();
    assert!(seen.lock().await.is_empty());
    drop(foreground);
    app.refresh_stale_media_analysis("fixture-library")
        .await
        .unwrap();
    assert_eq!(seen.lock().await.len(), 1);
}

#[tokio::test]
async fn stale_analysis_keeps_single_flight_after_caller_cancellation() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let started = Arc::new(tokio::sync::Notify::new());
    let resume = Arc::new(tokio::sync::Notify::new());
    let analyzer = Arc::new(SelectionAnalyzer {
        seen: seen.clone(),
        started: started.clone(),
        resume: Some(resume.clone()),
    });
    let (app, _admin, files, id, _dir) = selection_fixture(analyzer).await;
    files.pending_analysis_ids.lock().await.push(id.clone());
    let caller = tokio::spawn({
        let app = app.clone();
        async move { app.refresh_stale_media_analysis("fixture-library").await }
    });
    timeout(Duration::from_secs(5), started.notified())
        .await
        .unwrap();
    caller.abort();
    let _ = caller.await;
    timeout(
        Duration::from_secs(1),
        app.refresh_stale_media_analysis("fixture-library"),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(seen.lock().await.len(), 1);
    resume.notify_one();
    let _finished = timeout(
        Duration::from_secs(5),
        app.runtime
            .library
            .media_analysis_refresh_lock
            .clone()
            .lock_owned(),
    )
    .await
    .unwrap();
    assert_eq!(
        files
            .get_media_file_by_id(&id)
            .await
            .unwrap()
            .unwrap()
            .analysis_details
            .revision,
        scryer_media_types::ANALYSIS_REVISION
    );
}

#[tokio::test]
async fn stale_analysis_does_not_record_results_for_replaced_sources() {
    let started = Arc::new(tokio::sync::Notify::new());
    let resume = Arc::new(tokio::sync::Notify::new());
    let analyzer = Arc::new(SelectionAnalyzer {
        seen: Arc::new(Mutex::new(Vec::new())),
        started: started.clone(),
        resume: Some(resume.clone()),
    });
    let (app, _admin, files, id, dir) = selection_fixture(analyzer).await;
    files.pending_analysis_ids.lock().await.push(id.clone());
    let caller = tokio::spawn({
        let app = app.clone();
        async move { app.refresh_stale_media_analysis("fixture-library").await }
    });
    timeout(Duration::from_secs(5), started.notified())
        .await
        .unwrap();
    std::fs::write(
        dir.path().join("Movie.iso"),
        b"replacement image of different length",
    )
    .unwrap();
    resume.notify_one();
    caller.await.unwrap().unwrap();
    assert_eq!(
        files
            .get_media_file_by_id(&id)
            .await
            .unwrap()
            .unwrap()
            .analysis_details
            .revision,
        0
    );
    assert!(
        files.analysis_attempts.lock().await.is_empty(),
        "a rejected source change must not suppress a fresh probe"
    );
}

#[tokio::test]
async fn disc_title_override_survives_catalogued_reprobe() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let analyzer = Arc::new(SelectionAnalyzer {
        seen: seen.clone(),
        started: Arc::new(tokio::sync::Notify::new()),
        resume: None,
    });
    let (app, admin, files, id, _dir) = selection_fixture(analyzer).await;
    let chosen = app
        .select_media_file_disc_title(&admin, &id, Some("00002".into()))
        .await
        .unwrap();
    assert_eq!(
        chosen
            .analysis_details
            .disc
            .unwrap()
            .selection
            .title_id
            .as_deref(),
        Some("00002")
    );
    let row = files.get_media_file_by_id(&id).await.unwrap().unwrap();
    app.analyze_catalogued_media_file(Some(&id), PathBuf::from(row.file_path))
        .await
        .unwrap();
    assert_eq!(
        seen.lock()
            .await
            .iter()
            .map(|selection| selection.title_id.as_deref())
            .collect::<Vec<_>>(),
        vec![Some("00002"), Some("00002")]
    );
    let automatic = app
        .select_media_file_disc_title(&admin, &id, None)
        .await
        .unwrap();
    assert!(automatic.analysis_details.disc.unwrap().automatic_selection);
}

#[tokio::test]
async fn disc_title_selection_rejects_concurrent_selection_and_source_changes() {
    let started = Arc::new(tokio::sync::Notify::new());
    let resume = Arc::new(tokio::sync::Notify::new());
    let analyzer = Arc::new(SelectionAnalyzer {
        seen: Arc::new(Mutex::new(Vec::new())),
        started: started.clone(),
        resume: Some(resume.clone()),
    });
    let (app, admin, files, id, dir) = selection_fixture(analyzer).await;
    for change_source in [false, true] {
        let task = tokio::spawn({
            let (app, admin, id) = (app.clone(), admin.clone(), id.clone());
            async move {
                app.select_media_file_disc_title(&admin, &id, Some("00002".into()))
                    .await
            }
        });
        timeout(Duration::from_secs(5), started.notified())
            .await
            .expect("inspection started");
        if change_source {
            std::fs::write(
                dir.path().join("Movie.iso"),
                b"different length of disc bytes",
            )
            .unwrap();
        } else {
            let mut rows = files.store.lock().await;
            rows.iter_mut()
                .find(|row| row.id == id)
                .unwrap()
                .analysis_details
                .revision = 7;
        }
        resume.notify_one();
        assert!(matches!(task.await.unwrap(), Err(AppError::Validation(_))));
        assert!(
            files
                .get_media_file_by_id(&id)
                .await
                .unwrap()
                .unwrap()
                .analysis_details
                .disc
                .is_none()
        );
    }
}

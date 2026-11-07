use super::*;
use crate::{JobRunStatus, ScoreRefreshSelection};

async fn fixture() -> (AppUseCase, User, Arc<MockMediaFileRepo>, String) {
    let (app, user) = bootstrap();
    let files = Arc::new(MockMediaFileRepo::default());
    app.services
        .identity
        .users
        .create(user.clone())
        .await
        .unwrap();
    app.services
        .catalog
        .libraries
        .set_app_permission_mask_for_user(&user.id, user.authorization.app)
        .await
        .unwrap();
    let app = app.with_test_overrides(|services| {
        services
            .with_media_files(files.clone())
            .with_job_runs(Arc::new(RecordingJobRunRepo::default()))
    });
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Synthetic Feature".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec!["scryer:quality-profile:1080p".into()],
                external_ids: vec![],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let id = files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id,
            file_path: "/synthetic/feature.mkv".into(),
            size_bytes: 2_000_000_000,
            grabbed_release_title: Some("Synthetic.Feature.2024.1080p.WEB-DL-GROUP".into()),
            acquisition_score: Some(41),
            scoring_log: Some("original scoring evidence".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    (app, user, files, id)
}

async fn terminal(app: &AppUseCase, id: &str) -> crate::JobRunRecord {
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            let run = app
                .services
                .events
                .job_runs
                .get_job_run(id)
                .await
                .unwrap()
                .unwrap();
            if run.completed_at.is_some() {
                return run;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("score job must terminate")
}

#[tokio::test]
async fn score_evaluation_read_is_passive_and_refresh_preserves_acquisition() {
    let (app, user, files, id) = fixture().await;
    assert!(
        app.media_file_score_evaluation(&user, &id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(files.score_evaluations.lock().await.is_empty());
    let run = app
        .refresh_selected_file_scores(
            &user,
            ScoreRefreshSelection::Files(vec![id.clone(), id.clone()]),
        )
        .await
        .unwrap();
    let finished = terminal(&app, &run.id).await;
    assert_eq!(finished.status, JobRunStatus::Completed);
    let saved = app
        .media_file_score_evaluation(&user, &id)
        .await
        .unwrap()
        .unwrap();
    assert!(!saved.stale);
    assert_eq!(files.score_evaluations.lock().await.len(), 1);
    let row = files.get_media_file_by_id(&id).await.unwrap().unwrap();
    assert_eq!(row.acquisition_score, Some(41));
    assert_eq!(
        row.scoring_log.as_deref(),
        Some("original scoring evidence")
    );
    let title = app.get_title(&user, &row.title_id).await.unwrap().unwrap();
    let profile = app.resolve_quality_profile_for_title(&title).await.unwrap();
    let context = app
        .resolve_canonical_scoring_context(&title, &profile)
        .await;
    let expected = crate::canonical_scoring::score_media_file_preview(
        &row,
        &context.view(Default::default(), false),
    );
    assert_eq!(saved.evaluation.score, expected.scored.total);
    files
        .store
        .lock()
        .await
        .iter_mut()
        .find(|file| file.id == id)
        .unwrap()
        .size_bytes += 1;
    assert!(
        app.media_file_score_evaluation(&user, &id)
            .await
            .unwrap()
            .unwrap()
            .stale
    );
    assert_eq!(
        files.score_evaluations.lock().await[&id].fingerprint,
        saved.evaluation.fingerprint
    );
}

#[tokio::test]
async fn score_evaluation_cancel_and_deleted_selection_do_not_silently_succeed() {
    let (app, user, files, id) = fixture().await;
    assert!(
        app.refresh_selected_file_scores(&user, ScoreRefreshSelection::Files(vec![]))
            .await
            .is_err()
    );
    let worker = app.runtime.jobs.score_refresh.worker.lock().await;
    let canceled = app
        .refresh_selected_file_scores(&user, ScoreRefreshSelection::Files(vec![id.clone()]))
        .await
        .unwrap();
    assert!(
        app.cancel_file_score_refresh(&user, &canceled.id)
            .await
            .unwrap()
    );
    assert_eq!(
        terminal(&app, &canceled.id).await.status,
        JobRunStatus::Warning
    );
    assert!(files.score_evaluations.lock().await.is_empty());
    let missing = app
        .refresh_selected_file_scores(&user, ScoreRefreshSelection::Files(vec![id.clone()]))
        .await
        .unwrap();
    files.store.lock().await.clear();
    drop(worker);
    let result = terminal(&app, &missing.id).await;
    assert_eq!(result.status, JobRunStatus::Warning);
    assert!(result.summary_json.unwrap().contains(&id));
}

#[tokio::test]
async fn score_evaluation_rechecks_permissions_before_queued_work() {
    let (app, user, files, id) = fixture().await;
    let worker = app.runtime.jobs.score_refresh.worker.lock().await;
    let run = app
        .refresh_selected_file_scores(&user, ScoreRefreshSelection::Files(vec![id]))
        .await
        .unwrap();
    app.services
        .catalog
        .libraries
        .set_app_permission_mask_for_user(&user.id, AppPermissionMask::NONE)
        .await
        .unwrap();
    drop(worker);
    assert_eq!(terminal(&app, &run.id).await.status, JobRunStatus::Warning);
    assert!(files.score_evaluations.lock().await.is_empty());
}

#[tokio::test]
async fn score_evaluation_episode_selection_validates_before_acceptance() {
    let (app, user, files, file_id) = fixture().await;
    let title_id = files
        .get_media_file_by_id(&file_id)
        .await
        .unwrap()
        .unwrap()
        .title_id;
    app.services
        .catalog
        .shows
        .create_episode(scryer_domain::Episode {
            id: "selected-episode".into(),
            title_id,
            collection_id: None,
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: None,
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
            contiguous_absolute_number: None,
            overview: None,
            tvdb_id: None,
            tmdb_id: None,
            image_url: None,
            monitored: false,
            created_at: chrono::Utc::now(),
        })
        .await
        .unwrap();
    for ids in [
        vec![],
        vec!["selected-episode".into(); 1001],
        vec!["missing".into()],
    ] {
        assert!(
            app.refresh_selected_file_scores(&user, ScoreRefreshSelection::Episodes(ids))
                .await
                .is_err()
        );
    }
    let worker = app.runtime.jobs.score_refresh.worker.lock().await;
    let run = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        app.refresh_selected_file_scores(
            &user,
            ScoreRefreshSelection::Episodes(vec!["selected-episode".into(); 2]),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(app.cancel_file_score_refresh(&user, &run.id).await.unwrap());
    assert_eq!(terminal(&app, &run.id).await.status, JobRunStatus::Warning);
    drop(worker);
    app.services
        .catalog
        .libraries
        .set_app_permission_mask_for_user(&user.id, AppPermissionMask::NONE)
        .await
        .unwrap();
    let denied = app.attach_user_authorization(user).await.unwrap();
    assert!(
        app.refresh_selected_file_scores(
            &denied,
            ScoreRefreshSelection::Episodes(vec!["selected-episode".into()])
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn score_evaluation_selection_limits_and_cancellation_are_owner_scoped() {
    let (app, user, files, id) = fixture().await;
    assert!(
        app.refresh_selected_file_scores(
            &user,
            ScoreRefreshSelection::Files(vec![id.clone(); 1001])
        )
        .await
        .is_err()
    );
    let worker = app.runtime.jobs.score_refresh.worker.lock().await;
    let run = app
        .refresh_selected_file_scores(&user, ScoreRefreshSelection::Files(vec![id.clone(), id]))
        .await
        .unwrap();
    let mut other = user.clone();
    other.id = "other-operator".into();
    assert!(matches!(
        app.cancel_file_score_refresh(&other, &run.id).await,
        Err(AppError::Unauthorized(_) | AppError::UnauthorizedReason { .. })
    ));
    assert!(app.cancel_file_score_refresh(&user, &run.id).await.unwrap());
    assert_eq!(terminal(&app, &run.id).await.status, JobRunStatus::Warning);
    assert!(files.score_evaluations.lock().await.is_empty());
    drop(worker);
}

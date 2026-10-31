use super::*;
use crate::integration::download_enrichment::DownloadEnrichmentPass;

fn fixture() -> (AppUseCase, Arc<TrackingDownloadSubmissionRepo>) {
    let repo = Arc::new(TrackingDownloadSubmissionRepo::default());
    let (app, _) = bootstrap_with_cleanup_tracking(Arc::default(), repo.clone(), Arc::default());
    (app, repo)
}
fn submission(n: usize) -> DownloadSubmission {
    DownloadSubmission {
        download_id: scryer_domain::download_identity::DownloadId::new(),
        title_id: format!("title-{n}"),
        purpose: crate::DownloadSubmissionPurpose::Standard,
        facet: "anime".into(),
        download_client_id: Some(format!("client-{}", n % 4)),
        download_client_type: "qbittorrent".into(),
        download_client_item_id: format!("item-{n}"),
        source_title: Some(format!("Synthetic.Series.{n}.S01E01")),
        source_hint: None,
        source_provider_id: None,
        source_provider_name: Some("Fixture".into()),
        source_kind: None,
        info_hash: None,
        release_size_bytes: None,
        request_signature: None,
        scope: SubmissionScope::Episode {
            episode_id: format!("episode-{n}"),
        },
        release_listing_json: Some("not retained in the projection".into()),
    }
}
async fn calls(repo: &TrackingDownloadSubmissionRepo) -> Vec<(&'static str, usize)> {
    repo.enrichment_calls
        .lock()
        .await
        .iter()
        .map(|(family, keys)| (*family, keys.len()))
        .collect()
}

#[tokio::test]
async fn download_enrichment_reuses_pass_and_refreshes_mutable_state_each_poll() {
    let (app, repo) = fixture();
    let row = submission(0);
    let key = ClientJobLocator::from_submission(&row);
    repo.record_submission(row.clone()).await.unwrap();
    let keys = [key.clone()];
    let mut pass = DownloadEnrichmentPass::default();
    for _ in 0..3 {
        pass.load(&app, &keys).await;
    }
    assert_eq!(
        calls(&repo).await,
        vec![("submission", 1), ("goals", 1), ("state", 1)]
    );
    repo.update_tracked_state(&key, "ignored").await.unwrap();
    let mut next = DownloadEnrichmentPass::default();
    next.load(&app, &keys).await;
    assert_eq!(next.states[&key].as_deref(), Some("ignored"));
    assert_eq!(
        next.submissions[&key].as_ref().unwrap().download_id,
        row.download_id
    );
    assert_eq!(
        calls(&repo).await,
        vec![("submission", 1), ("goals", 1), ("state", 1), ("state", 1)]
    );
}

#[tokio::test]
async fn download_enrichment_only_loads_new_locators_and_separates_clients() {
    let (app, repo) = fixture();
    let row = submission(0);
    let first = ClientJobLocator::from_submission(&row);
    let mut second_row = submission(1);
    second_row.download_client_item_id = first.item_id.clone();
    let second = ClientJobLocator::from_submission(&second_row);
    repo.record_submission(row.clone()).await.unwrap();
    repo.record_submission(second_row.clone()).await.unwrap();
    let mut pass = DownloadEnrichmentPass::default();
    pass.load(&app, std::slice::from_ref(&first)).await;
    pass.load(&app, &[first.clone(), second.clone()]).await;
    assert_eq!(
        pass.submissions[&first].as_ref().unwrap().download_id,
        row.download_id
    );
    assert_eq!(
        pass.submissions[&second].as_ref().unwrap().download_id,
        second_row.download_id
    );
    assert!(calls(&repo).await.iter().all(|(_, count)| *count == 1));
    assert_eq!(calls(&repo).await.len(), 6);
}

#[tokio::test]
async fn download_enrichment_generation_replaces_positive_and_negative_results() {
    let (app, repo) = fixture();
    let row = submission(0);
    let key = ClientJobLocator::from_submission(&row);
    let keys = [key.clone()];
    let mut pass = DownloadEnrichmentPass::default();
    pass.load(&app, &keys).await;
    assert!(pass.submissions[&key].is_none());
    repo.record_submission(row.clone()).await.unwrap();
    app.runtime
        .acquisition
        .invalidate_download_registry_observations();
    pass.load(&app, &keys).await;
    assert_eq!(
        pass.submissions[&key].as_ref().unwrap().download_id,
        row.download_id
    );
    let mut replacement = row;
    replacement.download_id = scryer_domain::download_identity::DownloadId::new();
    replacement.title_id = "replacement".into();
    *repo.store.lock().await = vec![replacement.clone()];
    app.runtime
        .acquisition
        .invalidate_download_registry_observations();
    pass.load(&app, &keys).await;
    assert_eq!(
        pass.submissions[&key].as_ref().unwrap().download_id,
        replacement.download_id
    );
    assert_eq!(
        pass.submissions[&key].as_ref().unwrap().title_id,
        "replacement"
    );
    repo.store.lock().await.clear();
    app.runtime
        .acquisition
        .invalidate_download_registry_observations();
    pass.load(&app, &keys).await;
    assert!(pass.submissions[&key].is_none());
}

#[tokio::test]
async fn download_enrichment_failures_do_not_poison_other_families() {
    for family in ["submission", "goals", "state"] {
        let (app, repo) = fixture();
        let row = submission(0);
        let keys = [ClientJobLocator::from_submission(&row)];
        repo.record_submission(row).await.unwrap();
        repo.enrichment_failures.lock().await.insert(family);
        let mut pass = DownloadEnrichmentPass::default();
        pass.load(&app, &keys).await;
        repo.enrichment_failures.lock().await.clear();
        pass.load(&app, &keys).await;
        assert!(pass.submissions[&keys[0]].is_some());
        assert_eq!(
            calls(&repo)
                .await
                .iter()
                .filter(|(f, _)| *f == family)
                .count(),
            2
        );
        assert_eq!(calls(&repo).await.len(), 4);
    }
}

#[tokio::test]
async fn download_enrichment_fill_racing_generation_is_reloaded() {
    let (app, repo) = fixture();
    let row = submission(0);
    let keys = vec![ClientJobLocator::from_submission(&row)];
    repo.record_submission(row).await.unwrap();
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    *repo.enrichment_gate.lock().await = Some((entered.clone(), release.clone()));
    let task_app = app.clone();
    let task_keys = keys.clone();
    let task = tokio::spawn(async move {
        let mut pass = DownloadEnrichmentPass::default();
        pass.load(&task_app, &task_keys).await;
        pass
    });
    timeout(Duration::from_secs(30), entered.notified())
        .await
        .unwrap();
    repo.store.lock().await[0].title_id = "changed-during-fill".into();
    app.runtime
        .acquisition
        .invalidate_download_registry_observations();
    release.notify_one();
    let pass = timeout(Duration::from_secs(30), task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        pass.submissions[&keys[0]].as_ref().unwrap().title_id,
        "changed-during-fill"
    );
    assert_eq!(
        calls(&repo)
            .await
            .iter()
            .filter(|(f, _)| *f == "submission")
            .count(),
        2
    );
    let before = calls(&repo).await.len();
    DownloadEnrichmentPass::default().load(&app, &keys).await;
    assert_eq!(calls(&repo).await.len(), before + 1);
}

#[tokio::test]
async fn download_enrichment_concurrent_fills_share_loaded_projection() {
    let (app, repo) = fixture();
    let row = submission(0);
    let keys = vec![ClientJobLocator::from_submission(&row)];
    repo.record_submission(row).await.unwrap();
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    *repo.enrichment_gate.lock().await = Some((entered.clone(), release.clone()));
    let a = app.clone();
    let k = keys.clone();
    let first = tokio::spawn(async move {
        DownloadEnrichmentPass::default().load(&a, &k).await;
    });
    timeout(Duration::from_secs(30), entered.notified())
        .await
        .unwrap();
    let a = app.clone();
    let k = keys.clone();
    let second = tokio::spawn(async move {
        DownloadEnrichmentPass::default().load(&a, &k).await;
    });
    release.notify_one();
    timeout(Duration::from_secs(30), async {
        first.await.unwrap();
        second.await.unwrap();
    })
    .await
    .unwrap();
    assert_eq!(
        calls(&repo)
            .await
            .iter()
            .filter(|(f, _)| *f == "submission")
            .count(),
        1
    );
    assert_eq!(
        calls(&repo)
            .await
            .iter()
            .filter(|(f, _)| *f == "state")
            .count(),
        2
    );
}

/// Operation counts are assertions; elapsed timings are observations only.
#[tokio::test]
async fn download_enrichment_synthetic_workloads() {
    for count in [100, 1_000, 10_000] {
        let (app, repo) = fixture();
        let rows: Vec<_> = (0..count).map(submission).collect();
        let keys: Vec<_> = rows.iter().map(ClientJobLocator::from_submission).collect();
        *repo.store.lock().await = rows
            .iter()
            .enumerate()
            .filter(|(i, _)| i % 4 != 3)
            .map(|(_, r)| r.clone())
            .collect();
        let mut expected = None;
        for cycle in 0..3 {
            let start = std::time::Instant::now();
            let mut pass = DownloadEnrichmentPass::default();
            for _ in 0..3 {
                pass.load(&app, &keys).await;
            }
            let actual: Vec<_> = keys
                .iter()
                .map(|k| pass.submissions[k].as_ref().map(|s| s.download_id))
                .collect();
            if let Some(expected) = &expected {
                assert_eq!(&actual, expected);
            } else {
                expected = Some(actual);
            }
            eprintln!(
                "download_enrichment rows={count} cycle={cycle} elapsed_us={}",
                start.elapsed().as_micros()
            );
        }
        let observed = calls(&repo).await;
        assert_eq!(observed.iter().filter(|(f, _)| *f == "state").count(), 3);
        if count <= 1_000 {
            assert_eq!(
                observed.iter().filter(|(f, _)| *f == "submission").count(),
                1
            );
            assert_eq!(observed.iter().filter(|(f, _)| *f == "goals").count(), 1);
        }
        eprintln!("download_enrichment rows={count} batches={observed:?}");
        // Model the former three independent enrichment loads per poll using
        // the same loader with a cold generation each time. This comparison
        // measures reuse only, not production datastore latency or old code.
        repo.enrichment_calls.lock().await.clear();
        for cycle in 0..3 {
            let start = std::time::Instant::now();
            for _ in 0..3 {
                app.runtime
                    .acquisition
                    .invalidate_download_registry_observations();
                let mut uncached = DownloadEnrichmentPass::default();
                uncached.load(&app, &keys).await;
                let actual: Vec<_> = keys
                    .iter()
                    .map(|k| uncached.submissions[k].as_ref().map(|s| s.download_id))
                    .collect();
                assert_eq!(Some(&actual), expected.as_ref());
            }
            eprintln!(
                "download_enrichment rows={count} cold_three_pass_cycle={cycle} elapsed_us={}",
                start.elapsed().as_micros()
            );
        }
        assert_eq!(calls(&repo).await.len(), 27);
    }
}

#[tokio::test]
async fn download_enrichment_collection_preserves_progress_and_reads_once() {
    let client = Arc::new(StubDownloadClient::default());
    let repo = Arc::new(TrackingDownloadSubmissionRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(client.clone(), repo.clone(), Arc::default());
    let config =
        create_enabled_download_client_config(&app, &user, "Fixture Client", "nzbget").await;
    let mut row = submission(0);
    row.download_client_type = "nzbget".into();
    row.download_client_id = Some(config.id.clone());
    repo.record_submission(row.clone()).await.unwrap();
    let mut item = queue_history_fixture_item(
        &row.download_client_item_id,
        DownloadQueueState::Downloading,
        0,
    );
    item.client_id = config.id.clone();
    item.client_name = config.name.clone();
    item.title_id = None;
    item.facet = None;
    item.progress_percent = 17;
    *client.queue_items.lock().await = vec![item.clone()];
    let first = app
        .collect_download_snapshot_items_excluding_client_types(true, false, false, &[])
        .await
        .unwrap();
    assert_eq!(first.items.len(), 1);
    assert_eq!(
        first.items[0].title_id.as_deref(),
        Some(row.title_id.as_str())
    );
    assert_eq!(first.items[0].progress_percent, 17);
    assert_eq!(
        calls(&repo).await,
        vec![("submission", 1), ("goals", 1), ("state", 1)]
    );
    item.progress_percent = 63;
    *client.queue_items.lock().await = vec![item];
    let second = app
        .collect_download_snapshot_items_excluding_client_types(true, false, false, &[])
        .await
        .unwrap();
    assert_eq!(second.items[0].progress_percent, 63);
    assert_eq!(
        calls(&repo).await,
        vec![("submission", 1), ("goals", 1), ("state", 1), ("state", 1)]
    );
}

#[tokio::test]
async fn download_enrichment_invalidated_pass_restores_attribution_without_rolling_back_progress() {
    let (app, repo) = fixture();
    let row = submission(0);
    let key = ClientJobLocator::from_submission(&row);
    repo.record_submission(row.clone()).await.unwrap();
    let mut item = queue_history_fixture_item(&key.item_id, DownloadQueueState::Downloading, 0);
    item.client_id = key.client_id.clone().unwrap();
    item.client_type = key.client_type.clone();
    item.title_id = None;
    item.episode_id = None;
    item.facet = None;
    item.is_scryer_origin = false;
    let mut items = vec![item];
    let mut pass = DownloadEnrichmentPass::default();
    crate::integration::workflow::enrich_download_queue_items_from_submissions_with_original_identities(&app,&mut items,None,&mut pass).await;
    assert_eq!(items[0].title_id.as_deref(), Some(row.title_id.as_str()));
    let mut replacement = row;
    replacement.title_id = "replacement".into();
    replacement.scope = SubmissionScope::Episode {
        episode_id: "replacement-episode".into(),
    };
    replacement.download_id = scryer_domain::download_identity::DownloadId::new();
    *repo.store.lock().await = vec![replacement];
    app.runtime
        .acquisition
        .invalidate_download_registry_observations();
    items[0].progress_percent = 73;
    crate::integration::workflow::enrich_download_queue_items_from_submissions_with_original_identities(&app,&mut items,None,&mut pass).await;
    assert_eq!(items[0].title_id.as_deref(), Some("replacement"));
    assert_eq!(items[0].episode_id.as_deref(), Some("replacement-episode"));
    assert_eq!(items[0].progress_percent, 73);
}

#[tokio::test]
async fn download_enrichment_normalized_clientless_keys_share_one_result() {
    let (app, repo) = fixture();
    let mut row = submission(0);
    row.download_client_id = None;
    let key = ClientJobLocator::from_submission(&row);
    repo.record_submission(row).await.unwrap();
    let mut pass = DownloadEnrichmentPass::default();
    pass.load(&app, std::slice::from_ref(&key)).await;
    assert!(pass.submissions[&key].is_some());
}

#[tokio::test]
async fn download_enrichment_goals_survive_hits_but_not_a_new_attempt() {
    let (app, repo) = fixture();
    let row = submission(0);
    let key = ClientJobLocator::from_submission(&row);
    repo.record_submission(row.clone()).await.unwrap();
    let goals = crate::PersistedSeedGoals {
        seed_goal_ratio: Some(2.0),
        seed_goal_seconds: Some(3600),
        never_remove: true,
        ..crate::PersistedSeedGoals::default()
    };
    repo.enrichment_goals.lock().await.insert(
        key.clone(),
        crate::PersistedSeedGoals {
            seeding_profile_id: Some("profile-at-grab".into()),
            info_hash: Some("fixture-hash".into()),
            ..goals.clone()
        },
    );
    for _ in 0..2 {
        let mut pass = DownloadEnrichmentPass::default();
        pass.load(&app, std::slice::from_ref(&key)).await;
        assert_eq!(pass.goals[&key], Some(goals.clone()));
    }
    assert_eq!(
        calls(&repo)
            .await
            .iter()
            .filter(|(f, _)| *f == "goals")
            .count(),
        1
    );
    let mut next = row;
    next.download_id = scryer_domain::download_identity::DownloadId::new();
    *repo.store.lock().await = vec![next.clone()];
    repo.enrichment_goals.lock().await.clear();
    app.runtime
        .acquisition
        .invalidate_download_registry_observations();
    let mut pass = DownloadEnrichmentPass::default();
    pass.load(&app, std::slice::from_ref(&key)).await;
    assert!(pass.goals[&key].is_none());
    assert_eq!(
        pass.submissions[&key].as_ref().unwrap().download_id,
        next.download_id
    );
}

#[tokio::test]
async fn download_enrichment_sustained_churn_uses_bounded_uncached_fallback() {
    let (app, repo) = fixture();
    let row = submission(0);
    let key = ClientJobLocator::from_submission(&row);
    repo.record_submission(row).await.unwrap();
    *repo.enrichment_generation_on_read.lock().await =
        Some(app.runtime.acquisition.download_registry_generation.clone());
    let mut pass = DownloadEnrichmentPass::default();
    pass.load(&app, std::slice::from_ref(&key)).await;
    assert!(pass.submissions[&key].is_some());
    assert_eq!(
        calls(&repo)
            .await
            .iter()
            .filter(|(f, _)| *f == "submission")
            .count(),
        3
    );
    repo.enrichment_generation_on_read.lock().await.take();
    pass.load(&app, std::slice::from_ref(&key)).await;
    assert_eq!(
        calls(&repo)
            .await
            .iter()
            .filter(|(f, _)| *f == "submission")
            .count(),
        4
    );
    DownloadEnrichmentPass::default()
        .load(&app, std::slice::from_ref(&key))
        .await;
    assert_eq!(
        calls(&repo)
            .await
            .iter()
            .filter(|(f, _)| *f == "submission")
            .count(),
        4
    );
}

#[tokio::test]
async fn download_enrichment_canonical_fallback_hits_keep_observation_resolution_and_regrab_isolation()
 {
    let (mut app, repo) = fixture();
    let row = submission(0);
    repo.record_submission(row.clone()).await.unwrap();
    let registry = Arc::new(RecordingDownloadRegistry::default());
    app.services.workflow.download_registry = registry.clone();
    let mut native =
        queue_history_fixture_item("observed-native", DownloadQueueState::Completed, 0);
    native.client_id = row.download_client_id.clone().unwrap();
    native.client_type = row.download_client_type.clone();
    native.title_id = None;
    native.episode_id = None;
    native.facet = None;
    native.is_scryer_origin = false;
    native.download_id = Some(row.download_id.to_wire());
    for _ in 0..2 {
        let mut items = vec![native.clone()];
        crate::enrich_download_queue_items_from_submissions(&app, &mut items).await;
        assert_eq!(items[0].title_id.as_deref(), Some(row.title_id.as_str()));
    }
    assert_eq!(repo.enrichment_canonical_reads.load(Ordering::SeqCst), 1);
    assert_eq!(registry.resolutions.load(Ordering::SeqCst), 1);
    let mut replacement = row;
    replacement.download_id = scryer_domain::download_identity::DownloadId::new();
    replacement.title_id = "new-attempt".into();
    repo.record_submission(replacement.clone()).await.unwrap();
    app.runtime
        .acquisition
        .invalidate_download_registry_observations();
    native.download_id = Some(replacement.download_id.to_wire());
    let mut items = vec![native];
    crate::enrich_download_queue_items_from_submissions(&app, &mut items).await;
    assert_eq!(items[0].title_id.as_deref(), Some("new-attempt"));
    assert_eq!(repo.enrichment_canonical_reads.load(Ordering::SeqCst), 2);
    assert_eq!(registry.resolutions.load(Ordering::SeqCst), 2);
}

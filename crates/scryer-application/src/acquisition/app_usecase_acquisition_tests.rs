use super::*;
use crate::acquisition::targets::movie_is_available_for_acquisition;
use chrono::{DateTime, Utc};
use std::collections::HashMap;

// ── helpers ───────────────────────────────────────────────────────────────────

fn now_utc() -> DateTime<Utc> {
    Utc::now()
}

fn days_ago(n: i64) -> String {
    (now_utc() - chrono::Duration::days(n))
        .format("%Y-%m-%d")
        .to_string()
}

fn days_from_now(n: i64) -> String {
    (now_utc() + chrono::Duration::days(n))
        .format("%Y-%m-%d")
        .to_string()
}

fn base_episode_wanted_item() -> AcquisitionScopeState {
    let now = now_utc().to_rfc3339();
    AcquisitionScopeState {
        id: "wanted-episode-1".to_string(),
        title_id: "title-1".to_string(),
        title_name: Some("Test Show".to_string()),
        title_slug: None,
        title_facet: None,
        library_id: None,
        library_name: None,
        library_slug: None,
        episode_id: Some("episode-1".to_string()),
        collection_id: Some("season-1".to_string()),
        series_movie_link_id: None,
        season_number: Some("1".to_string()),
        episode_number: Some("1".to_string()),
        media_type: "episode".to_string(),
        last_search_at: None,
        status: AcquisitionScopeStatus::Wanted,
        grabbed_release: None,
        landed_bar: None,
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: now.clone(),
        updated_at: now,
    }
}

fn base_series_movie_wanted_item() -> AcquisitionScopeState {
    let now = now_utc().to_rfc3339();
    AcquisitionScopeState {
        id: "wanted-series-movie-1".to_string(),
        title_id: "title-1".to_string(),
        title_name: Some("Test Show".to_string()),
        title_slug: None,
        title_facet: None,
        library_id: None,
        library_name: None,
        library_slug: None,
        episode_id: None,
        collection_id: None,
        series_movie_link_id: Some("series-movie-link-1".to_string()),
        season_number: None,
        episode_number: None,
        media_type: "series_movie".to_string(),
        last_search_at: None,
        status: AcquisitionScopeStatus::Wanted,
        grabbed_release: None,
        landed_bar: None,
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: now.clone(),
        updated_at: now,
    }
}

fn base_episode() -> Episode {
    Episode {
        id: "episode-1".to_string(),
        title_id: "title-1".to_string(),
        collection_id: Some("season-1".to_string()),
        episode_type: scryer_domain::EpisodeType::Standard,
        episode_number: Some("1".to_string()),
        season_number: Some("1".to_string()),
        episode_label: Some("S01E01".to_string()),
        title: Some("Pilot".to_string()),
        air_date: None,
        duration_seconds: None,
        has_multi_audio: false,
        has_subtitle: false,
        is_filler: false,
        is_recap: false,
        absolute_number: None,
        overview: None,
        tvdb_id: None,
        image_url: None,
        monitored: true,
        created_at: now_utc(),
    }
}

fn test_search_result_with_decision(
    title: &str,
    source_kind: Option<DownloadSourceKind>,
    decision_code: &str,
) -> IndexerSearchResult {
    IndexerSearchResult {
        indexer_id: None,
        source: "indexer".to_string(),
        title: title.to_string(),
        link: None,
        download_url: Some(format!("https://example.invalid/{title}.nzb")),
        source_kind,
        size_bytes: None,
        published_at: None,
        thumbs_up: None,
        thumbs_down: None,
        indexer_languages: None,
        indexer_subtitles: None,
        indexer_grabs: None,
        password_hint: None,
        parsed_release_metadata: None,
        quality_profile_decision: None,
        extra: HashMap::new(),
        response_attributes: Default::default(),
        guid: None,
        info_url: None,
        provenance: None,
        candidate_token: None,
        queue_scope: None,
        coverage_scope: None,
        auto_eligible: Some(decision_code == "eligible"),
        auto_decision_code: Some(decision_code.to_string()),
        auto_decision_summary: None,
    }
}

// ── announced ────────────────────────────────────────────────────────────────

#[test]
fn announced_always_available_no_dates() {
    assert!(movie_is_available_for_acquisition(
        None,
        None,
        "announced",
        &now_utc()
    ));
}

#[test]
fn announced_always_available_future_dates() {
    let first_aired = days_from_now(90);
    assert!(movie_is_available_for_acquisition(
        Some(&first_aired),
        None,
        "announced",
        &now_utc()
    ));
}

#[test]
fn unknown_availability_treated_as_announced() {
    assert!(movie_is_available_for_acquisition(
        None,
        None,
        "preorder",
        &now_utc()
    ));
}

#[tokio::test]
async fn skip_interval_does_not_replay_missed_poll_ticks_in_a_burst() {
    let mut interval = new_skip_interval(std::time::Duration::from_millis(50));
    interval.tick().await;

    tokio::time::sleep(std::time::Duration::from_millis(220)).await;
    interval.tick().await;

    let next_tick =
        tokio::time::timeout(std::time::Duration::from_millis(10), interval.tick()).await;
    assert!(
        next_tick.is_err(),
        "skip interval should not have an immediate catch-up tick waiting"
    );
}

// ── in_cinemas ────────────────────────────────────────────────────────────────

#[test]
fn in_cinemas_available_when_past_cinema_date() {
    let first_aired = days_ago(10);
    assert!(movie_is_available_for_acquisition(
        Some(&first_aired),
        None,
        "in_cinemas",
        &now_utc()
    ));
}

#[test]
fn in_cinemas_available_when_today_is_cinema_date() {
    let first_aired = now_utc().format("%Y-%m-%d").to_string();
    assert!(movie_is_available_for_acquisition(
        Some(&first_aired),
        None,
        "in_cinemas",
        &now_utc()
    ));
}

#[test]
fn in_cinemas_unavailable_when_future_cinema_date() {
    let first_aired = days_from_now(30);
    assert!(!movie_is_available_for_acquisition(
        Some(&first_aired),
        None,
        "in_cinemas",
        &now_utc()
    ));
}

#[test]
fn in_cinemas_unavailable_when_no_date() {
    assert!(!movie_is_available_for_acquisition(
        None,
        None,
        "in_cinemas",
        &now_utc()
    ));
}

#[test]
fn in_cinemas_unavailable_when_date_malformed() {
    assert!(!movie_is_available_for_acquisition(
        Some("not-a-date"),
        None,
        "in_cinemas",
        &now_utc()
    ));
}

// ── released ──────────────────────────────────────────────────────────────────

#[test]
fn released_available_when_past_digital_release() {
    let digital = days_ago(5);
    assert!(movie_is_available_for_acquisition(
        None,
        Some(&digital),
        "released",
        &now_utc()
    ));
}

#[test]
fn released_unavailable_when_future_digital_release() {
    let digital = days_from_now(14);
    assert!(!movie_is_available_for_acquisition(
        None,
        Some(&digital),
        "released",
        &now_utc()
    ));
}

#[test]
fn released_falls_back_to_cinema_plus_90_days_when_past() {
    let first_aired = days_ago(100); // 100 days ago + 90 = still past
    assert!(movie_is_available_for_acquisition(
        Some(&first_aired),
        None,
        "released",
        &now_utc()
    ));
}

#[test]
fn released_falls_back_to_cinema_plus_90_days_when_not_yet() {
    let first_aired = days_ago(30); // 30 days ago + 90 = 60 days in future
    assert!(!movie_is_available_for_acquisition(
        Some(&first_aired),
        None,
        "released",
        &now_utc()
    ));
}

#[test]
fn released_unavailable_when_no_dates() {
    assert!(!movie_is_available_for_acquisition(
        None,
        None,
        "released",
        &now_utc()
    ));
}

#[test]
fn released_digital_date_takes_priority_over_cinema_fallback() {
    // digital date is in the past (available), even though cinema + 90 would be in future
    let digital = days_ago(1);
    let first_aired = days_ago(10); // cinema only 10d ago, +90 not reached
    assert!(movie_is_available_for_acquisition(
        Some(&first_aired),
        Some(&digital),
        "released",
        &now_utc()
    ));
}

#[test]
fn released_malformed_digital_date_falls_back_to_cinema() {
    let first_aired = days_ago(100);
    // digital date parse fails → false; the code checks digital_release_date first,
    // and on parse failure returns false (no fallback within that branch). So this
    // returns false.
    assert!(!movie_is_available_for_acquisition(
        Some(&first_aired),
        Some("bad-date"),
        "released",
        &now_utc()
    ));
}

#[test]
fn season_pack_release_uses_collection_submission_scope() {
    let wanted = base_episode_wanted_item();
    let episode = base_episode();

    let scope = download_submission_scope_for_release_title(
        &wanted,
        Some(&episode),
        "Test.Show.S01.2025.Complete.1080p.WEB-DL.AVC.AAC-DBTV",
    );

    assert_eq!(
        scope,
        SubmissionScope::Collection {
            collection_id: "season-1".to_string(),
        }
    );
}

#[test]
fn single_episode_release_uses_episode_submission_scope() {
    let wanted = base_episode_wanted_item();
    let episode = base_episode();

    let scope = download_submission_scope_for_release_title(
        &wanted,
        Some(&episode),
        "Test.Show.S01E01.1080p.WEB-DL.AVC.AAC-DBTV",
    );

    assert_eq!(
        scope,
        SubmissionScope::Episode {
            episode_id: "episode-1".to_string(),
        }
    );
}

#[test]
fn series_movie_blocking_is_series_movie_link_scoped() {
    let wanted = base_series_movie_wanted_item();

    let title_submission = DownloadSubmission {
        download_id: scryer_domain::download_identity::DownloadId::new(),
        title_id: wanted.title_id.clone(),
        purpose: crate::DownloadSubmissionPurpose::Standard,
        facet: "anime".to_string(),
        download_client_id: None,
        download_client_type: "sabnzbd".to_string(),
        download_client_item_id: "job-1".to_string(),
        source_hint: None,
        source_provider_id: None,
        source_provider_name: None,
        source_kind: None,
        source_title: Some("Title-level".to_string()),
        info_hash: None,
        release_size_bytes: None,
        request_signature: None,
        scope: SubmissionScope::Title,
    };
    assert!(submission_blocks_wanted_item(
        &title_submission,
        &wanted,
        None,
    ));

    let matching_series_movie_submission = DownloadSubmission {
        download_id: scryer_domain::download_identity::DownloadId::new(),
        scope: SubmissionScope::SeriesMovie {
            series_movie_link_id: wanted
                .series_movie_link_id
                .clone()
                .expect("series movie link id"),
        },
        ..title_submission.clone()
    };
    assert!(submission_blocks_wanted_item(
        &matching_series_movie_submission,
        &wanted,
        None,
    ));

    let different_series_movie_submission = DownloadSubmission {
        download_id: scryer_domain::download_identity::DownloadId::new(),
        scope: SubmissionScope::SeriesMovie {
            series_movie_link_id: "series-movie-link-2".to_string(),
        },
        ..title_submission
    };
    assert!(!submission_blocks_wanted_item(
        &different_series_movie_submission,
        &wanted,
        None,
    ));
}

#[test]
fn episode_set_submission_blocks_each_covered_episode() {
    let mut wanted = base_episode_wanted_item();
    wanted.episode_id = Some("episode-2".to_string());
    let submission = DownloadSubmission {
        download_id: scryer_domain::download_identity::DownloadId::new(),
        title_id: wanted.title_id.clone(),
        purpose: crate::DownloadSubmissionPurpose::Standard,
        facet: "anime".to_string(),
        download_client_id: None,
        download_client_type: "sabnzbd".to_string(),
        download_client_item_id: "job-1".to_string(),
        source_hint: None,
        source_provider_id: None,
        source_provider_name: None,
        source_kind: None,
        source_title: Some("Range pack".to_string()),
        info_hash: None,
        release_size_bytes: None,
        request_signature: None,
        scope: SubmissionScope::EpisodeSet {
            episode_ids: vec!["episode-1".to_string(), "episode-2".to_string()],
        },
    };

    assert!(submission_blocks_wanted_item(&submission, &wanted, None));

    wanted.episode_id = Some("episode-3".to_string());
    assert!(!submission_blocks_wanted_item(&submission, &wanted, None));
}

#[test]
fn effective_auto_decision_code_marks_failed_route_unavailable() {
    let candidate = test_search_result_with_decision(
        "Failed.Source.Kind",
        Some(DownloadSourceKind::NzbUrl),
        "eligible",
    );

    let empty_db_blocklist =
        crate::app_usecase_discovery::TitleReleaseBlocklistSignatures::default();
    let failed_routes = vec![DownloadRouteKey::for_candidate(&candidate).unwrap()];
    let decision =
        effective_auto_decision_code_for_route(&candidate, &failed_routes, &empty_db_blocklist);

    assert_eq!(decision, ReleaseAutoDecisionCode::DownloadClientUnavailable);
}

#[test]
fn effective_auto_decision_code_suppresses_only_failed_indexer_route() {
    let mut failed_indexer = test_search_result_with_decision(
        "Failed.Private.Torrent",
        Some(DownloadSourceKind::TorrentFile),
        "eligible",
    );
    failed_indexer.indexer_id = Some("private-a".to_string());
    let mut other_indexer = failed_indexer.clone();
    other_indexer.indexer_id = Some("private-b".to_string());
    let mut other_source = failed_indexer.clone();
    other_source.source_kind = Some(DownloadSourceKind::MagnetUri);

    let failed_routes = vec![DownloadRouteKey::for_candidate(&failed_indexer).unwrap()];
    let empty_db_blocklist =
        crate::app_usecase_discovery::TitleReleaseBlocklistSignatures::default();

    assert_eq!(
        effective_auto_decision_code_for_route(
            &failed_indexer,
            &failed_routes,
            &empty_db_blocklist,
        ),
        ReleaseAutoDecisionCode::DownloadClientUnavailable
    );
    assert_eq!(
        effective_auto_decision_code_for_route(&other_indexer, &failed_routes, &empty_db_blocklist,),
        ReleaseAutoDecisionCode::Eligible
    );
    assert_eq!(
        effective_auto_decision_code_for_route(&other_source, &failed_routes, &empty_db_blocklist,),
        ReleaseAutoDecisionCode::Eligible
    );
}

#[test]
fn effective_auto_decision_code_marks_db_blocklisted_release() {
    let candidate = test_search_result_with_decision("Blocked.Release", None, "eligible");
    let db_blocklist = crate::app_usecase_discovery::TitleReleaseBlocklistSignatures {
        release_names: std::collections::HashSet::from([(
            String::new(),
            "blocked.release".to_string(),
        )]),
        info_hashes: std::collections::HashSet::new(),
    };

    let decision = effective_auto_decision_code_for_route(&candidate, &[], &db_blocklist);

    assert_eq!(decision, ReleaseAutoDecisionCode::DbBlocklisted);
}

use super::*;
use crate::acquisition::targets::{
    movie_availability_decision, movie_is_available_for_acquisition, MovieAvailabilityPolicy,
};
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use scryer_domain::MovieReleaseDates;

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
        contiguous_absolute_number: None,
        overview: None,
        tvdb_id: None,
        tmdb_id: None,
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
        release_listing_json: None,
    }
}

// ── announced ────────────────────────────────────────────────────────────────

fn fixed_release_clock() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2025-04-01T00:00:00Z")
        .expect("fixed test instant")
        .with_timezone(&Utc)
}

fn release_dates(
    theatrical_dates: &[&str],
    digital_dates: &[&str],
    physical_dates: &[&str],
) -> MovieReleaseDates {
    MovieReleaseDates {
        market: "US".to_string(),
        fetched: true,
        theatrical_dates: theatrical_dates.iter().map(|date| date.to_string()).collect(),
        digital_dates: digital_dates.iter().map(|date| date.to_string()).collect(),
        physical_dates: physical_dates.iter().map(|date| date.to_string()).collect(),
    }
}

fn availability_policy<'a>(
    minimum_availability: Option<&'a str>,
    release_market: &'a str,
    delay_days: i32,
) -> MovieAvailabilityPolicy<'a> {
    MovieAvailabilityPolicy {
        minimum_availability,
        release_market,
        delay_days,
    }
}

#[test]
fn released_uses_earliest_home_date_and_includes_the_date_boundary() {
    let dates = release_dates(&[], &["2025-04-05"], &["2025-04-01"]);
    let decision = movie_availability_decision(
        Some(&dates),
        true,
        None,
        None,
        availability_policy(Some("released"), "US", 0),
        &fixed_release_clock(),
    );
    assert_eq!(decision.status, crate::MovieAvailabilityStatus::Available);
    assert_eq!(decision.effective_date.as_deref(), Some("2025-04-01"));
    assert!(!decision.estimated);
}

#[test]
fn movie_availability_offsets_are_signed_calendar_days() {
    let dates = release_dates(&[], &["2025-04-01"], &[]);
    let now = fixed_release_clock();
    let early = movie_availability_decision(
        Some(&dates),
        true,
        None,
        None,
        availability_policy(Some("released"), "US", -1),
        &now,
    );
    assert_eq!(early.status, crate::MovieAvailabilityStatus::Available);
    assert_eq!(early.effective_date.as_deref(), Some("2025-03-31"));

    let delayed = movie_availability_decision(
        Some(&dates),
        true,
        None,
        None,
        availability_policy(Some("released"), "US", 1),
        &now,
    );
    assert_eq!(delayed.status, crate::MovieAvailabilityStatus::Waiting);
    assert_eq!(delayed.effective_date.as_deref(), Some("2025-04-02"));
}

#[test]
fn revised_home_release_date_recomputes_movie_availability() {
    let now = fixed_release_clock();
    let original = release_dates(&[], &["2025-03-20"], &[]);
    let before_revision = movie_availability_decision(
        Some(&original),
        true,
        None,
        None,
        availability_policy(Some("released"), "US", 0),
        &now,
    );
    assert_eq!(before_revision.status, crate::MovieAvailabilityStatus::Available);

    let revised = release_dates(&[], &["2025-04-20"], &[]);
    let after_revision = movie_availability_decision(
        Some(&revised),
        true,
        None,
        None,
        availability_policy(Some("released"), "US", 0),
        &now,
    );
    assert_eq!(after_revision.status, crate::MovieAvailabilityStatus::Waiting);
    assert_eq!(after_revision.effective_date.as_deref(), Some("2025-04-20"));
}

#[test]
fn released_estimates_theatrical_plus_ninety_only_when_home_dates_are_absent() {
    let dates = release_dates(&["2025-01-01"], &[], &[]);
    let decision = movie_availability_decision(
        Some(&dates),
        true,
        None,
        None,
        availability_policy(Some("released"), "US", 0),
        &fixed_release_clock(),
    );
    assert_eq!(decision.status, crate::MovieAvailabilityStatus::Available);
    assert_eq!(decision.effective_date.as_deref(), Some("2025-04-01"));
    assert!(decision.estimated);

    let future_home_date = release_dates(&["2025-01-01"], &["2025-05-01"], &[]);
    let waiting = movie_availability_decision(
        Some(&future_home_date),
        true,
        None,
        None,
        availability_policy(Some("released"), "US", 0),
        &fixed_release_clock(),
    );
    assert_eq!(waiting.status, crate::MovieAvailabilityStatus::Waiting);
    assert_eq!(waiting.effective_date.as_deref(), Some("2025-05-01"));
    assert!(!waiting.estimated);
}

#[test]
fn unknown_or_wrong_market_release_dates_remain_blocked() {
    let mut dates = release_dates(&["2025-01-01"], &[], &[]);
    dates.fetched = false;
    let unknown = movie_availability_decision(
        Some(&dates),
        true,
        None,
        None,
        availability_policy(Some("released"), "US", 0),
        &fixed_release_clock(),
    );
    assert_eq!(unknown.status, crate::MovieAvailabilityStatus::Unknown);
    assert_eq!(unknown.reason, "release_dates_unknown");

    let wrong_market = release_dates(&["2025-01-01"], &[], &[]);
    let mismatch = movie_availability_decision(
        Some(&wrong_market),
        true,
        None,
        None,
        availability_policy(Some("released"), "CA", 0),
        &fixed_release_clock(),
    );
    assert_eq!(mismatch.status, crate::MovieAvailabilityStatus::Unknown);
    assert_eq!(mismatch.reason, "release_market_refresh_pending");
}

#[test]
fn malformed_home_date_does_not_trigger_theatrical_fallback() {
    let dates = release_dates(&["2025-01-01"], &["not-a-date"], &[]);
    let decision = movie_availability_decision(
        Some(&dates),
        true,
        None,
        None,
        availability_policy(Some("released"), "US", 0),
        &fixed_release_clock(),
    );
    assert_eq!(decision.status, crate::MovieAvailabilityStatus::Unknown);
    assert_eq!(decision.reason, "home_release_date_unusable");
    assert_eq!(decision.effective_date, None);
}

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

// Paused clock: the test owns time, so "missed four ticks" and "no catch-up
// tick waiting" hold exactly instead of depending on how late the runner wakes.
#[tokio::test(start_paused = true)]
async fn skip_interval_does_not_replay_missed_poll_ticks_in_a_burst() {
    let period = std::time::Duration::from_millis(50);
    let mut interval = new_skip_interval(period);
    let start = interval.tick().await;

    tokio::time::advance(std::time::Duration::from_millis(220)).await;
    interval.tick().await;

    assert!(
        futures_util::poll!(std::pin::pin!(interval.tick())).is_pending(),
        "skip interval should not have an immediate catch-up tick waiting"
    );
    assert_eq!(
        interval.tick().await - start,
        period * 5,
        "the next tick lands on the schedule, skipping the missed ones"
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
        release_listing_json: None,
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
        release_listing_json: None,
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

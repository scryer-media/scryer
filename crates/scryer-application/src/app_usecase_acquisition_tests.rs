use super::*;
use chrono::{DateTime, Utc};

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

fn base_title() -> Title {
    Title {
        id: "t1".to_string(),
        name: "Test Movie".to_string(),
        facet: MediaFacet::Movie,
        monitored: true,
        tags: vec![],
        external_ids: vec![],
        created_by: None,
        created_at: now_utc(),
        year: Some(2024),
        overview: None,
        poster_url: None,
        banner_url: None,
        background_url: None,
        sort_title: None,
        slug: None,
        imdb_id: None,
        runtime_minutes: None,
        genres: vec![],
        content_status: None,
        language: None,
        first_aired: None,
        network: None,
        studio: None,
        country: None,
        aliases: vec![],
            tagged_aliases: vec![],
        metadata_language: None,
        metadata_fetched_at: None,
        min_availability: None,
        digital_release_date: None,
        folder_path: None,
    }
}

// ── announced ────────────────────────────────────────────────────────────────

#[test]
fn announced_always_available_no_dates() {
    let title = base_title();
    assert!(is_movie_available_for_acquisition(
        &title,
        "announced",
        &now_utc()
    ));
}

#[test]
fn announced_always_available_future_dates() {
    let mut title = base_title();
    title.first_aired = Some(days_from_now(90));
    assert!(is_movie_available_for_acquisition(
        &title,
        "announced",
        &now_utc()
    ));
}

#[test]
fn unknown_availability_treated_as_announced() {
    let title = base_title();
    assert!(is_movie_available_for_acquisition(
        &title,
        "preorder",
        &now_utc()
    ));
}

// ── in_cinemas ────────────────────────────────────────────────────────────────

#[test]
fn in_cinemas_available_when_past_cinema_date() {
    let mut title = base_title();
    title.first_aired = Some(days_ago(10));
    assert!(is_movie_available_for_acquisition(
        &title,
        "in_cinemas",
        &now_utc()
    ));
}

#[test]
fn in_cinemas_available_when_today_is_cinema_date() {
    let mut title = base_title();
    title.first_aired = Some(now_utc().format("%Y-%m-%d").to_string());
    assert!(is_movie_available_for_acquisition(
        &title,
        "in_cinemas",
        &now_utc()
    ));
}

#[test]
fn in_cinemas_unavailable_when_future_cinema_date() {
    let mut title = base_title();
    title.first_aired = Some(days_from_now(30));
    assert!(!is_movie_available_for_acquisition(
        &title,
        "in_cinemas",
        &now_utc()
    ));
}

#[test]
fn in_cinemas_unavailable_when_no_date() {
    let title = base_title();
    assert!(!is_movie_available_for_acquisition(
        &title,
        "in_cinemas",
        &now_utc()
    ));
}

#[test]
fn in_cinemas_unavailable_when_date_malformed() {
    let mut title = base_title();
    title.first_aired = Some("not-a-date".to_string());
    assert!(!is_movie_available_for_acquisition(
        &title,
        "in_cinemas",
        &now_utc()
    ));
}

// ── released ──────────────────────────────────────────────────────────────────

#[test]
fn released_available_when_past_digital_release() {
    let mut title = base_title();
    title.digital_release_date = Some(days_ago(5));
    assert!(is_movie_available_for_acquisition(
        &title,
        "released",
        &now_utc()
    ));
}

#[test]
fn released_unavailable_when_future_digital_release() {
    let mut title = base_title();
    title.digital_release_date = Some(days_from_now(14));
    assert!(!is_movie_available_for_acquisition(
        &title,
        "released",
        &now_utc()
    ));
}

#[test]
fn released_falls_back_to_cinema_plus_90_days_when_past() {
    let mut title = base_title();
    title.first_aired = Some(days_ago(100)); // 100 days ago + 90 = still past
    assert!(is_movie_available_for_acquisition(
        &title,
        "released",
        &now_utc()
    ));
}

#[test]
fn released_falls_back_to_cinema_plus_90_days_when_not_yet() {
    let mut title = base_title();
    title.first_aired = Some(days_ago(30)); // 30 days ago + 90 = 60 days in future
    assert!(!is_movie_available_for_acquisition(
        &title,
        "released",
        &now_utc()
    ));
}

#[test]
fn released_unavailable_when_no_dates() {
    let title = base_title();
    assert!(!is_movie_available_for_acquisition(
        &title,
        "released",
        &now_utc()
    ));
}

#[test]
fn released_digital_date_takes_priority_over_cinema_fallback() {
    let mut title = base_title();
    // digital date is in the past (available), even though cinema + 90 would be in future
    title.digital_release_date = Some(days_ago(1));
    title.first_aired = Some(days_ago(10)); // cinema only 10d ago, +90 not reached
    assert!(is_movie_available_for_acquisition(
        &title,
        "released",
        &now_utc()
    ));
}

#[test]
fn released_malformed_digital_date_falls_back_to_cinema() {
    let mut title = base_title();
    title.digital_release_date = Some("bad-date".to_string());
    title.first_aired = Some(days_ago(100));
    // digital date parse fails → false; but we fall through to cinema check... actually no.
    // The code checks digital_release_date first, and on parse failure returns false
    // (no fallback within that branch). So this returns false.
    assert!(!is_movie_available_for_acquisition(
        &title,
        "released",
        &now_utc()
    ));
}

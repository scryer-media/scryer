use super::*;

// ── helpers ──────────────────────────────────────────────────────────────────

fn make_event(message: &str) -> ActivityEvent {
    ActivityEvent::with_default_channels(
        ActivityKind::SystemNotice,
        None,
        None,
        message.to_string(),
    )
}

// ── ActivityStream::push ──────────────────────────────────────────────────────

#[tokio::test]
async fn push_adds_event_to_stream() {
    let stream = ActivityStream::new();
    stream.push(make_event("hello")).await;
    let events = stream.list(10, 0).await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].message, "hello");
}

#[tokio::test]
async fn push_evicts_oldest_when_over_limit() {
    let stream = ActivityStream::new();
    for i in 0..=ACTIVITY_EVENT_LIMIT {
        stream.push(make_event(&format!("event-{i}"))).await;
    }
    let events = stream.list(200, 0).await;
    assert_eq!(events.len(), ACTIVITY_EVENT_LIMIT);
    // list() returns newest first; last pushed should be first
    assert_eq!(events[0].message, format!("event-{}", ACTIVITY_EVENT_LIMIT));
    // event-0 was the oldest and must have been evicted
    assert!(events.iter().all(|e| e.message != "event-0"));
}

#[tokio::test]
async fn push_preserves_order_within_limit() {
    let stream = ActivityStream::new();
    stream.push(make_event("first")).await;
    stream.push(make_event("second")).await;
    let events = stream.list(10, 0).await;
    assert_eq!(events[0].message, "second");
    assert_eq!(events[1].message, "first");
}

// ── ActivityStream::list ──────────────────────────────────────────────────────

#[tokio::test]
async fn list_empty_stream_returns_empty() {
    let stream = ActivityStream::new();
    let events = stream.list(10, 0).await;
    assert!(events.is_empty());
}

#[tokio::test]
async fn list_with_limit_truncates_results() {
    let stream = ActivityStream::new();
    for i in 0..5 {
        stream.push(make_event(&format!("event-{i}"))).await;
    }
    let events = stream.list(2, 0).await;
    assert_eq!(events.len(), 2);
}

#[tokio::test]
async fn list_with_offset_skips_newest() {
    let stream = ActivityStream::new();
    for i in 0..5 {
        stream.push(make_event(&format!("event-{i}"))).await;
    }
    // offset=2 skips the 2 newest; 3 remain
    let events = stream.list(100, 2).await;
    assert_eq!(events.len(), 3);
}

#[tokio::test]
async fn list_negative_limit_defaults_to_event_limit() {
    let stream = ActivityStream::new();
    for i in 0..5 {
        stream.push(make_event(&format!("event-{i}"))).await;
    }
    // negative limit → default (ACTIVITY_EVENT_LIMIT=100), which covers all 5
    let events = stream.list(-1, 0).await;
    assert_eq!(events.len(), 5);
}

#[tokio::test]
async fn list_negative_offset_treated_as_zero() {
    let stream = ActivityStream::new();
    stream.push(make_event("only")).await;
    let events_neg = stream.list(10, -5).await;
    let events_zero = stream.list(10, 0).await;
    assert_eq!(events_neg.len(), events_zero.len());
    assert_eq!(events_neg[0].message, events_zero[0].message);
}

#[tokio::test]
async fn list_offset_beyond_length_returns_empty() {
    let stream = ActivityStream::new();
    stream.push(make_event("only")).await;
    let events = stream.list(10, 10).await;
    assert!(events.is_empty());
}

// ── enum string mappings ──────────────────────────────────────────────────────

#[test]
fn activity_kind_as_str_all_variants() {
    assert_eq!(ActivityKind::SettingSaved.as_str(), "setting_saved");
    assert_eq!(ActivityKind::MovieFetched.as_str(), "movie_fetched");
    assert_eq!(ActivityKind::MovieAdded.as_str(), "movie_added");
    assert_eq!(
        ActivityKind::MetadataHydrationStarted.as_str(),
        "metadata_hydration_started"
    );
    assert_eq!(
        ActivityKind::MetadataHydrationCompleted.as_str(),
        "metadata_hydration_completed"
    );
    assert_eq!(
        ActivityKind::MetadataHydrationFailed.as_str(),
        "metadata_hydration_failed"
    );
    assert_eq!(ActivityKind::MovieDownloaded.as_str(), "movie_downloaded");
    assert_eq!(
        ActivityKind::SeriesEpisodeImported.as_str(),
        "series_episode_imported"
    );
    assert_eq!(
        ActivityKind::AcquisitionSearchCompleted.as_str(),
        "acquisition_search_completed"
    );
    assert_eq!(
        ActivityKind::AcquisitionCandidateAccepted.as_str(),
        "acquisition_candidate_accepted"
    );
    assert_eq!(
        ActivityKind::AcquisitionCandidateRejected.as_str(),
        "acquisition_candidate_rejected"
    );
    assert_eq!(
        ActivityKind::AcquisitionDownloadFailed.as_str(),
        "acquisition_download_failed"
    );
    assert_eq!(ActivityKind::SystemNotice.as_str(), "system_notice");
}

#[test]
fn activity_severity_as_str_all_variants() {
    assert_eq!(ActivitySeverity::Info.as_str(), "info");
    assert_eq!(ActivitySeverity::Success.as_str(), "success");
    assert_eq!(ActivitySeverity::Warning.as_str(), "warning");
    assert_eq!(ActivitySeverity::Error.as_str(), "error");
}

#[test]
fn activity_channel_as_str_all_variants() {
    assert_eq!(ActivityChannel::WebUi.as_str(), "web_ui");
    assert_eq!(ActivityChannel::Toast.as_str(), "toast");
}

use super::*;
use chrono::Utc;
use scryer_domain::{ExternalId, MediaFacet, Title};

fn sample_title() -> Title {
    Title {
        id: "title-1".to_string(),
        name: "Example Show".to_string(),
        facet: MediaFacet::Series,
        monitored: true,
        tags: vec![],
        external_ids: vec![
            ExternalId {
                source: "tvdb".to_string(),
                value: "123".to_string(),
            },
            ExternalId {
                source: "imdb".to_string(),
                value: "tt456".to_string(),
            },
        ],
        created_by: None,
        created_at: Utc::now(),
        year: Some(2024),
        overview: None,
        poster_url: Some("https://example.invalid/poster.jpg".to_string()),
        poster_source_url: None,
        banner_url: None,
        banner_source_url: None,
        background_url: None,
        background_source_url: None,
        sort_title: None,
        slug: None,
        imdb_id: Some("tt456".to_string()),
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

// ── enum string mappings ──────────────────────────────────────────────────────

#[test]
fn activity_kind_as_str_all_variants() {
    assert_eq!(ActivityKind::SettingSaved.as_str(), "setting_saved");
    assert_eq!(ActivityKind::MovieFetched.as_str(), "movie_fetched");
    assert_eq!(ActivityKind::MovieAdded.as_str(), "movie_added");
    assert_eq!(ActivityKind::TitleUpdated.as_str(), "title_updated");
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

#[test]
fn lifecycle_metadata_includes_updates_and_external_ids() {
    let metadata = build_lifecycle_notification_metadata(
        &sample_title(),
        [
            NotificationMediaUpdate::deleted("/data/old.mkv"),
            NotificationMediaUpdate::created("/data/new.mkv"),
        ],
    );

    assert_eq!(
        metadata
            .get("title_name")
            .and_then(serde_json::Value::as_str),
        Some("Example Show")
    );
    assert_eq!(
        metadata
            .get("title_facet")
            .and_then(serde_json::Value::as_str),
        Some("series")
    );
    assert_eq!(
        metadata
            .get("file_path")
            .and_then(serde_json::Value::as_str),
        Some("/data/old.mkv")
    );
    assert_eq!(metadata["external_ids"]["tvdb_id"].as_str(), Some("123"));
    assert_eq!(metadata["external_ids"]["imdb_id"].as_str(), Some("tt456"));
    assert_eq!(metadata["media_updates"].as_array().map(Vec::len), Some(2));
}

use super::*;
use chrono::{DateTime, NaiveDate, Utc};

const METADATA_REFRESH_MINIMUM_INTERVAL: chrono::Duration = chrono::Duration::hours(12);
const EPISODIC_ACTIVE_MONITORED_INTERVAL: chrono::Duration = chrono::Duration::hours(12);
const EPISODIC_ACTIVE_UNMONITORED_INTERVAL: chrono::Duration = chrono::Duration::hours(24);
const EPISODIC_INACTIVE_INTERVAL: chrono::Duration = chrono::Duration::days(14);
const MOVIE_PRERELEASE_INTERVAL: chrono::Duration = chrono::Duration::hours(12);
const MOVIE_RECENT_RELEASE_INTERVAL: chrono::Duration = chrono::Duration::hours(24);
const MOVIE_OLD_RELEASE_INTERVAL: chrono::Duration = chrono::Duration::days(30);
const MOVIE_RECENT_RELEASE_WINDOW: chrono::Duration = chrono::Duration::days(14);
const BACKGROUND_METADATA_REFRESH_MAX_TITLES_PER_RUN: usize = 100;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum LibraryScanMetadataRefreshMode {
    BackgroundRefresh,
    UserInitiatedTitleRefresh,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MetadataRefreshTransport {
    SingleApq,
    Bulk,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MetadataRefreshClass {
    EpisodicActive,
    EpisodicInactive,
    MoviePrerelease,
    MovieRecentRelease,
    MovieOldRelease,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct MetadataRefreshDecision {
    pub class: MetadataRefreshClass,
    pub transport: MetadataRefreshTransport,
    pub forced: bool,
    interval: chrono::Duration,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct MetadataRefreshRunSummary {
    pub considered: usize,
    pub refreshed: usize,
    pub failed: usize,
    pub single_apq_requests: usize,
    pub bulk_titles: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct RecommendationRefreshRunSummary {
    pub considered: usize,
    pub queued: usize,
    pub failed: usize,
}

struct PendingRefresh {
    index: usize,
    target: crate::catalog_workflow::HydrationTarget,
}

pub(super) async fn refresh_titles_metadata_for_scan_policy(
    app: &AppUseCase,
    titles: &mut [Title],
    forced_ids: &HashSet<String>,
    mode: LibraryScanMetadataRefreshMode,
) -> AppResult<MetadataRefreshRunSummary> {
    if titles.is_empty() {
        return Ok(MetadataRefreshRunSummary::default());
    }

    let now = Utc::now();
    let mut summary = MetadataRefreshRunSummary::default();
    let mut single = Vec::new();
    let mut bulk = Vec::new();
    let max_titles = match mode {
        LibraryScanMetadataRefreshMode::BackgroundRefresh => {
            BACKGROUND_METADATA_REFRESH_MAX_TITLES_PER_RUN
        }
        LibraryScanMetadataRefreshMode::UserInitiatedTitleRefresh => usize::MAX,
    };

    for (index, title) in titles.iter().enumerate() {
        if summary.considered >= max_titles {
            break;
        }
        let forced = forced_ids.contains(&title.id);
        let Some(decision) = title_metadata_refresh_decision(title, now, forced) else {
            continue;
        };
        summary.considered = summary.considered.saturating_add(1);
        let target = crate::catalog_workflow::HydrationTarget {
            title: title.clone(),
            requested_tvdb_id: None,
            requested_movie_ref: None,
            sync_wanted_after_completion: false,
            source: metadata_refresh_hydration_source(mode),
        };
        let pending = PendingRefresh { index, target };
        match decision.transport {
            MetadataRefreshTransport::SingleApq => single.push(pending),
            MetadataRefreshTransport::Bulk => bulk.push(pending),
        }
    }

    for pending in single {
        summary.single_apq_requests = summary.single_apq_requests.saturating_add(1);
        match app.hydrate_title_single_apq(pending.target).await {
            Ok(refreshed) => {
                titles[pending.index] = refreshed;
                summary.refreshed = summary.refreshed.saturating_add(1);
            }
            Err(error) => {
                summary.failed = summary.failed.saturating_add(1);
                warn!(
                    error = %error,
                    title_id = %titles[pending.index].id,
                    title_name = %titles[pending.index].name,
                    "metadata refresh single-title request failed"
                );
            }
        }
    }

    for chunk in bulk.chunks(crate::catalog_workflow::HYDRATION_BULK_BATCH_SIZE) {
        let targets = chunk
            .iter()
            .map(|pending| pending.target.clone())
            .collect::<Vec<_>>();
        summary.bulk_titles = summary.bulk_titles.saturating_add(targets.len());
        let outcome = match app.hydrate_titles_bulk(targets).await {
            Ok(outcome) => outcome,
            Err(error) => {
                summary.failed = summary.failed.saturating_add(chunk.len());
                warn!(
                    error = %error,
                    target_count = chunk.len(),
                    mode = ?mode,
                    "metadata refresh bulk request failed; continuing scan policy work"
                );
                continue;
            }
        };
        for pending in chunk {
            if let Some(refreshed) = outcome.hydrated_titles.get(&pending.target.title.id) {
                titles[pending.index] = refreshed.clone();
                summary.refreshed = summary.refreshed.saturating_add(1);
            } else if outcome.failed_titles.contains_key(&pending.target.title.id) {
                summary.failed = summary.failed.saturating_add(1);
            }
        }
    }

    if summary.considered > 0 {
        debug!(
            considered = summary.considered,
            refreshed = summary.refreshed,
            failed = summary.failed,
            single_apq_requests = summary.single_apq_requests,
            bulk_titles = summary.bulk_titles,
            mode = ?mode,
            "scan policy metadata refresh complete"
        );
    }

    Ok(summary)
}

pub(super) async fn queue_title_recommendations_for_background_refresh(
    app: &AppUseCase,
    titles: &[Title],
) -> RecommendationRefreshRunSummary {
    let mut summary = RecommendationRefreshRunSummary::default();
    for title in titles {
        summary.considered = summary.considered.saturating_add(1);
        match app
            .queue_title_more_like_this_refresh_if_due(
                title,
                crate::catalog_workflow::HydrationSource::BackgroundDue,
            )
            .await
        {
            Ok(true) => {
                summary.queued = summary.queued.saturating_add(1);
            }
            Ok(false) => {}
            Err(error) => {
                summary.failed = summary.failed.saturating_add(1);
                warn!(
                    error = %error,
                    title_id = %title.id,
                    title_name = %title.name,
                    "failed to queue background title recommendations refresh; continuing library refresh"
                );
            }
        }
    }

    if summary.queued > 0 || summary.failed > 0 {
        debug!(
            considered = summary.considered,
            queued = summary.queued,
            failed = summary.failed,
            "background title recommendations refresh queue pass complete"
        );
    }

    summary
}

fn metadata_refresh_hydration_source(
    mode: LibraryScanMetadataRefreshMode,
) -> crate::catalog_workflow::HydrationSource {
    match mode {
        LibraryScanMetadataRefreshMode::BackgroundRefresh => {
            crate::catalog_workflow::HydrationSource::BackgroundDue
        }
        LibraryScanMetadataRefreshMode::UserInitiatedTitleRefresh => {
            crate::catalog_workflow::HydrationSource::Interactive
        }
    }
}

pub(super) fn title_metadata_refresh_decision(
    title: &Title,
    now: DateTime<Utc>,
    forced: bool,
) -> Option<MetadataRefreshDecision> {
    let (class, transport, interval) = title_refresh_class(title, now)?;
    let interval = interval.max(METADATA_REFRESH_MINIMUM_INTERVAL);
    if !forced
        && let Some(fetched_at) = title.metadata_fetched_at
        && fetched_at + interval > now
    {
        return None;
    }

    Some(MetadataRefreshDecision {
        class,
        transport,
        forced,
        interval,
    })
}

fn title_refresh_class(
    title: &Title,
    now: DateTime<Utc>,
) -> Option<(
    MetadataRefreshClass,
    MetadataRefreshTransport,
    chrono::Duration,
)> {
    match title.facet {
        MediaFacet::Series | MediaFacet::Anime => {
            if episodic_status_is_inactive(title.content_status.as_deref()) {
                Some((
                    MetadataRefreshClass::EpisodicInactive,
                    MetadataRefreshTransport::Bulk,
                    EPISODIC_INACTIVE_INTERVAL,
                ))
            } else {
                let interval = if title.monitored {
                    EPISODIC_ACTIVE_MONITORED_INTERVAL
                } else {
                    EPISODIC_ACTIVE_UNMONITORED_INTERVAL
                };
                Some((
                    MetadataRefreshClass::EpisodicActive,
                    MetadataRefreshTransport::SingleApq,
                    interval,
                ))
            }
        }
        MediaFacet::Movie => {
            if movie_is_prerelease(title, now) {
                return Some((
                    MetadataRefreshClass::MoviePrerelease,
                    MetadataRefreshTransport::SingleApq,
                    MOVIE_PRERELEASE_INTERVAL,
                ));
            }

            if movie_is_recently_released(title, now) {
                Some((
                    MetadataRefreshClass::MovieRecentRelease,
                    MetadataRefreshTransport::Bulk,
                    MOVIE_RECENT_RELEASE_INTERVAL,
                ))
            } else {
                Some((
                    MetadataRefreshClass::MovieOldRelease,
                    MetadataRefreshTransport::Bulk,
                    MOVIE_OLD_RELEASE_INTERVAL,
                ))
            }
        }
    }
}

fn normalized_status(value: Option<&str>) -> Option<String> {
    let normalized = value?
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>();
    (!normalized.is_empty()).then_some(normalized)
}

fn episodic_status_is_inactive(value: Option<&str>) -> bool {
    matches!(
        normalized_status(value).as_deref(),
        Some("ended" | "canceled" | "cancelled" | "deleted")
    )
}

fn movie_is_prerelease(title: &Title, now: DateTime<Utc>) -> bool {
    if title
        .digital_release_date
        .as_deref()
        .and_then(parse_title_date)
        .is_some_and(|date| date > now.date_naive())
    {
        return true;
    }

    matches!(
        normalized_status(title.content_status.as_deref()).as_deref(),
        Some(
            "announced"
                | "incinemas"
                | "planned"
                | "upcoming"
                | "preproduction"
                | "postproduction"
                | "inproduction"
        )
    )
}

fn movie_is_recently_released(title: &Title, now: DateTime<Utc>) -> bool {
    let Some(release_date) = title
        .digital_release_date
        .as_deref()
        .and_then(parse_title_date)
    else {
        return false;
    };
    let today = now.date_naive();
    release_date <= today && release_date >= today - MOVIE_RECENT_RELEASE_WINDOW
}

fn parse_title_date(value: &str) -> Option<NaiveDate> {
    let value = value.trim();
    if value.len() >= 10 {
        NaiveDate::parse_from_str(&value[..10], "%Y-%m-%d").ok()
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn title(facet: MediaFacet) -> Title {
        Title {
            id: "title-1".to_string(),
            library_id: "library-1".to_string(),
            name: "Example Title".to_string(),
            facet,
            monitored: false,
            tags: Vec::new(),
            canonical_tags: vec![],
            external_ids: vec![ExternalId {
                source: "tvdb".to_string(),
                value: "123".to_string(),
            }],
            root_folder_id: "root-1".to_string(),
            created_by: None,
            created_at: DateTime::from_timestamp(0, 0).expect("valid timestamp"),
            year: None,
            overview: None,
            poster_url: None,
            poster_source_url: None,
            background_url: None,
            background_source_url: None,
            sort_title: None,
            catalog_sort_key: "example title".to_string(),
            slug: None,
            imdb_id: None,
            runtime_minutes: None,
            popularity: None,
            content_status: None,
            language: None,
            first_aired: None,
            network: None,
            studio: None,
            country: None,
            aliases: Vec::new(),
            tagged_aliases: Vec::new(),
            metadata_language: Some("en".to_string()),
            metadata_fetched_at: Some(DateTime::from_timestamp(0, 0).expect("valid timestamp")),
            min_availability: None,
            digital_release_date: None,
            folder_path: None,
        }
    }

    #[test]
    fn active_monitored_episodic_is_due_after_twelve_hours() {
        let now = DateTime::from_timestamp(12 * 60 * 60, 0).expect("valid timestamp");
        let mut title = title(MediaFacet::Series);
        title.monitored = true;
        title.content_status = Some("Continuing".to_string());

        let decision = title_metadata_refresh_decision(&title, now, false).expect("due");
        assert_eq!(decision.class, MetadataRefreshClass::EpisodicActive);
        assert_eq!(decision.transport, MetadataRefreshTransport::SingleApq);
        assert_eq!(decision.interval, chrono::Duration::hours(12));
    }

    #[test]
    fn never_hydrated_title_is_due_for_refresh() {
        let now = DateTime::from_timestamp(60, 0).expect("valid timestamp");
        let mut title = title(MediaFacet::Series);
        title.monitored = true;
        title.content_status = Some("Continuing".to_string());
        title.metadata_fetched_at = None;

        let decision = title_metadata_refresh_decision(&title, now, false).expect("due");
        assert_eq!(decision.class, MetadataRefreshClass::EpisodicActive);
        assert_eq!(decision.transport, MetadataRefreshTransport::SingleApq);
    }

    #[test]
    fn unmonitored_active_episodic_waits_twenty_four_hours() {
        let now = DateTime::from_timestamp(12 * 60 * 60, 0).expect("valid timestamp");
        let mut title = title(MediaFacet::Anime);
        title.content_status = Some("Continuing".to_string());

        assert!(title_metadata_refresh_decision(&title, now, false).is_none());

        let later = DateTime::from_timestamp(24 * 60 * 60, 0).expect("valid timestamp");
        let decision = title_metadata_refresh_decision(&title, later, false).expect("due");
        assert_eq!(decision.interval, chrono::Duration::hours(24));
    }

    #[test]
    fn inactive_episodic_waits_fourteen_days() {
        let now = DateTime::from_timestamp(13 * 24 * 60 * 60, 0).expect("valid timestamp");
        let mut title = title(MediaFacet::Series);
        title.content_status = Some("Ended".to_string());

        assert!(title_metadata_refresh_decision(&title, now, false).is_none());

        let later = DateTime::from_timestamp(14 * 24 * 60 * 60, 0).expect("valid timestamp");
        let decision = title_metadata_refresh_decision(&title, later, false).expect("due");
        assert_eq!(decision.class, MetadataRefreshClass::EpisodicInactive);
        assert_eq!(decision.transport, MetadataRefreshTransport::Bulk);
    }

    #[test]
    fn prerelease_movie_uses_single_apq_after_twelve_hours() {
        let now = DateTime::from_timestamp(12 * 60 * 60, 0).expect("valid timestamp");
        let mut title = title(MediaFacet::Movie);
        title.content_status = Some("In Cinemas".to_string());

        let decision = title_metadata_refresh_decision(&title, now, false).expect("due");
        assert_eq!(decision.class, MetadataRefreshClass::MoviePrerelease);
        assert_eq!(decision.transport, MetadataRefreshTransport::SingleApq);
        assert_eq!(decision.interval, chrono::Duration::hours(12));
    }

    #[test]
    fn old_released_movie_waits_thirty_days() {
        let now = DateTime::from_timestamp(29 * 24 * 60 * 60, 0).expect("valid timestamp");
        let mut title = title(MediaFacet::Movie);
        title.content_status = Some("Released".to_string());
        title.digital_release_date = Some("1970-01-01".to_string());

        assert!(title_metadata_refresh_decision(&title, now, false).is_none());

        let later = DateTime::from_timestamp(30 * 24 * 60 * 60, 0).expect("valid timestamp");
        let decision = title_metadata_refresh_decision(&title, later, false).expect("due");
        assert_eq!(decision.class, MetadataRefreshClass::MovieOldRelease);
        assert_eq!(decision.interval, chrono::Duration::days(30));
    }

    #[test]
    fn forced_refresh_bypasses_minimum_floor() {
        let now = DateTime::from_timestamp(60, 0).expect("valid timestamp");
        let mut title = title(MediaFacet::Series);
        title.monitored = true;
        title.content_status = Some("Continuing".to_string());

        assert!(title_metadata_refresh_decision(&title, now, false).is_none());
        let decision = title_metadata_refresh_decision(&title, now, true).expect("forced");
        assert!(decision.forced);
    }
}

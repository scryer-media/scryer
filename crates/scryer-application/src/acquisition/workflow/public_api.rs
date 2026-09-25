const ACQUISITION_SCAN_QUIET_WAIT: std::time::Duration = std::time::Duration::from_secs(30);
const ACQUISITION_SLICE_YIELD_INTERVAL: usize = 10;
fn active_scan_facet_labels(facets: &[MediaFacet]) -> Vec<&'static str> {
    facets.iter().map(MediaFacet::as_str).collect()
}
fn push_unique_episode_id(ids: &mut Vec<String>, episode_id: Option<&str>) {
    let Some(episode_id) = episode_id.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };

    if !ids.iter().any(|existing| existing == episode_id) {
        ids.push(episode_id.to_string());
    }
}
fn normalized_non_empty_owned(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}
fn title_scoped_domain_event(
    title_id: Option<&str>,
    title: Option<&Title>,
    payload: DomainEventPayload,
) -> NewDomainEvent {
    if let Some(title) = title {
        return new_title_domain_event(None, title, payload);
    }

    if let Some(title_id) = title_id {
        return NewDomainEvent {
            event_id: Id::new().0,
            occurred_at: Utc::now(),
            actor_kind: scryer_domain::DomainEventActorKind::System,
            actor_user_id: None,
            actor_display_name: "System".to_string(),
            title_id: Some(title_id.to_string()),
            facet: None,
            correlation_id: None,
            causation_id: None,
            schema_version: 1,
            stream: DomainEventStream::Title {
                title_id: title_id.to_string(),
            },
            payload,
        };
    }

    new_global_domain_event(None, payload)
}
fn episode_collection_id_for_wanted_item(
    item: &AcquisitionScopeState,
    episode: Option<&Episode>,
) -> Option<String> {
    episode
        .and_then(|episode| episode.collection_id.clone())
        .or_else(|| item.collection_id.clone())
}
fn episode_submission_scope(episode_id: Option<String>) -> SubmissionScope {
    episode_id
        .map(|episode_id| SubmissionScope::Episode { episode_id })
        .unwrap_or(SubmissionScope::Title)
}
fn collection_submission_scope(collection_id: Option<String>) -> SubmissionScope {
    collection_id
        .map(|collection_id| SubmissionScope::Collection { collection_id })
        .unwrap_or(SubmissionScope::Title)
}
fn series_movie_submission_scope(series_movie_link_id: Option<String>) -> SubmissionScope {
    series_movie_link_id
        .map(|series_movie_link_id| SubmissionScope::SeriesMovie {
            series_movie_link_id,
        })
        .unwrap_or(SubmissionScope::Title)
}
pub(crate) fn direct_download_submission_scope_for_wanted_item(
    item: &AcquisitionScopeState,
    _episode: Option<&Episode>,
) -> SubmissionScope {
    match item.media_type.as_str() {
        "episode" => episode_submission_scope(item.episode_id.clone()),
        "series_movie" => series_movie_submission_scope(item.series_movie_link_id.clone()),
        _ => SubmissionScope::Title,
    }
}
pub(crate) fn collection_download_submission_scope_for_wanted_item(
    item: &AcquisitionScopeState,
    episode: Option<&Episode>,
) -> SubmissionScope {
    match item.media_type.as_str() {
        "episode" => {
            collection_submission_scope(episode_collection_id_for_wanted_item(item, episode))
        }
        "series_movie" => series_movie_submission_scope(item.series_movie_link_id.clone()),
        _ => SubmissionScope::Title,
    }
}
fn new_skip_interval(period: std::time::Duration) -> tokio::time::Interval {
    let mut interval = tokio::time::interval(period);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    interval
}
#[cfg(test)]
#[path = "../app_usecase_acquisition_tests.rs"]
mod app_usecase_acquisition_tests;

use super::*;
use crate::domain_events::{
    DomainEventActor, new_download_queue_domain_event, new_global_domain_event,
    new_title_domain_event, title_context_snapshot,
};
use crate::event_views::{
    activity_event_from_domain_event, history_event_from_domain_event,
    title_history_record_from_domain_event, title_history_records_from_domain_event,
};
use crate::events::retention::user_facing_domain_event_types;
use scryer_domain::{
    AcquisitionCandidateRejectedEventData, AppPermission, ConfigurationChangeAction,
    ConfigurationChangedEventData, DomainEventPayload, DownloadQueueCommandAction,
    DownloadQueueItemCommandIssuedEventData, ImportRequestKind, ImportRequestedEventData,
    LibraryPermission, MediaFacet, MetadataHydrationState, MetadataHydrationUpdatedEventData,
    PostProcessingCompletedEventData, PostProcessingResult, SubtitleDownloadedEventData,
    SubtitleSearchFailedEventData, TitleUpdatedEventData,
};
use std::collections::{HashMap, HashSet};

/// Shortest trailing window the dashboard activity aggregate will honour.
pub const DASHBOARD_ACTIVITY_MIN_WINDOW_HOURS: i64 = 1;
/// Longest trailing window the dashboard activity aggregate will honour. The
/// previous window doubles the scanned range, so one week per window is the
/// practical ceiling for an interactive dashboard read.
pub const DASHBOARD_ACTIVITY_MAX_WINDOW_HOURS: i64 = 168;

async fn load_library_scan_visibility(
    app: &AppUseCase,
    actor: &User,
) -> AppResult<HashMap<MediaFacet, HashSet<String>>> {
    let mut visibility = HashMap::new();
    for facet in [MediaFacet::Movie, MediaFacet::Series, MediaFacet::Anime] {
        let library_ids = app
            .authorized_library_ids(actor, Some(facet.clone()), LibraryPermission::View)
            .await?;
        if !library_ids.is_empty() {
            visibility.insert(facet, library_ids.into_iter().collect());
        }
    }
    Ok(visibility)
}

fn library_scan_session_visible(
    session: &LibraryScanSession,
    visibility: &HashMap<MediaFacet, HashSet<String>>,
) -> bool {
    let Some(visible_library_ids) = visibility.get(&session.facet) else {
        return false;
    };
    match session.library_id.as_deref() {
        Some(library_id) => visible_library_ids.contains(library_id),
        None => !visible_library_ids.is_empty(),
    }
}

async fn load_recent_projected_domain_events<T, F>(
    app: &AppUseCase,
    mut filter: DomainEventFilter,
    target_len: usize,
    mut map: F,
) -> AppResult<Vec<T>>
where
    F: FnMut(&DomainEvent) -> Option<T>,
{
    if target_len == 0 {
        return Ok(Vec::new());
    }

    let mut projected = Vec::new();
    let mut before_sequence = None;

    loop {
        filter.after_sequence = None;
        filter.before_sequence = before_sequence;
        filter.limit = 500;

        let batch = app.services.events.domain_events.list(&filter).await?;
        if batch.is_empty() {
            break;
        }

        before_sequence = batch.last().map(|event| event.sequence);
        for event in &batch {
            if let Some(item) = map(event) {
                projected.push(item);
                if projected.len() >= target_len {
                    return Ok(projected);
                }
            }
        }

        if batch.len() < 500 {
            break;
        }
    }

    Ok(projected)
}

async fn load_recent_authorized_projected_domain_events<T, F>(
    app: &AppUseCase,
    actor: &User,
    mut filter: DomainEventFilter,
    target_len: usize,
    mut map: F,
) -> AppResult<Vec<T>>
where
    F: FnMut(&DomainEvent) -> Option<T>,
{
    if target_len == 0 {
        return Ok(Vec::new());
    }

    let allowed_library_ids = app
        .authorized_library_ids(actor, None, scryer_domain::LibraryPermission::View)
        .await?
        .into_iter()
        .collect::<HashSet<_>>();
    let mut title_library_cache = HashMap::new();
    let mut projected = Vec::new();
    let mut before_sequence = None;

    loop {
        filter.after_sequence = None;
        filter.before_sequence = before_sequence;
        filter.limit = 500;

        let batch = app.services.events.domain_events.list(&filter).await?;
        if batch.is_empty() {
            break;
        }

        before_sequence = batch.last().map(|event| event.sequence);
        for event in &batch {
            if !event_allowed(
                app,
                actor,
                event,
                &allowed_library_ids,
                &mut title_library_cache,
            )
            .await?
            {
                continue;
            }
            if let Some(item) = map(event) {
                projected.push(item);
                if projected.len() >= target_len {
                    return Ok(projected);
                }
            }
        }

        if batch.len() < 500 {
            break;
        }
    }

    Ok(projected)
}

async fn event_title_allowed(
    app: &AppUseCase,
    title_id: Option<&str>,
    allowed_library_ids: &HashSet<String>,
    title_library_cache: &mut HashMap<String, Option<String>>,
) -> AppResult<bool> {
    let Some(title_id) = title_id else {
        return Ok(false);
    };
    if let Some(library_id) = title_library_cache.get(title_id) {
        return Ok(library_id
            .as_ref()
            .is_some_and(|library_id| allowed_library_ids.contains(library_id)));
    }
    let library_id = app
        .services
        .catalog
        .titles
        .get_by_id(title_id)
        .await?
        .map(|title| title.library_id);
    let allowed = library_id
        .as_ref()
        .is_some_and(|library_id| allowed_library_ids.contains(library_id));
    title_library_cache.insert(title_id.to_string(), library_id);
    Ok(allowed)
}

async fn actor_can_view_titleless_operational_event(
    app: &AppUseCase,
    actor: &User,
) -> AppResult<bool> {
    app.has_app_permission(actor, AppPermission::ManageSystemSettings)
        .await
}

async fn require_actor_view_library(app: &AppUseCase, actor: &User) -> AppResult<()> {
    if app
        .has_any_library_permission(actor, scryer_domain::LibraryPermission::View)
        .await?
    {
        Ok(())
    } else {
        Err(AppError::Unauthorized(
            "You do not have access to this library".to_string(),
        ))
    }
}

async fn event_allowed(
    app: &AppUseCase,
    actor: &User,
    event: &DomainEvent,
    allowed_library_ids: &HashSet<String>,
    title_library_cache: &mut HashMap<String, Option<String>>,
) -> AppResult<bool> {
    if event.title_id.is_some() {
        return event_title_allowed(
            app,
            event.title_id.as_deref(),
            allowed_library_ids,
            title_library_cache,
        )
        .await;
    }

    match &event.payload {
        DomainEventPayload::ConfigurationChanged(data)
            if data.resource_type == "library"
                && data
                    .resource_id
                    .as_ref()
                    .is_some_and(|library_id| allowed_library_ids.contains(library_id)) =>
        {
            Ok(true)
        }
        DomainEventPayload::MediaRequestSubmitted(data) => {
            Ok(allowed_library_ids.contains(&data.library_id))
        }
        DomainEventPayload::MediaRequestUpdated(data) => {
            Ok(allowed_library_ids.contains(&data.library_id))
        }
        DomainEventPayload::MediaRequestApproved(data) => {
            Ok(allowed_library_ids.contains(&data.library_id))
        }
        DomainEventPayload::MediaRequestRejected(data) => {
            Ok(allowed_library_ids.contains(&data.library_id))
        }
        DomainEventPayload::MediaRequestCanceled(data) => {
            Ok(allowed_library_ids.contains(&data.library_id))
        }
        DomainEventPayload::LibraryScanStarted(data) => Ok(data
            .library_id
            .as_ref()
            .is_some_and(|library_id| allowed_library_ids.contains(library_id))),
        DomainEventPayload::LibraryScanTitleDiscovered(data) => {
            event_title_allowed(
                app,
                Some(&data.title_id),
                allowed_library_ids,
                title_library_cache,
            )
            .await
        }
        _ => actor_can_view_titleless_operational_event(app, actor).await,
    }
}

pub const SUPPORTED_TITLE_HISTORY_EVENT_TYPES: &[TitleHistoryEventType] = &[
    TitleHistoryEventType::Requested,
    TitleHistoryEventType::Grabbed,
    TitleHistoryEventType::DownloadFailed,
    TitleHistoryEventType::Blocklisted,
    TitleHistoryEventType::Scanned,
    TitleHistoryEventType::Imported,
    TitleHistoryEventType::ImportFailed,
    TitleHistoryEventType::ImportSkipped,
    TitleHistoryEventType::FileUpgraded,
    TitleHistoryEventType::FileRecycled,
    TitleHistoryEventType::FileDeleted,
    TitleHistoryEventType::FileRenamed,
    TitleHistoryEventType::DownloadIgnored,
    TitleHistoryEventType::Rematched,
    TitleHistoryEventType::SeedingStarted,
    TitleHistoryEventType::SeedingCompleted,
];

const TITLE_HISTORY_DOMAIN_EVENT_TYPES: &[DomainEventType] = &[
    DomainEventType::TitleRematched,
    DomainEventType::ReleaseGrabbed,
    DomainEventType::ImportCompleted,
    DomainEventType::ImportRejected,
    DomainEventType::DownloadFailed,
    DomainEventType::ReleaseBlocklisted,
    DomainEventType::MediaFileAnalyzed,
    DomainEventType::MediaFileUpgraded,
    DomainEventType::MediaFileDeleted,
    DomainEventType::MediaFileRenamed,
    DomainEventType::MediaRequestSubmitted,
    DomainEventType::DownloadIgnored,
    DomainEventType::SeedingStarted,
    DomainEventType::SeedingCompleted,
];

pub fn supported_title_history_event_types() -> &'static [TitleHistoryEventType] {
    SUPPORTED_TITLE_HISTORY_EVENT_TYPES
}

pub fn is_supported_title_history_event_type(event_type: TitleHistoryEventType) -> bool {
    SUPPORTED_TITLE_HISTORY_EVENT_TYPES.contains(&event_type)
}

fn title_history_record_matches(record: &TitleHistoryRecord, filter: &TitleHistoryFilter) -> bool {
    filter
        .event_types
        .as_ref()
        .is_none_or(|event_types| event_types.contains(&record.event_type))
        && filter
            .title_ids
            .as_ref()
            .is_none_or(|title_ids| title_ids.contains(&record.title_id))
        && filter
            .download_id
            .as_ref()
            .is_none_or(|download_id| record.download_id.as_deref() == Some(download_id))
        && filter
            .episode_id
            .as_ref()
            .is_none_or(|expected| record.episode_id.as_deref() == Some(expected.as_str()))
}

async fn project_title_history_page(
    app: &AppUseCase,
    filter: &TitleHistoryFilter,
) -> AppResult<TitleHistoryPage> {
    // Title and episode history are projected exclusively from durable domain events.
    // The legacy `title_history` table is deprecated compatibility state and must not
    // be used for live reads or writes.
    let matched_title_ids = resolve_title_history_title_ids(app, filter).await?;
    if (filter.title_search.is_some() || filter.library_ids.is_some())
        && matched_title_ids.is_empty()
    {
        return Ok(TitleHistoryPage {
            records: Vec::new(),
            total_count: 0,
        });
    }
    let effective_title_ids = match (
        &filter.title_ids,
        filter.title_search.as_ref(),
        filter.library_ids.as_ref(),
    ) {
        (Some(_), Some(_), _) | (None, Some(_), _) | (_, None, Some(_)) => {
            Some(matched_title_ids.clone())
        }
        (Some(title_ids), None, None) => Some(title_ids.clone()),
        (None, None, None) => None,
    };
    if effective_title_ids
        .as_ref()
        .is_some_and(|title_ids| title_ids.is_empty())
    {
        return Ok(TitleHistoryPage {
            records: Vec::new(),
            total_count: 0,
        });
    }

    let include_request_history =
        should_include_request_history(filter, effective_title_ids.as_deref());

    if filter.group_by_event && filter.episode_id.is_none() && !include_request_history {
        let limit = filter.limit.max(1);
        let total_count = app
            .services
            .events
            .domain_events
            .count_title_history_page_events(
                filter.event_types.as_deref(),
                effective_title_ids.as_deref(),
                filter.download_id.as_deref(),
            )
            .await?;
        if total_count == 0 {
            return Ok(TitleHistoryPage {
                records: Vec::new(),
                total_count: 0,
            });
        }

        let page_events = app
            .services
            .events
            .domain_events
            .list_title_history_page_events(
                filter.event_types.as_deref(),
                effective_title_ids.as_deref(),
                filter.download_id.as_deref(),
                limit,
                filter.offset,
            )
            .await?;
        let mut records = page_events
            .iter()
            .filter_map(title_history_record_from_domain_event)
            .collect::<Vec<_>>();
        hydrate_title_history_record_contexts(app, &mut records).await?;
        return Ok(TitleHistoryPage {
            records,
            total_count,
        });
    }

    let mut domain_filter = DomainEventFilter {
        title_id: (!include_request_history)
            .then(|| {
                filter
                    .title_ids
                    .as_ref()
                    .and_then(|title_ids| (title_ids.len() == 1).then(|| title_ids[0].clone()))
                    .or_else(|| {
                        (matched_title_ids.len() == 1).then(|| matched_title_ids[0].clone())
                    })
            })
            .flatten(),
        event_types: Some(TITLE_HISTORY_DOMAIN_EVENT_TYPES.to_vec()),
        ..DomainEventFilter::default()
    };
    let limit = filter.limit.max(1);
    let media_request_match_titles = if include_request_history {
        load_title_history_media_request_match_titles(app, effective_title_ids.as_deref()).await?
    } else {
        Vec::new()
    };
    let mut before_sequence = None;
    let mut total_count = 0i64;
    let mut records = Vec::new();

    loop {
        domain_filter.after_sequence = None;
        domain_filter.before_sequence = before_sequence;
        domain_filter.limit = 500;

        let batch = app
            .services
            .events
            .domain_events
            .list(&domain_filter)
            .await?;
        if batch.is_empty() {
            break;
        }

        before_sequence = batch.last().map(|event| event.sequence);
        for event in &batch {
            let event_records =
                if matches!(&event.payload, DomainEventPayload::MediaRequestSubmitted(_)) {
                    media_request_title_history_records(event, &media_request_match_titles)
                } else if filter.group_by_event {
                    crate::event_views::title_history_record_from_domain_event(event)
                        .into_iter()
                        .collect::<Vec<_>>()
                } else {
                    title_history_records_from_domain_event(event)
                };

            for record in event_records {
                if !matched_title_ids.is_empty() && !matched_title_ids.contains(&record.title_id) {
                    continue;
                }
                if !title_history_record_matches(&record, filter) {
                    continue;
                }

                let current_index = total_count as usize;
                total_count += 1;
                if current_index >= filter.offset && records.len() < limit {
                    records.push(record);
                }
            }
        }

        if batch.len() < 500 {
            break;
        }
    }

    hydrate_title_history_record_contexts(app, &mut records).await?;

    Ok(TitleHistoryPage {
        records,
        total_count,
    })
}

async fn load_title_history_media_request_match_titles(
    app: &AppUseCase,
    title_ids: Option<&[String]>,
) -> AppResult<Vec<Title>> {
    let Some(title_ids) = title_ids else {
        return Ok(Vec::new());
    };

    let mut titles = Vec::new();
    for title_id in title_ids {
        if let Some(title) = app.services.catalog.titles.get_by_id(title_id).await? {
            titles.push(title);
        }
    }
    Ok(titles)
}

fn media_request_title_history_records(
    event: &DomainEvent,
    titles: &[Title],
) -> Vec<TitleHistoryRecord> {
    let DomainEventPayload::MediaRequestSubmitted(data) = &event.payload else {
        return Vec::new();
    };
    let data_json = serde_json::to_string(&event.payload).ok();

    titles
        .iter()
        .filter(|title| title.library_id == data.library_id)
        .filter(|title| media_request_external_ids_overlap(&title.external_ids, &data.external_ids))
        .map(|title| TitleHistoryRecord {
            id: format!("{}:{}", event.event_id, title.id),
            title_id: title.id.clone(),
            title_name: Some(title.name.clone()),
            poster_url: title.poster_url.clone(),
            library_id: Some(title.library_id.clone()),
            facet: Some(title.facet.clone()),
            episode_id: None,
            episode_ids: Vec::new(),
            collection_id: None,
            event_type: TitleHistoryEventType::Requested,
            actor_kind: Some(event.actor_kind),
            actor_user_id: event.actor_user_id.clone(),
            actor_display_name: Some(event.actor_display_name.clone()),
            source_title: Some(data.title_name.clone()),
            display_title: Some(data.title_name.clone()),
            source_system: Some("smg".to_string()),
            source_ref: Some(data.request_id.clone()),
            source_hint: None,
            quality: None,
            download_id: None,
            client_id: None,
            client_name: None,
            import_id: None,
            skip_reason: None,
            retry_requires_password: false,
            failure_reason: None,
            blocklist_reason: None,
            source_path: None,
            dest_path: None,
            // A media request predates any file, so there are no bytes to report.
            size_bytes: None,
            data_json: data_json.clone(),
            occurred_at: event.occurred_at.to_rfc3339(),
            created_at: event.occurred_at.to_rfc3339(),
        })
        .collect()
}

fn media_request_external_ids_overlap(
    title_ids: &[ExternalId],
    request_ids: &[ExternalId],
) -> bool {
    request_ids.iter().any(|request_id| {
        title_ids.iter().any(|title_id| {
            title_id.source.eq_ignore_ascii_case(&request_id.source)
                && title_id.value == request_id.value
        })
    })
}

fn should_include_request_history(
    filter: &TitleHistoryFilter,
    effective_title_ids: Option<&[String]>,
) -> bool {
    let Some(title_ids) = effective_title_ids else {
        return false;
    };
    if title_ids.is_empty() {
        return false;
    }

    match filter.event_types.as_ref() {
        Some(event_types) => event_types.contains(&TitleHistoryEventType::Requested),
        None => title_ids.len() == 1,
    }
}

async fn resolve_title_history_title_ids(
    app: &AppUseCase,
    filter: &TitleHistoryFilter,
) -> AppResult<Vec<String>> {
    let scoped_titles = match filter.library_ids.as_deref() {
        Some(library_ids) => Some(
            app.services
                .catalog
                .titles
                .list_for_libraries(
                    None,
                    library_ids,
                    filter.title_search.as_deref().map(str::to_string),
                )
                .await?,
        ),
        None if filter.title_search.is_some() => Some(
            app.services
                .catalog
                .titles
                .list(None, filter.title_search.as_deref().map(str::to_string))
                .await?,
        ),
        None => None,
    };
    let scoped_ids = scoped_titles
        .unwrap_or_default()
        .into_iter()
        .map(|title| title.id)
        .collect::<Vec<_>>();

    Ok(
        match (
            &filter.title_ids,
            filter.title_search.as_ref(),
            filter.library_ids.as_ref(),
        ) {
            (Some(title_ids), Some(_), _) | (Some(title_ids), None, Some(_)) => {
                let scoped_set = scoped_ids.iter().cloned().collect::<HashSet<_>>();
                title_ids
                    .iter()
                    .filter(|title_id| scoped_set.contains(*title_id))
                    .cloned()
                    .collect()
            }
            (Some(title_ids), None, None) => title_ids.clone(),
            (None, Some(_), _) | (None, None, Some(_)) => scoped_ids,
            (None, None, None) => Vec::new(),
        },
    )
}

async fn hydrate_title_history_record_contexts(
    app: &AppUseCase,
    records: &mut [TitleHistoryRecord],
) -> AppResult<()> {
    // `library_id` is never carried on the event payload, so every distinct
    // title in the page needs the lookup that title_name/facet already used.
    // The set is deduplicated by title id, so widening the condition adds at
    // most one lookup per distinct title on the page, never one per row.
    let missing_title_ids = records
        .iter()
        .filter(|record| {
            record.title_name.is_none()
                || record.facet.is_none()
                || record.library_id.is_none()
                || record.poster_url.is_none()
        })
        .map(|record| record.title_id.clone())
        .collect::<HashSet<_>>();

    for title_id in missing_title_ids {
        let Some(title) = app.services.catalog.titles.get_by_id(&title_id).await? else {
            continue;
        };

        for record in records
            .iter_mut()
            .filter(|record| record.title_id == title_id)
        {
            if record.title_name.is_none() {
                record.title_name = Some(title.name.clone());
            }
            if record.facet.is_none() {
                record.facet = Some(title.facet.clone());
            }
            if record.library_id.is_none() {
                record.library_id = Some(title.library_id.clone());
            }
            if record.poster_url.is_none() {
                record.poster_url = title.poster_url.clone();
            }
        }
    }

    Ok(())
}

async fn project_episode_title_history(
    app: &AppUseCase,
    episode_id: &str,
    limit: usize,
) -> AppResult<Vec<TitleHistoryRecord>> {
    if limit == 0 {
        return Ok(Vec::new());
    }

    let mut domain_filter = DomainEventFilter {
        event_types: Some(TITLE_HISTORY_DOMAIN_EVENT_TYPES.to_vec()),
        ..DomainEventFilter::default()
    };
    let mut before_sequence = None;
    let mut records = Vec::new();

    loop {
        domain_filter.after_sequence = None;
        domain_filter.before_sequence = before_sequence;
        domain_filter.limit = 500;

        let batch = app
            .services
            .events
            .domain_events
            .list(&domain_filter)
            .await?;
        if batch.is_empty() {
            break;
        }

        before_sequence = batch.last().map(|event| event.sequence);
        for event in &batch {
            for record in title_history_records_from_domain_event(event) {
                if record.episode_id.as_deref() != Some(episode_id) {
                    continue;
                }

                records.push(record);
                if records.len() >= limit {
                    return Ok(records);
                }
            }
        }

        if batch.len() < 500 {
            break;
        }
    }

    Ok(records)
}

impl AppUseCase {
    /// Canonical reactive bus event for title-list/detail refresh. Flows that
    /// change title-visible UI state should emit this instead of open-coding
    /// scan- or workflow-specific refresh signals.
    pub(crate) async fn emit_title_updated_activity(
        &self,
        actor: impl Into<DomainEventActor>,
        title: &Title,
    ) {
        let actor = actor.into();
        if let Err(error) = self
            .append_domain_event(new_title_domain_event(
                actor,
                title,
                DomainEventPayload::TitleUpdated(TitleUpdatedEventData {
                    title: title_context_snapshot(title),
                }),
            ))
            .await
        {
            tracing::warn!(
                title_id = %title.id,
                error = %error,
                "failed to append title updated domain event"
            );
        }
    }

    pub async fn emit_configuration_changed_event(
        &self,
        actor: impl Into<DomainEventActor>,
        resource_type: impl Into<String>,
        resource_id: Option<String>,
        action: ConfigurationChangeAction,
    ) {
        let actor = actor.into();
        if let Err(error) = self
            .append_domain_event(new_global_domain_event(
                actor,
                DomainEventPayload::ConfigurationChanged(ConfigurationChangedEventData {
                    resource_type: resource_type.into(),
                    resource_id,
                    action,
                }),
            ))
            .await
        {
            tracing::warn!(error = %error, "failed to append configuration changed domain event");
        }
    }

    pub(crate) async fn emit_metadata_hydration_updated_event(
        &self,
        title: &Title,
        state: MetadataHydrationState,
        reason: Option<String>,
    ) {
        if let Err(error) = self
            .append_domain_event(new_title_domain_event(
                None,
                title,
                DomainEventPayload::MetadataHydrationUpdated(MetadataHydrationUpdatedEventData {
                    title: title_context_snapshot(title),
                    state,
                    reason,
                }),
            ))
            .await
        {
            tracing::warn!(
                title_id = %title.id,
                error = %error,
                "failed to append metadata hydration domain event"
            );
        }
    }

    pub(crate) async fn emit_acquisition_candidate_rejected_event(
        &self,
        actor: impl Into<DomainEventActor>,
        title: &Title,
        source_title: impl Into<String>,
        reason_code: impl Into<String>,
    ) {
        let actor = actor.into();
        if let Err(error) = self
            .append_domain_event(new_title_domain_event(
                actor,
                title,
                DomainEventPayload::AcquisitionCandidateRejected(
                    AcquisitionCandidateRejectedEventData {
                        title: title_context_snapshot(title),
                        source_title: source_title.into(),
                        reason_code: reason_code.into(),
                    },
                ),
            ))
            .await
        {
            tracing::warn!(
                title_id = %title.id,
                error = %error,
                "failed to append acquisition candidate rejected domain event"
            );
        }
    }

    pub(crate) async fn emit_import_requested_event(
        &self,
        actor: impl Into<DomainEventActor>,
        title: Option<&Title>,
        client_type: impl Into<String>,
        source_ref: impl Into<String>,
        request_kind: ImportRequestKind,
    ) {
        let actor = actor.into();
        let client_type = client_type.into();
        let source_ref = source_ref.into();
        let payload = DomainEventPayload::ImportRequested(ImportRequestedEventData {
            title: title.map(title_context_snapshot),
            client_type: client_type.clone(),
            source_ref: source_ref.clone(),
            request_kind,
        });

        let result = match title {
            Some(title) => {
                self.append_domain_event(new_title_domain_event(actor, title, payload))
                    .await
            }
            None => {
                self.append_domain_event(new_global_domain_event(actor, payload))
                    .await
            }
        };

        if let Err(error) = result {
            tracing::warn!(error = %error, client_type, source_ref, "failed to append import requested domain event");
        }
    }

    pub(crate) async fn emit_download_queue_item_command_issued_event(
        &self,
        actor: impl Into<DomainEventActor>,
        item_id: impl Into<String>,
        action: DownloadQueueCommandAction,
    ) {
        let actor = actor.into();
        let item_id = item_id.into();
        if let Err(error) = self
            .append_domain_event(new_download_queue_domain_event(
                actor,
                item_id.clone(),
                DomainEventPayload::DownloadQueueItemCommandIssued(
                    DownloadQueueItemCommandIssuedEventData {
                        item_id: item_id.clone(),
                        action,
                    },
                ),
            ))
            .await
        {
            tracing::warn!(error = %error, item_id, "failed to append download queue command domain event");
        }
    }

    pub(crate) async fn emit_post_processing_completed_event(
        &self,
        actor: impl Into<DomainEventActor>,
        title: &Title,
        script_name: impl Into<String>,
        result: PostProcessingResult,
        exit_code: Option<i32>,
    ) {
        let actor = actor.into();
        let script_name = script_name.into();
        if let Err(error) = self
            .append_domain_event(new_title_domain_event(
                actor,
                title,
                DomainEventPayload::PostProcessingCompleted(PostProcessingCompletedEventData {
                    title: title_context_snapshot(title),
                    script_name: script_name.clone(),
                    result,
                    exit_code,
                }),
            ))
            .await
        {
            tracing::warn!(
                title_id = %title.id,
                error = %error,
                script_name,
                "failed to append post-processing domain event"
            );
        }
    }

    pub(crate) async fn emit_subtitle_downloaded_event(
        &self,
        title: &Title,
        subtitle_path: Option<String>,
        language: Option<String>,
        provider: Option<String>,
    ) {
        if let Err(error) = self
            .append_domain_event(new_title_domain_event(
                None,
                title,
                DomainEventPayload::SubtitleDownloaded(SubtitleDownloadedEventData {
                    title: title_context_snapshot(title),
                    subtitle_path,
                    language,
                    provider,
                }),
            ))
            .await
        {
            tracing::warn!(title_id = %title.id, error = %error, "failed to append subtitle downloaded domain event");
        }
    }

    pub(crate) async fn emit_subtitle_search_failed_event(
        &self,
        title: &Title,
        language: Option<String>,
        reason: Option<String>,
    ) {
        if let Err(error) = self
            .append_domain_event(new_title_domain_event(
                None,
                title,
                DomainEventPayload::SubtitleSearchFailed(SubtitleSearchFailedEventData {
                    title: title_context_snapshot(title),
                    language,
                    reason,
                }),
            ))
            .await
        {
            tracing::warn!(title_id = %title.id, error = %error, "failed to append subtitle search failed domain event");
        }
    }

    pub async fn evaluate_policy(
        &self,
        actor: &User,
        input: PolicyInput,
    ) -> AppResult<PolicyOutput> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(&input.title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", input.title_id)))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;

        let mut reason_codes = vec!["default_policy_evaluation".to_string()];
        if input.has_existing_file {
            reason_codes.push("existing_file_present".to_string());
        }

        let score = if input.requested_mode == scryer_domain::RequestedMode::Manual {
            100.0
        } else {
            80.0
        };

        Ok(PolicyOutput {
            decision: true,
            score,
            reason_codes,
            explanation: format!(
                "policy evaluation for title {} in {} mode",
                input.title_id,
                input.requested_mode.as_str()
            ),
            scoring_log: vec![],
        })
    }

    pub async fn recent_events(
        &self,
        actor: &User,
        title_id: Option<String>,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<HistoryEvent>> {
        if let Some(title_id) = title_id.as_deref() {
            let title = self
                .services
                .catalog
                .titles
                .get_by_id(title_id)
                .await?
                .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;
            self.require_library_permission(
                actor,
                &title.library_id,
                scryer_domain::LibraryPermission::View,
            )
            .await?;
        } else {
            require_actor_view_library(self, actor).await?;
        }
        let offset = offset.max(0) as usize;
        let limit = limit.max(1) as usize;
        let user_facing_event_types = user_facing_domain_event_types();
        let history = load_recent_authorized_projected_domain_events(
            self,
            actor,
            DomainEventFilter {
                title_id,
                event_types: Some(user_facing_event_types),
                ..DomainEventFilter::default()
            },
            offset.saturating_add(limit),
            history_event_from_domain_event,
        )
        .await?;
        Ok(history.into_iter().skip(offset).take(limit).collect())
    }

    pub(crate) async fn recent_activity_page(
        &self,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<ActivityEvent>> {
        let offset = offset.max(0) as usize;
        let limit = limit.max(1) as usize;
        let user_facing_event_types = user_facing_domain_event_types();
        let activities = load_recent_projected_domain_events(
            self,
            DomainEventFilter {
                event_types: Some(user_facing_event_types),
                ..DomainEventFilter::default()
            },
            offset.saturating_add(limit),
            activity_event_from_domain_event,
        )
        .await?;
        Ok(activities.into_iter().skip(offset).take(limit).collect())
    }

    pub async fn recent_activity(
        &self,
        actor: &User,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<ActivityEvent>> {
        require_actor_view_library(self, actor).await?;
        let offset = offset.max(0) as usize;
        let limit = limit.max(1) as usize;
        let user_facing_event_types = user_facing_domain_event_types();
        let activities = load_recent_authorized_projected_domain_events(
            self,
            actor,
            DomainEventFilter {
                event_types: Some(user_facing_event_types),
                ..DomainEventFilter::default()
            },
            offset.saturating_add(limit),
            activity_event_from_domain_event,
        )
        .await?;
        Ok(activities.into_iter().skip(offset).take(limit).collect())
    }

    pub async fn list_domain_events(
        &self,
        actor: &User,
        filter: &DomainEventFilter,
    ) -> AppResult<Vec<DomainEvent>> {
        require_actor_view_library(self, actor).await?;
        let target_len = if filter.limit == 0 {
            100
        } else {
            filter.limit.min(500)
        };
        let allowed_library_ids = self
            .authorized_library_ids(actor, None, scryer_domain::LibraryPermission::View)
            .await?
            .into_iter()
            .collect::<HashSet<_>>();
        let mut title_library_cache = HashMap::new();
        let mut visible = Vec::new();

        let forward = filter.after_sequence.is_some() && filter.before_sequence.is_none();
        let mut page_filter = filter.clone();
        page_filter.limit = 500;

        loop {
            let events = self
                .services
                .events
                .domain_events
                .list(&page_filter)
                .await?;
            if events.is_empty() {
                break;
            }

            let next_sequence = events.last().map(|event| event.sequence);
            let batch_len = events.len();
            for event in events {
                if event_allowed(
                    self,
                    actor,
                    &event,
                    &allowed_library_ids,
                    &mut title_library_cache,
                )
                .await?
                {
                    visible.push(event);
                    if visible.len() >= target_len {
                        return Ok(visible);
                    }
                }
            }

            if batch_len < 500 {
                break;
            }

            if forward {
                page_filter.after_sequence = next_sequence;
            } else {
                page_filter.before_sequence = next_sequence;
            }
        }
        Ok(visible)
    }

    pub async fn audit_log(
        &self,
        actor: &User,
        filter: &DomainEventFilter,
    ) -> AppResult<Vec<DomainEvent>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let mut filter = filter.clone();
        filter.limit = if filter.limit == 0 {
            100
        } else {
            filter.limit.min(500)
        };
        self.services.events.domain_events.list(&filter).await
    }

    pub async fn list_activity_events_after_sequence(
        &self,
        actor: &User,
        after_sequence: i64,
        limit: usize,
    ) -> AppResult<Vec<(i64, ActivityEvent)>> {
        require_actor_view_library(self, actor).await?;
        let user_facing_event_types = user_facing_domain_event_types();
        let mut visible = Vec::new();
        let events = self
            .list_domain_events(
                actor,
                &DomainEventFilter {
                    event_types: Some(user_facing_event_types),
                    after_sequence: Some(after_sequence),
                    limit: limit.max(1),
                    ..DomainEventFilter::default()
                },
            )
            .await?;
        for event in events {
            if let Some(activity) = activity_event_from_domain_event(&event) {
                visible.push((event.sequence, activity));
            }
        }
        Ok(visible)
    }

    pub async fn subscribe_activity_events(
        &self,
        actor: &User,
    ) -> AppResult<broadcast::Receiver<ActivityEvent>> {
        require_actor_view_library(self, actor).await?;
        let (tx, rx) = broadcast::channel(128);
        let app = self.clone();
        let actor = actor.clone();
        tokio::spawn(async move {
            let mut wake_rx = app.runtime.events.domain_event_broadcast.subscribe();
            let mut cursor = 0_i64;

            loop {
                match app
                    .list_activity_events_after_sequence(&actor, cursor, 100)
                    .await
                {
                    Ok(events) if !events.is_empty() => {
                        for (sequence, event) in events {
                            cursor = sequence;
                            if tx.send(event).is_err() {
                                return;
                            }
                        }
                        continue;
                    }
                    Ok(_) => {}
                    Err(error) => {
                        tracing::warn!("activity subscription replay failed: {error}");
                        break;
                    }
                }

                match wake_rx.recv().await {
                    Ok(sequence) => {
                        if sequence > cursor {
                            cursor = sequence.saturating_sub(1);
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::debug!("activity subscription lagged, skipped {n} wakeups");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        Ok(rx)
    }

    pub async fn subscribe_domain_event_sequences(
        &self,
        actor: &User,
    ) -> AppResult<broadcast::Receiver<i64>> {
        require_actor_view_library(self, actor).await?;
        Ok(self.runtime.events.domain_event_broadcast.subscribe())
    }

    pub async fn subscribe_import_history(
        &self,
        actor: &User,
    ) -> AppResult<broadcast::Receiver<()>> {
        require_actor_view_library(self, actor).await?;
        Ok(self.runtime.events.import_history_broadcast.subscribe())
    }

    pub async fn subscribe_indexers_changed(
        &self,
        actor: &User,
    ) -> AppResult<broadcast::Receiver<()>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        Ok(self.runtime.events.indexers_changed_broadcast.subscribe())
    }

    pub async fn active_library_scans(&self, actor: &User) -> AppResult<Vec<LibraryScanSession>> {
        let visibility = load_library_scan_visibility(self, actor).await?;
        Ok(self
            .runtime
            .library
            .library_scan_tracker
            .list_active()
            .await
            .into_iter()
            .filter(|session| library_scan_session_visible(session, &visibility))
            .collect())
    }

    pub async fn library_scan_session(
        &self,
        actor: &User,
        session_id: &str,
    ) -> AppResult<Option<LibraryScanSession>> {
        let session_id = session_id.trim();
        if session_id.is_empty() {
            return Ok(None);
        }

        let visibility = load_library_scan_visibility(self, actor).await?;
        if let Some(session) = self
            .runtime
            .library
            .library_scan_tracker
            .get_session(session_id)
            .await
        {
            return Ok(library_scan_session_visible(&session, &visibility).then_some(session));
        }

        let Some(session) =
            crate::library_scan_coordinator::load_projected_library_scan_session(self, session_id)
                .await?
        else {
            return Ok(None);
        };

        Ok(library_scan_session_visible(&session, &visibility).then_some(session))
    }

    pub async fn subscribe_library_scan_progress(
        &self,
        actor: &User,
    ) -> AppResult<broadcast::Receiver<LibraryScanSession>> {
        let visibility = load_library_scan_visibility(self, actor).await?;
        let (tx, rx) = broadcast::channel(128);
        let app = self.clone();
        tokio::spawn(async move {
            let (initial_sessions, mut receiver) = app
                .runtime
                .library
                .library_scan_tracker
                .subscribe_with_initial_snapshot()
                .await;
            for session in initial_sessions {
                if !library_scan_session_visible(&session, &visibility) {
                    continue;
                }
                if tx.send(session).is_err() {
                    return;
                }
            }

            loop {
                match receiver.recv().await {
                    Ok(session) => {
                        if !library_scan_session_visible(&session, &visibility) {
                            continue;
                        }
                        if tx.send(session).is_err() {
                            return;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::debug!("library scan subscription lagged, skipped {n} updates");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        Ok(rx)
    }

    pub async fn subscribe_library_scan_state(
        &self,
        actor: &User,
    ) -> AppResult<broadcast::Receiver<LibraryScanSession>> {
        self.subscribe_library_scan_progress(actor).await
    }

    pub async fn subscribe_settings_changed(
        &self,
        actor: &User,
    ) -> AppResult<broadcast::Receiver<Vec<String>>> {
        require_actor_view_library(self, actor).await?;
        Ok(self.runtime.events.settings_changed_broadcast.subscribe())
    }

    pub async fn subscribe_provider_catalog_changed(
        &self,
        actor: &User,
    ) -> AppResult<broadcast::Receiver<Vec<crate::ProviderCatalogFamily>>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        Ok(self
            .runtime
            .events
            .provider_catalog_changed_broadcast
            .subscribe())
    }

    pub async fn subscribe_plugin_install_progress(
        &self,
        actor: &User,
        plugin_id: &str,
    ) -> AppResult<tokio::sync::watch::Receiver<crate::PluginInstallProgressSnapshot>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        self.runtime
            .plugins
            .plugin_install_orchestrator
            .subscribe(&actor.id, plugin_id)
            .await
            .ok_or_else(|| {
                AppError::NotFound(format!(
                    "no active plugin install progress for '{plugin_id}'"
                ))
            })
    }

    pub fn subscribe_download_queue_state(
        &self,
        actor: &User,
    ) -> AppResult<broadcast::Receiver<Vec<DownloadQueueItem>>> {
        self.subscribe_download_queue(actor)
    }

    pub async fn subscribe_job_run_state(
        &self,
        actor: &User,
    ) -> AppResult<broadcast::Receiver<JobRun>> {
        self.subscribe_job_run_events(actor).await
    }

    pub async fn list_title_history(
        &self,
        actor: &User,
        filter: &TitleHistoryFilter,
    ) -> AppResult<TitleHistoryPage> {
        let library_ids = self
            .authorized_library_ids(actor, None, scryer_domain::LibraryPermission::View)
            .await?;
        let mut scoped_filter = filter.clone();
        scoped_filter.library_ids = Some(match filter.library_ids.as_ref() {
            Some(requested_library_ids) => {
                let allowed_library_ids = library_ids.into_iter().collect::<HashSet<_>>();
                requested_library_ids
                    .iter()
                    .filter(|library_id| allowed_library_ids.contains(*library_id))
                    .cloned()
                    .collect()
            }
            None => library_ids,
        });
        project_title_history_page(self, &scoped_filter).await
    }

    /// Count dashboard activity events over a trailing window and the window
    /// immediately before it.
    ///
    /// `window_hours` is clamped to [`DASHBOARD_ACTIVITY_MIN_WINDOW_HOURS`]
    /// through [`DASHBOARD_ACTIVITY_MAX_WINDOW_HOURS`]. Library visibility is
    /// resolved exactly as [`AppUseCase::list_title_history`] resolves it, so
    /// the tiles never count events the caller could not read in the history
    /// view. The counting itself is one grouped aggregate in the datastore.
    pub async fn dashboard_activity_stats(
        &self,
        actor: &User,
        window_hours: i64,
    ) -> AppResult<DashboardActivityStats> {
        let window_hours = window_hours.clamp(
            DASHBOARD_ACTIVITY_MIN_WINDOW_HOURS,
            DASHBOARD_ACTIVITY_MAX_WINDOW_HOURS,
        );
        let library_ids = self
            .authorized_library_ids(actor, None, scryer_domain::LibraryPermission::View)
            .await?;
        if library_ids.is_empty() {
            return Ok(DashboardActivityStats::default());
        }

        let current_end = Utc::now();
        let window = chrono::Duration::hours(window_hours);
        let current_start = current_end - window;
        let previous_start = current_start - window;
        self.services
            .events
            .domain_events
            .count_dashboard_activity_events(
                &library_ids,
                previous_start,
                current_start,
                current_end,
            )
            .await
    }

    pub async fn list_title_history_for_title(
        &self,
        actor: &User,
        title_id: &str,
        event_types: Option<&[TitleHistoryEventType]>,
        limit: usize,
        offset: usize,
    ) -> AppResult<TitleHistoryPage> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", title_id)))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::View,
        )
        .await?;
        project_title_history_page(
            self,
            &TitleHistoryFilter {
                event_types: event_types.map(|types| types.to_vec()),
                title_ids: Some(vec![title_id.to_string()]),
                library_ids: Some(vec![title.library_id]),
                title_search: None,
                download_id: None,
                episode_id: None,
                group_by_event: false,
                limit,
                offset,
            },
        )
        .await
    }

    pub async fn list_title_history_for_episode(
        &self,
        actor: &User,
        episode_id: &str,
        limit: usize,
    ) -> AppResult<Vec<TitleHistoryRecord>> {
        let episode = self
            .services
            .catalog
            .shows
            .get_episode_by_id(episode_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("episode {}", episode_id)))?;
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(&episode.title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", episode.title_id)))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::View,
        )
        .await?;
        project_episode_title_history(self, episode_id, limit).await
    }
}

#[cfg(test)]
mod title_history_request_filter_tests {
    use super::*;

    fn filter(event_types: Option<Vec<TitleHistoryEventType>>) -> TitleHistoryFilter {
        TitleHistoryFilter {
            event_types,
            ..Default::default()
        }
    }

    #[test]
    fn default_unfiltered_history_does_not_scan_for_requests() {
        assert!(!should_include_request_history(&filter(None), None));
    }

    #[test]
    fn default_single_title_history_includes_matching_requests() {
        let title_ids = vec!["title-1".to_string()];
        assert!(should_include_request_history(
            &filter(None),
            Some(&title_ids),
        ));
    }

    #[test]
    fn explicit_requested_filter_includes_requests_for_known_titles() {
        let title_ids = vec!["title-1".to_string(), "title-2".to_string()];
        assert!(should_include_request_history(
            &filter(Some(vec![TitleHistoryEventType::Requested])),
            Some(&title_ids),
        ));
    }

    #[test]
    fn explicit_requested_filter_without_title_matches_does_not_scan() {
        assert!(!should_include_request_history(
            &filter(Some(vec![TitleHistoryEventType::Requested])),
            None,
        ));
    }
}

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

fn dashboard_import_kind(
    event_type: &TitleHistoryEventType,
    evidence: Option<&crate::DashboardImportEvidence>,
) -> Option<crate::DashboardImportKind> {
    use crate::DashboardImportKind;
    if *event_type == TitleHistoryEventType::FileUpgraded {
        Some(DashboardImportKind::Upgrade)
    } else {
        match evidence {
            // Upgrade facts commit before completion artifacts. Suppress the
            // duplicate completion even when historical artifacts lack file IDs.
            Some(evidence) if evidence.is_upgrade => None,
            Some(_) => Some(DashboardImportKind::NewImport),
            None => Some(DashboardImportKind::Imported),
        }
    }
}

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
            // Actorless global projections must never include private facts.
            if matches!(event.stream, scryer_domain::DomainEventStream::User { .. }) {
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

    let visibility = EventVisibility::load(app, actor).await?;
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
        prefetch_event_title_libraries(app, &batch, &mut title_library_cache).await?;
        for event in &batch {
            if !event_allowed(app, actor, event, &visibility, &mut title_library_cache).await? {
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
        .list_library_ids_for_titles(&[title_id.to_string()])
        .await?
        .into_iter()
        .find(|(id, _)| id == title_id)
        .map(|(_, library_id)| library_id);
    let allowed = library_id
        .as_ref()
        .is_some_and(|library_id| allowed_library_ids.contains(library_id));
    title_library_cache.insert(title_id.to_string(), library_id);
    Ok(allowed)
}

/// Fill `title_library_cache` for every title the batch's events name, in one
/// read, so the per-event checks find their library already cached.
async fn prefetch_event_title_libraries(
    app: &AppUseCase,
    events: &[DomainEvent],
    title_library_cache: &mut HashMap<String, Option<String>>,
) -> AppResult<()> {
    let missing = events
        .iter()
        .filter(|event| !matches!(event.stream, scryer_domain::DomainEventStream::User { .. }))
        .filter_map(|event| event.title_id.as_ref())
        .filter(|title_id| !title_library_cache.contains_key(*title_id))
        .cloned()
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return Ok(());
    }
    let mut found = app
        .services
        .catalog
        .titles
        .list_library_ids_for_titles(&missing)
        .await?
        .into_iter()
        .collect::<HashMap<_, _>>();
    for title_id in missing {
        let library_id = found.remove(&title_id);
        title_library_cache.insert(title_id, library_id);
    }
    Ok(())
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
        Err(AppError::unauthorized_reason(
            crate::error_reasons::events::LIBRARY_ACCESS_DENIED,
            "You do not have access to this library".to_string(),
        ))
    }
}

/// Whether a title-bearing event is catalog-level, visible with `View`. Every
/// other title event carries acquisition, download, import, file or path
/// detail and needs privileged library access. Deny by default: a new event
/// type is privileged until it is listed here.
fn title_event_is_catalog_level(payload: &DomainEventPayload) -> bool {
    matches!(
        payload,
        DomainEventPayload::TitleAdded(_)
            | DomainEventPayload::TitleUpdated(_)
            | DomainEventPayload::TitleRematched(_)
            | DomainEventPayload::TitleDeleted(_)
            | DomainEventPayload::MetadataHydrationUpdated(_)
            | DomainEventPayload::MediaRequestSubmitted(_)
            | DomainEventPayload::MediaRequestUpdated(_)
            | DomainEventPayload::MediaRequestReopened(_)
            | DomainEventPayload::MediaRequestApproved(_)
            | DomainEventPayload::MediaRequestRejected(_)
            | DomainEventPayload::MediaRequestCanceled(_)
            | DomainEventPayload::ListTitleAdded(_)
            | DomainEventPayload::ListRequestSubmitted(_)
            | DomainEventPayload::ListTitleLeft(_)
    )
}

/// Activity kinds that change what the catalog shows for a title: a file
/// arrived, was replaced or analyzed, or a subtitle landed. A reader with only
/// `View` cannot see these events, but the live stream still owes them the
/// signal to reload the title, or their open page stays stale.
fn activity_kind_refreshes_catalog(kind: &ActivityKind) -> bool {
    matches!(
        kind,
        ActivityKind::MovieDownloaded
            | ActivityKind::SeriesEpisodeImported
            | ActivityKind::FileAnalyzed
            | ActivityKind::FileUpgraded
            | ActivityKind::SubtitleDownloaded
    )
}

/// The refresh signal sent in place of a privileged activity: which title and
/// episodes changed, and how, with no message, actor or channel. The message
/// carries paths and release detail; the rest is already visible in the
/// catalog once the reader reloads.
fn catalog_refresh_signal(activity: ActivityEvent) -> ActivityEvent {
    ActivityEvent {
        severity: ActivitySeverity::Info,
        channels: Vec::new(),
        actor_kind: scryer_domain::DomainEventActorKind::System,
        actor_user_id: None,
        actor_display_name: "System".to_string(),
        message: String::new(),
        ..activity
    }
}

/// Library ids an actor may read events for: `view` for catalog-level events,
/// `privileged` for everything that describes acquisition or files.
struct EventVisibility {
    view: HashSet<String>,
    privileged: HashSet<String>,
}

impl EventVisibility {
    async fn load(app: &AppUseCase, actor: &User) -> AppResult<Self> {
        Ok(Self {
            view: app
                .authorized_library_ids(actor, None, scryer_domain::LibraryPermission::View)
                .await?
                .into_iter()
                .collect(),
            privileged: app
                .privileged_library_ids(actor, None)
                .await?
                .into_iter()
                .collect(),
        })
    }
}

async fn event_allowed(
    app: &AppUseCase,
    actor: &User,
    event: &DomainEvent,
    visibility: &EventVisibility,
    title_library_cache: &mut HashMap<String, Option<String>>,
) -> AppResult<bool> {
    if let scryer_domain::DomainEventStream::User { user_id } = &event.stream {
        return Ok(user_id == &actor.id);
    }
    let allowed_library_ids = &visibility.view;
    if event.title_id.is_some() {
        let library_ids = if title_event_is_catalog_level(&event.payload) {
            &visibility.view
        } else {
            &visibility.privileged
        };
        return event_title_allowed(
            app,
            event.title_id.as_deref(),
            library_ids,
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
        // A followed public list is readable by every member, and these two
        // say only that one changed or finished a sync.
        DomainEventPayload::ListSyncFinished(_)
        | DomainEventPayload::ListSubscriptionChanged(_) => Ok(true),
        DomainEventPayload::ConfigurationChanged(data)
            if data
                .resource_type
                .starts_with(crate::lists::public::LIST_CONFIGURATION_RESOURCE_PREFIX) =>
        {
            Ok(app
                .has_app_permission(actor, AppPermission::ManageLists)
                .await?
                || actor_can_view_titleless_operational_event(app, actor).await?)
        }
        DomainEventPayload::MediaRequestSubmitted(data) => {
            Ok(allowed_library_ids.contains(&data.library_id))
        }
        DomainEventPayload::MediaRequestUpdated(data) => {
            Ok(allowed_library_ids.contains(&data.library_id))
        }
        DomainEventPayload::MediaRequestReopened(data) => {
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
        // Carries the discovered folder path.
        DomainEventPayload::LibraryScanTitleDiscovered(data) => {
            event_title_allowed(
                app,
                Some(&data.title_id),
                &visibility.privileged,
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
    TitleHistoryEventType::ImportRejectedByRule,
    TitleHistoryEventType::FileUpgraded,
    TitleHistoryEventType::FileRecycled,
    TitleHistoryEventType::FileDeleted,
    TitleHistoryEventType::FileRestored,
    TitleHistoryEventType::FileRenamed,
    TitleHistoryEventType::TitleMoved,
    TitleHistoryEventType::DownloadIgnored,
    TitleHistoryEventType::DownloadMissing,
    TitleHistoryEventType::Rematched,
    TitleHistoryEventType::SeedingStarted,
    TitleHistoryEventType::SeedingCompleted,
];

const TITLE_HISTORY_DOMAIN_EVENT_TYPES: &[DomainEventType] = &[
    DomainEventType::TitleMoved,
    DomainEventType::TitleRematched,
    DomainEventType::MediaFileReassigned,
    DomainEventType::ReleaseGrabbed,
    DomainEventType::ImportCompleted,
    DomainEventType::ImportRejected,
    DomainEventType::DownloadFailed,
    DomainEventType::ReleaseBlocklisted,
    DomainEventType::MediaFileAnalyzed,
    DomainEventType::MediaFileUpgraded,
    DomainEventType::MediaFileDeleted,
    DomainEventType::MediaFileRestored,
    DomainEventType::MediaFileRenamed,
    DomainEventType::MediaRequestSubmitted,
    DomainEventType::DownloadIgnored,
    DomainEventType::DownloadMissing,
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
        && filter.title_ids.as_ref().is_none_or(|title_ids| {
            // A record with no catalog title behind it (an unlinked grab)
            // can never satisfy a title-scoped filter.
            record
                .title_id
                .as_ref()
                .is_some_and(|title_id| title_ids.contains(title_id))
        })
        && filter
            .download_id
            .as_ref()
            .is_none_or(|download_id| record.download_id.as_deref() == Some(download_id))
        && filter
            .episode_id
            .as_ref()
            .is_none_or(|expected| record.episode_id.as_deref() == Some(expected.as_str()))
}

fn title_history_collection_matches(
    record: &TitleHistoryRecord,
    collection_id: Option<&str>,
    episode_ids: Option<&HashSet<String>>,
) -> bool {
    let Some(collection_id) = collection_id else {
        return true;
    };
    record.collection_id.as_deref() == Some(collection_id)
        || episode_ids.is_some_and(|ids| match record.episode_id.as_ref() {
            Some(id) => ids.contains(id),
            None => record.episode_ids.iter().any(|id| ids.contains(id)),
        })
}

/// `include_titleless` says whether records with no catalog title behind them
/// belong on this page. An unlinked grab (FR-026) is recorded against the
/// release and the indexer and has no title and no library, so it can only ever
/// be admitted explicitly: `list_title_history` sets this when the caller is
/// allowed to see title-less history and has not scoped the page to titles of
/// their own choosing. It is never set for a user-chosen title or title search,
/// where a record with no title genuinely does not match.
///
/// With `include_total` false the returned `total_count` is not the full
/// count; it only decides whether more records follow the page: it exceeds
/// `offset + records.len()` exactly when the full count would.
async fn project_title_history_page(
    app: &AppUseCase,
    filter: &TitleHistoryFilter,
    include_titleless: bool,
    include_total: bool,
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

    // Only an authorization-derived scope may be widened. If the user picked
    // titles or typed a title search, a record with no catalog title is not a
    // match and must stay out.
    let include_titleless =
        include_titleless && filter.title_ids.is_none() && filter.title_search.is_none();

    let include_request_history =
        should_include_request_history(filter, effective_title_ids.as_deref());

    let collection_episode_ids = if let Some(collection_id) = &filter.collection_id {
        Some(
            app.services
                .catalog
                .shows
                .list_episodes_for_collection(collection_id)
                .await?
                .into_iter()
                .map(|episode| episode.id)
                .collect::<HashSet<_>>(),
        )
    } else {
        None
    };

    if filter.group_by_event
        && filter.episode_id.is_none()
        && filter.collection_id.is_none()
        && !include_request_history
    {
        let limit = filter.limit.max(1);
        // The page read caps at 500 rows, so one extra row can only stand in
        // for the count below that.
        if !include_total && limit < 500 {
            let mut page_events = app
                .services
                .events
                .domain_events
                .list_title_history_page_events(
                    filter.event_types.as_deref(),
                    effective_title_ids.as_deref(),
                    include_titleless,
                    filter.download_id.as_deref(),
                    limit + 1,
                    filter.offset,
                )
                .await?;
            let total_count = filter.offset.saturating_add(page_events.len()) as i64;
            page_events.truncate(limit);
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
        let total_count = app
            .services
            .events
            .domain_events
            .count_title_history_page_events(
                filter.event_types.as_deref(),
                effective_title_ids.as_deref(),
                include_titleless,
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
                include_titleless,
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

    let domain_filter = DomainEventFilter {
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
    // With request history on for exactly one title and no title-less records
    // admitted, only that title's events and request submissions can yield a
    // record. Read those two streams and merge them newest first, in place of
    // walking every history event of every title.
    let single_request_title = if include_request_history && !include_titleless {
        let distinct = matched_title_ids.iter().collect::<HashSet<_>>();
        (distinct.len() == 1).then(|| matched_title_ids[0].clone())
    } else {
        None
    };
    let mut cursors = match single_request_title {
        Some(title_id) => vec![
            TitleHistoryEventCursor::new(DomainEventFilter {
                title_id: Some(title_id),
                event_types: Some(
                    TITLE_HISTORY_DOMAIN_EVENT_TYPES
                        .iter()
                        .copied()
                        .filter(|event_type| *event_type != DomainEventType::MediaRequestSubmitted)
                        .collect(),
                ),
                ..DomainEventFilter::default()
            }),
            TitleHistoryEventCursor::new(DomainEventFilter {
                event_types: Some(vec![DomainEventType::MediaRequestSubmitted]),
                ..DomainEventFilter::default()
            }),
        ],
        None => vec![TitleHistoryEventCursor::new(domain_filter)],
    };
    let mut total_count = 0i64;
    let mut records = Vec::new();

    'walk: while let Some(event) = next_title_history_event(app, &mut cursors).await? {
        let event = &event;
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
            // A title-scoped page is about those titles; a record with no
            // catalog title behind it belongs to none of them - unless the
            // scope came from the caller's library authorization rather
            // than titles they chose, in which case an unlinked grab
            // (FR-026) is admitted on its own terms, exactly as the grouped
            // page query above admits it.
            if !matched_title_ids.is_empty()
                && !record
                    .title_id
                    .as_ref()
                    .is_some_and(|title_id| matched_title_ids.contains(title_id))
                && !(include_titleless && record.title_id.is_none())
            {
                continue;
            }
            if !title_history_record_matches(&record, filter)
                || !title_history_collection_matches(
                    &record,
                    filter.collection_id.as_deref(),
                    collection_episode_ids.as_ref(),
                )
            {
                continue;
            }

            let current_index = total_count as usize;
            total_count += 1;
            if current_index >= filter.offset && records.len() < limit {
                records.push(record);
            }
            // One record past the page settles whether more follow.
            if !include_total && current_index >= filter.offset.saturating_add(limit) {
                break 'walk;
            }
        }
    }

    hydrate_title_history_record_contexts(app, &mut records).await?;

    Ok(TitleHistoryPage {
        records,
        total_count,
    })
}

/// A newest-first read of matching domain events in 500-row batches.
struct TitleHistoryEventCursor {
    filter: DomainEventFilter,
    batch: std::collections::VecDeque<DomainEvent>,
    exhausted: bool,
}

impl TitleHistoryEventCursor {
    fn new(filter: DomainEventFilter) -> Self {
        Self {
            filter,
            batch: std::collections::VecDeque::new(),
            exhausted: false,
        }
    }

    async fn head_sequence(&mut self, app: &AppUseCase) -> AppResult<Option<i64>> {
        if self.batch.is_empty() && !self.exhausted {
            self.filter.after_sequence = None;
            self.filter.limit = 500;
            let batch = app.services.events.domain_events.list(&self.filter).await?;
            self.exhausted = batch.len() < 500;
            self.filter.before_sequence = batch.last().map(|event| event.sequence);
            self.batch.extend(batch);
        }
        Ok(self.batch.front().map(|event| event.sequence))
    }
}

/// The next event across `cursors`, highest sequence first.
async fn next_title_history_event(
    app: &AppUseCase,
    cursors: &mut [TitleHistoryEventCursor],
) -> AppResult<Option<DomainEvent>> {
    let mut newest: Option<(usize, i64)> = None;
    for (index, cursor) in cursors.iter_mut().enumerate() {
        if let Some(sequence) = cursor.head_sequence(app).await?
            && newest.is_none_or(|(_, best)| sequence > best)
        {
            newest = Some((index, sequence));
        }
    }
    Ok(newest.and_then(|(index, _)| cursors[index].batch.pop_front()))
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
            title_id: Some(title.id.clone()),
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
            failure_reason_code: None,
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
    // Chosen titles inside a library scope: look up only those titles'
    // libraries instead of listing every title in the scope.
    if let (Some(title_ids), None, Some(library_ids)) = (
        filter.title_ids.as_ref(),
        filter.title_search.as_ref(),
        filter.library_ids.as_deref(),
    ) {
        if library_ids.is_empty() || title_ids.is_empty() {
            return Ok(Vec::new());
        }
        let library_ids = library_ids.iter().collect::<HashSet<_>>();
        let scoped_set = app
            .services
            .catalog
            .titles
            .list_library_ids_for_titles(title_ids)
            .await?
            .into_iter()
            .filter(|(_, library_id)| library_ids.contains(library_id))
            .map(|(title_id, _)| title_id)
            .collect::<HashSet<_>>();
        return Ok(title_ids
            .iter()
            .filter(|title_id| scoped_set.contains(*title_id))
            .cloned()
            .collect());
    }
    let scoped_titles = match filter.library_ids.as_deref() {
        Some(library_ids) => Some(
            app.services
                .catalog
                .titles
                .list_with_projection(
                    None,
                    Some(library_ids),
                    filter.title_search.as_deref().map(str::to_string),
                    crate::TitleListProjection::without_canonical_tags(),
                )
                .await?,
        ),
        None if filter.title_search.is_some() => Some(
            app.services
                .catalog
                .titles
                .list_with_projection(
                    None,
                    None,
                    filter.title_search.as_deref().map(str::to_string),
                    crate::TitleListProjection::without_canonical_tags(),
                )
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
        // A record with no catalog title has nothing to hydrate from.
        .filter_map(|record| record.title_id.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if missing_title_ids.is_empty() {
        return Ok(());
    }

    for title in app
        .services
        .catalog
        .titles
        .get_by_ids(&missing_title_ids)
        .await?
    {
        for record in records
            .iter_mut()
            .filter(|record| record.title_id.as_deref() == Some(title.id.as_str()))
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
        self.emit_download_queue_item_command_issued_event_with_detail(
            actor, item_id, action, None,
        )
        .await;
    }

    pub(crate) async fn emit_download_queue_item_command_issued_event_with_detail(
        &self,
        actor: impl Into<DomainEventActor>,
        item_id: impl Into<String>,
        action: DownloadQueueCommandAction,
        detail: Option<String>,
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
                        detail,
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
            .ok_or_else(|| {
                AppError::not_found_reason(
                    crate::error_reasons::catalog::TITLE_NOT_FOUND,
                    format!("title {}", input.title_id),
                )
            })?;
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
                .ok_or_else(|| {
                    AppError::not_found_reason(
                        crate::error_reasons::catalog::TITLE_NOT_FOUND,
                        format!("title {title_id}"),
                    )
                })?;
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
        let visibility = EventVisibility::load(self, actor).await?;
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
            prefetch_event_title_libraries(self, &events, &mut title_library_cache).await?;
            for event in events {
                if event_allowed(self, actor, &event, &visibility, &mut title_library_cache).await?
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
        // Audit permissions do not grant access to another member's facts.
        let target_len = filter.limit;
        let forward = filter.after_sequence.is_some() && filter.before_sequence.is_none();
        let mut visible = Vec::new();
        filter.limit = 500;
        loop {
            let events = self.services.events.domain_events.list(&filter).await?;
            let next_sequence = events.last().map(|event| event.sequence);
            let batch_len = events.len();
            for event in events {
                if matches!(&event.stream, scryer_domain::DomainEventStream::User { user_id } if user_id != &actor.id)
                {
                    continue;
                }
                visible.push(event);
                if visible.len() == target_len {
                    return Ok(visible);
                }
            }
            if batch_len < 500 {
                break;
            }
            if forward {
                filter.after_sequence = next_sequence;
            } else {
                filter.before_sequence = next_sequence;
            }
        }
        Ok(visible)
    }

    /// The current tail of the event log; subscriptions start from here so a
    /// fresh client only sees events that happen after it connected.
    pub async fn latest_domain_event_sequence(&self, actor: &User) -> AppResult<i64> {
        require_actor_view_library(self, actor).await?;
        self.services.events.domain_events.latest_sequence().await
    }

    pub async fn list_activity_events_after_sequence(
        &self,
        actor: &User,
        after_sequence: i64,
        limit: usize,
    ) -> AppResult<Vec<(i64, ActivityEvent)>> {
        require_actor_view_library(self, actor).await?;
        let target_len = limit.clamp(1, 500);
        let visibility = EventVisibility::load(self, actor).await?;
        let mut title_library_cache = HashMap::new();
        let mut visible = Vec::new();
        let mut page_filter = DomainEventFilter {
            event_types: Some(user_facing_domain_event_types()),
            after_sequence: Some(after_sequence),
            limit: 500,
            ..DomainEventFilter::default()
        };

        loop {
            let events = self
                .services
                .events
                .domain_events
                .list(&page_filter)
                .await?;
            let Some(last_sequence) = events.last().map(|event| event.sequence) else {
                break;
            };
            let batch_len = events.len();
            prefetch_event_title_libraries(self, &events, &mut title_library_cache).await?;
            for event in events {
                let Some(activity) = activity_event_from_domain_event(&event) else {
                    continue;
                };
                let activity =
                    if event_allowed(self, actor, &event, &visibility, &mut title_library_cache)
                        .await?
                    {
                        activity
                    } else if !matches!(event.stream, scryer_domain::DomainEventStream::User { .. })
                        && activity_kind_refreshes_catalog(&activity.kind)
                        && event_title_allowed(
                            self,
                            event.title_id.as_deref(),
                            &visibility.view,
                            &mut title_library_cache,
                        )
                        .await?
                    {
                        catalog_refresh_signal(activity)
                    } else {
                        continue;
                    };
                visible.push((event.sequence, activity));
                if visible.len() >= target_len {
                    return Ok(visible);
                }
            }
            if batch_len < 500 {
                break;
            }
            page_filter.after_sequence = Some(last_sequence);
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
        let (initial_sessions, mut receiver) = self
            .runtime
            .library
            .library_scan_tracker
            .subscribe_with_initial_snapshot()
            .await;
        let visibility = load_library_scan_visibility(self, actor).await?;
        let (tx, rx) = broadcast::channel(128);
        let app = self.clone();
        let actor = actor.clone();
        tokio::spawn(async move {
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
                        // Libraries can be created during this connection.
                        // Recheck the affected facet rather than retaining the
                        // subscription's original library allowlist indefinitely.
                        let visible_ids = match app
                            .authorized_library_ids(
                                &actor,
                                Some(session.facet.clone()),
                                LibraryPermission::View,
                            )
                            .await
                        {
                            Ok(ids) => ids,
                            Err(error) => {
                                tracing::warn!(%error, "library scan visibility refresh failed");
                                continue;
                            }
                        };
                        if !session
                            .library_id
                            .as_ref()
                            .map_or_else(|| !visible_ids.is_empty(), |id| visible_ids.contains(id))
                        {
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
                AppError::not_found_reason(
                    crate::error_reasons::events::ACTIVE_PLUGIN_INSTALL_PROGRESS_NOT_FOUND,
                    format!("no active plugin install progress for '{plugin_id}'"),
                )
            })
    }

    pub async fn subscribe_download_queue_state(
        &self,
        actor: &User,
    ) -> AppResult<broadcast::Receiver<Vec<DownloadQueueItem>>> {
        self.subscribe_download_queue(actor).await
    }

    pub async fn subscribe_job_run_state(
        &self,
        actor: &User,
    ) -> AppResult<broadcast::Receiver<JobRun>> {
        self.subscribe_job_run_events(actor).await
    }

    pub async fn dashboard_recent_imports(
        &self,
        actor: &User,
        limit: usize,
    ) -> AppResult<Vec<crate::DashboardRecentImport>> {
        let limit = limit.clamp(1, 50);
        let library_ids = self.require_privileged_library_ids(actor, None).await?;
        let mut items = Vec::with_capacity(limit);
        let mut before = None;
        let mut seen_files = HashSet::new();
        // Bound even pathological legacy logs with repeated facts. Normal pages
        // need one batch; paired completion/upgrade facts may need another.
        for _ in 0..25 {
            let events = self
                .services
                .events
                .domain_events
                .recent_import_events(&library_ids, before, 8)
                .await?;
            if events.is_empty() {
                break;
            }
            before = events.last().map(|event| event.sequence);
            'events: for event in &events {
                let file_id = match &event.payload {
                    DomainEventPayload::MediaFileUpgraded(data) => data.current_file_id.clone(),
                    _ => None,
                };
                let Some(mut base) = title_history_record_from_domain_event(event) else {
                    continue;
                };
                let mut episode_ids = std::mem::take(&mut base.episode_ids)
                    .into_iter()
                    .map(Some)
                    .collect::<Vec<_>>();
                if episode_ids.len() > 1 {
                    base.size_bytes = None;
                }
                base.data_json = None;
                if episode_ids.is_empty() {
                    episode_ids.push(None);
                }
                let import_ids = base.import_id.clone().into_iter().collect::<Vec<_>>();
                // Process mixed packs in small evidence batches before limiting
                // accepted rows, so upgrades cannot hide later new episodes.
                for chunk in episode_ids.chunks(50) {
                    let ids = chunk.iter().flatten().cloned().collect::<Vec<_>>();
                    let artifacts = self
                        .services
                        .workflow
                        .import_artifacts
                        .dashboard_import_artifacts(&import_ids, &ids)
                        .await?;
                    for episode_id in chunk {
                        let artifact = artifacts.iter().find(|artifact| {
                            artifact.import_id == base.import_id
                                && artifact.title_id == base.title_id
                                && artifact.episode_id == *episode_id
                        });
                        let Some(kind) = dashboard_import_kind(&base.event_type, artifact) else {
                            continue;
                        };
                        let identity = file_id
                            .clone()
                            .or_else(|| artifact.and_then(|a| a.imported_media_file_id.clone()));
                        if let Some(identity) = identity
                            && !seen_files.insert((identity, episode_id.clone()))
                        {
                            continue;
                        }
                        let mut record = base.clone();
                        record.episode_id = episode_id.clone();
                        record.id =
                            format!("{}:{}", record.id, episode_id.as_deref().unwrap_or("movie"));
                        items.push(crate::DashboardRecentImport {
                            record,
                            episode: None,
                            kind,
                        });
                        if items.len() == limit {
                            break 'events;
                        }
                    }
                }
            }
            if items.len() == limit || events.len() < 8 {
                break;
            }
        }
        let title_ids = items
            .iter()
            .filter_map(|i| i.record.title_id.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let episode_ids = items
            .iter()
            .filter_map(|i| i.record.episode_id.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let titles = self.services.catalog.titles.get_by_ids(&title_ids).await?;
        let episodes = self
            .services
            .catalog
            .shows
            .get_episodes_by_ids(&episode_ids)
            .await?;
        // Recheck current library ownership after hydration, including moves
        // racing the event read. Never expose metadata from a different title.
        items.retain_mut(|item| {
            let Some(title) = titles.iter().find(|t| {
                Some(&t.id) == item.record.title_id.as_ref() && library_ids.contains(&t.library_id)
            }) else {
                return false;
            };
            item.record.title_name = Some(title.name.clone());
            item.record.library_id = Some(title.library_id.clone());
            item.record.facet = Some(title.facet.clone());
            item.record.poster_url = title.poster_url.clone();
            item.episode = episodes
                .iter()
                .find(|e| Some(&e.id) == item.record.episode_id.as_ref() && e.title_id == title.id)
                .cloned();
            true
        });
        Ok(items)
    }

    pub async fn list_title_history(
        &self,
        actor: &User,
        filter: &TitleHistoryFilter,
    ) -> AppResult<TitleHistoryPage> {
        self.list_title_history_page(actor, filter, true).await
    }

    /// [`Self::list_title_history`]; with `include_total` false the page's
    /// `total_count` is only exact enough to tell whether more records follow.
    pub async fn list_title_history_page(
        &self,
        actor: &User,
        filter: &TitleHistoryFilter,
        include_total: bool,
    ) -> AppResult<TitleHistoryPage> {
        let library_ids = self.require_privileged_library_ids(actor, None).await?;
        let mut scoped_filter = filter.clone();
        if let Some(collection_id) = &filter.collection_id {
            let collection = self.get_collection(actor, collection_id).await?;
            scoped_filter.title_ids = Some(
                collection
                    .into_iter()
                    .filter(|collection| {
                        filter
                            .title_ids
                            .as_ref()
                            .is_none_or(|ids| ids.contains(&collection.title_id))
                    })
                    .map(|collection| collection.title_id)
                    .collect(),
            );
        }
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
        // The library scope above is authorization, not a filter the user
        // chose: with "All Libraries" selected the page still arrives here as
        // an explicit list of every library the actor holds privileged access
        // to. An unlinked
        // grab has no title and therefore no library, so it can never be named
        // by that list and has to be admitted separately. Gate it on the same
        // permission `event_allowed` uses for title-less events, which is also
        // the permission required to make an unlinked grab in the first place
        // (`interactive_release_search` requires ManageSystemSettings), so only
        // an actor who could have performed the grab can see it.
        let include_titleless = self
            .has_app_permission(actor, AppPermission::ManageSystemSettings)
            .await?;
        project_title_history_page(self, &scoped_filter, include_titleless, include_total).await
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
        // Activity counts summarise history, so they follow its privileged
        // scope. An actor without it gets empty tiles, not a refusal.
        let library_ids = self.privileged_library_ids(actor, None).await?;
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
            .ok_or_else(|| {
                AppError::not_found_reason(
                    crate::error_reasons::catalog::TITLE_NOT_FOUND,
                    format!("title {}", title_id),
                )
            })?;
        self.require_privileged_library_access(actor, &title.library_id)
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
                collection_id: None,
                group_by_event: false,
                limit,
                offset,
            },
            // A single title's own history page: a record with no catalog title
            // behind it is not part of it.
            false,
            true,
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
            .ok_or_else(|| {
                AppError::not_found_reason(
                    crate::error_reasons::events::EPISODE_NOT_FOUND,
                    format!("episode {}", episode_id),
                )
            })?;
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(&episode.title_id)
            .await?
            .ok_or_else(|| {
                AppError::not_found_reason(
                    crate::error_reasons::catalog::TITLE_NOT_FOUND,
                    format!("title {}", episode.title_id),
                )
            })?;
        self.require_privileged_library_access(actor, &title.library_id)
            .await?;
        project_episode_title_history(self, episode_id, limit).await
    }
}

#[cfg(test)]
mod title_history_request_filter_tests {
    use super::*;

    #[test]
    fn dashboard_import_classification_preserves_legacy_and_suppresses_upgrade_duplicates() {
        use crate::DashboardImportKind::{Imported, NewImport, Upgrade};
        let mut evidence = crate::DashboardImportEvidence {
            import_id: Some("attempt".into()),
            title_id: Some("title".into()),
            episode_id: Some("episode".into()),
            is_upgrade: true,
            imported_media_file_id: None,
        };
        assert_eq!(
            dashboard_import_kind(&TitleHistoryEventType::Imported, None),
            Some(Imported)
        );
        assert_eq!(
            dashboard_import_kind(&TitleHistoryEventType::Imported, Some(&evidence)),
            None
        );
        assert_eq!(
            dashboard_import_kind(&TitleHistoryEventType::FileUpgraded, Some(&evidence)),
            Some(Upgrade)
        );
        evidence.is_upgrade = false;
        assert_eq!(
            dashboard_import_kind(&TitleHistoryEventType::Imported, Some(&evidence)),
            Some(NewImport)
        );
    }

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

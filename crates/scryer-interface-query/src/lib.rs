#![recursion_limit = "256"]

use async_graphql::{Context, Enum, ID, MergedObject, Object, Result as GqlResult, SimpleObject};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

use chrono::{DateTime, Utc};
use scryer_application::{
    AppError, DownloadImportFilter, ExternalImportArrSourceKind as AppArrSourceKind,
    ExternalImportMonitorWarmupStatus,
    ExternalImportSetupSecretDraft as AppExternalImportSetupSecretDraft,
    ExternalImportSetupSecretDraftStatus, ExternalImportSetupSecretInstanceKind,
    ExternalImportSetupSecretOverrideDraft, ImageProxyKind, JwtSessionScope, MediaRequestCounts,
    OAuthAuthorizationSource, PendingImportCounts, RenamePlan, RenameWriteAction, RuntimePathStyle,
    SCRYER_VERSION, SortDirection, TitleCatalogContentStatus, TitleCatalogFilter, TitleCatalogSort,
    TitleCatalogSortKey, TitleHistoryFilter, is_supported_title_history_event_type,
    supported_title_history_event_types,
};
use scryer_domain::{AppPermission, LibraryPermission, TitleHistoryEventType};
use scryer_interface_metadata::MetadataQueries;
use scryer_interface_settings::SettingsQueries;
use std::{fs, io, path::Path};

use scryer_interface_core as context;
use scryer_interface_core::{
    actor_from_ctx, actor_has_any_library_permission, actor_has_app_permission, app_from_ctx,
    application_upgrade_assessment_from_ctx, current_user_from_ctx, mfa_verification_from_ctx,
    require_app_permission, require_config_app_permission, to_gql_error,
};
use scryer_interface_media::mappers;
use scryer_interface_media::mappers::{
    catalog_discovery_query_from_input, discovery_home_filter_options_query_from_input,
    discovery_home_query_from_input, discovery_item_detail_query_from_input,
    discovery_items_query_from_input, from_active_import_stream, from_activity_event,
    from_application_upgrade_status, from_backup_info, from_catalog_discovery,
    from_change_title_folder_preview, from_collection, from_dashboard_activity_stats,
    from_delete_episode_files_preview, from_delete_preview, from_delete_titles_preview,
    from_discovery_home, from_discovery_home_cards, from_discovery_home_filter_options,
    from_discovery_item, from_discovery_items_result, from_domain_event, from_download_queue_item,
    from_episode, from_external_import_monitor_warmup_progress, from_job_definition, from_job_run,
    from_library, from_library_scan_session, from_library_settings, from_linked_account,
    from_location_operation, from_location_operation_asset_listing, from_media_rename_plan,
    from_media_request, from_media_request_counts, from_pending_import_connection,
    from_pending_import_counts, from_pending_release, from_provider_type, from_root_move_preview,
    from_root_scope_preview, from_runtime_path_style, from_smg_scryer_update_notice,
    from_smg_version_compatibility_notice, from_storage_root_usage, from_system_health, from_title,
    from_title_acquisition_diagnostics, from_title_history_page,
    from_title_release_blocklist_entry, from_user_with_auth_factor_status, from_wanted_item,
    from_wanted_scope_view, location_destination_into_application,
    location_execution_mode_into_application, root_scope_destination,
};
use scryer_interface_media::types::*;

fn from_metadata_search_item(
    app: &scryer_application::AppUseCase,
    item: scryer_application::RichMetadataSearchItem,
) -> MetadataSearchItemPayload {
    let owner_id = item
        .smg_id
        .map(|id| id.to_string())
        .unwrap_or_else(|| item.tvdb_id.to_string());
    let poster_url = app.media_image_url(
        item.poster_url.as_deref(),
        Some("metadata_search"),
        Some(&owner_id),
        ImageProxyKind::Poster,
        "w250",
    );
    MetadataSearchItemPayload {
        tvdb_id: item.tvdb_id,
        smg_id: item.smg_id,
        tmdb_id: item
            .external_ids
            .iter()
            .find(|external_id| external_id.source.eq_ignore_ascii_case("tmdb"))
            .and_then(|external_id| external_id.value.parse().ok()),
        primary_source: item.primary_source,
        external_ids: item
            .external_ids
            .into_iter()
            .map(|external_id| ExternalIdPayload {
                source: external_id.source,
                value: external_id.value,
            })
            .collect(),
        name: item.name,
        imdb_id: item.imdb_id,
        slug: item.slug,
        type_hint: item.type_hint,
        year: item.year,
        status: item.status,
        overview: item.overview,
        popularity: item.popularity,
        poster_url,
        language: item.language,
        runtime_minutes: item.runtime_minutes,
        sort_title: item.sort_title,
    }
}

fn browse_path_read_dir(path: &str) -> Result<fs::ReadDir, AppError> {
    let target = Path::new(path);
    if !target.is_absolute() {
        return Err(AppError::Validation("Path must be absolute.".to_string()));
    }

    let metadata = fs::metadata(target).map_err(|error| browse_path_io_error(path, error))?;
    if !metadata.is_dir() {
        return Err(AppError::Validation(format!(
            "Path is not a directory: {path}"
        )));
    }

    fs::read_dir(target).map_err(|error| browse_path_io_error(path, error))
}

fn browse_path_io_error(path: &str, error: io::Error) -> AppError {
    let message = match error.kind() {
        io::ErrorKind::NotFound => format!("Directory does not exist: {path}"),
        io::ErrorKind::PermissionDenied => format!("Directory is not readable: {path}"),
        _ => format!("Directory cannot be opened: {path}"),
    };
    AppError::Validation(message)
}

async fn require_library_settings_permission(ctx: &Context<'_>) -> GqlResult<()> {
    let app = app_from_ctx(ctx)?;
    let actor = actor_from_ctx(ctx)?;
    app.require_library_settings_read_permission(&actor)
        .await
        .map_err(to_gql_error)
}

fn supported_title_history_values_message() -> String {
    supported_title_history_event_types()
        .iter()
        .map(TitleHistoryEventType::as_str)
        .collect::<Vec<_>>()
        .join(", ")
}

fn parse_required_datetime(value: &str, field: &str) -> Result<DateTime<Utc>, AppError> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|error| AppError::Validation(format!("invalid {field} timestamp: {error}")))
}

fn parse_supported_title_history_event_types(
    event_types: Option<Vec<TitleHistoryEventTypeValue>>,
) -> GqlResult<Option<Vec<TitleHistoryEventType>>> {
    let Some(event_types) = event_types else {
        return Ok(None);
    };

    let supported_values = supported_title_history_values_message();
    let mut parsed = Vec::with_capacity(event_types.len());
    for value in event_types {
        let event_type = value.into_domain();
        if !is_supported_title_history_event_type(event_type) {
            return Err(to_gql_error(AppError::Validation(format!(
                "unsupported title history event type `{}`. Supported values: {supported_values}",
                event_type.as_str()
            ))));
        }
        parsed.push(event_type);
    }

    Ok(Some(parsed))
}

const TITLE_CATALOG_PAGE_SIZE: usize = 300;

fn title_catalog_page_limit(limit: Option<i32>) -> usize {
    limit
        .unwrap_or(TITLE_CATALOG_PAGE_SIZE as i32)
        .clamp(1, TITLE_CATALOG_PAGE_SIZE as i32) as usize
}

fn title_catalog_page_offset(offset: Option<i32>) -> usize {
    offset.unwrap_or(0).max(0) as usize
}

fn title_catalog_sort_from_input(sort: Option<TitleCatalogSortInput>) -> TitleCatalogSort {
    let Some(sort) = sort else {
        return TitleCatalogSort::default();
    };
    let key = match sort.key {
        TitleCatalogSortKeyValue::Title => TitleCatalogSortKey::Title,
        TitleCatalogSortKeyValue::Library => TitleCatalogSortKey::Library,
        TitleCatalogSortKeyValue::Monitored => TitleCatalogSortKey::Monitored,
        TitleCatalogSortKeyValue::Quality => TitleCatalogSortKey::Quality,
        TitleCatalogSortKeyValue::Episodes => TitleCatalogSortKey::Episodes,
        TitleCatalogSortKeyValue::Status => TitleCatalogSortKey::Status,
        TitleCatalogSortKeyValue::Size => TitleCatalogSortKey::Size,
        TitleCatalogSortKeyValue::Added => TitleCatalogSortKey::Added,
        TitleCatalogSortKeyValue::Year => TitleCatalogSortKey::Year,
        TitleCatalogSortKeyValue::Runtime => TitleCatalogSortKey::Runtime,
        TitleCatalogSortKeyValue::Root => TitleCatalogSortKey::Root,
        TitleCatalogSortKeyValue::Popularity => TitleCatalogSortKey::Popularity,
        TitleCatalogSortKeyValue::MediaResolution => TitleCatalogSortKey::MediaResolution,
        TitleCatalogSortKeyValue::MediaHdr => TitleCatalogSortKey::MediaHdr,
        TitleCatalogSortKeyValue::MediaAudioCodec => TitleCatalogSortKey::MediaAudioCodec,
        TitleCatalogSortKeyValue::RatingScryer => TitleCatalogSortKey::RatingScryer,
        TitleCatalogSortKeyValue::RatingImdb => TitleCatalogSortKey::RatingImdb,
        TitleCatalogSortKeyValue::RatingRottenTomatoes => TitleCatalogSortKey::RatingRottenTomatoes,
        TitleCatalogSortKeyValue::RatingPopcornmeter => TitleCatalogSortKey::RatingPopcornmeter,
        TitleCatalogSortKeyValue::RatingMetacritic => TitleCatalogSortKey::RatingMetacritic,
        TitleCatalogSortKeyValue::RatingMetacriticUser => TitleCatalogSortKey::RatingMetacriticUser,
        TitleCatalogSortKeyValue::RatingLetterboxd => TitleCatalogSortKey::RatingLetterboxd,
        TitleCatalogSortKeyValue::RatingTmdb => TitleCatalogSortKey::RatingTmdb,
        TitleCatalogSortKeyValue::RatingTvdb => TitleCatalogSortKey::RatingTvdb,
        TitleCatalogSortKeyValue::RatingTrakt => TitleCatalogSortKey::RatingTrakt,
        TitleCatalogSortKeyValue::RatingMyanimelist => TitleCatalogSortKey::RatingMyanimelist,
        TitleCatalogSortKeyValue::RatingAnilist => TitleCatalogSortKey::RatingAnilist,
        TitleCatalogSortKeyValue::RatingAnidb => TitleCatalogSortKey::RatingAnidb,
        TitleCatalogSortKeyValue::RatingMdblist => TitleCatalogSortKey::RatingMdblist,
    };
    let direction = sort
        .direction
        .map(SortDirectionValue::into_application)
        .unwrap_or(SortDirection::Asc);
    TitleCatalogSort { key, direction }
}

fn title_catalog_tag_filter_keys(
    field_name: &str,
    keys: Option<Vec<String>>,
) -> Result<Vec<String>, AppError> {
    let keys = keys.unwrap_or_default();
    if keys.iter().any(|key| key.trim().is_empty()) {
        return Err(AppError::Validation(format!(
            "title catalog {field_name} entries must not be blank"
        )));
    }
    Ok(keys)
}

/// Normalize the requested user-tag labels the way the registry stores them,
/// so a filter typed as `Needs  Review` finds the titles carrying
/// `needs review`. Anything the registry could never hold — a blank label, the
/// reserved namespace, an over-long label — is refused by name rather than
/// silently matching nothing.
fn title_catalog_user_tags(tags: Option<Vec<String>>) -> Result<Vec<String>, AppError> {
    let tags = tags.unwrap_or_default();
    if tags.is_empty() {
        return Ok(Vec::new());
    }
    scryer_application::normalize_user_title_tags(&tags).map_err(|error| {
        AppError::Validation(format!("title catalog tags entries are invalid: {error}"))
    })
}

fn title_catalog_filter_from_input(
    filter: Option<TitleCatalogFilterInput>,
) -> Result<TitleCatalogFilter, AppError> {
    let Some(filter) = filter else {
        return Ok(TitleCatalogFilter::default());
    };
    if let (Some(minimum_year), Some(maximum_year)) = (filter.minimum_year, filter.maximum_year)
        && minimum_year > maximum_year
    {
        return Err(AppError::Validation(
            "title catalog minimumYear must not exceed maximumYear".to_owned(),
        ));
    }
    let minimum_rating = filter.minimum_rating;
    if minimum_rating.is_some_and(|rating| !rating.is_finite() || !(0.0..=10.0).contains(&rating)) {
        return Err(AppError::Validation(
            "title catalog minimumRating must be a finite value between 0 and 10".to_owned(),
        ));
    }

    Ok(TitleCatalogFilter {
        monitored: filter.monitored,
        content_statuses: filter
            .content_statuses
            .unwrap_or_default()
            .into_iter()
            .map(|status| match status {
                TitleCatalogContentStatusValue::Continuing => TitleCatalogContentStatus::Continuing,
                TitleCatalogContentStatusValue::Ended => TitleCatalogContentStatus::Ended,
            })
            .collect(),
        root_folder_ids: normalize_catalog_filter_values(
            filter
                .root_folder_ids
                .unwrap_or_default()
                .into_iter()
                .map(String::from),
        ),
        genre_tag_keys: title_catalog_tag_filter_keys("genreTagKeys", filter.genre_tag_keys)?,
        theme_tag_keys: title_catalog_tag_filter_keys("themeTagKeys", filter.theme_tag_keys)?,
        user_tags: title_catalog_user_tags(filter.tags)?,
        minimum_year: filter.minimum_year,
        maximum_year: filter.maximum_year,
        minimum_rating,
    })
}

fn normalize_catalog_filter_values(values: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut normalized = Vec::new();
    for value in values {
        let value = value.trim();
        if !value.is_empty() && !normalized.iter().any(|candidate| candidate == value) {
            normalized.push(value.to_string());
        }
    }
    normalized
}

fn usize_to_i32_saturating(value: usize) -> i32 {
    value.min(i32::MAX as usize) as i32
}

fn optional_ids_to_strings(ids: Option<Vec<ID>>) -> Option<Vec<String>> {
    ids.map(|ids| ids.into_iter().map(String::from).collect())
}

fn from_download_history_page(
    page: scryer_application::DownloadHistoryPage,
) -> DownloadHistoryPagePayload {
    DownloadHistoryPagePayload {
        items: page
            .items
            .into_iter()
            .map(from_download_queue_item)
            .collect(),
        has_more: page.has_more,
        total_count: page.total_count as i32,
        available_clients: page
            .available_clients
            .into_iter()
            .map(|client| DownloadClientFilterOptionPayload {
                client_id: client.client_id.into(),
                client_name: client.client_name,
                client_type: client.client_type,
            })
            .collect(),
    }
}

fn from_download_queue_page(
    page: scryer_application::DownloadQueuePage,
) -> DownloadQueuePagePayload {
    DownloadQueuePagePayload {
        items: page
            .items
            .into_iter()
            .map(from_download_queue_item)
            .collect(),
        has_more: page.has_more,
        total_count: usize_to_i32_saturating(page.total_count),
        available_clients: page
            .available_clients
            .into_iter()
            .map(|client| DownloadClientFilterOptionPayload {
                client_id: client.client_id.into(),
                client_name: client.client_name,
                client_type: client.client_type,
            })
            .collect(),
        revision: Long::from_u64_saturating(page.revision),
        updated_at: page.updated_at,
        ready: page.ready,
        stale: page.stale,
    }
}

fn from_download_import_page(
    page: scryer_application::DownloadImportPage,
) -> DownloadImportPagePayload {
    DownloadImportPagePayload {
        items: page
            .items
            .into_iter()
            .map(from_download_queue_item)
            .collect(),
        has_more: page.has_more,
        total_count: page.total_count as i32,
    }
}

fn wanted_kind_to_application(value: WantedKindValue) -> scryer_application::WantedKind {
    match value {
        WantedKindValue::Missing => scryer_application::WantedKind::Missing,
        WantedKindValue::CutoffUpgrade => scryer_application::WantedKind::CutoffUpgrade,
    }
}

fn from_acquisition_search_job_view(
    view: scryer_application::AcquisitionSearchJobView,
) -> scryer_application::AppResult<AcquisitionSearchJobPayload> {
    let state = match view.state.as_str() {
        "completed" => AcquisitionSearchJobStateValue::Completed,
        "cancelled" => AcquisitionSearchJobStateValue::Cancelled,
        "failed" => AcquisitionSearchJobStateValue::Failed,
        _ => AcquisitionSearchJobStateValue::Running,
    };
    let started_at = chrono::DateTime::parse_from_rfc3339(&view.started_at)
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .map_err(|error| {
            scryer_application::AppError::Validation(format!(
                "invalid acquisition search job started_at: {error}"
            ))
        })?;
    let finished_at = view
        .finished_at
        .as_deref()
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .map(|dt| dt.with_timezone(&chrono::Utc));
    Ok(AcquisitionSearchJobPayload {
        id: view.id.into(),
        state,
        total: view.total,
        processed: view.processed,
        grabbed_count: view.grabbed_count,
        failed_count: view.failed_count,
        current_title: view.current_title,
        started_at,
        finished_at,
    })
}

pub fn from_interactive_release_search_snapshot(
    snapshot: scryer_application::InteractiveReleaseSearchSnapshot,
) -> InteractiveReleaseSearchPayload {
    let state = match snapshot.state {
        scryer_application::InteractiveReleaseSearchState::Running => {
            InteractiveReleaseSearchStateValue::Running
        }
        scryer_application::InteractiveReleaseSearchState::Completed => {
            InteractiveReleaseSearchStateValue::Completed
        }
        scryer_application::InteractiveReleaseSearchState::Cancelled => {
            InteractiveReleaseSearchStateValue::Cancelled
        }
    };
    let indexers = snapshot
        .indexers
        .into_iter()
        .map(|indexer| InteractiveReleaseSearchIndexerPayload {
            indexer_id: indexer.indexer_id.into(),
            name: indexer.name,
            status: match indexer.status {
                scryer_application::InteractiveReleaseSearchIndexerStatus::Pending => {
                    InteractiveReleaseSearchIndexerStatusValue::Pending
                }
                scryer_application::InteractiveReleaseSearchIndexerStatus::Searching => {
                    InteractiveReleaseSearchIndexerStatusValue::Searching
                }
                scryer_application::InteractiveReleaseSearchIndexerStatus::Completed => {
                    InteractiveReleaseSearchIndexerStatusValue::Completed
                }
                scryer_application::InteractiveReleaseSearchIndexerStatus::Failed => {
                    InteractiveReleaseSearchIndexerStatusValue::Failed
                }
                scryer_application::InteractiveReleaseSearchIndexerStatus::Skipped => {
                    InteractiveReleaseSearchIndexerStatusValue::Skipped
                }
            },
            result_count: indexer.result_count as i32,
            priority: indexer.priority as i32,
            elapsed_ms: indexer
                .elapsed_ms
                .map(|elapsed| i32::try_from(elapsed).unwrap_or(i32::MAX)),
            failure_reason: indexer.failure_reason,
        })
        .collect();
    // Parity with the one-shot `searchReleases` resolver's limit handling.
    let safe_limit = snapshot.limit.unwrap_or(50).clamp(1, 200) as usize;
    InteractiveReleaseSearchPayload {
        id: snapshot.id.into(),
        state,
        results: snapshot
            .results
            .into_iter()
            .take(safe_limit)
            .map(crate::mappers::from_search_result)
            .collect(),
        indexers,
        started_at: snapshot.started_at,
        completed_at: snapshot.completed_at,
    }
}

fn from_cutoff_unmet_item(
    item: scryer_application::CutoffUnmetItem,
    convergence: scryer_application::WantedViewConvergence,
) -> CutoffUnmetItemPayload {
    CutoffUnmetItemPayload {
        title_id: item.title_id.into(),
        title_name: item.title_name,
        title_slug: item.title_slug,
        title_facet: MediaFacetValue::from_domain(item.title_facet),
        library_id: item.library_id.into(),
        library_name: item.library_name,
        library_slug: item.library_slug,
        episode_id: item.episode_id.map(Into::into),
        season_number: item.season_number,
        episode_number: item.episode_number,
        current_tier: item.current_tier,
        target_tier: item.target_tier,
        convergence_state: convergence_state_to_value(convergence.state),
        indexers_covered: convergence.indexers_covered,
        indexers_routed: convergence.indexers_routed,
    }
}

fn convergence_state_to_value(
    state: scryer_application::WantedConvergenceState,
) -> ConvergenceStateValue {
    match state {
        scryer_application::WantedConvergenceState::Queued => ConvergenceStateValue::Queued,
        scryer_application::WantedConvergenceState::Searching => ConvergenceStateValue::Searching,
        scryer_application::WantedConvergenceState::Converged => ConvergenceStateValue::Converged,
        scryer_application::WantedConvergenceState::Deferred => ConvergenceStateValue::Deferred,
    }
}

#[derive(Clone, Copy)]
struct TitlePayloadSelection {
    include_external_ids: bool,
}

impl TitlePayloadSelection {
    fn from_ctx(ctx: &Context<'_>) -> Self {
        let lookahead = ctx.look_ahead();
        let title_field_exists = |name: &str| {
            lookahead.field(name).exists() || lookahead.field("items").field(name).exists()
        };
        Self {
            include_external_ids: title_field_exists("externalIds"),
        }
    }
}

#[derive(Default)]
struct CatalogQueries;

#[derive(Default)]
struct ActivityQueries;

#[derive(Default)]
struct JobAndDownloadQueries;

#[derive(Default)]
struct SystemQueries;

#[derive(Default)]
struct AcquisitionQueries;

#[derive(Default)]
struct ExternalImportQueries;

#[derive(Default)]
struct UtilityQueries;

#[derive(Default)]
struct AccountQueries;

#[derive(Default)]
struct IndexerErrorQueries;

/// Identifies the indexer operation that produced a persisted HTTP error.
#[derive(Clone, Copy, Enum, Eq, PartialEq)]
enum IndexerErrorOperationValue {
    /// Connection validation against the configured indexer.
    ConnectionTest,
    /// An operator-initiated release search.
    InteractiveSearch,
    /// A scheduled or automated release search.
    AutomaticSearch,
    /// A periodic RSS synchronization.
    RssSync,
    /// A plugin-defined indexer action.
    IndexerAction,
    /// Synchronization with an indexer management service.
    ManagementSync,
    /// Refreshing a managed indexer's capabilities document.
    CapsRefresh,
}

/// A normalized, safe classification for an indexer HTTP error.
#[derive(Clone, Copy, Enum, Eq, PartialEq)]
enum IndexerErrorClassificationValue {
    /// Newznab reported an invalid API key.
    NewznabInvalidApiKey,
    /// Newznab reported a suspended account.
    NewznabAccountSuspended,
    /// Newznab reported insufficient account privileges.
    NewznabInsufficientPrivileges,
    /// Newznab denied registration.
    NewznabRegistrationDenied,
    /// Newznab has closed registrations.
    NewznabRegistrationsClosed,
    /// Newznab rejected registration data.
    NewznabInvalidRegistration,
    /// Newznab rejected the registration email address.
    NewznabInvalidRegistrationEmail,
    /// Newznab registration failed.
    NewznabRegistrationFailed,
    /// Newznab reported a missing request parameter.
    NewznabMissingParameter,
    /// Newznab reported an incorrect request parameter.
    NewznabIncorrectParameter,
    /// Newznab reported an unsupported function.
    NewznabNoSuchFunction,
    /// Newznab reported a function that is unavailable.
    NewznabFunctionNotAvailable,
    /// Newznab reported that no matching item exists.
    NewznabNoSuchItem,
    /// Newznab reported that the request limit was reached.
    NewznabRequestLimitReached,
    /// Newznab reported that the download limit was reached.
    NewznabDownloadLimitReached,
    /// Newznab reported its generic unknown error.
    NewznabUnknownError,
    /// Newznab reported that its API is disabled.
    NewznabApiDisabled,
    /// The server rejected the request as invalid.
    HttpBadRequest,
    /// The server rejected authentication.
    HttpUnauthorized,
    /// The server forbade access to the resource.
    HttpForbidden,
    /// The requested endpoint was not found.
    HttpNotFound,
    /// The server timed out the request.
    HttpRequestTimeout,
    /// The server rate limited the request.
    HttpRateLimited,
    /// The server returned a 5xx response.
    HttpServerError,
    /// The accepted response did not match a known error shape.
    Unknown,
}

/// List-safe metadata for a persisted indexer error.
#[derive(SimpleObject)]
struct IndexerErrorSummaryPayload {
    /// Stable error-event identifier.
    id: ID,
    /// Configured indexer identifier captured with the event.
    indexer_id: ID,
    /// Indexer name captured when the event occurred.
    indexer_name: String,
    /// Operation that received the failed response.
    operation: IndexerErrorOperationValue,
    /// RFC 3339 time at which the response was accepted.
    occurred_at: String,
    /// HTTP response status code, when the upstream returned a response.
    http_status: Option<i32>,
    /// Safe error classification derived from the response.
    classification: IndexerErrorClassificationValue,
    /// Newznab provider code when a known Newznab error was recognized.
    provider_error_code: Option<i32>,
    /// Canonical safe error message that excludes response-body text.
    message: String,
    /// Response Content-Type header when it was valid UTF-8.
    content_type: Option<String>,
}

/// One raw response header retained for privileged diagnostics.
#[derive(SimpleObject)]
struct IndexerErrorHeaderPayload {
    /// Original response header name.
    name: String,
    /// Raw header value encoded as base64.
    value_base64: String,
    /// Raw header value when it is valid UTF-8.
    value: Option<String>,
}

/// Complete persisted HTTP response retained for privileged diagnostics.
#[derive(SimpleObject)]
struct IndexerErrorResponsePayload {
    /// Original HTTP status code.
    status: i32,
    /// Repeated raw response headers in their retained order.
    headers: Vec<IndexerErrorHeaderPayload>,
    /// Raw response body encoded as base64.
    body_base64: String,
}

/// A persisted error event and, when available, its complete HTTP response.
#[derive(SimpleObject)]
struct IndexerErrorDetailPayload {
    /// List-safe metadata for the error event.
    error: IndexerErrorSummaryPayload,
    /// Complete response data when the upstream returned one.
    response: Option<IndexerErrorResponsePayload>,
}

/// A newest-first page of persisted indexer errors.
#[derive(SimpleObject)]
struct IndexerErrorConnectionPayload {
    /// Error events in newest-first order.
    items: Vec<IndexerErrorSummaryPayload>,
    /// Cursor for the following page, or null at the end of the result set.
    next_cursor: Option<String>,
}

#[derive(MergedObject, Default)]
/// Read-only GraphQL query root for authenticated HTTP requests.
pub struct QueryRoot(
    CatalogQueries,
    ActivityQueries,
    JobAndDownloadQueries,
    SettingsQueries,
    SystemQueries,
    AcquisitionQueries,
    ExternalImportQueries,
    MetadataQueries,
    UtilityQueries,
    AccountQueries,
    IndexerErrorQueries,
);

fn indexer_error_operation_value(
    operation: scryer_application::IndexerErrorOperation,
) -> IndexerErrorOperationValue {
    match operation {
        scryer_application::IndexerErrorOperation::ConnectionTest => {
            IndexerErrorOperationValue::ConnectionTest
        }
        scryer_application::IndexerErrorOperation::InteractiveSearch => {
            IndexerErrorOperationValue::InteractiveSearch
        }
        scryer_application::IndexerErrorOperation::AutomaticSearch => {
            IndexerErrorOperationValue::AutomaticSearch
        }
        scryer_application::IndexerErrorOperation::RssSync => IndexerErrorOperationValue::RssSync,
        scryer_application::IndexerErrorOperation::IndexerAction => {
            IndexerErrorOperationValue::IndexerAction
        }
        scryer_application::IndexerErrorOperation::ManagementSync => {
            IndexerErrorOperationValue::ManagementSync
        }
        scryer_application::IndexerErrorOperation::CapsRefresh => {
            IndexerErrorOperationValue::CapsRefresh
        }
    }
}

fn indexer_error_classification_value(
    classification: scryer_application::IndexerErrorClassification,
) -> IndexerErrorClassificationValue {
    use scryer_application::IndexerErrorClassification as Value;
    match classification {
        Value::NewznabInvalidApiKey => IndexerErrorClassificationValue::NewznabInvalidApiKey,
        Value::NewznabAccountSuspended => IndexerErrorClassificationValue::NewznabAccountSuspended,
        Value::NewznabInsufficientPrivileges => {
            IndexerErrorClassificationValue::NewznabInsufficientPrivileges
        }
        Value::NewznabRegistrationDenied => {
            IndexerErrorClassificationValue::NewznabRegistrationDenied
        }
        Value::NewznabRegistrationsClosed => {
            IndexerErrorClassificationValue::NewznabRegistrationsClosed
        }
        Value::NewznabInvalidRegistration => {
            IndexerErrorClassificationValue::NewznabInvalidRegistration
        }
        Value::NewznabInvalidRegistrationEmail => {
            IndexerErrorClassificationValue::NewznabInvalidRegistrationEmail
        }
        Value::NewznabRegistrationFailed => {
            IndexerErrorClassificationValue::NewznabRegistrationFailed
        }
        Value::NewznabMissingParameter => IndexerErrorClassificationValue::NewznabMissingParameter,
        Value::NewznabIncorrectParameter => {
            IndexerErrorClassificationValue::NewznabIncorrectParameter
        }
        Value::NewznabNoSuchFunction => IndexerErrorClassificationValue::NewznabNoSuchFunction,
        Value::NewznabFunctionNotAvailable => {
            IndexerErrorClassificationValue::NewznabFunctionNotAvailable
        }
        Value::NewznabNoSuchItem => IndexerErrorClassificationValue::NewznabNoSuchItem,
        Value::NewznabRequestLimitReached => {
            IndexerErrorClassificationValue::NewznabRequestLimitReached
        }
        Value::NewznabDownloadLimitReached => {
            IndexerErrorClassificationValue::NewznabDownloadLimitReached
        }
        Value::NewznabUnknownError => IndexerErrorClassificationValue::NewznabUnknownError,
        Value::NewznabApiDisabled => IndexerErrorClassificationValue::NewznabApiDisabled,
        Value::HttpBadRequest => IndexerErrorClassificationValue::HttpBadRequest,
        Value::HttpUnauthorized => IndexerErrorClassificationValue::HttpUnauthorized,
        Value::HttpForbidden => IndexerErrorClassificationValue::HttpForbidden,
        Value::HttpNotFound => IndexerErrorClassificationValue::HttpNotFound,
        Value::HttpRequestTimeout => IndexerErrorClassificationValue::HttpRequestTimeout,
        Value::HttpRateLimited => IndexerErrorClassificationValue::HttpRateLimited,
        Value::HttpServerError => IndexerErrorClassificationValue::HttpServerError,
        Value::Unknown => IndexerErrorClassificationValue::Unknown,
    }
}

fn from_indexer_error_summary(
    error: scryer_application::IndexerErrorSummary,
) -> IndexerErrorSummaryPayload {
    IndexerErrorSummaryPayload {
        id: ID::from(error.id),
        indexer_id: ID::from(error.indexer_id),
        indexer_name: error.indexer_name,
        operation: indexer_error_operation_value(error.operation),
        occurred_at: error.occurred_at.to_rfc3339(),
        http_status: error.http_status.map(i32::from),
        classification: indexer_error_classification_value(error.classification),
        provider_error_code: error.provider_error_code.map(i32::from),
        message: error.message,
        content_type: error.content_type,
    }
}

fn from_indexer_error_detail(
    detail: scryer_application::IndexerErrorDetail,
) -> IndexerErrorDetailPayload {
    IndexerErrorDetailPayload {
        error: from_indexer_error_summary(detail.summary),
        response: detail.response.map(|response| IndexerErrorResponsePayload {
            status: i32::from(response.status),
            headers: response
                .headers
                .into_iter()
                .map(|header| IndexerErrorHeaderPayload {
                    name: header.name,
                    value: String::from_utf8(header.value.clone()).ok(),
                    value_base64: BASE64.encode(header.value),
                })
                .collect(),
            body_base64: BASE64.encode(response.body),
        }),
    }
}

#[Object]
impl IndexerErrorQueries {
    /// List persisted indexer HTTP errors; requires system-settings management permission.
    async fn indexer_errors(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Restrict results to one configured indexer.")] indexer_id: Option<ID>,
        #[graphql(
            default = 50,
            desc = "Requested page size from 1 through 100; defaults to 50."
        )]
        first: Option<i32>,
        #[graphql(desc = "Opaque cursor returned by a prior indexerErrors page.")] after: Option<
            String,
        >,
    ) -> GqlResult<IndexerErrorConnectionPayload> {
        require_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let first = first.unwrap_or(50);
        if !(1..=100).contains(&first) {
            return Err(to_gql_error(AppError::Validation(
                "indexerErrors.first must be between 1 and 100".to_string(),
            )));
        }
        let app = app_from_ctx(ctx)?;
        let indexer_id = indexer_id.map(|id| id.to_string());
        let page = app
            .list_indexer_errors(indexer_id.as_deref(), first as usize, after.as_deref())
            .await
            .map_err(to_gql_error)?;
        Ok(IndexerErrorConnectionPayload {
            items: page
                .items
                .into_iter()
                .map(from_indexer_error_summary)
                .collect(),
            next_cursor: page.next_cursor,
        })
    }

    /// Return one complete persisted indexer HTTP response; requires system-settings management permission.
    async fn indexer_error(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Stable persisted error-event identifier.")] id: ID,
    ) -> GqlResult<Option<IndexerErrorDetailPayload>> {
        require_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let app = app_from_ctx(ctx)?;
        app.indexer_error_detail(id.as_str())
            .await
            .map_err(to_gql_error)
            .map(|detail| detail.map(from_indexer_error_detail))
    }
}

#[cfg(test)]
mod indexer_error_payload_tests {
    use super::*;

    #[test]
    fn detail_payload_base64_encodes_binary_response_and_only_decodes_utf8_headers() {
        let occurred_at = chrono::Utc::now();
        let payload = from_indexer_error_detail(scryer_application::IndexerErrorDetail {
            summary: scryer_application::IndexerErrorSummary {
                id: "error-1".to_string(),
                indexer_id: "indexer-1".to_string(),
                indexer_name: "Test indexer".to_string(),
                operation: scryer_application::IndexerErrorOperation::InteractiveSearch,
                http_status: Some(500),
                classification: scryer_application::IndexerErrorClassification::HttpServerError,
                provider_error_code: None,
                message: "Indexer server error".to_string(),
                content_type: Some("application/octet-stream".to_string()),
                occurred_at,
            },
            response: Some(scryer_application::CapturedIndexerHttpResponse {
                status: 500,
                headers: vec![
                    scryer_application::CapturedIndexerHttpHeader {
                        name: "x-text".to_string(),
                        value: b"visible".to_vec(),
                    },
                    scryer_application::CapturedIndexerHttpHeader {
                        name: "x-binary".to_string(),
                        value: vec![255],
                    },
                ],
                body: vec![255, 0],
            }),
        });

        assert_eq!(payload.error.id.as_str(), "error-1");
        let response = payload.response.expect("captured response");
        assert_eq!(response.status, 500);
        assert_eq!(response.body_base64, "/wA=");
        assert_eq!(response.headers[0].value.as_deref(), Some("visible"));
        assert_eq!(response.headers[0].value_base64, "dmlzaWJsZQ==");
        assert_eq!(response.headers[1].value, None);
        assert_eq!(response.headers[1].value_base64, "/w==");
    }

    #[test]
    fn detail_payload_preserves_transport_errors_without_an_http_response() {
        let payload = from_indexer_error_detail(scryer_application::IndexerErrorDetail {
            summary: scryer_application::IndexerErrorSummary {
                id: "error-transport".to_string(),
                indexer_id: "indexer-1".to_string(),
                indexer_name: "Test indexer".to_string(),
                operation: scryer_application::IndexerErrorOperation::AutomaticSearch,
                http_status: None,
                classification: scryer_application::IndexerErrorClassification::HttpRequestTimeout,
                provider_error_code: None,
                message: "Indexer search timed out".to_string(),
                content_type: None,
                occurred_at: chrono::Utc::now(),
            },
            response: None,
        });

        assert_eq!(payload.error.http_status, None);
        assert!(payload.response.is_none());
    }
}

fn gql_secret_instance_kind_query(
    kind: ExternalImportSetupSecretInstanceKind,
) -> ExternalImportConnectionKind {
    match kind {
        ExternalImportSetupSecretInstanceKind::Sonarr => ExternalImportConnectionKind::Sonarr,
        ExternalImportSetupSecretInstanceKind::Radarr => ExternalImportConnectionKind::Radarr,
        ExternalImportSetupSecretInstanceKind::Prowlarr => ExternalImportConnectionKind::Prowlarr,
    }
}

fn api_key_override_payload_query(
    override_entry: ExternalImportSetupSecretOverrideDraft,
) -> ExternalImportSetupApiKeyOverridePayload {
    ExternalImportSetupApiKeyOverridePayload {
        dedup_key: override_entry.dedup_key,
        api_key: override_entry.secret,
    }
}

fn password_override_payload_query(
    override_entry: ExternalImportSetupSecretOverrideDraft,
) -> ExternalImportSetupPasswordOverridePayload {
    ExternalImportSetupPasswordOverridePayload {
        dedup_key: override_entry.dedup_key,
        password: override_entry.secret,
    }
}

fn external_import_setup_secret_draft_payload_query(
    draft: AppExternalImportSetupSecretDraft,
) -> ExternalImportSetupSecretDraftPayload {
    let secrets = draft.secrets;
    ExternalImportSetupSecretDraftPayload {
        instance_api_keys: secrets
            .instance_api_keys
            .into_iter()
            .map(|entry| ExternalImportSetupInstanceApiKeyPayload {
                instance_id: ID::from(entry.instance_id),
                kind: gql_secret_instance_kind_query(entry.kind),
                api_key: entry.api_key,
            })
            .collect(),
        download_client_api_key_overrides: secrets
            .download_client_api_key_overrides
            .into_iter()
            .map(api_key_override_payload_query)
            .collect(),
        download_client_password_overrides: secrets
            .download_client_password_overrides
            .into_iter()
            .map(password_override_payload_query)
            .collect(),
        indexer_api_key_overrides: secrets
            .indexer_api_key_overrides
            .into_iter()
            .map(api_key_override_payload_query)
            .collect(),
        updated_at: draft.updated_at,
    }
}

fn external_import_setup_secret_status_payload_query(
    status: ExternalImportSetupSecretDraftStatus,
) -> ExternalImportSetupSecretDraftStatusPayload {
    ExternalImportSetupSecretDraftStatusPayload {
        has_draft: status.has_draft,
        owned_by_current_user: status.owned_by_current_user,
        updated_at: status.updated_at,
    }
}

#[Object]
impl ExternalImportQueries {
    /// Read the caller-visible external import setup secret draft; requires system-settings management permission.
    async fn external_import_setup_secret_draft(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Option<ExternalImportSetupSecretDraftPayload>> {
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let app = app_from_ctx(ctx)?;
        app.get_external_import_setup_secret_draft(&actor)
            .await
            .map(|draft| draft.map(external_import_setup_secret_draft_payload_query))
            .map_err(to_gql_error)
    }

    /// Report whether an external import setup secret draft exists and whether the caller owns it.
    async fn external_import_setup_secret_draft_status(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<ExternalImportSetupSecretDraftStatusPayload> {
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let app = app_from_ctx(ctx)?;
        app.external_import_setup_secret_draft_status(&actor)
            .await
            .map(external_import_setup_secret_status_payload_query)
            .map_err(to_gql_error)
    }
}

#[Object]
impl AccountQueries {
    /// List linked external accounts visible to the caller, optionally restricted to a user ID.
    async fn linked_accounts(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "User ID filter; omitted lists accounts visible to the caller."
        )]
        user_id: Option<ID>,
    ) -> GqlResult<Vec<LinkedAccountPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let user_id = user_id.map(String::from);
        app.list_linked_accounts(&actor, user_id.as_deref())
            .await
            .map(|accounts| accounts.into_iter().map(from_linked_account).collect())
            .map_err(to_gql_error)
    }

    /// List external account invitations visible to the caller.
    async fn external_account_invites(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Vec<LinkedAccountPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.list_external_account_invites(&actor)
            .await
            .map(|accounts| accounts.into_iter().map(from_linked_account).collect())
            .map_err(to_gql_error)
    }
}

#[allow(clippy::too_many_arguments)]
#[Object]
impl CatalogQueries {
    /// List a permission-filtered title catalog page with optional facet, filter, search, sort, and pagination controls.
    async fn titles(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Restrict results to one media facet; omitted includes all facets visible to the caller."
        )]
        facet: Option<MediaFacetValue>,
        #[graphql(
            desc = "Restrict results to these library IDs; omitted or empty uses all libraries visible to the caller."
        )]
        library_ids: Option<Vec<ID>>,
        #[graphql(desc = "Optional title search text; omitted applies no text filter.")]
        query: Option<String>,
        #[graphql(
            desc = "Optional catalog filters for monitoring, content status, roots, tags, year, and rating."
        )]
        filter: Option<TitleCatalogFilterInput>,
        #[graphql(desc = "Optional sort key and direction; omitted uses the application default.")]
        sort: Option<TitleCatalogSortInput>,
        #[graphql(desc = "Requested page size; defaults to 300 and is clamped to 1 through 300.")]
        limit: Option<i32>,
        #[graphql(
            desc = "Zero-based page offset; defaults to 0 and negative values are treated as 0."
        )]
        offset: Option<i32>,
    ) -> GqlResult<TitleCatalogPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let selection = TitlePayloadSelection::from_ctx(ctx);
        let lookahead = ctx.look_ahead();
        let catalog_filter = title_catalog_filter_from_input(filter).map_err(to_gql_error)?;
        let include_catalog_counts = lookahead.field("hasMore").exists()
            || lookahead.field("totalCount").exists()
            || lookahead.field("filterCounts").exists()
            || lookahead.field("managedBytes").exists();
        let page = app
            .list_titles(
                &actor,
                facet.map(MediaFacetValue::into_domain),
                optional_ids_to_strings(library_ids),
                query,
                catalog_filter,
                title_catalog_sort_from_input(sort),
                title_catalog_page_limit(limit),
                title_catalog_page_offset(offset),
                selection.include_external_ids,
                include_catalog_counts,
            )
            .await
            .map_err(to_gql_error)?;
        let has_more = page.has_more;
        let total_count = page.total_count;
        let filter_counts = page.filter_counts.clone();
        let managed_bytes = page.managed_bytes;
        let items = page
            .items
            .into_iter()
            .map(|title| from_title(&app, title))
            .collect();

        Ok(TitleCatalogPayload {
            items,
            has_more,
            total_count: usize_to_i32_saturating(total_count),
            filter_counts: TitleCatalogFilterCountsPayload {
                all: usize_to_i32_saturating(filter_counts.all),
                monitored: usize_to_i32_saturating(filter_counts.monitored),
                unmonitored: usize_to_i32_saturating(filter_counts.unmonitored),
                continuing: usize_to_i32_saturating(filter_counts.continuing),
                ended: usize_to_i32_saturating(filter_counts.ended),
            },
            managed_bytes: Long::from(managed_bytes),
        })
    }

    /// Return available catalog filter values for the requested facet and library scope.
    async fn title_catalog_filter_options(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Restrict options to one media facet; omitted includes all facets visible to the caller."
        )]
        facet: Option<MediaFacetValue>,
        #[graphql(
            desc = "Restrict options to these library IDs; omitted or empty uses all libraries visible to the caller."
        )]
        library_ids: Option<Vec<ID>>,
        #[graphql(
            desc = "Restrict options to these root-folder IDs; omitted or empty uses all roots, and blank or duplicate values are ignored."
        )]
        root_folder_ids: Option<Vec<ID>>,
    ) -> GqlResult<TitleCatalogFilterOptionsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let options = app
            .title_catalog_filter_options(
                &actor,
                facet.map(MediaFacetValue::into_domain),
                optional_ids_to_strings(library_ids),
                normalize_catalog_filter_values(
                    root_folder_ids
                        .unwrap_or_default()
                        .into_iter()
                        .map(String::from),
                ),
            )
            .await
            .map_err(to_gql_error)?;
        let map_options = |values: Vec<scryer_application::TitleCatalogTagFilterOption>| {
            values
                .into_iter()
                .map(|option| CanonicalTagFilterOptionPayload {
                    key: option.key,
                    name: option.name,
                })
                .collect()
        };

        Ok(TitleCatalogFilterOptionsPayload {
            genres: map_options(options.genres),
            themes: map_options(options.tags),
            minimum_year: options.minimum_year,
            maximum_year: options.maximum_year,
        })
    }

    /// List libraries visible to the caller that grant at least the requested permission.
    async fn libraries(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Restrict results to one media facet; omitted includes all facets.")]
        facet: Option<MediaFacetValue>,
        #[graphql(desc = "Minimum library permission required; defaults to View.")]
        permission: Option<LibraryPermissionValue>,
    ) -> GqlResult<Vec<LibraryPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let libraries = app
            .list_libraries_for_permission(
                &actor,
                facet.map(MediaFacetValue::into_domain),
                permission
                    .map(LibraryPermissionValue::into_domain)
                    .unwrap_or(scryer_domain::LibraryPermission::View),
            )
            .await
            .map_err(to_gql_error)?;
        Ok(libraries.into_iter().map(from_library).collect())
    }

    /// List root folders of libraries the caller can view, with the disk usage of each backing filesystem.
    async fn storage_roots(&self, ctx: &Context<'_>) -> GqlResult<Vec<StorageRootUsagePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let roots = app.storage_root_usage(&actor).await.map_err(to_gql_error)?;
        Ok(roots.into_iter().map(from_storage_root_usage).collect())
    }

    /// List media requests visible to the caller, optionally filtered by facet, libraries, and status.
    async fn media_requests(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Restrict requests to one media facet; omitted includes all facets.")]
        facet: Option<MediaFacetValue>,
        #[graphql(
            desc = "Restrict requests to these library IDs; omitted uses all permitted libraries and an empty list returns no rows."
        )]
        library_ids: Option<Vec<ID>>,
        #[graphql(desc = "Restrict requests to one status; omitted includes all statuses.")]
        status: Option<MediaRequestStatusValue>,
    ) -> GqlResult<Vec<MediaRequestPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let requests = app
            .list_media_requests(
                &actor,
                scryer_application::ListMediaRequestsInput {
                    facet: facet.map(MediaFacetValue::into_domain),
                    library_ids: optional_ids_to_strings(library_ids),
                    status: status.map(MediaRequestStatusValue::into_domain),
                },
            )
            .await
            .map_err(to_gql_error)?;
        Ok(requests
            .into_iter()
            .map(|request| from_media_request(&app, request))
            .collect())
    }

    /// List only the caller's media requests, optionally filtered by facet, libraries, and status.
    async fn my_media_requests(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Restrict requests to one media facet; omitted includes all facets.")]
        facet: Option<MediaFacetValue>,
        #[graphql(
            desc = "Restrict requests to these library IDs; omitted uses all permitted libraries and an empty list returns no rows."
        )]
        library_ids: Option<Vec<ID>>,
        #[graphql(desc = "Restrict requests to one status; omitted includes all statuses.")]
        status: Option<MediaRequestStatusValue>,
    ) -> GqlResult<Vec<MediaRequestPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let requests = app
            .list_my_media_requests(
                &actor,
                scryer_application::ListMediaRequestsInput {
                    facet: facet.map(MediaFacetValue::into_domain),
                    library_ids: optional_ids_to_strings(library_ids),
                    status: status.map(MediaRequestStatusValue::into_domain),
                },
            )
            .await
            .map_err(to_gql_error)?;
        Ok(requests
            .into_iter()
            .map(|request| from_media_request(&app, request))
            .collect())
    }

    /// Read settings for a library by ID, subject to the caller's library-settings permission.
    async fn library_settings(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Library ID whose settings are returned.")] library_id: ID,
    ) -> GqlResult<LibrarySettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let library_id = String::from(library_id);
        let settings = app
            .get_library_settings(&actor, &library_id)
            .await
            .map_err(to_gql_error)?;
        Ok(from_library_settings(settings))
    }

    /// Find visible titles by external IDs from a named metadata source.
    async fn titles_by_external_ids(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Metadata-provider namespace used to interpret the external IDs.")]
        source: String,
        #[graphql(desc = "External ID values to match; an empty list returns no titles.")]
        values: Vec<String>,
    ) -> GqlResult<Vec<TitlePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let titles = app
            .list_titles_by_external_ids(&actor, &source, &values)
            .await
            .map_err(to_gql_error)?;

        Ok(titles
            .into_iter()
            .map(|title| from_title(&app, title))
            .collect())
    }

    /// Fetch a visible title by ID, returning null when it does not exist or is not accessible.
    async fn title(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Title ID to fetch.")] id: ID,
    ) -> GqlResult<Option<TitlePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let selection = TitlePayloadSelection::from_ctx(ctx);
        let title = if selection.include_external_ids {
            app.get_title(&actor, id.as_ref()).await
        } else {
            app.get_title_without_external_ids(&actor, id.as_ref())
                .await
        }
        .map_err(to_gql_error)?;
        Ok(title.map(|title| from_title(&app, title)))
    }

    /// Fetch locally cached movie metadata through a visible title relationship.
    async fn movie_entity(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Title used to authorize access to the movie.")] title_id: ID,
        #[graphql(desc = "Movie entity ID to fetch.")] id: ID,
    ) -> GqlResult<Option<MovieEntityPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.get_movie_entity(&actor, title_id.as_ref(), id.as_ref())
            .await
            .map(|movie| {
                movie.map(|movie| mappers::from_movie_entity(&app, title_id.to_string(), movie))
            })
            .map_err(to_gql_error)
    }

    /// Fetch an episode by ID while verifying that it belongs to the supplied parent title.
    async fn episode(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Parent title ID used to verify the episode belongs to that title.")]
        title_id: ID,
        #[graphql(desc = "Episode ID to fetch and verify against title_id.")] episode_id: ID,
    ) -> GqlResult<Option<EpisodePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let episode = app
            .get_episode(&actor, episode_id.as_ref())
            .await
            .map_err(to_gql_error)?;
        Ok(episode
            .filter(|episode| episode.title_id == title_id.as_ref())
            .map(|episode| from_episode(&app, episode)))
    }

    /// Fetch an episode by globally unique ID without requiring a parent title ID.
    async fn episode_by_id(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Episode ID to fetch.")] id: ID,
    ) -> GqlResult<Option<EpisodePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let episode = app
            .get_episode(&actor, id.as_ref())
            .await
            .map_err(to_gql_error)?;
        Ok(episode.map(|episode| from_episode(&app, episode)))
    }

    /// Fetch a collection by globally unique ID, returning null when it is absent or inaccessible.
    async fn collection_by_id(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Collection ID to fetch.")] id: ID,
    ) -> GqlResult<Option<CollectionPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let collection = app
            .get_collection(&actor, id.as_ref())
            .await
            .map_err(to_gql_error)?;
        Ok(collection.map(from_collection))
    }

    /// Fetch a visible title by facet, optional library slug, and title slug.
    async fn title_by_slug(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Media facet used to scope the slug lookup.")] facet: MediaFacetValue,
        #[graphql(desc = "Optional library slug used to disambiguate the title.")]
        library_slug: Option<String>,
        #[graphql(desc = "Title slug to look up within the facet and optional library.")]
        slug: String,
    ) -> GqlResult<Option<TitlePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let Some(title) = app
            .get_title_by_slug(&actor, facet.into_domain(), None, library_slug, &slug)
            .await
            .map_err(to_gql_error)?
        else {
            return Ok(None);
        };
        Ok(Some(from_title(&app, title)))
    }

    /// Preview the rename plan for a title or an entire facet without changing files.
    async fn media_rename_preview(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Title or facet whose rename plan is returned; no files are changed.")]
        input: MediaRenamePreviewInput,
    ) -> GqlResult<MediaRenamePlanPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let _ = input.dry_run;
        let facet = input.facet.into_domain();
        let mut plan = if let Some(title_id) = input.title_id {
            let title_id = title_id.to_string();
            app.preview_rename_for_title(&actor, &title_id, facet)
                .await
                .map_err(to_gql_error)?
        } else {
            app.preview_rename_for_facet(&actor, facet)
                .await
                .map_err(to_gql_error)?
        };

        let mut budget = input.max_items;
        scope_rename_plan_items(
            &mut plan,
            input.renamable_only.unwrap_or(false),
            &mut budget,
        );

        Ok(from_media_rename_plan(plan))
    }

    /// Preview rename plans for several titles of one facet without changing files.
    async fn media_rename_preview_bulk(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Titles whose rename plans are returned, in the order supplied; no files are changed."
        )]
        input: MediaRenamePreviewBulkInput,
    ) -> GqlResult<Vec<MediaRenamePlanPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let facet = input.facet.into_domain();
        let title_ids = input
            .title_ids
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>();
        if title_ids.is_empty() {
            return Ok(Vec::new());
        }

        let plans = app
            .preview_rename_for_titles(&actor, &title_ids, facet)
            .await
            .map_err(to_gql_error)?;

        // One sampling budget spans the whole batch, so a caller asking for 50
        // items gets 50 across every plan rather than 50 per title.
        let renamable_only = input.renamable_only.unwrap_or(false);
        let mut budget = input.max_items;
        Ok(plans
            .into_iter()
            .map(|mut plan| {
                scope_rename_plan_items(&mut plan, renamable_only, &mut budget);
                from_media_rename_plan(plan)
            })
            .collect())
    }

    /// Preview correcting which existing folder a title owns; no files are moved.
    async fn change_title_folder_preview(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Title and candidate folder whose ownership change is described; nothing is changed."
        )]
        input: ChangeTitleFolderPreviewInput,
    ) -> GqlResult<ChangeTitleFolderPreviewPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let preview = app
            .change_title_folder_preview(&actor, input.title_id.as_str(), &input.folder_path)
            .await
            .map_err(to_gql_error)?;
        Ok(from_change_title_folder_preview(preview))
    }

    /// Preview moving selected titles to another library or root; nothing is moved.
    ///
    /// The returned fingerprint is what `startLocationOperation` confirms. A
    /// changed filesystem, catalog, selection, destination, or mode produces a
    /// different fingerprint and voids the confirmation.
    ///
    /// The mode picks which preview runs: the managed move plans the copy, and
    /// `FILES_ALREADY_THERE` instead accounts for what is already at the
    /// destination (FR-050 to FR-053). The reported mode can still come back as
    /// `CATALOG_ONLY` when the selection has no files on disk (FR-076).
    async fn location_operation_preview(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Titles to move, the destination library or root they would move to, and how the files get there."
        )]
        input: LocationOperationPreviewInput,
    ) -> GqlResult<LocationOperationPreviewPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let mode = location_execution_mode_into_application(input.mode);
        let request = scryer_application::location::operations::RootMovePreviewRequest {
            title_ids: input
                .title_ids
                .into_iter()
                .map(|id| id.to_string())
                .collect(),
            destination: location_destination_into_application(input.destination),
        };
        let preview = match mode {
            scryer_application::location::model::LocationExecutionMode::FilesAlreadyThere => {
                app.preview_adoption(&actor, request).await
            }
            _ => app.preview_root_move(&actor, request).await,
        }
        .map_err(to_gql_error)?;
        Ok(from_root_move_preview(&preview))
    }

    /// Preview a **Change root** (US4 + US5): moving one root's content to a
    /// new path, or folding it into another root of the same library.
    ///
    /// FR-020 is one settings action with two destinations, so it is one query
    /// with two destinations. Name `destinationPath` for a new location, or
    /// `destinationRootId` for a root that already exists; a `destinationPath`
    /// that resolves to a configured root of this library is the same request
    /// as naming that root, and is planned as a fold either way. Naming both,
    /// or neither, is refused.
    ///
    /// Root scoped, so there is no selection: every title assigned to the root
    /// goes, and the ledger says so rather than offering a way to exclude one
    /// (FR-023). The returned fingerprint is what `startLocationOperation`
    /// confirms with its `rootScope` target.
    async fn location_root_scope_preview(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "The root being moved, the destination it moves to, and how the files get there."
        )]
        input: LocationRootScopePreviewInput,
    ) -> GqlResult<LocationRootScopePreviewPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let call = scryer_application::location::root_scope_execution::RootScopeCall {
            library_id: input.library_id.to_string(),
            root_id: input.root_id.to_string(),
            destination: root_scope_destination(input.destination_path, input.destination_root_id)
                .map_err(to_gql_error)?,
            // The planner refuses an unsupported mode itself, by name
            // (`root_change_mode_not_supported`), so the request travels
            // through the shared mapper and the refusal is a routable code
            // rather than an interface sentence the client cannot translate.
            mode: location_execution_mode_into_application(input.mode),
        };
        let preview = app
            .preview_root_scope(&actor, &call)
            .await
            .map_err(to_gql_error)?;
        Ok(from_root_scope_preview(&preview))
    }

    /// Read one location operation with its per-title checkpoints.
    async fn location_operation(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Location-operation identity.")] id: ID,
    ) -> GqlResult<Option<LocationOperationPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let operation_id = id.to_string();
        let Some(operation) = app
            .location_operation(&operation_id)
            .await
            .map_err(to_gql_error)?
        else {
            return Ok(None);
        };
        // The same source-and-destination gate the operation was started under
        // (FR-083); reading an operation is reading both libraries' state.
        app.require_location_operation_permission(&actor, &operation)
            .await
            .map_err(to_gql_error)?;
        let checkpoints = app
            .location_operation_checkpoints(&operation_id)
            .await
            .map_err(to_gql_error)?;
        Ok(Some(from_location_operation(&operation, &checkpoints)))
    }

    /// Which files one location operation renames and deduplicates, per title.
    ///
    /// Activity's counters say how many; this says which ones, and whether each
    /// has happened yet or is still only what the confirmed plan intends
    /// (FR-091). It is a separate read from `locationOperation` because the
    /// per-file identities live in the operation's stored plan, which a
    /// progress poll has no reason to load.
    async fn location_operation_assets(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Location-operation identity.")] id: ID,
    ) -> GqlResult<Option<LocationOperationAssetListingPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let operation_id = id.to_string();
        let Some(operation) = app
            .location_operation(&operation_id)
            .await
            .map_err(to_gql_error)?
        else {
            return Ok(None);
        };
        // The same source-and-destination gate reading the operation itself
        // applies (FR-083); this reads the same two libraries' paths.
        app.require_location_operation_permission(&actor, &operation)
            .await
            .map_err(to_gql_error)?;
        let listing = app
            .location_operation_asset_listing(&operation_id)
            .await
            .map_err(to_gql_error)?;
        Ok(Some(from_location_operation_asset_listing(&listing)))
    }

    /// Preview deleting all media files for one title without changing files.
    async fn delete_title_preview(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Title ID whose associated media files would be deleted.")] title_id: ID,
    ) -> GqlResult<DeletePreviewPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let title_id = String::from(title_id);
        let preview = app
            .preview_delete_title_files(&actor, &title_id)
            .await
            .map_err(to_gql_error)?;
        Ok(from_delete_preview(preview))
    }

    /// Preview deleting media files for the supplied title IDs without changing files.
    async fn delete_titles_preview(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Title IDs whose associated media files would be deleted; no files are changed."
        )]
        input: DeleteTitlesPreviewInput,
    ) -> GqlResult<DeleteTitlesPreviewPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let title_ids = input
            .title_ids
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>();
        let preview = app
            .preview_delete_titles_files(&actor, &title_ids)
            .await
            .map_err(to_gql_error)?;
        Ok(from_delete_titles_preview(preview))
    }

    /// Preview deleting the media files of the supplied episodes without changing files.
    async fn delete_episode_files_preview(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Title and episode IDs whose linked media files would be deleted; no files are changed."
        )]
        input: DeleteEpisodeFilesPreviewInput,
    ) -> GqlResult<DeleteEpisodeFilesPreviewPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let title_id = String::from(input.title_id);
        let episode_ids = input
            .episode_ids
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>();
        let preview = app
            .preview_delete_episode_files(&actor, &title_id, &episode_ids)
            .await
            .map_err(to_gql_error)?;
        Ok(from_delete_episode_files_preview(preview))
    }

    /// Preview deleting one media file without changing files.
    async fn delete_media_file_preview(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Media-file ID whose deletion would be previewed.")] file_id: ID,
    ) -> GqlResult<DeletePreviewPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let file_id = String::from(file_id);
        let preview = app
            .preview_delete_media_file(&actor, &file_id)
            .await
            .map_err(to_gql_error)?;
        Ok(from_delete_preview(preview))
    }

    /// Preview deleting one external subtitle file without changing files.
    async fn delete_external_subtitle_preview(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "External-subtitle ID whose deletion would be previewed.")]
        external_subtitle_id: ID,
    ) -> GqlResult<DeletePreviewPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let external_subtitle_id = String::from(external_subtitle_id);
        let preview = app
            .preview_delete_external_subtitle_file(&actor, &external_subtitle_id)
            .await
            .map_err(to_gql_error)?;
        Ok(from_delete_preview(preview))
    }

    /// Fetch one wanted item by ID, returning null when it is absent or inaccessible.
    async fn wanted_item(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Wanted-item ID to fetch.")] id: ID,
    ) -> GqlResult<Option<WantedItemPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let item = app
            .get_wanted_item(&actor, id.as_ref())
            .await
            .map_err(to_gql_error)?
            .map(from_wanted_item)
            .transpose()
            .map_err(to_gql_error)?;
        Ok(item)
    }

    /// Search indexers for a title, series movie link, or episode and return at most 200 results.
    async fn search_releases(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Search target and optional season, episode, and result limit; season and episode must be supplied together."
        )]
        input: SearchReleasesInput,
    ) -> GqlResult<Vec<IndexerSearchResultPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;

        let SearchReleasesInput {
            title_id,
            series_movie_link_id,
            season,
            episode,
            limit,
            ..
        } = input;

        // One-shot search is title-anchored only; a query subject is a job
        // (`startInteractiveReleaseSearch`), never a blocking resolver.
        let Some(title_id) = title_id else {
            return Err(to_gql_error(AppError::Validation(
                "searchReleases requires a title id".to_string(),
            )));
        };

        let safe_limit = limit.unwrap_or(50).clamp(1, 200) as usize;
        let title_id = title_id.to_string();
        let cancel_token = tokio_util::sync::CancellationToken::new();
        struct CancelOnDrop(tokio_util::sync::CancellationToken);
        impl Drop for CancelOnDrop {
            fn drop(&mut self) {
                self.0.cancel();
            }
        }
        let _cancel_on_drop = CancelOnDrop(cancel_token.clone());
        let results = match (series_movie_link_id, season, episode) {
            (Some(series_movie_link_id), None, None) => app
                .search_indexers_for_series_movie(
                    &actor,
                    title_id,
                    series_movie_link_id.to_string(),
                    cancel_token.clone(),
                )
                .await
                .map_err(to_gql_error)?,
            (None, Some(season), Some(episode)) => app
                .search_indexers_for_episode(
                    &actor,
                    title_id,
                    season,
                    episode,
                    cancel_token.clone(),
                )
                .await
                .map_err(to_gql_error)?,
            (None, None, None) => app
                .search_indexers_for_title(&actor, title_id, cancel_token.clone())
                .await
                .map_err(to_gql_error)?,
            (None, Some(_), None) | (None, None, Some(_)) => {
                return Err(to_gql_error(AppError::Validation(
                    "episode searches require both season and episode".to_string(),
                )));
            }
            (Some(_), Some(_), _) | (Some(_), _, Some(_)) => {
                return Err(to_gql_error(AppError::Validation(
                    "series movie searches cannot include season or episode".to_string(),
                )));
            }
        };

        Ok(results
            .into_iter()
            .take(safe_limit)
            .map(crate::mappers::from_search_result)
            .collect())
    }

    /// Poll an interactive release-search job; null means no visible snapshot exists.
    async fn interactive_release_search(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Interactive release-search job ID to poll.")] id: ID,
    ) -> GqlResult<Option<InteractiveReleaseSearchPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let snapshot = app
            .interactive_release_search(&actor, id.as_ref())
            .await
            .map_err(to_gql_error)?;
        Ok(snapshot.map(from_interactive_release_search_snapshot))
    }

    /// List title history events with optional identity, event, grouping, and offset filters.
    async fn title_history(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Event, title, library, text, grouping, and pagination filters."
        )]
        filter: TitleHistoryFilterInput,
    ) -> GqlResult<TitleHistoryPagePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;

        let parsed_types = parse_supported_title_history_event_types(filter.event_types)?;

        let f = TitleHistoryFilter {
            event_types: parsed_types,
            title_ids: filter
                .title_ids
                .map(|ids| ids.into_iter().map(String::from).collect::<Vec<String>>()),
            library_ids: filter
                .library_ids
                .map(|ids| ids.into_iter().map(String::from).collect::<Vec<String>>()),
            title_search: filter.title_search,
            download_id: filter.download_id,
            episode_id: filter.episode_id.map(String::from),
            group_by_event: filter.group_by_event.unwrap_or(false),
            limit: filter.limit.unwrap_or(50).max(1) as usize,
            offset: filter.offset.unwrap_or(0).max(0) as usize,
        };

        let offset = f.offset;
        let page = app
            .list_title_history(&actor, &f)
            .await
            .map_err(to_gql_error)?;
        Ok(from_title_history_page(page, offset).map_err(to_gql_error)?)
    }

    /// List blocklisted releases for one title; the limit defaults to 100 and values below 1 become 1.
    async fn title_release_blocklist(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Title ID whose blocked releases are returned.")] title_id: ID,
        #[graphql(
            desc = "Maximum entries to return; defaults to 100 and values below 1 become 1."
        )]
        limit: Option<i32>,
    ) -> GqlResult<Vec<TitleReleaseBlocklistEntryPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let items = app
            .list_title_release_blocklist(
                &actor,
                title_id.as_ref(),
                limit.unwrap_or(100).max(1) as usize,
            )
            .await
            .map_err(to_gql_error)?;
        Ok(items
            .into_iter()
            .map(from_title_release_blocklist_entry)
            .collect())
    }
}

#[allow(clippy::too_many_arguments)]
#[Object]
impl ActivityQueries {
    /// Return grab, upgrade, import, and failure counts for a trailing window and the window before it.
    async fn dashboard_activity_stats(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            default = 24,
            desc = "Length of each window in hours; defaults to 24 and is clamped to 1 through 168."
        )]
        window_hours: i64,
    ) -> GqlResult<DashboardActivityStatsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let stats = app
            .dashboard_activity_stats(&actor, window_hours)
            .await
            .map_err(to_gql_error)?;
        Ok(from_dashboard_activity_stats(stats))
    }

    /// List recent activity events with zero-based offset pagination.
    async fn activity_events(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Maximum events to return; defaults to 100.")] limit: Option<i32>,
        #[graphql(desc = "Number of events to skip; defaults to 0.")] offset: Option<i32>,
    ) -> GqlResult<Vec<ActivityEventPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let events = app
            .recent_activity(
                &actor,
                limit.unwrap_or(100) as i64,
                offset.unwrap_or(0) as i64,
            )
            .await
            .map_err(to_gql_error)?;
        Ok(events.into_iter().map(from_activity_event).collect())
    }

    /// List audit events visible to the caller, filtered by event type, title, facet, sequence, and limit.
    async fn audit_log(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Optional event types to include; omitted includes all visible types.")]
        event_types: Option<Vec<DomainEventTypeValue>>,
        #[graphql(desc = "Include only events for this title ID.")] title_id: Option<ID>,
        #[graphql(desc = "Include only events for this media facet.")] facet: Option<
            MediaFacetValue,
        >,
        #[graphql(desc = "Return events after this exclusive sequence number.")]
        after_sequence: Option<Long>,
        #[graphql(desc = "Return events before this exclusive sequence number.")]
        before_sequence: Option<Long>,
        #[graphql(desc = "Maximum events to return; defaults to 100 and values below 1 become 1.")]
        limit: Option<i32>,
    ) -> GqlResult<Vec<DomainEventEnvelopePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let filter = scryer_domain::DomainEventFilter {
            event_types: event_types.map(|types| {
                types
                    .into_iter()
                    .map(DomainEventTypeValue::into_domain)
                    .collect()
            }),
            title_id: title_id.map(String::from),
            facet: facet.map(MediaFacetValue::into_domain),
            after_sequence: after_sequence.map(|value| value.0),
            before_sequence: before_sequence.map(|value| value.0),
            limit: limit.unwrap_or(100).max(1) as usize,
        };
        let events = app.audit_log(&actor, &filter).await.map_err(to_gql_error)?;
        Ok(events.into_iter().map(from_domain_event).collect())
    }

    /// List active library scan sessions visible to the caller.
    async fn active_library_scans(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Vec<LibraryScanProgressPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let sessions = app
            .active_library_scans(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(sessions
            .into_iter()
            .map(from_library_scan_session)
            .collect())
    }

    /// Poll deprecated external movie or series source warmup status for one session.
    #[graphql(deprecation = "use externalImportWarmupStatus")]
    async fn external_import_arr_source_warmup_status(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "External movie or series source warmup session ID to poll.")] session_id: ID,
    ) -> GqlResult<ExternalImportMonitorWarmupProgressPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.maintain_external_import_arr_source_sessions(&actor)
            .await
            .map_err(to_gql_error)?;
        let session_id = String::from(session_id);
        let snapshot = app
            .get_external_import_monitor_warmup_status(&actor, &session_id)
            .await
            .map_err(to_gql_error)?;
        Ok(from_external_import_monitor_warmup_progress(snapshot))
    }

    /// Poll warmup status for one external movie, series, or indexer discovery session.
    async fn external_import_warmup_status(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Warmup session ID to poll.")] session_id: ID,
    ) -> GqlResult<ExternalImportMonitorWarmupProgressPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.maintain_external_import_arr_source_sessions(&actor)
            .await
            .map_err(to_gql_error)?;
        let session_id = String::from(session_id);
        let snapshot = app
            .get_external_import_monitor_warmup_status(&actor, &session_id)
            .await
            .map_err(to_gql_error)?;
        Ok(from_external_import_monitor_warmup_progress(snapshot))
    }

    /// Aggregate Arr-source warmup progress; an empty list returns completed status with zero totals.
    async fn external_import_aggregate_warmup_progress(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "External movie or series source warmup session IDs to aggregate; an empty list returns COMPLETED with zero totals."
        )]
        input: ExternalImportAggregateWarmupProgressInput,
    ) -> GqlResult<ExternalImportAggregateWarmupProgressPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.maintain_external_import_arr_source_sessions(&actor)
            .await
            .map_err(to_gql_error)?;
        if input.source_warmup_session_ids.is_empty() {
            return Ok(ExternalImportAggregateWarmupProgressPayload {
                status: ExternalImportMonitorWarmupStatusValue::Completed,
                titles_total_known: true,
                titles_fetched: 0,
                titles_total: 0,
                error_message: None,
            });
        }

        let mut status = ExternalImportMonitorWarmupStatusValue::Completed;
        let mut titles_total_known = true;
        let mut titles_fetched = 0i32;
        let mut titles_total = 0i32;
        let mut error_message = None;

        for session_id in input.source_warmup_session_ids {
            let session_id_string = session_id.to_string();
            let snapshot = app
                .get_external_import_monitor_warmup_status(&actor, &session_id_string)
                .await
                .map_err(to_gql_error)?;
            let source = app
                .external_import_arr_source_warmup_result(&actor, &session_id_string)
                .await
                .map_err(to_gql_error)?;
            let (known, fetched, total) = match source.kind {
                AppArrSourceKind::Radarr => (
                    snapshot.movies_total_known,
                    snapshot.movies_progress.completed,
                    snapshot.movies_progress.total,
                ),
                AppArrSourceKind::Sonarr => (
                    snapshot.series_total_known,
                    snapshot.series_progress.completed,
                    snapshot.series_progress.total,
                ),
            };
            titles_total_known &= known;
            titles_fetched = titles_fetched.saturating_add(fetched);
            titles_total = titles_total.saturating_add(total);

            match snapshot.status {
                ExternalImportMonitorWarmupStatus::Failed => {
                    status = ExternalImportMonitorWarmupStatusValue::Failed;
                    error_message = snapshot.error_message;
                }
                ExternalImportMonitorWarmupStatus::Canceled
                    if status != ExternalImportMonitorWarmupStatusValue::Failed =>
                {
                    status = ExternalImportMonitorWarmupStatusValue::Canceled;
                    error_message = snapshot.error_message;
                }
                ExternalImportMonitorWarmupStatus::Queued
                | ExternalImportMonitorWarmupStatus::Running
                    if matches!(status, ExternalImportMonitorWarmupStatusValue::Completed) =>
                {
                    status = ExternalImportMonitorWarmupStatusValue::Running;
                }
                _ => {}
            }
        }

        Ok(ExternalImportAggregateWarmupProgressPayload {
            status,
            titles_total_known,
            titles_fetched,
            titles_total,
            error_message,
        })
    }

    /// Return pending-import counts visible to the caller.
    async fn pending_import_counts(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<PendingImportCountsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let counts = app
            .pending_import_counts(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(from_pending_import_counts(counts))
    }

    /// Return navigation counts while omitting categories the caller is not authorized to view.
    async fn navigation_badge_counts(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<NavigationBadgeCountsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;

        let can_resolve_imports =
            actor_has_any_library_permission(ctx, LibraryPermission::ResolveImports).await?;
        let can_manage_titles =
            actor_has_any_library_permission(ctx, LibraryPermission::ManageTitles).await?;
        let can_manage_system_settings =
            actor_has_app_permission(ctx, AppPermission::ManageSystemSettings).await?;

        let pending_import_counts = async {
            if can_resolve_imports {
                app.pending_import_counts(&actor).await
            } else {
                Ok(PendingImportCounts::default())
            }
        };
        let pending_media_request_counts = async {
            if can_manage_titles {
                app.pending_media_request_counts(&actor).await
            } else {
                Ok(MediaRequestCounts::default())
            }
        };
        let activity_import_count = async {
            if can_resolve_imports {
                app.count_download_import_items(&actor, DownloadImportFilter::Attention)
                    .await
            } else {
                Ok(0)
            }
        };
        let plugin_update_count = async {
            if can_manage_system_settings {
                app.plugin_update_count(&actor).await
            } else {
                Ok(0)
            }
        };

        let (
            pending_import_counts,
            pending_media_request_counts,
            activity_import_count,
            plugin_update_count,
        ) = tokio::try_join!(
            pending_import_counts,
            pending_media_request_counts,
            activity_import_count,
            plugin_update_count,
        )
        .map_err(to_gql_error)?;

        Ok(NavigationBadgeCountsPayload {
            pending_import_counts: from_pending_import_counts(pending_import_counts),
            pending_media_request_counts: from_media_request_counts(pending_media_request_counts),
            activity_import_count: activity_import_count as i32,
            plugin_update_count: plugin_update_count as i32,
        })
    }

    /// List pending imports by facet, library scope, status, and offset pagination.
    async fn pending_imports(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Media facet whose pending imports are returned.")] facet: MediaFacetValue,
        #[graphql(
            desc = "Library IDs to restrict the result; omitted or empty includes all permitted libraries."
        )]
        library_ids: Option<Vec<ID>>,
        #[graphql(desc = "Pending-import status to include.")] status: PendingImportStatusValue,
        #[graphql(
            default = 50,
            desc = "Maximum items to return; defaults to 50 and is clamped to 0 through 500."
        )]
        limit: i64,
        #[graphql(default = 0, desc = "Number of matching items to skip; defaults to 0.")]
        offset: i64,
    ) -> GqlResult<PendingImportConnectionPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let connection = app
            .pending_imports(
                &actor,
                facet.into_domain(),
                optional_ids_to_strings(library_ids),
                status.into_application(),
                limit,
                offset,
            )
            .await
            .map_err(to_gql_error)?;
        Ok(from_pending_import_connection(connection, offset).map_err(to_gql_error)?)
    }

    /// Search metadata candidates for one pending import.
    async fn pending_import_title_search(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Pending-import ID whose title candidates are searched.")]
        pending_import_id: ID,
        #[graphql(desc = "Text used to search metadata candidates.")] query: String,
        #[graphql(default = 8, desc = "Maximum candidates to return; defaults to 8.")] limit: i32,
        #[graphql(
            default_with = "\"eng\".to_string()",
            desc = "Metadata language code; defaults to \"eng\"."
        )]
        language: String,
        #[graphql(desc = "Release year filter, when supplied.")] year: Option<i32>,
    ) -> GqlResult<Vec<MetadataSearchItemPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let pending_import_id = String::from(pending_import_id);
        let results = app
            .pending_import_title_search(&actor, &pending_import_id, &query, limit, &language, year)
            .await
            .map_err(to_gql_error)?;
        Ok(results
            .into_iter()
            .map(|item| from_metadata_search_item(&app, item))
            .collect())
    }

    /// Preview the title binding for one pending import without applying it.
    async fn pending_import_binding_preview(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Pending-import ID whose title binding is previewed.")] pending_import_id: ID,
    ) -> GqlResult<PendingImportBindingPreviewPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let pending_import_id = String::from(pending_import_id);
        let preview = app
            .preview_title_bound_pending_import(&actor, &pending_import_id)
            .await
            .map_err(to_gql_error)?;
        Ok(PendingImportBindingPreviewPayload {
            title: from_title(&app, preview.title),
            file: PendingImportBindingFilePreviewPayload {
                file_path: preview.file.file_path,
                file_name: preview.file.file_name,
                size_bytes: Long::from(preview.file.size_bytes),
                parsed_season: preview.file.parsed_season.map(|value| value as i32),
                parsed_episodes: preview
                    .file
                    .parsed_episodes
                    .into_iter()
                    .map(|value| value as i32)
                    .collect(),
                parsed_absolute_numbers: preview
                    .file
                    .parsed_absolute_numbers
                    .into_iter()
                    .map(|value| value as i32)
                    .collect(),
                suggested_episode_ids: preview
                    .file
                    .suggested_episode_ids
                    .into_iter()
                    .map(Into::into)
                    .collect(),
            },
            available_episodes: preview
                .available_episodes
                .into_iter()
                .map(|episode| from_episode(&app, episode))
                .collect(),
        })
    }
}

#[allow(clippy::too_many_arguments)]
#[Object]
impl JobAndDownloadQueries {
    /// List job definitions visible to the caller.
    async fn jobs(&self, ctx: &Context<'_>) -> GqlResult<Vec<JobDefinitionPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let jobs = app.list_jobs(&actor).await.map_err(to_gql_error)?;
        Ok(jobs.into_iter().map(from_job_definition).collect())
    }

    /// List currently active job runs visible to the caller.
    async fn active_job_runs(&self, ctx: &Context<'_>) -> GqlResult<Vec<JobRunPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let runs = app.active_job_runs(&actor).await.map_err(to_gql_error)?;
        Ok(runs.into_iter().map(from_job_run).collect())
    }

    /// List recent runs for one job key; the limit defaults to 10 and values below 1 become 1.
    async fn job_runs(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Job key whose runs should be listed.")] job_key: JobKeyValue,
        #[graphql(desc = "Maximum runs to return; defaults to 10 and values below 1 become 1.")]
        limit: Option<i32>,
    ) -> GqlResult<Vec<JobRunPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let runs = app
            .list_job_runs(
                &actor,
                job_key.into_application(),
                limit.unwrap_or(10).max(1) as usize,
            )
            .await
            .map_err(to_gql_error)?;
        Ok(runs.into_iter().map(from_job_run).collect())
    }

    /// List recent job runs; the limit defaults to 50 and values below 1 become 1.
    async fn recent_job_runs(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Maximum runs to return; defaults to 50 and values below 1 become 1.")]
        limit: Option<i32>,
    ) -> GqlResult<Vec<JobRunPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let runs = app
            .list_recent_job_runs(&actor, limit.unwrap_or(50).max(1) as usize)
            .await
            .map_err(to_gql_error)?;
        Ok(runs.into_iter().map(from_job_run).collect())
    }

    /// Return discovery home results using optional facet, filter, and pagination input.
    async fn discovery_home(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Facet, filters, and pagination; omitted uses service defaults.")]
        input: Option<DiscoveryHomeInput>,
    ) -> GqlResult<DiscoveryHomePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let query = discovery_home_query_from_input(input).map_err(to_gql_error)?;
        let result = app
            .discovery_home(&actor, query)
            .await
            .map_err(to_gql_error)?;
        Ok(from_discovery_home(&app, result))
    }

    /// Return discovery home cards using optional facet, filter, and pagination input.
    async fn discovery_home_cards(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Facet, filters, and pagination; omitted uses service defaults.")]
        input: Option<DiscoveryHomeInput>,
    ) -> GqlResult<DiscoveryHomeCardsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let query = discovery_home_query_from_input(input).map_err(to_gql_error)?;
        let result = app
            .discovery_home_cards(&actor, query)
            .await
            .map_err(to_gql_error)?;
        from_discovery_home_cards(&app, result).map_err(to_gql_error)
    }

    /// Return discovery home filter options for an optional facet and library scope.
    async fn discovery_home_filter_options(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Facet and library scope for filter options; omitted uses service defaults."
        )]
        input: Option<DiscoveryHomeFilterOptionsInput>,
    ) -> GqlResult<DiscoveryHomeFilterOptionsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let options = app
            .discovery_home_filter_options(
                &actor,
                discovery_home_filter_options_query_from_input(input),
            )
            .await
            .map_err(to_gql_error)?;
        Ok(from_discovery_home_filter_options(options))
    }

    /// List discovery items using optional filters, sorting, and pagination.
    async fn discovery_items(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Discovery filters, sorting, and pagination; omitted uses service defaults."
        )]
        input: Option<DiscoveryItemsInput>,
    ) -> GqlResult<DiscoveryItemsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let result = app
            .discovery_items(&actor, discovery_items_query_from_input(input))
            .await
            .map_err(to_gql_error)?;
        Ok(from_discovery_items_result(&app, result))
    }

    /// Fetch one discovery item by identity and scope, returning null when unavailable.
    async fn discovery_item_detail(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Discovery item identity and scope used for the lookup.")]
        input: DiscoveryItemDetailInput,
    ) -> GqlResult<Option<DiscoveryItemPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let item = app
            .discovery_item_detail(&actor, discovery_item_detail_query_from_input(input))
            .await
            .map_err(to_gql_error)?;
        Ok(item.map(|item| from_discovery_item(&app, item)))
    }

    /// Return catalog discovery results for the requested target and filters.
    async fn catalog_discovery(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Catalog target, filters, sorting, and pagination.")]
        input: CatalogDiscoveryInput,
    ) -> GqlResult<CatalogDiscoveryPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let result = app
            .catalog_discovery(&actor, catalog_discovery_query_from_input(input))
            .await
            .map_err(to_gql_error)?;
        Ok(from_catalog_discovery(&app, result))
    }

    /// Return a bounded download queue page with readiness, revision, staleness, filters, and stable sorting metadata.
    async fn download_queue_page(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            default = 50,
            desc = "Number of queue items to return; defaults to 50 and is clamped to 1 through 200."
        )]
        limit: i32,
        #[graphql(
            default = 0,
            desc = "Number of matching queue items to skip; defaults to 0 and negative values become 0."
        )]
        offset: i32,
        #[graphql(
            desc = "Activity states to include; omitted includes all states and an empty list returns no rows."
        )]
        filters: Option<Vec<DownloadActivityFilterValue>>,
        #[graphql(
            desc = "Download-client IDs to include; omitted includes all clients and an empty list returns no rows."
        )]
        client_ids: Option<Vec<ID>>,
        #[graphql(
            default = true,
            desc = "When true, include only downloads submitted by Scryer; defaults to true."
        )]
        scryer_submitted_only: bool,
        #[graphql(desc = "Include only queue items for this title ID.")] title_id: Option<ID>,
        #[graphql(
            default_with = "DownloadQueueSortKeyValue::Status",
            desc = "Queue sort key; defaults to Status."
        )]
        sort_key: DownloadQueueSortKeyValue,
        #[graphql(
            default_with = "SortDirectionValue::Asc",
            desc = "Queue sort direction; defaults to Asc."
        )]
        sort_direction: SortDirectionValue,
    ) -> GqlResult<DownloadQueuePagePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let page = app
            .list_download_queue_page(
                &actor,
                limit.clamp(1, 200) as usize,
                offset.max(0) as usize,
                filters.map(|filters| {
                    filters
                        .into_iter()
                        .map(DownloadActivityFilterValue::into_application)
                        .collect()
                }),
                client_ids.map(|ids| ids.into_iter().map(String::from).collect()),
                scryer_submitted_only,
                title_id.as_ref().map(|id| id.as_ref()),
                scryer_application::DownloadHistorySort {
                    key: sort_key.into_application(),
                    direction: sort_direction.into_application(),
                },
            )
            .await
            .map_err(to_gql_error)?;
        Ok(from_download_queue_page(page))
    }

    /// Return queued and active filesystem import operations. Idle worker capacity is never included.
    async fn active_import_streams(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Vec<ActiveImportStreamPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let streams = app
            .list_active_import_streams(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(streams.into_iter().map(from_active_import_stream).collect())
    }

    /// Deprecated download activity listing without pagination or queue readiness metadata.
    #[graphql(deprecation = "use downloadQueuePage")]
    async fn download_queue(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Include all activity types instead of the default active subset; defaults to false."
        )]
        include_all_activity: Option<bool>,
        #[graphql(desc = "Include history-only activity; defaults to false.")]
        include_history_only: Option<bool>,
        #[graphql(desc = "Include import activity; defaults to false.")]
        include_import_activity: Option<bool>,
        #[graphql(desc = "Include only activity for this title ID.")] title_id: Option<ID>,
        #[graphql(desc = "Activity state filter; defaults to All.")] activity_filter: Option<
            DownloadActivityFilterValue,
        >,
    ) -> GqlResult<Vec<DownloadQueueItemPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let items = match title_id {
            Some(title_id) => {
                app.list_download_queue_for_title(
                    &actor,
                    title_id.as_ref(),
                    include_all_activity.unwrap_or(false),
                    include_history_only.unwrap_or(false),
                    include_import_activity.unwrap_or(false),
                    activity_filter
                        .unwrap_or(DownloadActivityFilterValue::All)
                        .into_application(),
                )
                .await
            }
            None => {
                app.list_download_queue(
                    &actor,
                    include_all_activity.unwrap_or(false),
                    include_history_only.unwrap_or(false),
                    include_import_activity.unwrap_or(false),
                    activity_filter
                        .unwrap_or(DownloadActivityFilterValue::All)
                        .into_application(),
                )
                .await
            }
        }
        .map_err(to_gql_error)?;
        Ok(items.into_iter().map(from_download_queue_item).collect())
    }

    /// List a bounded page of download-import activity; the limit defaults to 50 and is clamped to 1 through 100.
    async fn download_import(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Number of import records to return; defaults to 50 and is clamped to 1 through 100."
        )]
        limit: Option<i32>,
        #[graphql(
            desc = "Number of matching import records to skip; defaults to 0 and negative values become 0."
        )]
        offset: Option<i32>,
        #[graphql(desc = "Import activity filter; defaults to All.")] filter: Option<
            DownloadImportFilterValue,
        >,
    ) -> GqlResult<DownloadImportPagePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let limit = limit.unwrap_or(50).clamp(1, 100) as usize;
        let offset = offset.unwrap_or(0).max(0) as usize;
        let page = app
            .list_download_import_page(
                &actor,
                limit,
                offset,
                filter
                    .unwrap_or(DownloadImportFilterValue::All)
                    .into_application(),
            )
            .await
            .map_err(to_gql_error)?;
        Ok(from_download_import_page(page))
    }

    /// List a bounded page of download history with optional client, activity, and sort filters.
    async fn download_history(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Number of history records to return; defaults to 50 and is clamped to 1 through 50."
        )]
        limit: Option<i32>,
        #[graphql(
            desc = "Number of matching history records to skip; defaults to 0 and negative values become 0."
        )]
        offset: Option<i32>,
        #[graphql(
            desc = "History activity filters; omitted includes all states and an empty list returns no rows."
        )]
        filters: Option<Vec<DownloadHistoryFilterValue>>,
        #[graphql(
            desc = "Download-client IDs to include; omitted includes all clients and an empty list returns no rows."
        )]
        client_ids: Option<Vec<ID>>,
        #[graphql(
            desc = "When true, include only downloads submitted by Scryer; defaults to false."
        )]
        scryer_submitted_only: Option<bool>,
        #[graphql(desc = "History sort key; omitted uses the service default.")] sort_key: Option<
            DownloadHistorySortKeyValue,
        >,
        #[graphql(desc = "Sort direction when a sort key is supplied; defaults to Asc.")]
        sort_direction: Option<SortDirectionValue>,
    ) -> GqlResult<DownloadHistoryPagePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let limit = limit.unwrap_or(50).clamp(1, 50) as usize;
        let offset = offset.unwrap_or(0).max(0) as usize;
        let sort = sort_key.map(|key| scryer_application::DownloadHistorySort {
            key: key.into_application(),
            direction: sort_direction
                .unwrap_or(SortDirectionValue::Asc)
                .into_application(),
        });
        let page = app
            .list_download_history_page(
                &actor,
                limit,
                offset,
                filters.map(|filters| {
                    filters
                        .into_iter()
                        .map(DownloadHistoryFilterValue::into_application)
                        .collect()
                }),
                client_ids.map(|ids| ids.into_iter().map(String::from).collect()),
                scryer_submitted_only.unwrap_or(false),
                sort,
            )
            .await
            .map_err(to_gql_error)?;
        Ok(from_download_history_page(page))
    }
}

#[allow(clippy::too_many_arguments)]
#[Object]
impl SystemQueries {
    /// Return the path style supported by the running service.
    async fn runtime_info(&self, ctx: &Context<'_>) -> GqlResult<RuntimeInfoPayload> {
        let _actor = actor_from_ctx(ctx)?;
        Ok(RuntimeInfoPayload {
            runtime_path_style: from_runtime_path_style(RuntimePathStyle::current()),
        })
    }

    /// Return system health details visible to the authenticated caller.
    async fn system_health(&self, ctx: &Context<'_>) -> GqlResult<SystemHealthPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let health = app.system_health(&actor).await.map_err(to_gql_error)?;
        Ok(from_system_health(health))
    }

    /// Return the running Scryer version.
    async fn scryer_version(&self, ctx: &Context<'_>) -> GqlResult<String> {
        let _actor = actor_from_ctx(ctx)?;
        Ok(SCRYER_VERSION.to_string())
    }

    /// Instance-wide feature switches readable by any signed-in user.
    async fn instance_features(&self, ctx: &Context<'_>) -> GqlResult<InstanceFeaturesPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let features = app.instance_features(&actor).await.map_err(to_gql_error)?;
        Ok(InstanceFeaturesPayload {
            experimental_features_enabled: features.experimental_features_enabled,
            personalized_discovery_enabled: features.personalized_discovery_enabled,
        })
    }

    /// Return an SMG compatibility notice when the connected SMG version requires attention.
    async fn smg_version_compatibility_notice(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Option<SmgVersionCompatibilityNoticePayload>> {
        let app = app_from_ctx(ctx)?;
        let _actor = actor_from_ctx(ctx)?;
        let notice = app
            .smg_version_compatibility_notice()
            .await
            .map_err(to_gql_error)?;
        Ok(notice.map(from_smg_version_compatibility_notice))
    }

    /// Return an available Scryer update notice from SMG, if one exists.
    async fn smg_scryer_update_notice(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Option<SmgScryerUpdateNoticePayload>> {
        let app = app_from_ctx(ctx)?;
        let _actor = actor_from_ctx(ctx)?;
        let notice = app.smg_scryer_update_notice().await.map_err(to_gql_error)?;
        Ok(notice.map(from_smg_scryer_update_notice))
    }

    /// Return application update availability and installation eligibility; requires system-settings management permission.
    async fn application_upgrade_status(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<ApplicationUpgradeStatusPayload> {
        require_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let assessment = application_upgrade_assessment_from_ctx(ctx);
        let app = app_from_ctx(ctx)?;
        let update_notice = app.smg_scryer_update_notice().await.map_err(to_gql_error)?;
        let (active_run, latest_run) = app
            .application_upgrade_job_runs()
            .await
            .map_err(to_gql_error)?;

        Ok(from_application_upgrade_status(
            assessment,
            scryer_application::SCRYER_VERSION.to_string(),
            update_notice,
            active_run,
            latest_run,
        ))
    }

    /// List recycled media items in a bounded page, optionally restricted to library IDs.
    async fn recycled_items(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            default = 500,
            desc = "Number of recycled items to return; defaults to 500 and is clamped to 1 through 500."
        )]
        limit: i32,
        #[graphql(
            default = 0,
            desc = "Number of matching recycled items to skip; defaults to 0 and negative values become 0."
        )]
        offset: i32,
        #[graphql(
            desc = "Library IDs to include; omitted or empty includes all permitted libraries."
        )]
        library_ids: Option<Vec<ID>>,
    ) -> GqlResult<RecycledItemsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let library_ids = library_ids.map(|ids| ids.into_iter().map(|id| id.to_string()).collect());
        let all = app
            .list_recycled_items(&actor, library_ids)
            .await
            .map_err(to_gql_error)?;
        let total_count = all.len() as i32;
        let limit = limit.clamp(1, 500) as usize;
        let offset = offset.max(0) as usize;
        let items = all
            .into_iter()
            .skip(offset)
            .take(limit)
            .map(|item| {
                Ok(RecycledItemPayload {
                    id: ID::from(item.id),
                    original_path: item.original_path,
                    file_name: item.file_name,
                    size_bytes: Long::from_u64_saturating(item.size_bytes),
                    title_id: item.title_id.map(ID::from),
                    title_name: item.title_name,
                    reason: item.reason,
                    recycled_at: parse_required_datetime(
                        &item.recycled_at,
                        "recycled item recycled_at",
                    )
                    .map_err(to_gql_error)?,
                    scheduled_deletion_at: parse_required_datetime(
                        &item.scheduled_deletion_at,
                        "recycled item scheduled_deletion_at",
                    )
                    .map_err(to_gql_error)?,
                    media_root: item.media_root,
                    library_id: ID::from(item.library_id),
                    library_name: item.library_name,
                })
            })
            .collect::<GqlResult<Vec<_>>>()?;
        Ok(RecycledItemsPayload { items, total_count })
    }

    /// Preview restoring recycled items by ID without changing their files.
    async fn preview_restore_recycled_items(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Recycled-item IDs whose restore destinations are previewed; an empty list returns an empty preview."
        )]
        ids: Vec<ID>,
    ) -> GqlResult<RecycleRestorePreviewPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let preview = app
            .preview_restore_recycled_items(
                &actor,
                ids.iter().map(|id| id.as_str().to_string()).collect(),
            )
            .await
            .map_err(to_gql_error)?;
        Ok(RecycleRestorePreviewPayload {
            fingerprint: preview.fingerprint,
            items: preview
                .items
                .into_iter()
                .map(|item| RecycleRestorePreviewItemPayload {
                    id: ID::from(item.id),
                    original_path: item.original_path,
                    destination_occupied: item.destination_occupied,
                })
                .collect(),
        })
    }

    /// List available backups; requires system-settings management permission.
    async fn backups(&self, ctx: &Context<'_>) -> GqlResult<Vec<BackupInfoPayload>> {
        require_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let backups = app.list_backups(&actor).await.map_err(to_gql_error)?;
        backups
            .into_iter()
            .map(from_backup_info)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| to_gql_error(AppError::Validation(error)))
    }

    /// List a bounded page of pending releases with optional title, wanted-item, and status filters.
    async fn pending_releases(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Optional title, wanted-item, and release-status filters; omitted applies no filters."
        )]
        filter: Option<PendingReleaseFilterInput>,
        #[graphql(
            default = 50,
            desc = "Number of pending releases to return; defaults to 50 and is clamped to 1 through 500."
        )]
        limit: i32,
        #[graphql(
            default = 0,
            desc = "Number of matching pending releases to skip; defaults to 0 and negative values become 0."
        )]
        offset: i32,
    ) -> GqlResult<PendingReleasesPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let (title_id, wanted_item_id, statuses) = match filter {
            Some(filter) => (
                filter.title_id.map(String::from),
                filter.wanted_item_id.map(String::from),
                filter
                    .statuses
                    .unwrap_or_default()
                    .into_iter()
                    .map(PendingReleaseStatusValue::into_application)
                    .collect::<Vec<_>>(),
            ),
            None => (None, None, Vec::new()),
        };
        let limit = limit.clamp(1, 500);
        let offset = offset.max(0);
        let (releases, total_count) = app
            .list_pending_releases_page(
                &actor,
                title_id,
                wanted_item_id,
                statuses,
                i64::from(limit),
                i64::from(offset),
            )
            .await
            .map_err(to_gql_error)?;
        let items = releases
            .into_iter()
            .map(from_pending_release)
            .collect::<Vec<_>>();
        let has_more = i64::from(offset).saturating_add(items.len() as i64) < total_count;
        Ok(PendingReleasesPayload {
            items,
            has_more,
            total_count: total_count.min(i64::from(i32::MAX)) as i32,
        })
    }

    /// List recent import records; the limit defaults to 50 and is capped at 500.
    async fn import_history(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Number of import records to return; defaults to 50 and is clamped to 1 through 500."
        )]
        limit: Option<i32>,
    ) -> GqlResult<Vec<ImportRecordPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let limit = limit.unwrap_or(50).clamp(1, 500) as usize;
        let records = app
            .list_import_history(&actor, limit)
            .await
            .map_err(to_gql_error)?;
        Ok(records
            .into_iter()
            .map(crate::mappers::from_import_record)
            .collect())
    }

    /// Return the authenticated user payload, or null for an anonymous session; restricted enrollment and password-replacement sessions are rejected.
    async fn me(&self, ctx: &Context<'_>) -> GqlResult<Option<UserPayload>> {
        let auth_context = mfa_verification_from_ctx(ctx);
        if auth_context.session_scope == JwtSessionScope::MfaEnrollment {
            return Err(to_gql_error(AppError::MfaEnrollmentRequired(
                "MFA enrollment must be completed before accessing Scryer".into(),
            )));
        }
        if auth_context.session_scope == JwtSessionScope::PasswordChangeRequired {
            return Err(to_gql_error(AppError::PasswordChangeRequired(
                "password replacement must be completed before accessing Scryer".into(),
            )));
        }

        match current_user_from_ctx(ctx) {
            Some(user) => {
                let app = app_from_ctx(ctx)?;
                let effective_authorization = user.authorization.clone();
                let mut user = app
                    .load_user_for_auth_payload(&user)
                    .await
                    .map_err(to_gql_error)?;
                if auth_context.oauth_authorization_source == OAuthAuthorizationSource::Authless {
                    user.username = "Anonymous".to_string();
                }
                user.authorization = effective_authorization;
                let auth_factor_status = app
                    .user_auth_factor_status(&user.id)
                    .await
                    .map_err(to_gql_error)?;
                Ok(Some(from_user_with_auth_factor_status(
                    user,
                    auth_factor_status,
                )))
            }
            None => Ok(None),
        }
    }
}

#[allow(clippy::too_many_arguments)]
#[Object]
impl AcquisitionQueries {
    /// Return Missing or Cutoff Upgrade targets with activity state and per-scope convergence progress.
    async fn wanted_items(
        &self,
        ctx: &Context<'_>,
        #[graphql(default, desc = "Wanted target set to list; defaults to Missing.")]
        wanted_kind: WantedKindValue,
        #[graphql(desc = "Media facet filter, when supplied.")] facet: Option<MediaFacetValue>,
        #[graphql(
            desc = "Library IDs to include; omitted or empty includes all permitted libraries."
        )]
        library_ids: Option<Vec<ID>>,
        #[graphql(desc = "Title search text, when supplied.")] title_search: Option<String>,
        #[graphql(default = 50, desc = "Number of targets to return; defaults to 50.")] limit: i64,
        #[graphql(
            default = 0,
            desc = "Number of matching targets to skip; defaults to 0."
        )]
        offset: i64,
    ) -> GqlResult<WantedItemsListPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let (views, total) = app
            .list_wanted_scope_views(
                &actor,
                wanted_kind_to_application(wanted_kind),
                facet.map(MediaFacetValue::into_domain),
                optional_ids_to_strings(library_ids).unwrap_or_default(),
                title_search,
                limit,
                offset,
            )
            .await
            .map_err(to_gql_error)?;
        let items = views
            .into_iter()
            .map(from_wanted_scope_view)
            .collect::<scryer_application::AppResult<Vec<_>>>()
            .map_err(to_gql_error)?;
        let has_more = offset.saturating_add(items.len() as i64) < total;
        Ok(WantedItemsListPayload {
            items,
            total_count: total,
            has_more,
        })
    }

    /// Return one page of cutoff-unmet targets with the full match count and per-item convergence progress.
    async fn cutoff_unmet_titles_page(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Media facet filter, when supplied.")] facet: Option<MediaFacetValue>,
        #[graphql(
            desc = "Library IDs to include; omitted or empty includes all permitted libraries."
        )]
        library_ids: Option<Vec<ID>>,
        #[graphql(desc = "Number of targets to return; negative values become 0.")] limit: i32,
        #[graphql(desc = "Number of matching targets to skip; negative values become 0.")]
        offset: i32,
    ) -> GqlResult<CutoffUnmetTitlesPagePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let (items, total) = app
            .list_cutoff_unmet_titles_page_with_convergence(
                &actor,
                facet.map(MediaFacetValue::into_domain),
                optional_ids_to_strings(library_ids),
                limit.max(0) as usize,
                offset.max(0) as usize,
            )
            .await
            .map_err(to_gql_error)?;
        let items: Vec<_> = items
            .into_iter()
            .map(|(item, convergence)| from_cutoff_unmet_item(item, convergence))
            .collect();
        let has_more = (offset.max(0) as usize).saturating_add(items.len()) < total;
        Ok(CutoffUnmetTitlesPagePayload {
            items,
            total_count: total as i64,
            has_more,
        })
    }

    /// Return acquisition diagnostics for one title ID.
    async fn title_acquisition_diagnostics(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Title ID whose acquisition diagnostics are returned.")] title_id: ID,
    ) -> GqlResult<TitleAcquisitionDiagnosticsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let diagnostics = app
            .title_acquisition_diagnostics(&actor, title_id.as_ref())
            .await
            .map_err(to_gql_error)?;
        Ok(from_title_acquisition_diagnostics(diagnostics).map_err(to_gql_error)?)
    }

    /// Return progress for an interactive acquisition-search job, or null when no such job exists.
    async fn acquisition_search_job(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Acquisition-search job ID to poll.")] id: ID,
    ) -> GqlResult<Option<AcquisitionSearchJobPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let view = app
            .acquisition_search_job(&actor, id.as_ref())
            .await
            .map_err(to_gql_error)?;
        view.map(from_acquisition_search_job_view)
            .transpose()
            .map_err(to_gql_error)
    }

    // ── Rule Sets ──────────────────────────────────────────────────────

    /// List rule sets visible to the caller.
    async fn rule_sets(&self, ctx: &Context<'_>) -> GqlResult<Vec<RuleSetPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;

        let rule_sets = app.list_rule_sets(&actor).await.map_err(to_gql_error)?;
        Ok(rule_sets
            .into_iter()
            .map(crate::mappers::from_rule_set)
            .collect())
    }

    // ── Maintenance Rule Sets ────────────────────────────────────────────

    /// List maintenance rule sets; requires catalog-settings management permission.
    async fn maintenance_rule_sets(&self, ctx: &Context<'_>) -> GqlResult<Vec<MaintenanceRuleSet>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;

        // Details, not bare rows: the payload carries the action and grace
        // period of the revision in force so a list view never has to fetch
        // each rule set again to render them.
        let details = app
            .list_maintenance_rule_set_details(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(details
            .iter()
            .map(crate::mappers::from_maintenance_rule_set)
            .collect())
    }

    /// Return one maintenance rule set with the revision currently in force, or null when no such rule set exists.
    async fn maintenance_rule_set(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Maintenance rule-set ID to load.")] id: ID,
    ) -> GqlResult<Option<MaintenanceRuleSetDetail>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;

        let detail = app
            .get_maintenance_rule_set(&actor, id.as_ref())
            .await
            .map_err(to_gql_error)?;
        Ok(detail.map(crate::mappers::from_maintenance_rule_set_detail))
    }

    /// List every stored revision of one maintenance rule set, newest first.
    async fn maintenance_rule_revisions(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Maintenance rule-set ID whose revisions are returned.")] rule_set_id: ID,
    ) -> GqlResult<Vec<MaintenanceRuleRevision>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;

        let revisions = app
            .list_maintenance_rule_revisions(&actor, rule_set_id.as_ref())
            .await
            .map_err(to_gql_error)?;
        Ok(revisions
            .into_iter()
            .map(crate::mappers::from_maintenance_rule_revision)
            .collect())
    }

    /// List the static maintenance action catalog the rule builder chooses from.
    async fn maintenance_action_descriptors(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Vec<MaintenanceActionDescriptor>> {
        // The catalog is static, so no service call enforces the permission the
        // rest of this surface requires; the gate has to happen here.
        require_app_permission(ctx, AppPermission::ManageCatalogSettings).await?;
        Ok(crate::mappers::maintenance_action_descriptors())
    }

    /// List lifecycle candidates the maintenance evaluator has recorded; requires catalog-settings management permission.
    ///
    /// Two independent things hide rows, and they are not the same thing. A
    /// rule in shadow mode is dark by definition, so its candidates are
    /// returned only when `includeShadow` is true. Separately, the instance
    /// result-display gate hides everything while it is off.
    ///
    /// `includeShadow` overrides the gate as well as the mode filter, on
    /// purpose: an operator deciding whether to arm result display has to be
    /// able to see what shadow evaluation actually found first, and reaching
    /// this query already requires catalog-settings management.
    async fn maintenance_candidates(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Return candidates of this rule set only.")] rule_set_id: Option<ID>,
        #[graphql(desc = "Return candidates in these lifecycle states only.")] states: Option<
            Vec<MaintenanceCandidateState>,
        >,
        #[graphql(desc = "Return candidates whose subject belongs to this library only.")]
        library_id: Option<ID>,
        #[graphql(desc = "Include candidates produced by rules running in shadow mode.")]
        include_shadow: Option<bool>,
        #[graphql(desc = "Maximum number of candidates to return.")] limit: Option<i32>,
    ) -> GqlResult<Vec<MaintenanceCandidate>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;

        let candidates = app
            .list_maintenance_candidates(
                &actor,
                scryer_application::maintenance_rules::MaintenanceCandidateFilter {
                    rule_set_id: rule_set_id.map(String::from),
                    states: states
                        .unwrap_or_default()
                        .into_iter()
                        .map(crate::mappers::maintenance_candidate_state_into_application)
                        .collect(),
                    library_id: library_id.map(String::from),
                    include_shadow: include_shadow.unwrap_or(false),
                    limit: limit.filter(|limit| *limit > 0).map(|limit| limit as usize),
                },
            )
            .await
            .map_err(to_gql_error)?;
        Ok(candidates
            .into_iter()
            .map(crate::mappers::from_maintenance_candidate)
            .collect())
    }

    /// List recorded maintenance evaluation runs, newest first.
    ///
    /// Runs carry counts and timing rather than subjects, so the result-display
    /// gate does not hide them: they are how an operator sees that dark
    /// evaluation is working at all.
    async fn maintenance_evaluation_runs(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Return runs of this rule set only.")] rule_set_id: Option<ID>,
        #[graphql(desc = "Maximum number of runs to return.")] limit: Option<i32>,
    ) -> GqlResult<Vec<MaintenanceEvaluationRun>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;

        let runs = app
            .list_maintenance_evaluation_runs(
                &actor,
                rule_set_id.as_ref().map(|id| id.as_str()),
                limit.filter(|limit| *limit > 0).map(|limit| limit as usize),
            )
            .await
            .map_err(to_gql_error)?;
        Ok(runs
            .into_iter()
            .map(crate::mappers::from_maintenance_evaluation_run)
            .collect())
    }

    /// Read the five instance-wide maintenance gates; requires system-settings management permission.
    async fn maintenance_instance_gates(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<MaintenanceInstanceGates> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;

        let gates = app
            .maintenance_instance_gates(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(crate::mappers::from_maintenance_instance_gates(gates))
    }

    /// List maintenance exclusions, newest first.
    ///
    /// Narrowing by rule returns that rule's own exclusions together with every
    /// global one, because both are what actually stop it acting.
    async fn maintenance_exclusions(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Return the exclusions that apply to this rule set.")] rule_set_id: Option<
            ID,
        >,
    ) -> GqlResult<Vec<MaintenanceExclusion>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;

        let exclusions = app
            .list_maintenance_exclusions(&actor, rule_set_id.as_ref().map(|id| id.as_str()))
            .await
            .map_err(to_gql_error)?;
        Ok(exclusions
            .into_iter()
            .map(crate::mappers::from_maintenance_exclusion)
            .collect())
    }

    /// List recorded maintenance action-handler attempts, newest first.
    async fn maintenance_action_runs(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Return only attempts for this rule set.")] rule_set_id: Option<ID>,
        #[graphql(desc = "Return only attempts for this candidate.")] candidate_id: Option<ID>,
        #[graphql(desc = "Maximum rows to return; defaults to fifty.")] limit: Option<i32>,
    ) -> GqlResult<Vec<MaintenanceActionRun>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;

        let runs = app
            .list_maintenance_action_runs(
                &actor,
                rule_set_id.as_ref().map(|id| id.as_str()),
                candidate_id.as_ref().map(|id| id.as_str()),
                limit.and_then(|limit| usize::try_from(limit).ok()),
            )
            .await
            .map_err(to_gql_error)?;
        Ok(runs
            .into_iter()
            .filter_map(crate::mappers::from_maintenance_action_run)
            .collect())
    }

    // ── Post-Processing Scripts ──────────────────────────────────────────

    /// List post-processing scripts visible to the caller.
    async fn post_processing_scripts(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Vec<PostProcessingScriptPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;

        let scripts = app
            .list_post_processing_scripts(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(scripts
            .into_iter()
            .map(crate::mappers::from_pp_script)
            .collect())
    }

    /// List runs for one post-processing script; the limit defaults to 50 and is clamped to 1 through 500.
    async fn post_processing_script_runs(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Post-processing script ID whose runs are returned.")] script_id: ID,
        #[graphql(
            desc = "Maximum runs to return; defaults to 50 and is clamped to 1 through 500."
        )]
        limit: Option<i32>,
    ) -> GqlResult<Vec<PostProcessingScriptRunPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let script_id = String::from(script_id);

        let limit = limit.unwrap_or(50).clamp(1, 500) as usize;
        let runs = app
            .list_post_processing_script_runs(&actor, &script_id, limit)
            .await
            .map_err(to_gql_error)?;
        Ok(runs
            .into_iter()
            .map(crate::mappers::from_pp_script_run)
            .collect())
    }

    // ── Plugins ──────────────────────────────────────────────────────────

    /// List available registry plugins visible to the caller.
    async fn plugins(&self, ctx: &Context<'_>) -> GqlResult<Vec<RegistryPluginPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let plugins = app
            .list_available_plugins(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(plugins
            .into_iter()
            .map(crate::mappers::from_registry_plugin)
            .collect())
    }

    /// Return plugin catalog readiness and update status visible to the caller.
    async fn plugin_catalog_status(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<PluginCatalogStatusPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let status = app
            .plugin_catalog_status(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(crate::mappers::from_plugin_catalog_status(status))
    }

    /// List community rule packs from the plugin registry.
    async fn rule_pack_registry(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Vec<RulePackRegistryEntryPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let packs = app
            .list_rule_pack_registry(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(packs
            .into_iter()
            .map(|p| RulePackRegistryEntryPayload {
                id: p.id,
                name: p.name,
                description: p.description,
                author: p.author,
                version: p.version,
            })
            .collect())
    }

    /// Fetch templates from a community rule pack by registry ID.
    async fn rule_pack_templates(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Registry ID of the community rule pack.")] pack_id: String,
    ) -> GqlResult<Vec<RulePackTemplatePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let templates = app
            .fetch_rule_pack_templates(&actor, &pack_id)
            .await
            .map_err(to_gql_error)?;
        Ok(templates
            .into_iter()
            .map(|t| RulePackTemplatePayload {
                id: t.id,
                title: t.title,
                description: t.description,
                category: t.category,
                rego_source: t.rego_source,
                applied_facets: t.applied_facets,
            })
            .collect())
    }

    /// Return available indexer provider types and their configuration fields; requires system-settings permission.
    async fn indexer_provider_types(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Vec<ProviderTypePayload>> {
        let app = app_from_ctx(ctx)?;
        require_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let provider_types = app.available_indexer_provider_types();
        Ok(provider_types
            .into_iter()
            .map(|(pt, name, fields, default_base_url)| {
                from_provider_type(
                    pt,
                    name,
                    fields,
                    default_base_url,
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    false,
                )
            })
            .collect())
    }

    /// Return available download-client provider types and their configuration fields; requires system-settings permission.
    async fn download_client_provider_types(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Vec<ProviderTypePayload>> {
        let app = app_from_ctx(ctx)?;
        require_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let provider_types = app.available_download_client_provider_types();
        Ok(provider_types
            .into_iter()
            .map(|(pt, name, fields, default_base_url)| {
                from_provider_type(
                    pt,
                    name,
                    fields,
                    default_base_url,
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    false,
                )
            })
            .collect())
    }

    /// Return available subtitle-provider types, host bindings, and recommended facets; requires catalog-settings permission.
    async fn subtitle_provider_types(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Vec<ProviderTypePayload>> {
        let app = app_from_ctx(ctx)?;
        require_app_permission(ctx, AppPermission::ManageCatalogSettings).await?;
        let available_host_bindings = app
            .subtitle_host_bindings()
            .await
            .map_err(to_gql_error)?
            .into_keys()
            .map(|binding| binding.as_str().to_string())
            .collect::<Vec<_>>();
        let provider_types = app.available_subtitle_provider_types();
        Ok(provider_types
            .into_iter()
            .map(|provider_type| {
                let name = app
                    .subtitle_provider_name(&provider_type)
                    .unwrap_or_else(|| provider_type.clone());
                let fields = app.subtitle_provider_config_fields(&provider_type);
                let recommended_facets = app.subtitle_provider_recommended_facets(&provider_type);
                from_provider_type(
                    provider_type,
                    name,
                    fields,
                    None,
                    available_host_bindings.clone(),
                    recommended_facets,
                    Vec::new(),
                    false,
                )
            })
            .collect())
    }
}

#[allow(clippy::too_many_arguments)]
#[Object]
impl UtilityQueries {
    // ── Notifications ────────────────────────────────────────────────────

    /// List notification channels visible to the caller, including provider configuration metadata.
    async fn notification_channels(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Vec<NotificationChannelPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let channels = app
            .list_notification_channels(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(channels
            .into_iter()
            .map(|channel| {
                let fields = app.notification_provider_config_fields(channel.channel_type.as_str());
                crate::mappers::from_notification_channel_with_fields(channel, &fields)
            })
            .collect())
    }

    /// List notification targets visible to the caller.
    async fn notification_targets(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Vec<NotificationTargetPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let targets = app
            .list_notification_targets(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(targets
            .into_iter()
            .map(crate::mappers::from_notification_target)
            .collect())
    }

    /// List notification subscriptions visible to the caller.
    async fn notification_subscriptions(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Vec<NotificationSubscriptionPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let subs = app
            .list_notification_subscriptions(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(subs
            .into_iter()
            .map(crate::mappers::from_notification_subscription)
            .collect())
    }

    /// Return available notification provider types and their configuration fields; requires system-settings permission.
    async fn notification_provider_types(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Vec<ProviderTypePayload>> {
        let app = app_from_ctx(ctx)?;
        require_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let provider_types = app.available_notification_provider_types();
        Ok(provider_types
            .into_iter()
            .map(|pt| {
                let name = app
                    .notification_provider_name(&pt)
                    .unwrap_or_else(|| pt.clone());
                let fields = app.notification_provider_config_fields(&pt);
                let supported_events = app
                    .notification_provider_supported_events(&pt)
                    .into_iter()
                    .map(|event| event.as_str().to_string())
                    .collect();
                let supports_test = app.notification_provider_supports_test(&pt);
                from_provider_type(
                    pt,
                    name,
                    fields,
                    None,
                    Vec::new(),
                    Vec::new(),
                    supported_events,
                    supports_test,
                )
            })
            .collect())
    }

    /// Return notification event types that can be subscribed to; requires system-settings permission.
    async fn notification_event_types(&self, ctx: &Context<'_>) -> GqlResult<Vec<String>> {
        let app = app_from_ctx(ctx)?;
        require_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        Ok(app
            .subscribable_notification_event_types()
            .iter()
            .map(|e| e.as_str().to_string())
            .collect())
    }

    // ── Service Logs ────────────────────────────────────────────────────

    /// Return whether setup is complete and whether download clients and indexers are configured.
    async fn setup_status(&self, ctx: &Context<'_>) -> GqlResult<SetupStatusPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let setup_complete = app.setup_complete().await.map_err(to_gql_error)?;

        let has_download_clients = !app
            .list_download_client_configs(&actor, None)
            .await
            .map_err(to_gql_error)?
            .is_empty();

        let has_indexers = !app
            .list_indexer_configs(&actor, None)
            .await
            .map_err(to_gql_error)?
            .is_empty();

        Ok(SetupStatusPayload {
            setup_complete,
            has_download_clients,
            has_indexers,
        })
    }

    /// List non-hidden entries under an absolute directory path; requires library-settings read permission.
    async fn browse_path(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            default_with = "String::from(\"/\")",
            desc = "Absolute directory path to list; defaults to \"/\"."
        )]
        path: String,
        #[graphql(
            desc = "Include regular files as well as directories; defaults to false; hidden names are excluded."
        )]
        include_files: Option<bool>,
    ) -> GqlResult<Vec<DirectoryEntryPayload>> {
        require_library_settings_permission(ctx).await?;
        let read_dir = browse_path_read_dir(&path).map_err(to_gql_error)?;
        let mut entries: Vec<DirectoryEntryPayload> = Vec::new();
        let include_files = include_files.unwrap_or(false);
        for entry in read_dir.flatten() {
            let ft = match entry.file_type() {
                Ok(ft) => ft,
                Err(_) => continue,
            };
            if !(ft.is_dir() || include_files && ft.is_file()) {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let full_path = entry.path().to_string_lossy().into_owned();
            entries.push(DirectoryEntryPayload {
                name,
                path: full_path,
                is_directory: ft.is_dir(),
            });
        }
        entries.sort_by_key(|a| a.name.to_lowercase());
        Ok(entries)
    }

    /// Return the most recent buffered service log lines; requires system-settings permission.
    async fn service_logs(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            default = 250,
            desc = "Maximum log lines; defaults to 250 and is clamped to 1 through 2000."
        )]
        limit: i32,
    ) -> GqlResult<ServiceLogsPayload> {
        require_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let safe_limit = (limit.clamp(1, 2000)) as usize;
        let lines = match ctx.data_opt::<crate::context::LogBuffer>() {
            Some(buf) => buf.snapshot(safe_limit),
            None => vec![],
        };
        let count = lines.len() as i32;
        Ok(ServiceLogsPayload {
            generated_at: Utc::now(),
            lines,
            count,
        })
    }

    /// List external subtitles associated with a title ID.
    async fn external_subtitles(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Title ID whose external subtitles are returned.")] title_id: ID,
    ) -> GqlResult<Vec<ExternalSubtitlePayload>> {
        let actor = actor_from_ctx(ctx)?;
        let app = app_from_ctx(ctx)?;
        let downloads = app
            .list_external_subtitles_for_title(&actor, title_id.as_ref())
            .await
            .map_err(to_gql_error)?;
        downloads
            .into_iter()
            .map(|listing| {
                let score_percent = listing.score_percent;
                let d = listing.download;
                Ok(ExternalSubtitlePayload {
                    id: d.id.into(),
                    media_file_id: d.media_file_id.into(),
                    title_id: d.title_id.into(),
                    episode_id: d.episode_id.map(Into::into),
                    source_kind: d.source_kind.as_str().to_string(),
                    language: d.language,
                    provider: d.provider,
                    provider_file_id: d.provider_file_id,
                    file_path: d.file_path,
                    score: d.score,
                    score_percent,
                    hearing_impaired: d.hearing_impaired,
                    forced: d.forced,
                    ai_translated: d.ai_translated,
                    machine_translated: d.machine_translated,
                    uploader: d.uploader,
                    release_info: d.release_info,
                    synced: d.synced,
                    downloaded_at: parse_required_datetime(
                        &d.downloaded_at,
                        "external subtitle downloaded_at",
                    )
                    .map_err(to_gql_error)?,
                })
            })
            .collect::<GqlResult<Vec<_>>>()
    }

    /// List external subtitle blocklist entries for one media file ID.
    async fn external_subtitle_blocklist_entries(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Media-file ID whose subtitle blocklist entries are returned.")]
        media_file_id: ID,
    ) -> GqlResult<Vec<ExternalSubtitleBlocklistEntryPayload>> {
        let actor = actor_from_ctx(ctx)?;
        let app = app_from_ctx(ctx)?;
        let media_file_id = String::from(media_file_id);
        let entries = app
            .list_external_subtitle_blocklist_for_media_file(&actor, &media_file_id)
            .await
            .map_err(to_gql_error)?;
        entries
            .into_iter()
            .map(|entry| {
                Ok(ExternalSubtitleBlocklistEntryPayload {
                    id: entry.id.into(),
                    media_file_id: entry.media_file_id.into(),
                    provider: entry.provider,
                    provider_file_id: entry.provider_file_id,
                    language: entry.language,
                    reason: entry.reason,
                    created_at: parse_required_datetime(
                        &entry.created_at,
                        "external subtitle blocklist created_at",
                    )?,
                })
            })
            .collect::<Result<Vec<_>, AppError>>()
            .map_err(to_gql_error)
    }
}

/// Narrows the items a rename plan returns without touching what it reports.
///
/// `total`, `renamable`, `noop`, `conflicts`, `errors`, and the fingerprint
/// keep describing every file in the plan, so apply still validates the whole
/// plan against the fingerprint the caller previewed. `budget`, when set, is
/// consumed across successive calls so one sample can span several plans.
fn scope_rename_plan_items(plan: &mut RenamePlan, renamable_only: bool, budget: &mut Option<i32>) {
    if renamable_only {
        plan.items
            .retain(|item| matches!(item.write_action, RenameWriteAction::Move));
    }
    if let Some(remaining) = budget.as_mut() {
        plan.items.truncate((*remaining).max(0) as usize);
        *remaining = remaining.saturating_sub(i32::try_from(plan.items.len()).unwrap_or(i32::MAX));
    }
}

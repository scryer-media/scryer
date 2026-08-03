use async_graphql::{Context, ID, MergedObject, Object, Result as GqlResult};

use chrono::{DateTime, Utc};
use scryer_application::{
    AppError, DownloadImportFilter, ExternalImportArrSourceKind as AppArrSourceKind,
    ExternalImportMonitorWarmupStatus,
    ExternalImportSetupSecretDraft as AppExternalImportSetupSecretDraft,
    ExternalImportSetupSecretDraftStatus, ExternalImportSetupSecretInstanceKind,
    ExternalImportSetupSecretOverrideDraft, ImageProxyKind, JwtSessionScope, MediaRequestCounts,
    OAuthAuthorizationSource, PendingImportCounts, RuntimePathStyle, SCRYER_VERSION, SortDirection,
    TitleCatalogContentStatus, TitleCatalogFilter, TitleCatalogSort, TitleCatalogSortKey,
    TitleHistoryFilter, is_supported_title_history_event_type, supported_title_history_event_types,
};
use scryer_domain::{AppPermission, LibraryPermission, TitleHistoryEventType};
use scryer_interface_metadata::MetadataQueries;
use scryer_interface_settings::SettingsQueries;
use std::{fs, io, path::Path};

use scryer_interface_core as context;
use scryer_interface_core::{
    actor_from_ctx, actor_has_any_library_permission, actor_has_app_permission, app_from_ctx,
    current_user_from_ctx, mfa_verification_from_ctx, require_app_permission,
    require_config_app_permission, to_gql_error,
};
use scryer_interface_media::mappers;
use scryer_interface_media::mappers::{
    catalog_discovery_query_from_input, discovery_home_filter_options_query_from_input,
    discovery_home_query_from_input, discovery_item_detail_query_from_input,
    discovery_items_query_from_input, from_activity_event, from_backup_info,
    from_catalog_discovery, from_collection, from_delete_preview, from_delete_titles_preview,
    from_discovery_home, from_discovery_home_cards, from_discovery_home_filter_options,
    from_discovery_item, from_discovery_items_result, from_domain_event, from_download_queue_item,
    from_episode, from_external_import_monitor_warmup_progress, from_job_definition, from_job_run,
    from_library, from_library_scan_session, from_library_settings, from_linked_account,
    from_media_rename_plan, from_media_request, from_media_request_counts,
    from_pending_import_connection, from_pending_import_counts, from_pending_release,
    from_provider_type, from_runtime_path_style, from_smg_scryer_update_notice,
    from_smg_version_compatibility_notice, from_system_health, from_title,
    from_title_acquisition_diagnostics, from_title_history_page,
    from_title_release_blocklist_entry, from_user_with_auth_factor_status, from_wanted_item,
    from_wanted_scope_view,
};
use scryer_interface_media::types::*;

fn from_metadata_search_item(
    app: &scryer_application::AppUseCase,
    item: scryer_application::RichMetadataSearchItem,
) -> MetadataSearchItemPayload {
    let owner_id = item.tvdb_id.to_string();
    let poster_url = app.media_image_url(
        item.poster_url.as_deref(),
        Some("metadata_search"),
        Some(&owner_id),
        ImageProxyKind::Poster,
        "w250",
    );
    MetadataSearchItemPayload {
        tvdb_id: item.tvdb_id,
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

#[derive(MergedObject, Default)]
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
);

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
    async fn linked_accounts(
        &self,
        ctx: &Context<'_>,
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
    async fn titles(
        &self,
        ctx: &Context<'_>,
        facet: Option<MediaFacetValue>,
        library_ids: Option<Vec<ID>>,
        query: Option<String>,
        filter: Option<TitleCatalogFilterInput>,
        sort: Option<TitleCatalogSortInput>,
        limit: Option<i32>,
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

    async fn title_catalog_filter_options(
        &self,
        ctx: &Context<'_>,
        facet: Option<MediaFacetValue>,
        library_ids: Option<Vec<ID>>,
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

    async fn libraries(
        &self,
        ctx: &Context<'_>,
        facet: Option<MediaFacetValue>,
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

    async fn media_requests(
        &self,
        ctx: &Context<'_>,
        facet: Option<MediaFacetValue>,
        library_ids: Option<Vec<ID>>,
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

    async fn my_media_requests(
        &self,
        ctx: &Context<'_>,
        facet: Option<MediaFacetValue>,
        library_ids: Option<Vec<ID>>,
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

    async fn library_settings(
        &self,
        ctx: &Context<'_>,
        library_id: ID,
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

    async fn titles_by_external_ids(
        &self,
        ctx: &Context<'_>,
        source: String,
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

    async fn title(&self, ctx: &Context<'_>, id: ID) -> GqlResult<Option<TitlePayload>> {
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

    async fn episode(
        &self,
        ctx: &Context<'_>,
        title_id: ID,
        episode_id: ID,
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

    /// Fetch an episode by its globally unique id — targeted-refetch primitive;
    /// unlike `episode`, no parent title id is required.
    async fn episode_by_id(&self, ctx: &Context<'_>, id: ID) -> GqlResult<Option<EpisodePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let episode = app
            .get_episode(&actor, id.as_ref())
            .await
            .map_err(to_gql_error)?;
        Ok(episode.map(|episode| from_episode(&app, episode)))
    }

    /// Fetch a collection by its globally unique id — targeted-refetch primitive.
    async fn collection_by_id(
        &self,
        ctx: &Context<'_>,
        id: ID,
    ) -> GqlResult<Option<CollectionPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let collection = app
            .get_collection(&actor, id.as_ref())
            .await
            .map_err(to_gql_error)?;
        Ok(collection.map(from_collection))
    }

    async fn title_by_slug(
        &self,
        ctx: &Context<'_>,
        facet: MediaFacetValue,
        library_slug: Option<String>,
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

    async fn media_rename_preview(
        &self,
        ctx: &Context<'_>,
        input: MediaRenamePreviewInput,
    ) -> GqlResult<MediaRenamePlanPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let _ = input.dry_run;
        let facet = input.facet.into_domain();
        let plan = if let Some(title_id) = input.title_id {
            let title_id = title_id.to_string();
            app.preview_rename_for_title(&actor, &title_id, facet)
                .await
                .map_err(to_gql_error)?
        } else {
            app.preview_rename_for_facet(&actor, facet)
                .await
                .map_err(to_gql_error)?
        };

        Ok(from_media_rename_plan(plan))
    }

    async fn delete_title_preview(
        &self,
        ctx: &Context<'_>,
        title_id: ID,
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

    async fn delete_titles_preview(
        &self,
        ctx: &Context<'_>,
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

    async fn delete_media_file_preview(
        &self,
        ctx: &Context<'_>,
        file_id: ID,
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

    async fn delete_external_subtitle_preview(
        &self,
        ctx: &Context<'_>,
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

    async fn wanted_item(&self, ctx: &Context<'_>, id: ID) -> GqlResult<Option<WantedItemPayload>> {
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

    async fn search_releases(
        &self,
        ctx: &Context<'_>,
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
        } = input;

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

    /// Poll an interactive release-search job started by
    /// `startInteractiveReleaseSearch`. `null` for an unknown, evicted, or
    /// another user's job.
    async fn interactive_release_search(
        &self,
        ctx: &Context<'_>,
        id: ID,
    ) -> GqlResult<Option<InteractiveReleaseSearchPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let snapshot = app
            .interactive_release_search(&actor, id.as_ref())
            .await
            .map_err(to_gql_error)?;
        Ok(snapshot.map(from_interactive_release_search_snapshot))
    }

    async fn title_history(
        &self,
        ctx: &Context<'_>,
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

    async fn title_release_blocklist(
        &self,
        ctx: &Context<'_>,
        title_id: ID,
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
    async fn activity_events(
        &self,
        ctx: &Context<'_>,
        limit: Option<i32>,
        offset: Option<i32>,
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

    async fn audit_log(
        &self,
        ctx: &Context<'_>,
        event_types: Option<Vec<DomainEventTypeValue>>,
        title_id: Option<ID>,
        facet: Option<MediaFacetValue>,
        after_sequence: Option<Long>,
        before_sequence: Option<Long>,
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

    #[graphql(deprecation = "use externalImportWarmupStatus")]
    async fn external_import_arr_source_warmup_status(
        &self,
        ctx: &Context<'_>,
        session_id: ID,
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

    /// Kind-neutral warmup status lookup — covers Arr source and Prowlarr
    /// discovery sessions alike.
    async fn external_import_warmup_status(
        &self,
        ctx: &Context<'_>,
        session_id: ID,
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

    async fn external_import_aggregate_warmup_progress(
        &self,
        ctx: &Context<'_>,
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
                app.count_download_import_items(&actor, DownloadImportFilter::All)
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

    async fn pending_imports(
        &self,
        ctx: &Context<'_>,
        facet: MediaFacetValue,
        library_ids: Option<Vec<ID>>,
        status: PendingImportStatusValue,
        #[graphql(default = 50)] limit: i64,
        #[graphql(default = 0)] offset: i64,
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
        Ok(from_pending_import_connection(connection, offset))
    }

    async fn pending_import_title_search(
        &self,
        ctx: &Context<'_>,
        pending_import_id: ID,
        query: String,
        #[graphql(default = 8)] limit: i32,
        #[graphql(default_with = "\"eng\".to_string()")] language: String,
        year: Option<i32>,
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

    async fn pending_import_binding_preview(
        &self,
        ctx: &Context<'_>,
        pending_import_id: ID,
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
    async fn jobs(&self, ctx: &Context<'_>) -> GqlResult<Vec<JobDefinitionPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let jobs = app.list_jobs(&actor).await.map_err(to_gql_error)?;
        Ok(jobs.into_iter().map(from_job_definition).collect())
    }

    async fn active_job_runs(&self, ctx: &Context<'_>) -> GqlResult<Vec<JobRunPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let runs = app.active_job_runs(&actor).await.map_err(to_gql_error)?;
        Ok(runs.into_iter().map(from_job_run).collect())
    }

    async fn job_runs(
        &self,
        ctx: &Context<'_>,
        job_key: JobKeyValue,
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

    async fn recent_job_runs(
        &self,
        ctx: &Context<'_>,
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

    async fn discovery_home(
        &self,
        ctx: &Context<'_>,
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

    async fn discovery_home_cards(
        &self,
        ctx: &Context<'_>,
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

    async fn discovery_home_filter_options(
        &self,
        ctx: &Context<'_>,
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

    async fn discovery_items(
        &self,
        ctx: &Context<'_>,
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

    async fn discovery_item_detail(
        &self,
        ctx: &Context<'_>,
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

    async fn catalog_discovery(
        &self,
        ctx: &Context<'_>,
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

    async fn download_queue(
        &self,
        ctx: &Context<'_>,
        include_all_activity: Option<bool>,
        include_history_only: Option<bool>,
        include_import_activity: Option<bool>,
        title_id: Option<ID>,
        activity_filter: Option<DownloadActivityFilterValue>,
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

    async fn download_import(
        &self,
        ctx: &Context<'_>,
        limit: Option<i32>,
        offset: Option<i32>,
        filter: Option<DownloadImportFilterValue>,
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

    async fn download_history(
        &self,
        ctx: &Context<'_>,
        limit: Option<i32>,
        offset: Option<i32>,
        filters: Option<Vec<DownloadHistoryFilterValue>>,
        client_ids: Option<Vec<ID>>,
        scryer_submitted_only: Option<bool>,
        sort_key: Option<DownloadHistorySortKeyValue>,
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
    async fn runtime_info(&self, ctx: &Context<'_>) -> GqlResult<RuntimeInfoPayload> {
        let _actor = actor_from_ctx(ctx)?;
        Ok(RuntimeInfoPayload {
            runtime_path_style: from_runtime_path_style(RuntimePathStyle::current()),
        })
    }

    async fn system_health(&self, ctx: &Context<'_>) -> GqlResult<SystemHealthPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let health = app.system_health(&actor).await.map_err(to_gql_error)?;
        Ok(from_system_health(health))
    }

    async fn scryer_version(&self, ctx: &Context<'_>) -> GqlResult<String> {
        let _actor = actor_from_ctx(ctx)?;
        Ok(SCRYER_VERSION.to_string())
    }

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

    async fn smg_scryer_update_notice(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Option<SmgScryerUpdateNoticePayload>> {
        let app = app_from_ctx(ctx)?;
        let _actor = actor_from_ctx(ctx)?;
        let notice = app.smg_scryer_update_notice().await.map_err(to_gql_error)?;
        Ok(notice.map(from_smg_scryer_update_notice))
    }

    async fn recycled_items(
        &self,
        ctx: &Context<'_>,
        #[graphql(default = 500)] limit: i32,
        #[graphql(default = 0)] offset: i32,
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
                    reason: item.reason,
                    recycled_at: parse_required_datetime(
                        &item.recycled_at,
                        "recycled item recycled_at",
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

    async fn pending_releases(
        &self,
        ctx: &Context<'_>,
        filter: Option<PendingReleaseFilterInput>,
        #[graphql(default = 50)] limit: i32,
        #[graphql(default = 0)] offset: i32,
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

    async fn import_history(
        &self,
        ctx: &Context<'_>,
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

    async fn me(&self, ctx: &Context<'_>) -> GqlResult<Option<UserPayload>> {
        let auth_context = mfa_verification_from_ctx(ctx);
        if auth_context.session_scope == JwtSessionScope::MfaEnrollment {
            return Err(to_gql_error(AppError::MfaEnrollmentRequired(
                "MFA enrollment must be completed before accessing Scryer".into(),
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
    /// The derived Missing / Upgrades view. `wantedKind` selects the
    /// target set (`MISSING` derived from fileless monitored scopes, `CUTOFF_UPGRADE`
    /// from below-cutoff files). Results are the derived targets joined to the
    /// activity-state row (when one exists) and enriched with per-scope convergence
    /// progress. The retired state-row status / decision-code filters are dropped —
    /// they only distinguished state rows, which are no longer the target source.
    async fn wanted_items(
        &self,
        ctx: &Context<'_>,
        #[graphql(default)] wanted_kind: WantedKindValue,
        facet: Option<MediaFacetValue>,
        library_ids: Option<Vec<ID>>,
        title_search: Option<String>,
        #[graphql(default = 50)] limit: i64,
        #[graphql(default = 0)] offset: i64,
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

    /// Bounded view: a single page of cutoff-unmet (Upgrades) targets plus
    /// the full unmet count and per-item convergence progress, so the UI paginates
    /// instead of loading the whole set. The unpaged `cutoffUnmetTitles` query was
    /// removed in this release: the full-array browser load is retired.
    async fn cutoff_unmet_titles_page(
        &self,
        ctx: &Context<'_>,
        facet: Option<MediaFacetValue>,
        library_ids: Option<Vec<ID>>,
        limit: i32,
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

    async fn title_acquisition_diagnostics(
        &self,
        ctx: &Context<'_>,
        title_id: ID,
    ) -> GqlResult<TitleAcquisitionDiagnosticsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let diagnostics = app
            .title_acquisition_diagnostics(&actor, title_id.as_ref())
            .await
            .map_err(to_gql_error)?;
        Ok(from_title_acquisition_diagnostics(diagnostics).map_err(to_gql_error)?)
    }

    /// Progress of an interactive acquisition-search job, polled by
    /// the UI alongside the `jobRunEvents` push. `None` when no such job exists.
    async fn acquisition_search_job(
        &self,
        ctx: &Context<'_>,
        id: ID,
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

    async fn rule_sets(&self, ctx: &Context<'_>) -> GqlResult<Vec<RuleSetPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;

        let rule_sets = app.list_rule_sets(&actor).await.map_err(to_gql_error)?;
        Ok(rule_sets
            .into_iter()
            .map(crate::mappers::from_rule_set)
            .collect())
    }

    // ── Post-Processing Scripts ──────────────────────────────────────────

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

    async fn post_processing_script_runs(
        &self,
        ctx: &Context<'_>,
        script_id: ID,
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

    /// Fetch templates from a community rule pack by its registry ID.
    async fn rule_pack_templates(
        &self,
        ctx: &Context<'_>,
        pack_id: String,
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

    /// Returns all available indexer provider types from loaded plugins,
    /// with their config field schemas for dynamic form rendering.
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

    /// Browses local paths after the caller passes the library-settings read permission check.
    ///
    /// Directories are returned by default; set `includeFiles` to `true` to include regular files.
    async fn browse_path(
        &self,
        ctx: &Context<'_>,
        #[graphql(default_with = "String::from(\"/\")")] path: String,
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

    async fn service_logs(
        &self,
        ctx: &Context<'_>,
        #[graphql(default = 250)] limit: i32,
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

    /// List external subtitles for a title.
    async fn external_subtitles(
        &self,
        ctx: &Context<'_>,
        title_id: ID,
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

    /// List external subtitle blocklist entries for a specific media file.
    async fn external_subtitle_blocklist_entries(
        &self,
        ctx: &Context<'_>,
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

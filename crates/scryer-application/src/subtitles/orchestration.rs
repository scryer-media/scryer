use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use chrono::Utc;
use tracing::{debug, info, warn};

use crate::media::release_labels::resolve_release_labels_from_analysis;
use crate::normalize::normalize_imdb_id;
use crate::stored_paths::{path_to_stored_string, stored_path_to_path_buf};
use crate::subtitles::provider::{
    SubtitleFile, SubtitleMatch, SubtitleMediaKind, SubtitleProvider, SubtitleQuery,
};
use crate::subtitles::scoring::{
    SubtitleScoreKind, normalized_score_percent, percent_to_raw_threshold,
};
use crate::subtitles::search::SubtitleSearchOrchestrator;
use crate::subtitles::sync;
use crate::subtitles::wanted::{SubtitleLanguagePref, compute_missing_subtitles_from_streams};
use crate::{
    AccountQuotaKey, AppError, AppResult, AppUseCase, EstimatedCost, JobKey, JobTriggerSource,
    ParsedReleaseMetadata, RateLimitCooldownAction, RateLimitSignal, SchedulerAdmission,
    SchedulerBatchRequest, SchedulerCandidate, SchedulerCandidateId, SchedulerFeedback,
    SchedulerFeedbackOutcome, SchedulerIntent, SchedulerLease, SchedulerOperation,
    SchedulerPluginKind, ScopedExternalId, SubtitleProviderClient, SubtitleProviderConfigUpdate,
    SubtitleSettings as AppSubtitleSettings, TitleMediaFile, parse_release_metadata,
};
use scryer_domain::{
    ExternalSubtitleSourceKind, SubtitleBlocklistEntry, SubtitleDownload, SubtitleProviderConfig,
    Title, User,
};
use scryer_outbound_http::{DestinationKey, HostKey};
use scryer_plugin_sdk::{
    SubtitleSyncAudioStreamMetadata, SubtitleSyncMediaMetadataSnapshot,
    SubtitleSyncSubtitleStreamMetadata,
};

const ON_IMPORT_SUBTITLE_SEARCH_CONCURRENCY_LIMIT: usize = 2;
static ON_IMPORT_SUBTITLE_SEARCH_LIMIT: LazyLock<Arc<tokio::sync::Semaphore>> =
    LazyLock::new(|| {
        Arc::new(tokio::sync::Semaphore::new(
            ON_IMPORT_SUBTITLE_SEARCH_CONCURRENCY_LIMIT,
        ))
    });

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SubtitleAdmissionFailureMode {
    OmitProvider,
    ReturnTemporaryError,
}

pub struct DownloadSubtitleForMediaFileRequest {
    pub media_file_id: String,
    pub provider_name: String,
    pub provider_file_id: String,
    pub language: String,
    pub forced: bool,
    pub hearing_impaired: bool,
    pub score: Option<i32>,
    pub release_info: Option<String>,
    pub uploader: Option<String>,
    pub ai_translated: bool,
    pub machine_translated: bool,
}

pub struct ExternalSubtitleListing {
    pub download: SubtitleDownload,
    pub score_percent: Option<i32>,
}

#[derive(Clone)]
struct PluginSubtitleProviderAdapter {
    client: Arc<dyn SubtitleProviderClient>,
    config_id: String,
    provider_name: String,
    enabled_facets: Vec<String>,
    host_key: HostKey,
    destination_key: DestinationKey,
}

#[async_trait::async_trait]
impl SubtitleProvider for PluginSubtitleProviderAdapter {
    async fn search(&self, query: &SubtitleQuery) -> AppResult<Vec<SubtitleMatch>> {
        self.client.search(query).await
    }

    async fn download(&self, provider_file_id: &str) -> AppResult<SubtitleFile> {
        self.client.download(provider_file_id).await
    }

    fn name(&self) -> &str {
        self.provider_name.as_str()
    }
}

fn subtitle_scheduler_keys(config: &SubtitleProviderConfig) -> (HostKey, DestinationKey) {
    subtitle_config_host(config.config_json.as_str())
        .map(|host| (HostKey::from(host.clone()), DestinationKey::from(host)))
        .unwrap_or_else(|| {
            let fallback = format!("subtitle:{}:{}", config.provider_type, config.id);
            (
                HostKey::from(fallback.clone()),
                DestinationKey::from(fallback),
            )
        })
}

fn subtitle_config_host(config_json: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(config_json).ok()?;
    let object = value.as_object()?;
    object.values().find_map(|value| {
        let raw = value.as_str()?.trim();
        let url = reqwest::Url::parse(raw).ok()?;
        url.host_str().map(|host| host.to_ascii_lowercase())
    })
}

async fn admit_subtitle_provider(
    app: &AppUseCase,
    provider: &PluginSubtitleProviderAdapter,
    intent: SchedulerIntent,
    operation: SchedulerOperation,
    failure_mode: SubtitleAdmissionFailureMode,
) -> AppResult<Option<SchedulerLease>> {
    let candidate_id = SchedulerCandidateId::new();
    let decision = app
        .services
        .integrations
        .upstream_scheduler
        .admit_batch(SchedulerBatchRequest {
            batch_id: format!("subtitle:{}:{}", provider.config_id, candidate_id),
            now: Utc::now(),
            candidates: vec![SchedulerCandidate {
                candidate_id,
                plugin_config_id: Some(provider.config_id.clone()),
                plugin_kind: SchedulerPluginKind::Subtitle,
                operation,
                intent,
                host_key: provider.host_key.clone(),
                destination_key: provider.destination_key.clone(),
                account_quota_key: Some(AccountQuotaKey::from(provider.config_id.clone())),
                rss_request_key: None,
                estimated_cost: EstimatedCost::ONE_API_CALL,
                expected_value: Default::default(),
                learning_context: None,
                deadline_at: None,
                freshness: None,
                cancel_token: tokio_util::sync::CancellationToken::new(),
            }],
        })
        .await?;

    match decision.decisions.into_iter().next() {
        Some(SchedulerAdmission::Admit { reason, lease, .. }) => {
            debug!(
                subtitle_provider = provider.name(),
                scheduler_reason = ?reason,
                "scheduler admitted subtitle provider"
            );
            Ok(Some(lease))
        }
        Some(SchedulerAdmission::Defer {
            reason,
            retry_after,
            ..
        }) => {
            info!(
                subtitle_provider = provider.name(),
                scheduler_reason = ?reason,
                retry_after_secs = retry_after.map(|delay| delay.as_secs()),
                "scheduler deferred subtitle provider"
            );
            if failure_mode == SubtitleAdmissionFailureMode::ReturnTemporaryError {
                let message = match retry_after {
                    Some(delay) if !delay.is_zero() => format!(
                        "subtitle provider '{}' is temporarily deferred by upstream scheduler ({reason:?}, retry after {}s)",
                        provider.name(),
                        delay.as_secs()
                    ),
                    _ => format!(
                        "subtitle provider '{}' is temporarily deferred by upstream scheduler ({reason:?})",
                        provider.name()
                    ),
                };
                return Err(AppError::temporary_unavailable(message, retry_after));
            }
            Ok(None)
        }
        Some(SchedulerAdmission::Skip { reason, .. }) => {
            info!(
                subtitle_provider = provider.name(),
                scheduler_reason = ?reason,
                "scheduler skipped subtitle provider"
            );
            if failure_mode == SubtitleAdmissionFailureMode::ReturnTemporaryError {
                return Err(AppError::temporary_unavailable(
                    format!(
                        "subtitle provider '{}' is unavailable by upstream scheduler ({reason:?})",
                        provider.name()
                    ),
                    None,
                ));
            }
            Ok(None)
        }
        None => Ok(None),
    }
}

async fn record_subtitle_scheduler_feedback(
    app: &AppUseCase,
    provider: &PluginSubtitleProviderAdapter,
    lease: Option<SchedulerLease>,
    outcome: SchedulerFeedbackOutcome,
    retry_after: Option<Duration>,
    cooldown_action: RateLimitCooldownAction,
) {
    if let Err(error) = app
        .services
        .integrations
        .upstream_scheduler
        .record_feedback(SchedulerFeedback {
            lease,
            host_key: provider.host_key.clone(),
            destination_key: provider.destination_key.clone(),
            account_quota_key: Some(AccountQuotaKey::from(provider.config_id.clone())),
            outcome,
            observed_api_current: None,
            observed_api_max: None,
            observed_grab_current: None,
            observed_grab_max: None,
            retry_after,
            cooldown_action,
            rss_last_seen_release_identity: None,
            rss_last_seen_release_published_at: None,
            rss_feed_result_count: None,
            rss_seen_release_identities: Vec::new(),
            observed_at: Utc::now(),
        })
        .await
    {
        warn!(
            subtitle_provider = provider.name(),
            error = %error,
            "failed to record subtitle scheduler feedback"
        );
    }
}

fn subtitle_retry_after_from_error(error: &AppError) -> Option<Duration> {
    subtitle_rate_limit_signal_from_error(error).and_then(|signal| signal.retry_after)
}

fn subtitle_cooldown_action_from_error(error: &AppError) -> RateLimitCooldownAction {
    subtitle_rate_limit_signal_from_error(error)
        .map(|signal| signal.cooldown_action)
        .unwrap_or(RateLimitCooldownAction::None)
}

fn subtitle_scheduler_outcome_for_error(error: &AppError) -> SchedulerFeedbackOutcome {
    if subtitle_rate_limit_signal_from_error(error).is_some() {
        SchedulerFeedbackOutcome::RateLimited
    } else {
        SchedulerFeedbackOutcome::ProviderFailure
    }
}

fn subtitle_rate_limit_signal_from_error(error: &AppError) -> Option<RateLimitSignal> {
    RateLimitSignal::from_error(error)
}

async fn configured_runtime_subtitle_providers(
    app: &AppUseCase,
    _settings: &AppSubtitleSettings,
) -> AppResult<Vec<PluginSubtitleProviderAdapter>> {
    let Some(plugin_provider) = app
        .services
        .integrations
        .subtitle_plugin_provider
        .available()
    else {
        return Err(AppError::Repository(
            "subtitle plugin provider is not configured".to_string(),
        ));
    };
    let Some(repo) = app
        .services
        .integrations
        .subtitle_provider_configs
        .available()
    else {
        return Err(AppError::Repository(
            "subtitle provider config repository is not configured".to_string(),
        ));
    };

    let now = Utc::now();
    let configs = repo.list(None).await?;
    let enabled_configs = configs
        .into_iter()
        .filter(|config| config.is_enabled)
        .filter(|config| config.disabled_until.is_none_or(|until| until <= now))
        .filter(|config| {
            plugin_provider.supports_catalog_search_for_provider(&config.provider_type)
        })
        .collect::<Vec<_>>();

    if enabled_configs.is_empty() {
        return Err(AppError::Validation(
            "no enabled subtitle providers are configured".to_string(),
        ));
    }

    let mut providers = Vec::new();
    let mut first_error = None;
    for config in enabled_configs {
        match app.subtitle_provider_client_for_config(&config).await {
            Ok(client) => {
                let (host_key, destination_key) = subtitle_scheduler_keys(&config);
                providers.push(PluginSubtitleProviderAdapter {
                    client,
                    config_id: config.id,
                    provider_name: config.provider_type,
                    enabled_facets: config.enabled_facets,
                    host_key,
                    destination_key,
                })
            }
            Err(error) => {
                warn!(
                    subtitle_provider = config.name.as_str(),
                    provider_type = config.provider_type.as_str(),
                    error = %error,
                    "failed to instantiate subtitle provider config"
                );
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
    }

    if providers.is_empty() {
        if let Some(error) = first_error {
            return Err(error);
        }
        return Err(AppError::Repository(
            "no enabled subtitle providers could be instantiated".to_string(),
        ));
    }

    Ok(providers)
}

async fn runtime_subtitle_provider_for_download(
    app: &AppUseCase,
    _settings: &AppSubtitleSettings,
    provider_name: &str,
) -> AppResult<PluginSubtitleProviderAdapter> {
    let Some(plugin_provider) = app
        .services
        .integrations
        .subtitle_plugin_provider
        .available()
    else {
        return Err(AppError::Repository(
            "subtitle plugin provider is not configured".to_string(),
        ));
    };
    let now = Utc::now();
    if let Some(repo) = app
        .services
        .integrations
        .subtitle_provider_configs
        .available()
        && let Some(config) = repo
            .list(Some(provider_name.trim().to_ascii_lowercase()))
            .await?
            .into_iter()
            .find(|config| {
                config.is_enabled
                    && config.disabled_until.is_none_or(|until| until <= now)
                    && plugin_provider.supports_catalog_search_for_provider(&config.provider_type)
            })
    {
        return match app.subtitle_provider_client_for_config(&config).await {
            Ok(client) => {
                let (host_key, destination_key) = subtitle_scheduler_keys(&config);
                Ok(PluginSubtitleProviderAdapter {
                    client,
                    config_id: config.id,
                    provider_name: config.provider_type,
                    enabled_facets: config.enabled_facets,
                    host_key,
                    destination_key,
                })
            }
            Err(error) => Err(error),
        };
    }

    Err(AppError::NotFound(format!(
        "subtitle provider '{provider_name}' is not configured"
    )))
}

async fn search_all_subtitle_providers(
    app: &AppUseCase,
    providers: &[PluginSubtitleProviderAdapter],
    file_path: &Path,
    query: &SubtitleQuery,
    min_score: i32,
) -> AppResult<Vec<SubtitleMatch>> {
    let filtered_providers = providers
        .iter()
        .filter(|provider| provider_enabled_for_query_facet(provider, query.facet.as_deref()))
        .cloned()
        .collect::<Vec<_>>();
    if filtered_providers.is_empty() {
        return Ok(Vec::new());
    }
    let mut merged = HashMap::<(String, String), SubtitleMatch>::new();
    let mut first_error = None;
    let concurrency_limit = subtitle_provider_search_concurrency_limit();
    let semaphore = Arc::new(tokio::sync::Semaphore::new(concurrency_limit));
    let mut tasks = tokio::task::JoinSet::new();
    let file_path = file_path.to_path_buf();

    for provider in filtered_providers {
        let Some(scheduler_lease) = admit_subtitle_provider(
            app,
            &provider,
            SchedulerIntent::SubtitleSearch,
            SchedulerOperation::Search,
            SubtitleAdmissionFailureMode::OmitProvider,
        )
        .await?
        else {
            continue;
        };
        let permit = Arc::clone(&semaphore)
            .acquire_owned()
            .await
            .map_err(|err| {
                AppError::Repository(format!("subtitle search semaphore closed: {err}"))
            })?;
        let task_file_path = file_path.clone();
        let task_query = query.clone();
        tasks.spawn(async move {
            let _permit = permit;
            let provider_config_id = provider.config_id.clone();
            let provider_name = provider.name().to_string();
            let feedback_provider = provider.clone();
            let orchestrator = SubtitleSearchOrchestrator::new(min_score);
            let result = orchestrator
                .search(&provider, &task_file_path, &task_query)
                .await;
            (
                provider_name,
                provider_config_id,
                feedback_provider,
                scheduler_lease,
                result,
            )
        });
    }

    while let Some(join_result) = tasks.join_next().await {
        let (provider_name, provider_config_id, provider, scheduler_lease, result) = join_result
            .map_err(|err| AppError::Repository(format!("subtitle provider task failed: {err}")))?;
        match result {
            Ok(results) => {
                record_subtitle_scheduler_feedback(
                    app,
                    &provider,
                    Some(scheduler_lease),
                    SchedulerFeedbackOutcome::Success,
                    None,
                    RateLimitCooldownAction::None,
                )
                .await;
                record_subtitle_provider_search_success(
                    app,
                    &provider_name,
                    Some(provider_config_id.as_str()),
                    results.len(),
                )
                .await;
                for result in results {
                    let key = (result.provider.clone(), result.provider_file_id.clone());
                    match merged.get(&key) {
                        Some(existing) if existing.score >= result.score => {}
                        _ => {
                            merged.insert(key, result);
                        }
                    }
                }
            }
            Err(error) => {
                let outcome = subtitle_scheduler_outcome_for_error(&error);
                let retry_after = subtitle_retry_after_from_error(&error);
                record_subtitle_scheduler_feedback(
                    app,
                    &provider,
                    Some(scheduler_lease),
                    outcome,
                    retry_after,
                    subtitle_cooldown_action_from_error(&error),
                )
                .await;
                record_subtitle_provider_search_failure(
                    app,
                    &provider_name,
                    Some(provider_config_id.as_str()),
                    &error,
                )
                .await;
                warn!(
                    subtitle_provider = provider_name.as_str(),
                    error = %error,
                    "subtitle provider search failed"
                );
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
    }

    if merged.is_empty()
        && let Some(error) = first_error
    {
        return Err(error);
    }

    let mut results = merged.into_values().collect::<Vec<_>>();
    results.sort_by_key(|result| std::cmp::Reverse(result.score));
    Ok(results)
}

async fn record_subtitle_provider_search_success(
    app: &AppUseCase,
    provider_name: &str,
    config_id: Option<&str>,
    result_count: usize,
) {
    let Some(config_id) = config_id else {
        return;
    };
    let Some(repo) = app
        .services
        .integrations
        .subtitle_provider_configs
        .available()
    else {
        return;
    };
    let status = format!("Last search: {result_count} result(s)");
    if let Err(error) = repo
        .update(SubtitleProviderConfigUpdate {
            id: config_id.to_string(),
            last_health_status: Some(status),
            last_error: Some(None),
            last_error_at: Some(None),
            ..Default::default()
        })
        .await
    {
        warn!(
            subtitle_provider = provider_name,
            error = %error,
            "failed to update subtitle provider search status"
        );
    }
}

async fn record_subtitle_provider_search_failure(
    app: &AppUseCase,
    provider_name: &str,
    config_id: Option<&str>,
    error: &AppError,
) {
    let Some(config_id) = config_id else {
        return;
    };
    let Some(repo) = app
        .services
        .integrations
        .subtitle_provider_configs
        .available()
    else {
        return;
    };
    if let Err(update_error) = repo
        .update(SubtitleProviderConfigUpdate {
            id: config_id.to_string(),
            last_health_status: Some("Last search failed".to_string()),
            last_error: Some(Some(error.to_string())),
            last_error_at: Some(Some(Utc::now())),
            ..Default::default()
        })
        .await
    {
        warn!(
            subtitle_provider = provider_name,
            error = %update_error,
            "failed to update subtitle provider search failure status"
        );
    }
}

fn provider_enabled_for_query_facet(
    provider: &PluginSubtitleProviderAdapter,
    query_facet: Option<&str>,
) -> bool {
    let Some(query_facet) = query_facet.map(str::trim).filter(|value| !value.is_empty()) else {
        return true;
    };
    provider
        .enabled_facets
        .iter()
        .any(|facet| facet.eq_ignore_ascii_case(query_facet))
}

fn subtitle_provider_search_concurrency_limit() -> usize {
    4
}

impl AppUseCase {
    pub async fn list_external_subtitles_for_title(
        &self,
        actor: &User,
        title_id: &str,
    ) -> AppResult<Vec<ExternalSubtitleListing>> {
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
        let score_kind = subtitle_score_kind(subtitle_media_kind(&title));
        let downloads = self
            .services
            .workflow
            .subtitle_downloads
            .list_for_title(title_id)
            .await?;
        Ok(downloads
            .into_iter()
            .map(|download| ExternalSubtitleListing {
                score_percent: download
                    .score
                    .map(|score| normalized_score_percent(score_kind, score)),
                download,
            })
            .collect())
    }

    pub async fn list_external_subtitle_blocklist_for_media_file(
        &self,
        actor: &User,
        media_file_id: &str,
    ) -> AppResult<Vec<SubtitleBlocklistEntry>> {
        let media_file = self
            .services
            .library
            .media_files
            .get_media_file_by_id(media_file_id)
            .await?
            .ok_or_else(|| AppError::NotFound("media file not found".to_string()))?;
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(&media_file.title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", media_file.title_id)))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::View,
        )
        .await?;
        self.services
            .workflow
            .subtitle_downloads
            .list_blocklist_for_media_file(media_file_id)
            .await
    }

    pub async fn search_subtitles_for_media_file(
        &self,
        actor: &User,
        media_file_id: &str,
        language: &str,
    ) -> AppResult<Vec<crate::subtitles::SubtitleMatch>> {
        let media_file = self
            .services
            .library
            .media_files
            .get_media_file_by_id(media_file_id)
            .await?
            .ok_or_else(|| AppError::NotFound("media file not found".to_string()))?;
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(&media_file.title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", media_file.title_id)))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::View,
        )
        .await?;
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(&media_file.title_id)
            .await?
            .ok_or_else(|| AppError::NotFound("title not found".to_string()))?;
        let settings = self.subtitle_settings().await?;
        let providers = configured_runtime_subtitle_providers(self, &settings).await?;

        let languages = vec![language.trim().to_string()];
        let episode_context = media_file_episode_context(self, &media_file).await;
        let query = build_subtitle_query(
            &title,
            &media_file,
            &episode_context,
            &languages,
            None,
            settings.include_ai_translated,
            settings.include_machine_translated,
        );

        let results = search_all_subtitle_providers(
            self,
            &providers,
            &stored_path_to_path_buf(&media_file.file_path),
            &query,
            0,
        )
        .await?;

        let mut filtered = Vec::with_capacity(results.len());
        for result in results {
            let is_blocklisted = self
                .services
                .workflow
                .subtitle_downloads
                .is_blocklisted(media_file_id, &result.provider, &result.provider_file_id)
                .await
                .unwrap_or(false);
            if !is_blocklisted {
                filtered.push(result);
            }
        }

        Ok(filtered)
    }

    pub async fn download_subtitle_for_media_file(
        &self,
        actor: &User,
        request: DownloadSubtitleForMediaFileRequest,
    ) -> AppResult<()> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;
        let DownloadSubtitleForMediaFileRequest {
            media_file_id,
            provider_name,
            provider_file_id,
            language,
            forced,
            hearing_impaired,
            score,
            release_info,
            uploader,
            ai_translated,
            machine_translated,
        } = request;

        let media_file = self
            .services
            .library
            .media_files
            .get_media_file_by_id(&media_file_id)
            .await?
            .ok_or_else(|| AppError::NotFound("media file not found".to_string()))?;
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(&media_file.title_id)
            .await?
            .ok_or_else(|| AppError::NotFound("title not found".to_string()))?;
        let settings = self.subtitle_settings().await?;
        let provider =
            runtime_subtitle_provider_for_download(self, &settings, &provider_name).await?;
        let Some(scheduler_lease) = admit_subtitle_provider(
            self,
            &provider,
            SchedulerIntent::SubtitleDownload,
            SchedulerOperation::Download,
            SubtitleAdmissionFailureMode::ReturnTemporaryError,
        )
        .await?
        else {
            return Err(AppError::Repository(format!(
                "subtitle provider '{provider_name}' was deferred by upstream scheduler"
            )));
        };

        let file_path = stored_path_to_path_buf(&media_file.file_path);
        let episode_context = media_file_episode_context(self, &media_file).await;
        let download_result = crate::subtitles::download::download_and_save_with_selection(
            &provider,
            &provider_file_id,
            file_path.as_path(),
            &language,
            forced,
            hearing_impaired,
            crate::subtitles::download::SubtitleDownloadSelection {
                episode: episode_context.episode,
                absolute_episode: episode_context.absolute_episode,
                archive_provider: self
                    .services
                    .integrations
                    .archive_extractor_plugin_provider
                    .available()
                    .cloned(),
            },
        )
        .await;
        match &download_result {
            Ok(_) => {
                record_subtitle_scheduler_feedback(
                    self,
                    &provider,
                    Some(scheduler_lease),
                    SchedulerFeedbackOutcome::Success,
                    None,
                    RateLimitCooldownAction::None,
                )
                .await;
            }
            Err(error) => {
                record_subtitle_scheduler_feedback(
                    self,
                    &provider,
                    Some(scheduler_lease),
                    subtitle_scheduler_outcome_for_error(error),
                    subtitle_retry_after_from_error(error),
                    subtitle_cooldown_action_from_error(error),
                )
                .await;
            }
        }
        let (dest_path, _) = download_result?;

        let record = SubtitleDownload {
            id: scryer_domain::Id::new().0,
            media_file_id: media_file.id.clone(),
            title_id: media_file.title_id.clone(),
            episode_id: media_file.episode_id.clone(),
            source_kind: ExternalSubtitleSourceKind::Downloaded,
            language,
            provider: Some(provider_name),
            provider_file_id: Some(provider_file_id),
            file_path: path_to_stored_string(&dest_path),
            score,
            hearing_impaired,
            forced,
            ai_translated,
            machine_translated,
            uploader,
            release_info,
            synced: false,
            downloaded_at: chrono::Utc::now().to_rfc3339(),
        };
        self.services
            .workflow
            .subtitle_downloads
            .insert(&record)
            .await?;

        let sync_result = maybe_sync_downloaded_subtitle(
            self,
            read_subtitle_sync_settings(&settings),
            subtitle_media_kind(&title),
            file_path.as_path(),
            &media_file.id,
            &dest_path,
            Some(&record.id),
            score,
            forced,
        )
        .await;

        info!(
            media_file_id = %media_file.id,
            subtitle_download_id = %record.id,
            sync_summary = %sync_summary_suffix(sync_result.as_ref()),
            "manual subtitle download completed"
        );

        Ok(())
    }

    pub(crate) async fn run_subtitle_search_job(&self) -> AppResult<String> {
        run_subtitle_search_cycle(self).await?;
        Ok("Subtitle search cycle completed".to_string())
    }
}

/// Background subtitle poller — searches for missing subtitles on a schedule.
pub async fn start_background_subtitle_poller(
    app: AppUseCase,
    token: tokio_util::sync::CancellationToken,
) {
    info!("background subtitle poller started");
    let mut settings_changed = app.runtime.events.settings_changed_broadcast.subscribe();
    let mut settings = load_poller_settings(&app).await;
    let mut next_trigger_source = if settings.as_ref().is_some_and(|current| current.enabled) {
        JobTriggerSource::ScheduledStartup
    } else {
        JobTriggerSource::ScheduledInterval
    };
    let mut next_run_at =
        schedule_subtitle_poller(&app, settings.as_ref(), next_trigger_source).await;

    loop {
        if let Some(when) = next_run_at {
            let delay = when
                .signed_duration_since(Utc::now())
                .to_std()
                .unwrap_or_default();
            tokio::select! {
                _ = token.cancelled() => {
                    info!("subtitle poller shutting down");
                    app.clear_job_next_run_at(JobKey::SubtitleSearch).await;
                    return;
                }
                changed = settings_changed.recv() => {
                    if !should_reload_subtitle_poller(changed) {
                        continue;
                    }
                    let was_enabled = settings.as_ref().is_some_and(|current| current.enabled);
                    settings = load_poller_settings(&app).await;
                    next_trigger_source = if settings.as_ref().is_some_and(|current| current.enabled) && !was_enabled {
                        JobTriggerSource::ScheduledStartup
                    } else {
                        JobTriggerSource::ScheduledInterval
                    };
                    next_run_at = schedule_subtitle_poller(&app, settings.as_ref(), next_trigger_source).await;
                }
                _ = tokio::time::sleep(delay) => {
                    if let Err(err) = app
                        .run_scheduled_job_now(JobKey::SubtitleSearch, next_trigger_source)
                        .await
                    {
                        warn!(error = %err, "subtitle search cycle failed");
                    }
                    settings = load_poller_settings(&app).await;
                    next_trigger_source = JobTriggerSource::ScheduledInterval;
                    next_run_at = schedule_subtitle_poller(&app, settings.as_ref(), next_trigger_source).await;
                }
            }
        } else {
            tokio::select! {
                _ = token.cancelled() => {
                    info!("subtitle poller shutting down");
                    app.clear_job_next_run_at(JobKey::SubtitleSearch).await;
                    return;
                }
                changed = settings_changed.recv() => {
                    if !should_reload_subtitle_poller(changed) {
                        continue;
                    }
                    let was_enabled = settings.as_ref().is_some_and(|current| current.enabled);
                    settings = load_poller_settings(&app).await;
                    next_trigger_source = if settings.as_ref().is_some_and(|current| current.enabled) && !was_enabled {
                        JobTriggerSource::ScheduledStartup
                    } else {
                        JobTriggerSource::ScheduledInterval
                    };
                    next_run_at = schedule_subtitle_poller(&app, settings.as_ref(), next_trigger_source).await;
                }
            }
        }
    }
}

/// Spawn a fire-and-forget subtitle search for a newly imported file.
/// Called from import code when `subtitles.auto_download_on_import` is true.
pub fn spawn_subtitle_search_for_file(app: AppUseCase, title_id: String, media_file_id: String) {
    tokio::spawn(async move {
        let permit = match Arc::clone(&ON_IMPORT_SUBTITLE_SEARCH_LIMIT)
            .acquire_owned()
            .await
        {
            Ok(permit) => permit,
            Err(err) => {
                warn!(error = %err, title_id, media_file_id, "on-import subtitle search limiter closed");
                return;
            }
        };
        let _permit = permit;
        if let Err(err) = run_subtitle_search_for_file(&app, &title_id, &media_file_id).await {
            warn!(error = %err, title_id, media_file_id, "on-import subtitle search failed");
        }
    });
}

fn is_series_title(title: &scryer_domain::Title) -> bool {
    title.facet == scryer_domain::MediaFacet::Series
        || title.facet == scryer_domain::MediaFacet::Anime
}

fn subtitle_media_kind(title: &scryer_domain::Title) -> SubtitleMediaKind {
    if is_series_title(title) {
        SubtitleMediaKind::Episode
    } else {
        SubtitleMediaKind::Movie
    }
}

fn title_imdb_ids(
    title: &Title,
    preferred_release: Option<&ParsedReleaseMetadata>,
) -> (Option<String>, Option<String>) {
    let imdb_id = title
        .external_ids
        .iter()
        .find(|external_id| external_id.source.eq_ignore_ascii_case("imdb"))
        .and_then(|external_id| normalize_imdb_id(&external_id.value))
        .or_else(|| title.imdb_id.as_deref().and_then(normalize_imdb_id));

    if is_series_title(title) {
        (None, imdb_id)
    } else {
        (
            imdb_id.or_else(|| preferred_release.and_then(|release| release.imdb_id.clone())),
            None,
        )
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct SubtitleEpisodeContext {
    season: Option<i32>,
    episode: Option<i32>,
    absolute_episode: Option<i32>,
    external_ids: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Copy)]
struct SubtitleSyncSettings {
    enabled: bool,
    threshold_series: i32,
    threshold_movie: i32,
    max_offset_seconds: i64,
}

impl SubtitleSyncSettings {
    fn threshold_for(self, media_kind: SubtitleMediaKind) -> i32 {
        let kind = subtitle_score_kind(media_kind);
        let percent = match media_kind {
            SubtitleMediaKind::Episode => self.threshold_series,
            SubtitleMediaKind::Movie => self.threshold_movie,
        };
        percent_to_raw_threshold(kind, percent)
    }
}

fn subtitle_score_kind(media_kind: SubtitleMediaKind) -> SubtitleScoreKind {
    match media_kind {
        SubtitleMediaKind::Episode => SubtitleScoreKind::Episode,
        SubtitleMediaKind::Movie => SubtitleScoreKind::Movie,
    }
}

fn subtitle_minimum_raw_score(media_kind: SubtitleMediaKind, series: i32, movie: i32) -> i32 {
    let percent = match media_kind {
        SubtitleMediaKind::Episode => series,
        SubtitleMediaKind::Movie => movie,
    };
    percent_to_raw_threshold(subtitle_score_kind(media_kind), percent)
}

fn read_subtitle_sync_settings(settings: &AppSubtitleSettings) -> SubtitleSyncSettings {
    SubtitleSyncSettings {
        enabled: settings.sync_enabled,
        threshold_series: settings.sync_threshold_series,
        threshold_movie: settings.sync_threshold_movie,
        max_offset_seconds: settings.sync_max_offset_seconds as i64,
    }
}

async fn reference_subtitle_path_for_sync(
    app: &AppUseCase,
    media_file_id: &str,
    subtitle_path: &Path,
    download_id: Option<&str>,
) -> Option<PathBuf> {
    let mut candidates = app
        .services
        .workflow
        .subtitle_downloads
        .list_for_media_file(media_file_id)
        .await
        .ok()?
        .into_iter()
        .filter(|record| download_id != Some(record.id.as_str()))
        .filter(|record| !record.forced)
        .filter_map(|record| {
            let path = stored_path_to_path_buf(&record.file_path);
            if same_filesystem_path(&path, subtitle_path)
                || !path.exists()
                || !is_supported_reference_subtitle_path(&path)
            {
                return None;
            }
            Some((record, path))
        })
        .collect::<Vec<_>>();

    candidates.sort_by(|(left, _), (right, _)| {
        right
            .synced
            .cmp(&left.synced)
            .then_with(|| {
                right
                    .score
                    .unwrap_or(i32::MIN)
                    .cmp(&left.score.unwrap_or(i32::MIN))
            })
            .then_with(|| right.downloaded_at.cmp(&left.downloaded_at))
    });

    candidates.into_iter().map(|(_, path)| path).next()
}

fn is_supported_reference_subtitle_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "srt" | "vtt" | "ass" | "ssa"
            )
        })
        .unwrap_or(false)
}

fn same_filesystem_path(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    match (std::fs::canonicalize(left), std::fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

async fn load_poller_settings(app: &AppUseCase) -> Option<AppSubtitleSettings> {
    match app.subtitle_settings().await {
        Ok(settings) => Some(settings),
        Err(err) => {
            warn!(error = %err, "failed to load subtitle settings");
            None
        }
    }
}

async fn schedule_subtitle_poller(
    app: &AppUseCase,
    settings: Option<&AppSubtitleSettings>,
    trigger_source: JobTriggerSource,
) -> Option<chrono::DateTime<Utc>> {
    let settings = match settings {
        Some(settings) if settings.enabled => settings,
        _ => {
            debug!("subtitle poller idle");
            app.clear_job_next_run_at(JobKey::SubtitleSearch).await;
            return None;
        }
    };

    let next_run_at = match trigger_source {
        JobTriggerSource::ScheduledStartup => Utc::now() + chrono::Duration::seconds(120),
        _ => Utc::now() + chrono::Duration::hours(settings.search_interval_hours.max(1) as i64),
    };
    app.set_job_next_run_at(JobKey::SubtitleSearch, next_run_at)
        .await;
    Some(next_run_at)
}

fn should_reload_subtitle_poller(
    changed: Result<Vec<String>, tokio::sync::broadcast::error::RecvError>,
) -> bool {
    match changed {
        Ok(keys) => keys.iter().any(|key| key.starts_with("subtitles.")),
        Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
            debug!(skipped, "subtitle poller lagged settings updates");
            true
        }
        Err(tokio::sync::broadcast::error::RecvError::Closed) => false,
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "subtitle sync needs the full media, policy, and download context for one operation"
)]
async fn maybe_sync_downloaded_subtitle(
    app: &AppUseCase,
    sync_settings: SubtitleSyncSettings,
    media_kind: SubtitleMediaKind,
    video_path: &Path,
    media_file_id: &str,
    subtitle_path: &Path,
    download_id: Option<&str>,
    score: Option<i32>,
    forced: bool,
) -> Option<sync::SyncResult> {
    let policy = sync::SyncPolicy {
        enabled: sync_settings.enabled,
        forced,
        score,
        threshold: Some(sync_settings.threshold_for(media_kind)),
        max_offset_seconds: sync_settings.max_offset_seconds,
    };

    let subtitle_plugin_provider = app
        .services
        .integrations
        .subtitle_plugin_provider
        .available();
    let subtitle_sync_client =
        subtitle_plugin_provider.and_then(|provider| provider.subtitle_sync_client());
    let plugin_installed = subtitle_plugin_provider.is_some_and(|provider| {
        provider
            .available_provider_types()
            .iter()
            .any(|provider_type| provider_type == "enhanced-subtitle-sync")
    });
    let reference_subtitle_path =
        reference_subtitle_path_for_sync(app, media_file_id, subtitle_path, download_id).await;
    let media_metadata = subtitle_sync_media_metadata(app, media_file_id).await;

    match sync::sync_subtitle_with_policy_and_plugin_sync(
        video_path,
        subtitle_path,
        policy,
        subtitle_sync_client,
        plugin_installed,
        reference_subtitle_path.as_deref(),
        media_metadata,
    )
    .await
    {
        Ok(result) => {
            if result.applied
                && let Some(id) = download_id
                && let Err(err) = app
                    .services
                    .workflow
                    .subtitle_downloads
                    .set_synced(id, true)
                    .await
            {
                warn!(error = %err, download_id = id, "failed to persist subtitle sync status");
            }

            if result.applied {
                info!(
                    offset_ms = result.offset_ms,
                    consistency_ratio = result.consistency_ratio,
                    nosplit_score = result.nosplit_score,
                    split_score = result.split_score,
                    path = %subtitle_path.display(),
                    "subtitle timing synced"
                );
            } else {
                debug!(
                    reason = ?result.skipped_reason,
                    score,
                    path = %subtitle_path.display(),
                    "subtitle sync skipped"
                );
            }

            Some(result)
        }
        Err(err) => {
            warn!(error = %err, path = %subtitle_path.display(), "subtitle sync failed (non-fatal)");
            None
        }
    }
}

async fn subtitle_sync_media_metadata(
    app: &AppUseCase,
    media_file_id: &str,
) -> Option<SubtitleSyncMediaMetadataSnapshot> {
    match app
        .services
        .library
        .media_files
        .get_media_file_by_id(media_file_id)
        .await
    {
        Ok(Some(media_file)) => Some(media_file_to_subtitle_sync_metadata(&media_file)),
        Ok(None) => {
            debug!(
                media_file_id,
                "subtitle sync metadata unavailable: media file row not found"
            );
            None
        }
        Err(error) => {
            debug!(
                media_file_id,
                error = %error,
                "subtitle sync metadata unavailable: failed to load media file row"
            );
            None
        }
    }
}

fn media_file_to_subtitle_sync_metadata(
    media_file: &TitleMediaFile,
) -> SubtitleSyncMediaMetadataSnapshot {
    SubtitleSyncMediaMetadataSnapshot {
        analysis_source: "scryer_import".to_string(),
        container_format: media_file.container_format.clone(),
        duration_seconds: media_file.duration_seconds,
        video_codec: media_file.video_codec.as_ref().map(ToString::to_string),
        video_width: media_file.video_width,
        video_height: media_file.video_height,
        video_bitrate_kbps: media_file.video_bitrate_kbps,
        video_bit_depth: media_file.video_bit_depth,
        video_hdr_format: media_file.video_hdr_format.clone(),
        video_frame_rate: media_file.video_frame_rate.clone(),
        video_profile: media_file.video_profile.clone(),
        audio_codec: media_file.audio_codec.clone(),
        audio_profile: media_file.audio_profile.clone(),
        audio_channels: media_file.audio_channels,
        audio_bitrate_kbps: media_file.audio_bitrate_kbps,
        audio_languages: media_file.audio_languages.clone(),
        audio_streams: media_file
            .audio_streams
            .iter()
            .enumerate()
            .map(|(index, stream)| SubtitleSyncAudioStreamMetadata {
                index: index as u32,
                codec: stream.codec.clone(),
                profile: stream.profile.clone(),
                channels: stream.channels,
                language: stream.language.clone(),
                name: stream.name.clone(),
                bitrate_kbps: stream.bitrate_kbps,
            })
            .collect(),
        subtitle_languages: media_file.subtitle_languages.clone(),
        subtitle_codecs: media_file.subtitle_codecs.clone(),
        subtitle_streams: media_file
            .subtitle_streams
            .iter()
            .enumerate()
            .map(|(index, stream)| SubtitleSyncSubtitleStreamMetadata {
                index: index as u32,
                codec: stream.codec.clone(),
                language: stream.language.clone(),
                name: stream.name.clone(),
                forced: stream.forced,
                default: stream.default,
            })
            .collect(),
        has_multiaudio: media_file.has_multiaudio,
        num_chapters: media_file.num_chapters,
    }
}

fn sync_summary_suffix(result: Option<&sync::SyncResult>) -> String {
    result
        .map(|sync_result| format!(" [sync: {}]", sync_result.summary()))
        .unwrap_or_default()
}

fn parse_release_metadata_candidate(raw: Option<&str>) -> Option<ParsedReleaseMetadata> {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(parse_release_metadata)
}

fn release_title_candidates(parsed: &ParsedReleaseMetadata) -> Vec<String> {
    let mut candidates = Vec::new();

    if !parsed.normalized_title.is_empty() {
        candidates.push(parsed.normalized_title.clone());
    }
    candidates.extend(parsed.normalized_title_variants.clone());

    let mut deduped = Vec::with_capacity(candidates.len());
    let mut seen = std::collections::HashSet::new();
    for candidate in candidates {
        let trimmed = candidate.trim();
        if !trimmed.is_empty() && seen.insert(trimmed.to_string()) {
            deduped.push(trimmed.to_string());
        }
    }

    deduped
}

fn release_audio_codec(parsed: &ParsedReleaseMetadata) -> Option<String> {
    parsed
        .audio
        .as_ref()
        .or_else(|| parsed.audio_codecs.first())
        .map(ToString::to_string)
}

fn parsed_episode_context(parsed: &ParsedReleaseMetadata) -> SubtitleEpisodeContext {
    let episode = parsed
        .episode
        .as_ref()
        .and_then(|episode| episode.episode_numbers.first().copied())
        .map(|episode| episode as i32);

    SubtitleEpisodeContext {
        season: parsed
            .episode
            .as_ref()
            .and_then(|episode| episode.season.map(|season| season as i32)),
        episode,
        absolute_episode: None,
        external_ids: BTreeMap::new(),
    }
}

async fn media_file_episode_context(
    app: &AppUseCase,
    media_file: &crate::TitleMediaFile,
) -> SubtitleEpisodeContext {
    let Some(episode_id) = media_file.episode_id.as_deref() else {
        return SubtitleEpisodeContext::default();
    };

    match app
        .services
        .catalog
        .shows
        .get_episode_by_id(episode_id)
        .await
    {
        Ok(Some(episode)) => {
            let external_ids = subtitle_episode_external_ids(app, &episode).await;
            SubtitleEpisodeContext {
                season: episode
                    .season_number
                    .as_deref()
                    .and_then(|value| value.parse::<i32>().ok()),
                episode: episode
                    .episode_number
                    .as_deref()
                    .and_then(|value| value.parse::<i32>().ok()),
                absolute_episode: episode
                    .absolute_number
                    .as_deref()
                    .and_then(|value| value.parse::<i32>().ok()),
                external_ids,
            }
        }
        _ => SubtitleEpisodeContext::default(),
    }
}

async fn subtitle_episode_external_ids(
    app: &AppUseCase,
    episode: &scryer_domain::Episode,
) -> BTreeMap<String, Vec<String>> {
    let mut ids = episode_external_ids(episode);

    if let Some(collection_id) = episode.collection_id.as_deref()
        && let Ok(collection_ids) = app
            .services
            .catalog
            .shows
            .list_collection_external_ids(collection_id)
            .await
    {
        insert_scoped_external_ids(&mut ids, collection_ids, false);
    }

    if let Ok(episode_ids) = app
        .services
        .catalog
        .shows
        .list_episode_external_ids(&episode.id)
        .await
    {
        insert_scoped_external_ids(&mut ids, episode_ids, true);
    }

    ids
}

fn insert_scoped_external_ids(
    ids: &mut BTreeMap<String, Vec<String>>,
    scoped_ids: Vec<ScopedExternalId>,
    replace_existing_source: bool,
) {
    let mut seen_sources = std::collections::HashSet::new();
    for scoped_id in scoped_ids {
        let Some(source) = normalize_external_id_source(&scoped_id.source) else {
            continue;
        };
        if replace_existing_source && seen_sources.insert(source.clone()) {
            ids.remove(&source);
        }
        insert_external_id(ids, &source, &scoped_id.external_id);
    }
}

fn episode_external_ids(episode: &scryer_domain::Episode) -> BTreeMap<String, Vec<String>> {
    let mut ids = BTreeMap::new();
    if let Some(tvdb_id) = episode.tvdb_id.as_deref() {
        insert_external_id(&mut ids, "tvdb", tvdb_id);
    }
    ids
}

fn subtitle_external_ids(
    title: &Title,
    episode_context: &SubtitleEpisodeContext,
) -> BTreeMap<String, Vec<String>> {
    let mut ids = BTreeMap::new();

    for external_id in &title.external_ids {
        let normalized_source = normalize_external_id_source(&external_id.source);
        if normalized_source
            .as_deref()
            .is_some_and(|source| should_suppress_title_external_id(source, episode_context))
        {
            continue;
        }
        insert_external_id(&mut ids, &external_id.source, &external_id.value);
    }
    if let Some(imdb_id) = title.imdb_id.as_deref() {
        insert_external_id(&mut ids, "imdb", imdb_id);
    }

    for (source, values) in &episode_context.external_ids {
        for value in values {
            insert_external_id(&mut ids, source, value);
        }
    }

    ids
}

fn should_suppress_title_external_id(
    source: &str,
    episode_context: &SubtitleEpisodeContext,
) -> bool {
    matches!(source, "anilist" | "mal" | "anidb" | "kitsu")
        && episode_context
            .external_ids
            .get(source)
            .is_some_and(|values| !values.is_empty())
}

fn insert_external_id(ids: &mut BTreeMap<String, Vec<String>>, source: &str, value: &str) {
    let Some(source) = normalize_external_id_source(source) else {
        return;
    };
    let value = value.trim();
    if value.is_empty() {
        return;
    }

    let values = ids.entry(source).or_default();
    if !values.iter().any(|existing| existing == value) {
        values.push(value.to_string());
    }
}

fn normalize_external_id_source(source: &str) -> Option<String> {
    let normalized = source.trim().to_ascii_lowercase().replace('-', "_");
    let source = match normalized.as_str() {
        "imdb" | "imdb_id" => "imdb",
        "tvdb" | "tvdb_id" => "tvdb",
        "tmdb" | "tmdb_id" => "tmdb",
        "anidb" | "anidb_id" => "anidb",
        "anidb_episode" | "anidb_episode_id" => "anidb_episode",
        "anilist" | "anilist_id" => "anilist",
        "mal" | "mal_id" | "myanimelist" | "myanimelist_id" => "mal",
        _ => normalized.as_str(),
    };

    if source.is_empty() {
        None
    } else {
        Some(source.to_string())
    }
}

fn build_subtitle_query(
    title: &Title,
    media_file: &crate::TitleMediaFile,
    episode_context: &SubtitleEpisodeContext,
    languages: &[String],
    hearing_impaired: Option<bool>,
    include_ai_translated: bool,
    include_machine_translated: bool,
) -> SubtitleQuery {
    let scene_release = parse_release_metadata_candidate(media_file.scene_name.as_deref());
    let grabbed_release =
        parse_release_metadata_candidate(media_file.grabbed_release_title.as_deref());

    let preferred_release = scene_release.as_ref().or(grabbed_release.as_ref());
    let fallback_episode_context = preferred_release
        .map(parsed_episode_context)
        .unwrap_or_default();
    let episode_context = SubtitleEpisodeContext {
        season: episode_context.season.or(fallback_episode_context.season),
        episode: episode_context.episode.or(fallback_episode_context.episode),
        absolute_episode: episode_context
            .absolute_episode
            .or(fallback_episode_context.absolute_episode),
        external_ids: episode_context.external_ids.clone(),
    };

    let title_candidates = scene_release
        .as_ref()
        .map(release_title_candidates)
        .filter(|candidates| !candidates.is_empty())
        .or_else(|| {
            grabbed_release
                .as_ref()
                .map(release_title_candidates)
                .filter(|candidates| !candidates.is_empty())
        })
        .unwrap_or_default();

    let (imdb_id, series_imdb_id) = title_imdb_ids(title, preferred_release);
    let analysis_labels = resolve_release_labels_from_analysis(
        media_file.video_width,
        media_file.video_height,
        media_file.video_codec.as_ref(),
        media_file.audio_codec.as_deref(),
        media_file.audio_profile.as_deref(),
        media_file.audio_channels,
        &media_file.audio_streams,
    );

    SubtitleQuery {
        media_kind: subtitle_media_kind(title),
        facet: Some(title.facet.as_str().to_string()),
        file_hash: None,
        imdb_id,
        series_imdb_id,
        title: title.name.clone(),
        title_aliases: title.aliases.clone(),
        title_candidates,
        year: title
            .year
            .or_else(|| preferred_release.and_then(|release| release.year)),
        season: episode_context.season,
        episode: episode_context.episode,
        absolute_episode: episode_context.absolute_episode,
        external_ids: subtitle_external_ids(title, &episode_context),
        languages: languages.to_vec(),
        release_group: preferred_release
            .and_then(|release| release.release_group.clone())
            .or_else(|| media_file.release_group.clone()),
        source: preferred_release
            .and_then(|release| release.source.as_ref().map(ToString::to_string))
            .or_else(|| media_file.source_type.clone()),
        video_codec: preferred_release
            .and_then(|release| release.video_codec.as_ref().map(ToString::to_string))
            .or_else(|| media_file.video_codec_parsed.map(|codec| codec.to_string()))
            .or(analysis_labels.video_codec),
        audio_codec: preferred_release
            .and_then(release_audio_codec)
            .or_else(|| media_file.audio_codec_parsed.clone())
            .or(analysis_labels.audio_codec),
        resolution: preferred_release
            .and_then(|release| release.quality.clone())
            .or_else(|| media_file.resolution.clone())
            .or(analysis_labels.quality),
        hearing_impaired,
        include_ai_translated,
        include_machine_translated,
    }
}

fn embedded_subtitle_streams(
    media_file: &crate::TitleMediaFile,
) -> Vec<crate::SubtitleStreamDetail> {
    if !media_file.subtitle_streams.is_empty() {
        return media_file.subtitle_streams.clone();
    }

    media_file
        .subtitle_languages
        .iter()
        .map(|language| crate::SubtitleStreamDetail {
            codec: None,
            language: Some(language.clone()),
            name: None,
            forced: false,
            default: false,
        })
        .collect()
}

async fn run_subtitle_search_for_file(
    app: &AppUseCase,
    title_id: &str,
    media_file_id: &str,
) -> AppResult<()> {
    // Short delay to let the media file be fully committed
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;

    let settings = app.subtitle_settings().await?;
    if !settings.enabled {
        return Ok(());
    }

    let wanted_languages: Vec<SubtitleLanguagePref> = settings.languages.clone();
    if wanted_languages.is_empty() {
        return Ok(());
    }

    let include_ai = settings.include_ai_translated;
    let include_machine = settings.include_machine_translated;

    let title = app
        .services
        .catalog
        .titles
        .get_by_id(title_id)
        .await?
        .ok_or_else(|| crate::AppError::NotFound("title not found".into()))?;
    let mf = app
        .services
        .library
        .media_files
        .get_media_file_by_id(media_file_id)
        .await?
        .ok_or_else(|| crate::AppError::NotFound("media file not found".into()))?;

    let is_series = is_series_title(&title);
    let media_kind = if is_series {
        SubtitleMediaKind::Episode
    } else {
        SubtitleMediaKind::Movie
    };
    let min_score = subtitle_minimum_raw_score(
        media_kind,
        settings.minimum_score_series,
        settings.minimum_score_movie,
    );
    let sync_settings = read_subtitle_sync_settings(&settings);

    let providers = match configured_runtime_subtitle_providers(app, &settings).await {
        Ok(providers) => providers,
        Err(err) => {
            warn!(error = %err, title_id, media_file_id, "skipping on-import subtitle search");
            return Ok(());
        }
    };

    let existing = app
        .services
        .workflow
        .subtitle_downloads
        .list_for_media_file(&mf.id)
        .await
        .unwrap_or_default();
    let embedded = embedded_subtitle_streams(&mf);
    let missing = compute_missing_subtitles_from_streams(&wanted_languages, &existing, &embedded);

    let episode_context = media_file_episode_context(app, &mf).await;
    let file_path = stored_path_to_path_buf(&mf.file_path);

    for lang_pref in &missing {
        let query = build_subtitle_query(
            &title,
            &mf,
            &episode_context,
            std::slice::from_ref(&lang_pref.code),
            Some(lang_pref.hearing_impaired),
            include_ai,
            include_machine,
        );
        let results = match search_all_subtitle_providers(
            app,
            &providers,
            file_path.as_path(),
            &query,
            min_score,
        )
        .await
        {
            Ok(r) => r,
            Err(err) => {
                warn!(error = %err, language = %lang_pref.code, "on-import subtitle search failed");
                continue;
            }
        };

        let mut filtered_results = Vec::new();
        for result in &results {
            let blocklisted = app
                .services
                .workflow
                .subtitle_downloads
                .is_blocklisted(&mf.id, &result.provider, &result.provider_file_id)
                .await
                .unwrap_or(false);
            if !blocklisted {
                filtered_results.push(result);
            }
        }

        let best = filtered_results
            .iter()
            .filter(|r| r.score >= min_score)
            .filter(|r| r.hearing_impaired == lang_pref.hearing_impaired)
            .filter(|r| r.forced == lang_pref.forced)
            .max_by_key(|r| r.score);

        let best = match best {
            Some(b) => b,
            None => continue,
        };

        let download_provider =
            match runtime_subtitle_provider_for_download(app, &settings, &best.provider).await {
                Ok(provider) => provider,
                Err(err) => {
                    warn!(
                        error = %err,
                        provider = %best.provider,
                        language = %lang_pref.code,
                        "on-import subtitle provider unavailable for download"
                    );
                    continue;
                }
            };
        let Some(scheduler_lease) = admit_subtitle_provider(
            app,
            &download_provider,
            SchedulerIntent::SubtitleDownload,
            SchedulerOperation::Download,
            SubtitleAdmissionFailureMode::OmitProvider,
        )
        .await?
        else {
            continue;
        };

        let download_result = crate::subtitles::download::download_and_save_with_selection(
            &download_provider,
            &best.provider_file_id,
            file_path.as_path(),
            &best.language,
            best.forced,
            best.hearing_impaired,
            crate::subtitles::download::SubtitleDownloadSelection {
                episode: query.episode,
                absolute_episode: query.absolute_episode,
                archive_provider: app
                    .services
                    .integrations
                    .archive_extractor_plugin_provider
                    .available()
                    .cloned(),
            },
        )
        .await;
        let scheduler_outcome = match &download_result {
            Ok(_) => SchedulerFeedbackOutcome::Success,
            Err(error) => subtitle_scheduler_outcome_for_error(error),
        };
        let scheduler_retry_after = match &download_result {
            Ok(_) => None,
            Err(error) => subtitle_retry_after_from_error(error),
        };
        record_subtitle_scheduler_feedback(
            app,
            &download_provider,
            Some(scheduler_lease),
            scheduler_outcome,
            scheduler_retry_after,
            match &download_result {
                Ok(_) => RateLimitCooldownAction::None,
                Err(error) => subtitle_cooldown_action_from_error(error),
            },
        )
        .await;
        match download_result {
            Ok((dest_path, _)) => {
                let record = SubtitleDownload {
                    id: scryer_domain::Id::new().0,
                    media_file_id: mf.id.clone(),
                    title_id: title.id.clone(),
                    episode_id: mf.episode_id.clone(),
                    source_kind: ExternalSubtitleSourceKind::Downloaded,
                    language: best.language.clone(),
                    provider: Some(best.provider.clone()),
                    provider_file_id: Some(best.provider_file_id.clone()),
                    file_path: path_to_stored_string(&dest_path),
                    score: Some(best.score),
                    hearing_impaired: best.hearing_impaired,
                    forced: best.forced,
                    ai_translated: best.ai_translated,
                    machine_translated: best.machine_translated,
                    uploader: best.uploader.clone(),
                    release_info: best.release_info.clone(),
                    synced: false,
                    downloaded_at: Utc::now().to_rfc3339(),
                };
                let record_id = record.id.clone();
                let record_inserted = match app
                    .services
                    .workflow
                    .subtitle_downloads
                    .insert(&record)
                    .await
                {
                    Ok(()) => true,
                    Err(err) => {
                        warn!(error = %err, "failed to persist on-import subtitle download record");
                        false
                    }
                };
                let sync_result = maybe_sync_downloaded_subtitle(
                    app,
                    sync_settings,
                    query.media_kind,
                    file_path.as_path(),
                    &mf.id,
                    &dest_path,
                    record_inserted.then_some(record_id.as_str()),
                    Some(best.score),
                    best.forced,
                )
                .await;
                info!(
                    title = %title.name,
                    language = %lang_pref.code,
                    sync = sync_summary_suffix(sync_result.as_ref()),
                    "on-import subtitle downloaded"
                );
            }
            Err(err) => {
                warn!(error = %err, "on-import subtitle download failed");
            }
        }

        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    }

    Ok(())
}

/// Run a single subtitle search cycle across all monitored titles.
async fn run_subtitle_search_cycle(app: &AppUseCase) -> AppResult<()> {
    let settings = app.subtitle_settings().await?;
    if !settings.enabled {
        debug!("subtitle management disabled, skipping subtitle search");
        return Ok(());
    }

    let include_ai = settings.include_ai_translated;
    let include_machine = settings.include_machine_translated;
    let min_score_series: i32 = settings.minimum_score_series;
    let min_score_movie: i32 = settings.minimum_score_movie;

    let wanted_languages: Vec<SubtitleLanguagePref> = settings.languages.clone();

    if wanted_languages.is_empty() {
        debug!("no subtitle languages configured, skipping");
        return Ok(());
    }

    let sync_settings = read_subtitle_sync_settings(&settings);

    let providers = match configured_runtime_subtitle_providers(app, &settings).await {
        Ok(providers) => providers,
        Err(err) => {
            warn!(error = %err, "skipping subtitle search cycle");
            return Ok(());
        }
    };

    // Get all monitored titles with media files
    let titles = app
        .services
        .catalog
        .titles
        .list_for_matching(None, None)
        .await?;
    let mut searched = 0u32;
    let mut downloaded = 0u32;

    for title in &titles {
        if !title.monitored {
            continue;
        }

        let media_files = app
            .services
            .library
            .media_files
            .list_media_files_for_title(&title.id)
            .await?;

        for mf in &media_files {
            let existing = app
                .services
                .workflow
                .subtitle_downloads
                .list_for_media_file(&mf.id)
                .await
                .unwrap_or_default();

            let embedded = embedded_subtitle_streams(mf);

            let missing =
                compute_missing_subtitles_from_streams(&wanted_languages, &existing, &embedded);
            if missing.is_empty() {
                continue;
            }

            let media_kind = subtitle_media_kind(title);
            let min_score =
                subtitle_minimum_raw_score(media_kind, min_score_series, min_score_movie);

            let file_path = stored_path_to_path_buf(&mf.file_path);
            let episode_context = media_file_episode_context(app, mf).await;

            for lang_pref in &missing {
                searched += 1;
                let query = build_subtitle_query(
                    title,
                    mf,
                    &episode_context,
                    std::slice::from_ref(&lang_pref.code),
                    Some(lang_pref.hearing_impaired),
                    include_ai,
                    include_machine,
                );

                let results = match search_all_subtitle_providers(
                    app,
                    &providers,
                    file_path.as_path(),
                    &query,
                    min_score,
                )
                .await
                {
                    Ok(r) => r,
                    Err(err) => {
                        warn!(
                            error = %err,
                            title = %title.name,
                            language = %lang_pref.code,
                            "subtitle search failed"
                        );
                        continue;
                    }
                };

                // Filter blocklisted results
                let mut filtered_results = Vec::new();
                for r in &results {
                    let blocklisted = app
                        .services
                        .workflow
                        .subtitle_downloads
                        .is_blocklisted(&mf.id, &r.provider, &r.provider_file_id)
                        .await
                        .unwrap_or(false);
                    if !blocklisted {
                        filtered_results.push(r);
                    }
                }

                // Pick the best result above min_score
                let best = filtered_results
                    .iter()
                    .filter(|r| r.score >= min_score)
                    .filter(|r| r.hearing_impaired == lang_pref.hearing_impaired)
                    .filter(|r| r.forced == lang_pref.forced)
                    .max_by_key(|r| r.score)
                    .copied();

                let best = match best {
                    Some(b) => b,
                    None => {
                        debug!(
                            title = %title.name,
                            language = %lang_pref.code,
                            results = results.len(),
                            "no subtitle above min_score"
                        );
                        continue;
                    }
                };

                // Download and save
                let download_provider =
                    match runtime_subtitle_provider_for_download(app, &settings, &best.provider)
                        .await
                    {
                        Ok(provider) => provider,
                        Err(err) => {
                            warn!(
                                error = %err,
                                title = %title.name,
                                provider = %best.provider,
                                "subtitle provider unavailable for download"
                            );
                            continue;
                        }
                    };
                let Some(scheduler_lease) = admit_subtitle_provider(
                    app,
                    &download_provider,
                    SchedulerIntent::SubtitleDownload,
                    SchedulerOperation::Download,
                    SubtitleAdmissionFailureMode::OmitProvider,
                )
                .await?
                else {
                    continue;
                };

                let download_result = crate::subtitles::download::download_and_save_with_selection(
                    &download_provider,
                    &best.provider_file_id,
                    file_path.as_path(),
                    &best.language,
                    best.forced,
                    best.hearing_impaired,
                    crate::subtitles::download::SubtitleDownloadSelection {
                        episode: query.episode,
                        absolute_episode: query.absolute_episode,
                        archive_provider: app
                            .services
                            .integrations
                            .archive_extractor_plugin_provider
                            .available()
                            .cloned(),
                    },
                )
                .await;
                let scheduler_outcome = match &download_result {
                    Ok(_) => SchedulerFeedbackOutcome::Success,
                    Err(error) => subtitle_scheduler_outcome_for_error(error),
                };
                let scheduler_retry_after = match &download_result {
                    Ok(_) => None,
                    Err(error) => subtitle_retry_after_from_error(error),
                };
                record_subtitle_scheduler_feedback(
                    app,
                    &download_provider,
                    Some(scheduler_lease),
                    scheduler_outcome,
                    scheduler_retry_after,
                    match &download_result {
                        Ok(_) => RateLimitCooldownAction::None,
                        Err(error) => subtitle_cooldown_action_from_error(error),
                    },
                )
                .await;
                match download_result {
                    Ok((dest_path, _file)) => {
                        // Record in database
                        let record = SubtitleDownload {
                            id: scryer_domain::Id::new().0,
                            media_file_id: mf.id.clone(),
                            title_id: title.id.clone(),
                            episode_id: mf.episode_id.clone(),
                            source_kind: ExternalSubtitleSourceKind::Downloaded,
                            language: best.language.clone(),
                            provider: Some(best.provider.clone()),
                            provider_file_id: Some(best.provider_file_id.clone()),
                            file_path: path_to_stored_string(&dest_path),
                            score: Some(best.score),
                            hearing_impaired: best.hearing_impaired,
                            forced: best.forced,
                            ai_translated: best.ai_translated,
                            machine_translated: best.machine_translated,
                            uploader: best.uploader.clone(),
                            release_info: best.release_info.clone(),
                            synced: false,
                            downloaded_at: Utc::now().to_rfc3339(),
                        };

                        let record_inserted = match app
                            .services
                            .workflow
                            .subtitle_downloads
                            .insert(&record)
                            .await
                        {
                            Ok(()) => true,
                            Err(err) => {
                                warn!(error = %err, "failed to persist subtitle download record");
                                false
                            }
                        };

                        let sync_result = maybe_sync_downloaded_subtitle(
                            app,
                            sync_settings,
                            query.media_kind,
                            file_path.as_path(),
                            &mf.id,
                            &dest_path,
                            record_inserted.then_some(record.id.as_str()),
                            Some(best.score),
                            best.forced,
                        )
                        .await;

                        downloaded += 1;
                        let event_msg = format!(
                            "{} subtitle downloaded for {} (score: {}, provider: {}){}",
                            lang_pref.code,
                            title.name,
                            best.score,
                            best.provider,
                            sync_summary_suffix(sync_result.as_ref()),
                        );
                        info!(
                            title = %title.name,
                            language = %lang_pref.code,
                            provider = %best.provider,
                            score = best.score,
                            path = %dest_path.display(),
                            sync = sync_summary_suffix(sync_result.as_ref()),
                            "subtitle downloaded"
                        );
                        let _ = event_msg;
                        app.emit_subtitle_downloaded_event(
                            title,
                            Some(dest_path.display().to_string()),
                            Some(lang_pref.code.clone()),
                            Some(best.provider.clone()),
                        )
                        .await;
                    }
                    Err(err) => {
                        let event_msg = format!(
                            "{} subtitle download failed for {}: {}",
                            lang_pref.code, title.name, err,
                        );
                        warn!(
                            error = %err,
                            title = %title.name,
                            language = %lang_pref.code,
                            "subtitle download failed"
                        );
                        let _ = event_msg;
                        app.emit_subtitle_search_failed_event(
                            title,
                            Some(lang_pref.code.clone()),
                            Some(err.to_string()),
                        )
                        .await;
                    }
                }

                // Rate limiting: small delay between provider requests
                tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
            }
        }
    }

    debug!(searched, downloaded, "subtitle search cycle completed");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use scryer_domain::{ExternalId, MediaFacet, Title};

    #[test]
    fn subtitle_minimum_settings_are_percentages_converted_to_raw_scores() {
        assert_eq!(
            subtitle_minimum_raw_score(SubtitleMediaKind::Episode, 90, 70),
            324
        );
        assert_eq!(
            subtitle_minimum_raw_score(SubtitleMediaKind::Movie, 90, 70),
            84
        );
    }

    #[test]
    fn subtitle_sync_threshold_settings_are_percentages_converted_to_raw_scores() {
        let settings = SubtitleSyncSettings {
            enabled: true,
            threshold_series: 90,
            threshold_movie: 70,
            max_offset_seconds: 60,
        };

        assert_eq!(settings.threshold_for(SubtitleMediaKind::Episode), 324);
        assert_eq!(settings.threshold_for(SubtitleMediaKind::Movie), 84);
    }

    #[test]
    fn subtitle_retry_after_parser_extracts_seconds_from_rate_limit_errors() {
        let error =
            AppError::Repository("subtitle provider rate limited, retry after 42s".to_string());

        assert_eq!(
            subtitle_retry_after_from_error(&error),
            Some(Duration::from_secs(42))
        );
        assert_eq!(
            subtitle_scheduler_outcome_for_error(&error),
            SchedulerFeedbackOutcome::RateLimited
        );
        assert_eq!(
            subtitle_cooldown_action_from_error(&error),
            RateLimitCooldownAction::RecordFallback
        );
    }

    fn sample_title(facet: MediaFacet, year: Option<i32>) -> Title {
        Title {
            id: "title-1".into(),
            name: "Canonical Title".into(),
            library_id: scryer_domain::default_library_id_for_facet(&facet),
            root_folder_id: scryer_domain::root_folder_id_for_path("/data/test"),
            facet,
            monitored: true,
            tags: vec![],
            canonical_tags: vec![],
            external_ids: vec![ExternalId {
                source: "imdb".into(),
                value: "tt7654321".into(),
            }],
            created_by: None,
            created_at: Utc::now(),
            year,
            overview: None,
            poster_url: None,
            poster_source_url: None,
            background_url: None,
            background_source_url: None,
            sort_title: None,
            catalog_sort_key: String::new(),
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
            aliases: vec!["Canonical Alt".into()],
            tagged_aliases: vec![],
            metadata_language: None,
            metadata_fetched_at: None,
            min_availability: None,
            digital_release_date: None,
            folder_path: None,
        }
    }

    fn sample_media_file(
        scene_name: Option<&str>,
        grabbed_release_title: Option<&str>,
    ) -> crate::TitleMediaFile {
        crate::TitleMediaFile {
            id: "mf-1".into(),
            title_id: "title-1".into(),
            episode_id: Some("episode-1".into()),
            series_movie_link_ids: Vec::new(),
            role: crate::MediaFileRole::Primary,
            file_path: "/tmp/video.mkv".into(),
            size_bytes: 1024,
            announced_size_bytes: None,
            source_signature_scheme: None,
            source_signature_value: None,
            content_hashes: None,
            quality_label: None,
            scan_status: "scanned".into(),
            created_at: Utc::now().to_rfc3339(),
            video_codec: None,
            video_width: None,
            video_height: None,
            video_bitrate_kbps: None,
            video_bit_depth: None,
            video_hdr_format: None,
            dovi_profile: None,
            dovi_bl_compat_id: None,
            video_frame_rate: None,
            video_profile: None,
            audio_codec: None,
            audio_profile: None,
            audio_channels: None,
            audio_bitrate_kbps: None,
            audio_languages: vec![],
            audio_streams: vec![],
            subtitle_languages: vec![],
            subtitle_codecs: vec![],
            subtitle_streams: vec![],
            has_multiaudio: false,
            duration_seconds: None,
            num_chapters: None,
            container_format: None,
            scene_name: scene_name.map(str::to_string),
            release_group: Some("fallback-group".into()),
            source_type: Some("BluRay".into()),
            resolution: Some("720p".into()),
            video_codec_parsed: Some(
                crate::release_parser::VideoCodec::parse("x265").expect("parse codec"),
            ),
            audio_codec_parsed: Some("DTS".into()),
            audio_channels_parsed: None,
            acquisition_score: None,
            scoring_log: None,
            indexer_source: None,
            grabbed_release_title: grabbed_release_title.map(str::to_string),
            grabbed_at: None,
            edition: None,
            original_file_path: None,
            release_hash: None,
        }
    }

    #[test]
    fn subtitle_external_ids_prefers_scoped_anime_ids_over_title_level_ids() {
        let mut title = sample_title(MediaFacet::Anime, None);
        title.external_ids.push(ExternalId {
            source: "anilist".into(),
            value: "161645".into(),
        });
        title.external_ids.push(ExternalId {
            source: "mal".into(),
            value: "54492".into(),
        });

        let episode_context = SubtitleEpisodeContext {
            season: Some(2),
            episode: Some(23),
            absolute_episode: Some(47),
            external_ids: BTreeMap::from([
                ("anilist".into(), vec!["176301".into()]),
                ("mal".into(), vec!["58514".into()]),
                ("tvdb".into(), vec!["1234567".into()]),
            ]),
        };

        let ids = subtitle_external_ids(&title, &episode_context);

        assert_eq!(
            ids.get("anilist").cloned().unwrap_or_default(),
            vec!["176301".to_string()]
        );
        assert_eq!(
            ids.get("mal").cloned().unwrap_or_default(),
            vec!["58514".to_string()]
        );
        assert_eq!(
            ids.get("tvdb").cloned().unwrap_or_default(),
            vec!["1234567".to_string()]
        );
        assert_eq!(
            ids.get("imdb").cloned().unwrap_or_default(),
            vec!["tt7654321".to_string()]
        );
    }

    #[test]
    fn build_subtitle_query_prefers_scene_release_metadata_over_grabbed_and_stored_fields() {
        let scene_name = "Silver.and.Sage.S01E01.1080p.WEB-DL.DDP5.1.H.264-NTb";
        let grabbed = "Wrong.Show.S01E01.720p.BluRay.x265-Different";
        let parsed = parse_release_metadata(scene_name);
        let title = sample_title(MediaFacet::Series, None);
        let media_file = sample_media_file(Some(scene_name), Some(grabbed));
        let episode_context = SubtitleEpisodeContext {
            season: Some(2),
            episode: Some(5),
            absolute_episode: Some(17),
            external_ids: BTreeMap::new(),
        };

        let query = build_subtitle_query(
            &title,
            &media_file,
            &episode_context,
            &["eng".into()],
            Some(false),
            false,
            false,
        );

        assert_eq!(query.media_kind, SubtitleMediaKind::Episode);
        assert_eq!(query.title_candidates, release_title_candidates(&parsed));
        assert_eq!(query.release_group, parsed.release_group);
        assert_eq!(
            query.source,
            parsed.source.as_ref().map(ToString::to_string)
        );
        assert_eq!(
            query.video_codec,
            parsed.video_codec.as_ref().map(ToString::to_string)
        );
        assert_eq!(query.audio_codec, release_audio_codec(&parsed));
        assert_eq!(query.resolution, parsed.quality);
        assert_eq!(query.season, Some(2));
        assert_eq!(query.episode, Some(5));
        assert_eq!(query.absolute_episode, Some(17));
        assert_eq!(query.year, parsed.year);
    }

    #[test]
    fn build_subtitle_query_uses_grabbed_release_title_when_scene_name_is_missing() {
        let grabbed = "Silver.and.Sage.S01E01.1080p.WEB-DL.DDP5.1.H.264-NTb";
        let parsed = parse_release_metadata(grabbed);
        let title = sample_title(MediaFacet::Series, None);
        let media_file = sample_media_file(None, Some(grabbed));

        let query = build_subtitle_query(
            &title,
            &media_file,
            &SubtitleEpisodeContext::default(),
            &["eng".into()],
            None,
            false,
            false,
        );

        assert_eq!(query.title_candidates, release_title_candidates(&parsed));
        assert_eq!(query.release_group, parsed.release_group);
        assert_eq!(
            query.source,
            parsed.source.as_ref().map(ToString::to_string)
        );
        assert_eq!(
            query.video_codec,
            parsed.video_codec.as_ref().map(ToString::to_string)
        );
        assert_eq!(query.audio_codec, release_audio_codec(&parsed));
        assert_eq!(query.resolution, parsed.quality);
        assert_eq!(query.season, Some(1));
        assert_eq!(query.episode, Some(1));
    }

    #[test]
    fn build_subtitle_query_uses_media_analysis_when_release_labels_are_missing() {
        let scene_name = "Movie.Title.2024.BluRay-Group";
        let title = sample_title(MediaFacet::Movie, None);
        let mut media_file = sample_media_file(Some(scene_name), None);
        media_file.resolution = None;
        media_file.video_codec_parsed = None;
        media_file.audio_codec_parsed = None;
        media_file.video_codec =
            Some(crate::release_parser::VideoCodec::parse("h264").expect("parse codec"));
        media_file.video_height = Some(1080);
        media_file.audio_codec = Some("aac".into());
        media_file.audio_profile = Some("LC".into());

        let query = build_subtitle_query(
            &title,
            &media_file,
            &SubtitleEpisodeContext::default(),
            &["eng".into()],
            None,
            false,
            false,
        );

        assert_eq!(query.video_codec.as_deref(), Some("H.264"));
        assert_eq!(query.audio_codec.as_deref(), Some("AAC"));
        assert_eq!(query.resolution.as_deref(), Some("1080p"));
    }

    #[test]
    fn build_subtitle_query_keeps_movie_imdb_id_and_falls_back_to_release_year() {
        let scene_name = "Movie.Title.2024.1080p.WEB-DL.H.264-Group";
        let parsed = parse_release_metadata(scene_name);
        let title = sample_title(MediaFacet::Movie, None);
        let media_file = sample_media_file(Some(scene_name), None);

        let query = build_subtitle_query(
            &title,
            &media_file,
            &SubtitleEpisodeContext::default(),
            &["eng".into()],
            None,
            false,
            false,
        );

        assert_eq!(query.media_kind, SubtitleMediaKind::Movie);
        assert_eq!(query.imdb_id.as_deref(), Some("tt7654321"));
        assert!(query.series_imdb_id.is_none());
        assert_eq!(query.year, parsed.year);
    }
}

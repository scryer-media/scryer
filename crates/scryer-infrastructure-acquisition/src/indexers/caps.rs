use std::collections::BTreeMap;
use std::io::Cursor;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use scryer_application::{
    AppError, AppResult, CapturedIndexerHttpResponse, EstimatedCost, ExpectedValueHint,
    IndexerCapsSnapshotRefresher, IndexerRequestResult, NullUpstreamScheduler,
    RateLimitCooldownAction, RateLimitSignal, SchedulerAdmission, SchedulerBatchRequest,
    SchedulerCandidate, SchedulerCandidateId, SchedulerFeedback, SchedulerFeedbackOutcome,
    SchedulerIntent, SchedulerLease, SchedulerOperation, SchedulerPluginKind, UpstreamScheduler,
};
use scryer_domain::{
    IndexerCapsSearchNode, IndexerCapsSnapshot, IndexerCategoryDescriptor, IndexerCategoryModel,
    IndexerCategoryValueKind, IndexerConfig, ProxyConfig,
};
use scryer_outbound_http::{
    DestinationKey, HostKey, OutboundHttpClient, OutboundHttpError, RateLimitRegistry,
    RequestPolicy, indexer_reqwest_client,
};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

const DIRECT_NAB_CAPS_USER_AGENT: &str = "scryer-indexer-caps/0.1";

#[derive(Debug, Clone)]
struct DirectNabConfig {
    base_url: String,
    api_key: Option<String>,
    api_path: String,
    additional_params: Option<String>,
}

impl DirectNabConfig {
    fn from_indexer_config(config: &IndexerConfig) -> AppResult<Self> {
        let value = config
            .config_json
            .as_deref()
            .map(serde_json::from_str::<Value>)
            .transpose()
            .map_err(|error| {
                AppError::Validation(format!("indexer config_json is invalid: {error}"))
            })?
            .unwrap_or(Value::Null);

        let raw_base_url = value
            .get("base_url")
            .and_then(Value::as_str)
            .or_else(|| (!config.base_url.trim().is_empty()).then_some(config.base_url.as_str()))
            .unwrap_or_default()
            .trim();
        let normalized_connection = normalize_direct_nab_connection_url(raw_base_url);
        let base_url = normalized_connection
            .as_ref()
            .map(|parts| parts.base_url.clone())
            .unwrap_or_else(|| raw_base_url.trim_end_matches('/').to_string());
        if base_url.is_empty() {
            return Err(AppError::Validation(
                "indexer caps refresh requires a base_url".into(),
            ));
        }

        let api_key = value
            .get("api_key")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let api_path = normalized_connection
            .as_ref()
            .and_then(|parts| parts.api_path.clone())
            .or_else(|| {
                value
                    .get("api_path")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
            })
            .unwrap_or("/api".to_string());
        let additional_params = merge_additional_params(
            normalized_connection
                .as_ref()
                .and_then(|parts| parts.additional_params.as_deref()),
            value.get("additional_params").and_then(Value::as_str),
        );

        Ok(Self {
            base_url,
            api_key,
            api_path,
            additional_params,
        })
    }

    fn caps_url(&self) -> AppResult<String> {
        let normalized_path = if self.api_path.trim().is_empty() {
            "/api".to_string()
        } else if self.api_path.starts_with('/') {
            self.api_path.trim().to_string()
        } else {
            format!("/{}", self.api_path.trim())
        };
        let endpoint = format!("{}{}", self.base_url.trim_end_matches('/'), normalized_path);
        let mut url = reqwest::Url::parse(&endpoint).map_err(|error| {
            AppError::Validation(format!("indexer caps base_url is invalid: {error}"))
        })?;
        {
            let mut pairs = url.query_pairs_mut();
            pairs.append_pair("t", "caps");
            if let Some(api_key) = self.api_key.as_deref() {
                pairs.append_pair("apikey", api_key);
            }
            if let Some(additional_params) = self.additional_params.as_deref() {
                for (key, value) in url::form_urlencoded::parse(
                    additional_params
                        .trim()
                        .trim_start_matches(['?', '&'])
                        .as_bytes(),
                ) {
                    let key = key.trim();
                    if key.is_empty() || is_direct_nab_control_query_key(key) {
                        continue;
                    }
                    pairs.append_pair(key, value.trim());
                }
            }
        }
        Ok(url.to_string())
    }
}

#[derive(Debug, PartialEq, Eq)]
struct NormalizedDirectNabConnection {
    base_url: String,
    api_path: Option<String>,
    additional_params: Option<String>,
}

fn normalize_direct_nab_connection_url(raw: &str) -> Option<NormalizedDirectNabConnection> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let Ok(url) = reqwest::Url::parse(trimmed) else {
        return Some(NormalizedDirectNabConnection {
            base_url: trimmed.trim_end_matches('/').to_string(),
            api_path: None,
            additional_params: None,
        });
    };

    let mut normalized = url.clone();
    normalized.set_query(None);
    normalized.set_fragment(None);
    normalized.set_path("");

    let origin = normalized.to_string().trim_end_matches('/').to_string();
    if origin.is_empty() {
        return None;
    }

    let api_path = {
        let trimmed = url.path().trim().trim_matches('/');
        (!trimmed.is_empty()).then(|| format!("/{}", trimmed))
    };

    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    for (key, value) in url.query_pairs() {
        let key = key.trim();
        if key.is_empty() || is_direct_nab_control_query_key(key) {
            continue;
        }
        serializer.append_pair(key, value.trim());
    }
    let serialized_params = serializer.finish();
    let additional_params = (!serialized_params.is_empty()).then_some(serialized_params);

    Some(NormalizedDirectNabConnection {
        base_url: origin,
        api_path,
        additional_params,
    })
}

fn normalize_additional_params(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_start_matches(['?', '&']).trim();
    if trimmed.is_empty() {
        return None;
    }

    let pairs = url::form_urlencoded::parse(trimmed.as_bytes()).collect::<Vec<_>>();
    if pairs.is_empty() {
        return None;
    }

    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    for (key, value) in pairs {
        serializer.append_pair(&key, &value);
    }

    let normalized = serializer.finish();
    (!normalized.is_empty()).then_some(normalized)
}

fn merge_additional_params(extracted: Option<&str>, existing: Option<&str>) -> Option<String> {
    if extracted.is_none() {
        return existing.and_then(normalize_additional_params);
    }
    if existing.is_none() {
        return extracted.and_then(normalize_additional_params);
    }

    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    let mut any = false;

    for raw in [extracted, existing].into_iter().flatten() {
        let trimmed = raw.trim().trim_start_matches(['?', '&']).trim();
        if trimmed.is_empty() {
            continue;
        }

        for (key, value) in url::form_urlencoded::parse(trimmed.as_bytes()) {
            let key = key.trim();
            if key.is_empty() {
                continue;
            }
            serializer.append_pair(key, value.trim());
            any = true;
        }
    }

    any.then(|| serializer.finish())
}

/// The client a caps fetch for `config` is sent with.
///
/// `proxy` must be the indexer's own assignment: a caller that hands over a
/// different proxy, or none for an indexer that has one, is refused rather than
/// trusted, because the fallback would be a direct request from Scryer's own
/// address to an indexer the operator put behind a proxy.
fn caps_request_client(
    config: &IndexerConfig,
    proxy: Option<&ProxyConfig>,
    direct: &reqwest::Client,
) -> AppResult<reqwest::Client> {
    if config.proxy_config_id.as_deref() != proxy.map(|proxy| proxy.id.as_str()) {
        return Err(AppError::Validation(format!(
            "caps refresh for indexer {} was not given its assigned proxy; refusing to send it directly",
            config.name
        )));
    }
    scryer_application::transport_proxy::indexer_host_request_client(proxy, || direct.clone())
        .map_err(AppError::Repository)
}

fn is_direct_nab_control_query_key(key: &str) -> bool {
    matches!(
        key.trim().to_ascii_lowercase().as_str(),
        "apikey"
            | "api_key"
            | "key"
            | "token"
            | "t"
            | "q"
            | "cat"
            | "o"
            | "extended"
            | "limit"
            | "offset"
            | "imdbid"
            | "tvdbid"
            | "tmdbid"
            | "season"
            | "ep"
            | "rid"
            | "tvmazeid"
            | "traktid"
            | "doubanid"
            | "imdbtitle"
            | "imdbyear"
            | "genre"
            | "year"
            | "group"
    )
}

#[derive(Clone)]
pub struct DirectNabCapsSnapshotRefresher {
    outbound_http: OutboundHttpClient,
    upstream_scheduler: Arc<dyn UpstreamScheduler>,
    indexer_stats: Arc<dyn scryer_application::IndexerStatsTracker>,
}

impl DirectNabCapsSnapshotRefresher {
    pub fn new() -> Self {
        Self {
            outbound_http: OutboundHttpClient::new(
                indexer_reqwest_client(),
                RateLimitRegistry::indexers(),
            ),
            upstream_scheduler: Arc::new(NullUpstreamScheduler),
            indexer_stats: Arc::new(scryer_application::NullIndexerStatsTracker),
        }
    }

    pub fn with_upstream_scheduler(mut self, scheduler: Arc<dyn UpstreamScheduler>) -> Self {
        self.upstream_scheduler = scheduler;
        self
    }

    pub fn with_indexer_stats_tracker(
        mut self,
        stats: Arc<dyn scryer_application::IndexerStatsTracker>,
    ) -> Self {
        self.indexer_stats = stats;
        self
    }
}

impl Default for DirectNabCapsSnapshotRefresher {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl IndexerCapsSnapshotRefresher for DirectNabCapsSnapshotRefresher {
    async fn fetch_for_config(
        &self,
        config: &IndexerConfig,
        proxy: Option<&ProxyConfig>,
    ) -> AppResult<Option<IndexerCapsSnapshot>> {
        self.fetch_for_config_with_accounting(
            config,
            proxy,
            scryer_application::IndexerAccountingContext::for_config(config).as_ref(),
        )
        .await
    }

    async fn fetch_for_config_with_accounting(
        &self,
        config: &IndexerConfig,
        proxy: Option<&ProxyConfig>,
        accounting: Option<&scryer_application::IndexerAccountingContext>,
    ) -> AppResult<Option<IndexerCapsSnapshot>> {
        if !config.is_direct_nab() {
            return Ok(None);
        }

        let direct_config = DirectNabConfig::from_indexer_config(config)?;
        // Settled before the scheduler is asked, so a caps fetch that cannot
        // take the indexer's route neither sends nor spends an admission.
        let request_client = caps_request_client(config, proxy, self.outbound_http.client())?;
        let (host_key, destination_key) = scheduler_keys_for_caps(&direct_config.base_url, config);
        let candidate_id = SchedulerCandidateId::new();
        let scheduler_decision = self
            .upstream_scheduler
            .admit_batch(SchedulerBatchRequest {
                batch_id: format!("indexer-caps:{}:{}", config.id, candidate_id),
                now: Utc::now(),
                candidates: vec![SchedulerCandidate {
                    candidate_id: candidate_id.clone(),
                    plugin_config_id: Some(config.id.clone()),
                    plugin_kind: SchedulerPluginKind::Maintenance,
                    operation: SchedulerOperation::CapsRefresh,
                    intent: SchedulerIntent::Maintenance,
                    host_key: host_key.clone(),
                    destination_key: destination_key.clone(),
                    account_quota_key: Some(config.id.clone().into()),
                    rss_request_key: None,
                    estimated_cost: EstimatedCost::ONE_API_CALL,
                    expected_value: ExpectedValueHint::NEUTRAL,
                    learning_context: None,
                    deadline_at: None,
                    freshness: None,
                    cancel_token: CancellationToken::new(),
                }],
            })
            .await?;
        let scheduler_lease = match scheduler_decision.decisions.into_iter().next() {
            Some(SchedulerAdmission::Admit { lease, .. }) => Some(lease),
            Some(SchedulerAdmission::Defer {
                reason,
                retry_after,
                ..
            }) => {
                tracing::debug!(
                    indexer = config.name.as_str(),
                    scheduler_reason = ?reason,
                    retry_after_secs = retry_after.map(|delay| delay.as_secs()),
                    "scheduler deferred indexer caps refresh"
                );
                return Ok(None);
            }
            Some(SchedulerAdmission::Skip { reason, .. }) => {
                tracing::debug!(
                    indexer = config.name.as_str(),
                    scheduler_reason = ?reason,
                    "scheduler skipped indexer caps refresh"
                );
                return Ok(None);
            }
            None => return Ok(None),
        };
        let url = direct_config.caps_url()?;
        let tally = scryer_application::IndexerRequestTally::new(
            accounting
                .map(|value| value.indexer_id.clone())
                .unwrap_or_else(|| scryer_application::CONNECTION_TEST_INDEXER_ID.to_string()),
            accounting
                .map(|value| value.indexer_name.clone())
                .unwrap_or_else(|| config.name.clone()),
            scryer_application::IndexerErrorOperation::CapsRefresh,
            self.indexer_stats.clone(),
        );
        let response_result = self
            .outbound_http
            .send_with_rate_limit_and_dispatch_observer(
                RequestPolicy::safe_read(
                    format!("direct_nab_caps:{}", direct_config.base_url),
                    format!(
                        "direct_nab_caps:{}",
                        config.provider_type.trim().to_ascii_lowercase()
                    ),
                )
                .with_max_retries(2)
                .with_backoff(Duration::from_secs(1), Duration::from_secs(15))
                .with_destination_cooldown_key(destination_key.clone()),
                || {
                    request_client
                        .get(url.clone())
                        .header("Accept", "application/xml, text/xml, application/rss+xml")
                        .header("User-Agent", DIRECT_NAB_CAPS_USER_AGENT)
                },
                |_| async {},
                |actual_url| {
                    let admitted = if reqwest::Url::parse(&url)
                        .is_ok_and(|url| url.origin() == actual_url.origin())
                    {
                        tally.try_note_sent(actual_url.as_str())
                    } else {
                        tally.try_note_auxiliary_sent_call("external_redirect")
                    };
                    admitted
                        .then_some(())
                        .ok_or(OutboundHttpError::DispatchRejected)
                },
            )
            .await;
        let response = match response_result {
            Ok(response) => response,
            Err(error) => {
                let (outcome, retry_after, cooldown_action) = match &error {
                    OutboundHttpError::RateLimited(rate_limited) => (
                        SchedulerFeedbackOutcome::RateLimited,
                        rate_limited.retry_after,
                        RateLimitCooldownAction::AlreadyRecorded,
                    ),
                    OutboundHttpError::Transport { .. } | OutboundHttpError::DispatchRejected => (
                        SchedulerFeedbackOutcome::TransportFailure,
                        None,
                        RateLimitCooldownAction::None,
                    ),
                };
                record_caps_scheduler_feedback(
                    self.upstream_scheduler.as_ref(),
                    scheduler_lease.clone(),
                    host_key.clone(),
                    destination_key.clone(),
                    Some(config.id.clone().into()),
                    outcome,
                    retry_after,
                    cooldown_action,
                )
                .await;
                return Err(map_caps_outbound_error(error));
            }
        };

        let status = response.status();
        let body = match response.bytes().await {
            Ok(body) => body,
            Err(error) => {
                tally.note_result(IndexerRequestResult::NoResponse);
                return Err(AppError::Repository(format!(
                    "indexer caps response read failed: {error}"
                )));
            }
        };
        tally.note_response(&CapturedIndexerHttpResponse {
            status: status.as_u16(),
            headers: Vec::new(),
            body: body.to_vec(),
        });
        if !status.is_success() {
            let body_snippet = String::from_utf8_lossy(&body);
            record_caps_scheduler_feedback(
                self.upstream_scheduler.as_ref(),
                scheduler_lease,
                host_key,
                destination_key,
                Some(config.id.clone().into()),
                SchedulerFeedbackOutcome::ProviderFailure,
                None,
                RateLimitCooldownAction::None,
            )
            .await;
            return Err(AppError::Repository(format!(
                "indexer caps request failed with status {}: {}",
                status,
                body_snippet.trim()
            )));
        }

        let snapshot = parse_caps_snapshot_xml(&body).map(Some);
        let rate_limit_signal = snapshot
            .as_ref()
            .err()
            .and_then(RateLimitSignal::from_error);
        record_caps_scheduler_feedback(
            self.upstream_scheduler.as_ref(),
            scheduler_lease,
            host_key,
            destination_key,
            Some(config.id.clone().into()),
            if rate_limit_signal.is_some() {
                SchedulerFeedbackOutcome::RateLimited
            } else if snapshot.is_ok() {
                SchedulerFeedbackOutcome::Success
            } else {
                SchedulerFeedbackOutcome::ProviderFailure
            },
            rate_limit_signal
                .as_ref()
                .and_then(|signal| signal.retry_after),
            rate_limit_signal
                .as_ref()
                .map(|signal| signal.cooldown_action)
                .unwrap_or(RateLimitCooldownAction::None),
        )
        .await;
        snapshot
    }
}

fn scheduler_keys_for_caps(base_url: &str, config: &IndexerConfig) -> (HostKey, DestinationKey) {
    let host_key = reqwest::Url::parse(base_url)
        .ok()
        .and_then(|url| url.host_str().map(HostKey::from))
        .unwrap_or_else(|| {
            let fallback = base_url.trim().trim_end_matches('/').trim().to_string();
            let fallback = if fallback.is_empty() {
                "indexer-caps".to_string()
            } else {
                fallback
            };
            HostKey::from(fallback)
        });
    (
        host_key,
        DestinationKey::from(config.rate_limit_domain_key()),
    )
}

#[allow(clippy::too_many_arguments)]
async fn record_caps_scheduler_feedback(
    scheduler: &dyn UpstreamScheduler,
    lease: Option<SchedulerLease>,
    host_key: HostKey,
    destination_key: DestinationKey,
    account_quota_key: Option<scryer_application::AccountQuotaKey>,
    outcome: SchedulerFeedbackOutcome,
    retry_after: Option<Duration>,
    cooldown_action: RateLimitCooldownAction,
) {
    if let Err(error) = scheduler
        .record_feedback(SchedulerFeedback {
            lease,
            host_key,
            destination_key,
            account_quota_key,
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
        tracing::warn!(error = %error, "failed to record caps scheduler feedback");
    }
}

pub fn parse_caps_snapshot_xml(body: &[u8]) -> AppResult<IndexerCapsSnapshot> {
    let mut reader = Reader::from_reader(Cursor::new(body));
    reader.config_mut().trim_text(true);
    reader.config_mut().expand_empty_elements = true;
    let mut depth = 0_usize;
    let mut saw_root = false;
    let mut saw_declaration = false;
    let mut saw_doctype = false;
    let mut buf = Vec::new();
    let mut snapshot = IndexerCapsSnapshot::default();
    let mut categories = BTreeMap::<String, IndexerCategoryDescriptor>::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(element)) => {
                if depth == 0 {
                    if saw_root || !matches!(element.name().as_ref(), "caps" | "error") {
                        return Err(AppError::Repository(
                            "indexer returned an invalid caps document root".into(),
                        ));
                    }
                    saw_root = true;
                }
                depth += 1;
                for attribute in element.attributes() {
                    attribute.map_err(|error| {
                        AppError::Repository(format!(
                            "indexer returned invalid caps XML attributes: {error}"
                        ))
                    })?;
                }
                match element.name().as_ref() {
                    "error" => return Err(parse_caps_newznab_error(&element)?),
                    "server" => {
                        snapshot.server_title = attr_value(&element, "title")?;
                    }
                    "limits" => {
                        snapshot.limits_default = attr_i64(&element, "default")?;
                        snapshot.limits_max = attr_i64(&element, "max")?;
                    }
                    "search" => {
                        snapshot.search = Some(parse_caps_node(&element)?);
                    }
                    "tv-search" => {
                        snapshot.tv_search = Some(parse_caps_node(&element)?);
                    }
                    "movie-search" => {
                        snapshot.movie_search = Some(parse_caps_node(&element)?);
                    }
                    "music-search" => {
                        snapshot.music_search = Some(parse_caps_node(&element)?);
                    }
                    "audio-search" => {
                        snapshot.audio_search = Some(parse_caps_node(&element)?);
                    }
                    "book-search" => {
                        snapshot.book_search = Some(parse_caps_node(&element)?);
                    }
                    "category" | "subcat" => {
                        if let Some(id) = attr_value(&element, "id")?
                            .map(|value| value.trim().to_string())
                            .filter(|value| !value.is_empty())
                        {
                            let label = attr_value(&element, "name")?
                                .map(|value| value.trim().to_string())
                                .filter(|value| !value.is_empty());
                            categories
                                .entry(id.clone())
                                .and_modify(|existing| {
                                    if existing.label.is_none() && label.is_some() {
                                        existing.label = label.clone();
                                    }
                                })
                                .or_insert_with(|| IndexerCategoryDescriptor {
                                    value: id,
                                    label,
                                    value_kind: IndexerCategoryValueKind::Numeric,
                                    facets: Vec::new(),
                                });
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::End(_)) => {
                depth = depth.checked_sub(1).ok_or_else(|| {
                    AppError::Repository("indexer returned unmatched caps XML closing tag".into())
                })?;
            }
            Ok(Event::Text(text)) if depth == 0 && !text.as_ref().trim().is_empty() => {
                return Err(AppError::Repository(
                    "indexer returned text outside the caps document".into(),
                ));
            }
            Ok(Event::Decl(_)) => {
                if saw_root || saw_declaration || saw_doctype {
                    return Err(AppError::Repository(
                        "indexer returned a misplaced caps XML declaration".into(),
                    ));
                }
                saw_declaration = true;
            }
            Ok(Event::DocType(_)) => {
                if saw_root || saw_doctype {
                    return Err(AppError::Repository(
                        "indexer returned a misplaced caps XML doctype".into(),
                    ));
                }
                saw_doctype = true;
            }
            Ok(Event::CData(_)) if depth == 0 => {
                return Err(AppError::Repository(
                    "indexer returned an invalid caps document".into(),
                ));
            }
            Ok(Event::Eof) => {
                if !saw_root || depth != 0 {
                    return Err(AppError::Repository(
                        "indexer returned an empty or incomplete caps document".into(),
                    ));
                }
                break;
            }
            Err(error) => {
                return Err(AppError::Repository(format!(
                    "indexer returned invalid caps XML: {error}"
                )));
            }
            _ => {}
        }
        buf.clear();
    }

    if !categories.is_empty() {
        snapshot.categories = IndexerCategoryModel {
            value_kinds: vec![IndexerCategoryValueKind::Numeric],
            separate_anime_categories: false,
            provider_category_metadata: true,
            categories: categories.into_values().collect(),
        };
    }

    Ok(snapshot)
}

fn parse_caps_newznab_error(element: &BytesStart<'_>) -> AppResult<AppError> {
    let code = attr_value(element, "code")?
        .ok_or_else(|| {
            AppError::Repository("indexer caps returned a Newznab error without a code".into())
        })?
        .trim()
        .parse::<u16>()
        .map_err(|error| {
            AppError::Repository(format!(
                "indexer caps returned an invalid Newznab error code: {error}"
            ))
        })?;
    let message = attr_value(element, "description")?
        .or(attr_value(element, "message")?)
        .unwrap_or_else(|| format!("Newznab error {code}"));

    if matches!(code, 500 | 501) {
        return Ok(AppError::NewznabQuotaExceeded { code, message });
    }

    Ok(AppError::Repository(format!(
        "indexer caps returned Newznab error {code}: {message}"
    )))
}

fn parse_caps_node(element: &BytesStart<'_>) -> AppResult<IndexerCapsSearchNode> {
    Ok(IndexerCapsSearchNode {
        available: attr_value(element, "available")?
            .is_some_and(|value| value.eq_ignore_ascii_case("yes")),
        supported_params: attr_value(element, "supportedParams")?
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_ascii_lowercase())
            .collect(),
        search_engine: attr_value(element, "searchEngine")?
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
    })
}

fn attr_value(element: &BytesStart<'_>, key: &str) -> AppResult<Option<String>> {
    for attribute in element.attributes().with_checks(false) {
        let attribute = attribute.map_err(|error| {
            AppError::Repository(format!(
                "indexer returned invalid caps XML attributes: {error}"
            ))
        })?;
        if attribute.key.as_ref() == key {
            return Ok(Some(attribute.value.into_owned()));
        }
    }

    Ok(None)
}

fn attr_i64(element: &BytesStart<'_>, key: &str) -> AppResult<Option<i64>> {
    attr_value(element, key)?.map_or(Ok(None), |value| {
        value.trim().parse::<i64>().map(Some).map_err(|error| {
            AppError::Repository(format!(
                "indexer returned invalid numeric caps values: {error}"
            ))
        })
    })
}

fn map_caps_outbound_error(error: OutboundHttpError) -> AppError {
    match error {
        OutboundHttpError::RateLimited(rate_limited) => {
            let retry_after = rate_limited.retry_after.filter(|delay| !delay.is_zero());
            AppError::rate_limited_temporary_unavailable(
                match retry_after {
                    Some(delay) => format!(
                        "indexer caps refresh was rate limited (retry after {}s)",
                        delay.as_secs()
                    ),
                    None => "indexer caps refresh was rate limited".to_string(),
                },
                retry_after,
                RateLimitCooldownAction::AlreadyRecorded,
            )
        }
        OutboundHttpError::DispatchRejected => AppError::canceled("indexer dispatch is closed"),
        OutboundHttpError::Transport { source, .. } => {
            AppError::Repository(format!("indexer caps request failed: {source}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_document_requires_one_complete_caps_root() {
        for invalid in [
            "",
            " ",
            "<html/>",
            "<rss/>",
            "<caps>",
            "<caps><search/></caps",
            "<caps/><caps/>",
            "text<caps/>",
            "<caps/>text",
            "<caps duplicate='1' duplicate='2'/>",
            "<caps><!DOCTYPE caps></caps>",
            "<caps/><!DOCTYPE caps>",
            "<!DOCTYPE caps><!DOCTYPE caps><caps/>",
            "<caps><?xml version='1.0'?></caps>",
            "<caps/><?xml version='1.0'?>",
            "<?xml version='1.0'?><?xml version='1.0'?><caps/>",
        ] {
            assert!(
                parse_caps_snapshot_xml(invalid.as_bytes()).is_err(),
                "accepted {invalid:?}"
            );
        }
        for valid in [
            "<caps/>",
            "<?xml version='1.0'?><!DOCTYPE caps SYSTEM 'https://indexer.example/caps.dtd'><caps/>",
            "<?xml version='1.0'?><caps><server title='Example'/></caps>",
            "<caps><searching><search available='yes' supportedParams='q'/></searching></caps>",
        ] {
            assert!(
                parse_caps_snapshot_xml(valid.as_bytes()).is_ok(),
                "rejected {valid:?}"
            );
        }
    }

    #[test]
    fn caps_outbound_rate_limit_preserves_retry_after() {
        let error = OutboundHttpError::RateLimited(scryer_outbound_http::RateLimitedError {
            scope: scryer_outbound_http::RateLimitScopeKey::from("caps"),
            retry_after: Some(Duration::from_secs(55)),
            attempts: 1,
            retry_after_source: scryer_outbound_http::RetryAfterSource::Seconds,
            request_label: std::borrow::Cow::Borrowed("caps"),
        });
        let error = map_caps_outbound_error(error);

        match error {
            AppError::TemporaryUnavailable {
                message,
                retry_after,
                ..
            } => {
                assert!(message.contains("retry after 55s"));
                assert_eq!(retry_after, Some(Duration::from_secs(55)));
            }
            other => panic!("expected temporary unavailable error, got {other:?}"),
        }
    }

    #[test]
    fn parse_caps_snapshot_xml_preserves_quota_errors_and_rejects_other_newznab_errors() {
        for (code, typed_quota) in [(500, true), (501, true), (910, false), (777, false)] {
            let xml = format!(
                r#"<caps><error code="{code}" description="provider failure {code}" /></caps>"#
            );
            let error = parse_caps_snapshot_xml(xml.as_bytes()).expect_err("caps error document");
            assert_eq!(
                matches!(
                    &error,
                    AppError::NewznabQuotaExceeded { code: actual, .. } if *actual == code
                ),
                typed_quota,
                "Newznab code {code} should {} be a typed quota error",
                if typed_quota { "" } else { "not" },
            );
            if !typed_quota {
                assert!(
                    error.to_string().contains(&format!("Newznab error {code}")),
                    "unexpected error: {error}"
                );
            }
        }
    }

    #[test]
    fn parse_caps_snapshot_xml_parses_search_nodes_limits_and_categories() {
        let xml = br#"
            <caps>
              <server title="Synthetic Indexer" />
              <limits default="100" max="250" />
              <searching>
                <search available="yes" supportedParams="q" />
                <tv-search available="yes" supportedParams="q,season,ep,tvdbid,rid,tvmazeid" />
                <movie-search available="yes" supportedParams="q,imdbid,genre" searchEngine="raw" />
                <music-search available="no" supportedParams="q" />
                <audio-search available="yes" supportedParams="q" />
                <book-search available="no" supportedParams="q" />
              </searching>
              <categories>
                <category id="2000" name="Movies">
                  <subcat id="2010" name="Movies HD" />
                </category>
              </categories>
            </caps>
        "#;

        let snapshot = parse_caps_snapshot_xml(xml).expect("caps xml should parse");

        assert_eq!(snapshot.server_title.as_deref(), Some("Synthetic Indexer"));
        assert_eq!(snapshot.limits_default, Some(100));
        assert_eq!(snapshot.limits_max, Some(250));
        assert_eq!(
            snapshot
                .movie_search
                .as_ref()
                .expect("movie search node")
                .supported_params,
            vec!["q", "imdbid", "genre"]
        );
        assert_eq!(
            snapshot
                .tv_search
                .as_ref()
                .expect("tv search node")
                .supported_params,
            vec!["q", "season", "ep", "tvdbid", "rid", "tvmazeid"]
        );
        assert_eq!(
            snapshot
                .movie_search
                .as_ref()
                .expect("movie search node")
                .search_engine
                .as_deref(),
            Some("raw")
        );
        assert_eq!(
            snapshot
                .categories
                .categories
                .iter()
                .map(|category| (category.value.clone(), category.label.clone()))
                .collect::<Vec<_>>(),
            vec![
                ("2000".to_string(), Some("Movies".to_string())),
                ("2010".to_string(), Some("Movies HD".to_string()))
            ]
        );
    }

    #[test]
    fn parse_caps_snapshot_xml_lowercases_supported_params_and_respects_availability() {
        let xml = br#"
            <caps>
              <searching>
                <movie-search available="no" supportedParams="Q,TMDBID,IMDbId" />
              </searching>
            </caps>
        "#;

        let snapshot = parse_caps_snapshot_xml(xml).expect("caps xml should parse");
        let movie = snapshot.movie_search.expect("movie search node");

        assert!(!movie.available);
        assert_eq!(movie.supported_params, vec!["q", "tmdbid", "imdbid"]);
    }

    #[test]
    fn direct_nab_config_canonicalizes_query_bearing_connection_urls_for_caps() {
        let config = IndexerConfig {
            id: "cfg-1".to_string(),
            name: "NZBGeek".to_string(),
            provider_type: "nzbgeek".to_string(),
            base_url: "https://api.nzbgeek.info/api?t=search&q=legacy".to_string(),
            api_key_encrypted: None,
            rate_limit_seconds: None,
            rate_limit_burst: None,
            max_queries_per_minute: None,
            disabled_until: None,
            is_enabled: true,
            enable_interactive_search: true,
            enable_auto_search: true,
            proxy_config_id: None,
            download_client_id: None,
            seeding_profile_id: None,
            managed_parent_config_id: None,
            managed_child_key: None,
            managed_metadata_json: None,
            caps_snapshot_json: None,
            last_health_status: None,
            last_error_message: None,
            last_error_at: None,
            config_json: Some(
                serde_json::json!({
                    "base_url": "https://api.nzbgeek.info/api?t=search&q=legacy&attrs=poster&apikey=test-key",
                    "api_key": "test-key",
                    "api_path": "/api",
                    "additional_params": "lang=en",
                })
                .to_string(),
            ),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        let direct = DirectNabConfig::from_indexer_config(&config).expect("direct config");
        assert_eq!(direct.base_url, "https://api.nzbgeek.info");
        assert_eq!(direct.api_path, "/api");
        assert_eq!(
            direct.additional_params.as_deref(),
            Some("attrs=poster&lang=en")
        );
        assert_eq!(
            direct.caps_url().expect("caps url"),
            "https://api.nzbgeek.info/api?t=caps&apikey=test-key&attrs=poster&lang=en"
        );
    }

    #[tokio::test]
    async fn direct_nab_caps_refresher_ignores_prowlarr_proxy_configs() {
        let config = IndexerConfig {
            id: "proxy-1".to_string(),
            name: "Prowlarr Proxy".to_string(),
            provider_type: "newznab".to_string(),
            base_url: "http://localhost:9696/1".to_string(),
            api_key_encrypted: None,
            rate_limit_seconds: None,
            rate_limit_burst: None,
            max_queries_per_minute: None,
            disabled_until: None,
            is_enabled: true,
            enable_interactive_search: true,
            enable_auto_search: true,
            proxy_config_id: None,
            download_client_id: None,
            seeding_profile_id: None,
            managed_parent_config_id: Some("parent".to_string()),
            managed_child_key: Some("child".to_string()),
            managed_metadata_json: None,
            caps_snapshot_json: None,
            last_health_status: None,
            last_error_message: None,
            last_error_at: None,
            config_json: Some(
                serde_json::json!({
                    "base_url": "http://localhost:9696/1",
                    "api_key": "test-key",
                    "api_path": "/api",
                })
                .to_string(),
            ),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        let refresher = DirectNabCapsSnapshotRefresher::new();
        let snapshot = refresher
            .fetch_for_config(&config, None)
            .await
            .expect("proxy configs should be ignored");
        assert!(snapshot.is_none());
    }

    const PROXIED_CAPS_XML: &str = r#"<caps><server title="Through The Proxy" /><searching><search available="yes" supportedParams="q" /></searching></caps>"#;

    /// A direct Newznab indexer pointed at `origin`, assigned `proxy_config_id`.
    fn direct_newznab_config(origin: &str, proxy_config_id: Option<&str>) -> IndexerConfig {
        IndexerConfig {
            id: "cfg-proxied-caps".to_string(),
            name: "Proxied Newznab".to_string(),
            provider_type: "newznab".to_string(),
            base_url: origin.to_string(),
            api_key_encrypted: None,
            rate_limit_seconds: None,
            rate_limit_burst: None,
            max_queries_per_minute: None,
            disabled_until: None,
            is_enabled: true,
            enable_interactive_search: true,
            enable_auto_search: true,
            proxy_config_id: proxy_config_id.map(str::to_string),
            download_client_id: None,
            seeding_profile_id: None,
            managed_parent_config_id: None,
            managed_child_key: None,
            managed_metadata_json: None,
            caps_snapshot_json: None,
            last_health_status: None,
            last_error_message: None,
            last_error_at: None,
            config_json: Some(
                serde_json::json!({ "base_url": origin, "api_key": "test-key" }).to_string(),
            ),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    fn proxy_config(
        id: &str,
        provider_type: scryer_domain::ProxyProviderType,
        base_url: &str,
    ) -> ProxyConfig {
        let now = chrono::Utc::now();
        ProxyConfig {
            id: id.to_string(),
            name: "House Proxy".to_string(),
            provider_type,
            protocol: None,
            base_url: base_url.to_string(),
            request_timeout_seconds: 5,
            is_enabled: true,
            username_encrypted: None,
            password_encrypted: None,
            remote_dns: false,
            last_health_status: None,
            last_error_message: None,
            last_error_at: None,
            created_at: now,
            updated_at: now,
            host_key_fingerprint: None,
            host_key_pinned_at: None,
            private_key_encrypted: None,
            private_key_passphrase_encrypted: None,
            peer_public_key: None,
            preshared_key_encrypted: None,
            tunnel_public_key: None,
            tunnel_addresses: Vec::new(),
            tunnel_dns_servers: Vec::new(),
            tunnel_mtu: None,
            tunnel_keepalive_seconds: None,
        }
    }

    /// The indexer itself. Anything it receives arrived directly from Scryer,
    /// which is exactly what a proxied indexer must never see.
    async fn caps_origin() -> wiremock::MockServer {
        let origin = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(PROXIED_CAPS_XML))
            .mount(&origin)
            .await;
        origin
    }

    /// Minimal HTTP forward proxy: records each absolute-form request line and
    /// answers with a caps document itself, without contacting the origin.
    async fn spawn_recording_http_proxy() -> (String, Arc<std::sync::Mutex<Vec<String>>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("proxy double should bind");
        let address = listener.local_addr().expect("proxy double address");
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorder = Arc::clone(&seen);
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let recorder = Arc::clone(&recorder);
                tokio::spawn(async move {
                    let mut buffer = vec![0u8; 4096];
                    let mut received = Vec::new();
                    while !received.windows(4).any(|window| window == b"\r\n\r\n") {
                        match stream.read(&mut buffer).await {
                            Ok(0) | Err(_) => return,
                            Ok(read) => received.extend_from_slice(&buffer[..read]),
                        }
                    }
                    if let Some(line) = String::from_utf8_lossy(&received).lines().next() {
                        recorder.lock().unwrap().push(line.to_string());
                    }
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{PROXIED_CAPS_XML}",
                        PROXIED_CAPS_XML.len()
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                    let _ = stream.flush().await;
                });
            }
        });
        (format!("http://{address}"), seen)
    }

    /// The e2e proxy matrix caught Scryer's own address in a proxied indexer's
    /// log: the caps request went straight out while the searches went through
    /// the proxy. The caps request is indexer traffic like any other, so the
    /// operator's proxy carries it.
    #[tokio::test]
    async fn caps_for_a_proxied_indexer_travel_through_its_transport_proxy() {
        let origin = caps_origin().await;
        let (proxy_url, proxy_requests) = spawn_recording_http_proxy().await;
        let proxy = proxy_config(
            "house-proxy",
            scryer_domain::ProxyProviderType::Http,
            &proxy_url,
        );
        let config = direct_newznab_config(&origin.uri(), Some("house-proxy"));

        let snapshot = DirectNabCapsSnapshotRefresher::new()
            .fetch_for_config(&config, Some(&proxy))
            .await
            .expect("the proxy answers the caps request")
            .expect("a direct Newznab indexer yields a snapshot");

        assert_eq!(snapshot.server_title.as_deref(), Some("Through The Proxy"));
        let proxy_requests = proxy_requests.lock().unwrap().clone();
        assert_eq!(proxy_requests.len(), 1, "{proxy_requests:?}");
        assert!(
            proxy_requests[0].starts_with(&format!("GET {}/api?t=caps", origin.uri())),
            "the proxy must be asked for the indexer's caps: {proxy_requests:?}"
        );
        assert!(
            origin
                .received_requests()
                .await
                .unwrap_or_default()
                .is_empty(),
            "the indexer must never be reached directly"
        );
    }

    /// Fail closed: a proxy that cannot carry the request (disabled, or not
    /// buildable into a client) fails the refresh. It never degrades into a
    /// direct request.
    #[tokio::test]
    async fn caps_fail_closed_when_the_assigned_proxy_cannot_carry_them() {
        let origin = caps_origin().await;
        let config = direct_newznab_config(&origin.uri(), Some("house-proxy"));

        let unbuildable = proxy_config(
            "house-proxy",
            scryer_domain::ProxyProviderType::Http,
            "not a proxy url",
        );
        let mut disabled = proxy_config(
            "house-proxy",
            scryer_domain::ProxyProviderType::Socks5,
            "socks5://127.0.0.1:1",
        );
        disabled.is_enabled = false;

        for proxy in [unbuildable, disabled] {
            let error = DirectNabCapsSnapshotRefresher::new()
                .fetch_for_config(&config, Some(&proxy))
                .await
                .expect_err("a proxy that cannot carry caps must fail the refresh");
            assert!(
                error.to_string().contains("House Proxy"),
                "the failure must name the proxy: {error}"
            );
        }
        // The same holds when the caller hands over no proxy at all for an
        // indexer that has one assigned, or a different proxy than assigned.
        DirectNabCapsSnapshotRefresher::new()
            .fetch_for_config(&config, None)
            .await
            .expect_err("an assigned proxy that was not resolved must not go direct");
        let other = proxy_config(
            "other-proxy",
            scryer_domain::ProxyProviderType::Http,
            "http://127.0.0.1:1",
        );
        DirectNabCapsSnapshotRefresher::new()
            .fetch_for_config(&config, Some(&other))
            .await
            .expect_err("a proxy other than the assigned one must not be used");

        assert!(
            origin
                .received_requests()
                .await
                .unwrap_or_default()
                .is_empty(),
            "nothing may reach the indexer directly when its proxy is unusable"
        );
    }

    /// A challenge solver is not a hop: the plugin hosts send an indexer's
    /// requests directly and consult the solver only on a challenge page. The
    /// caps request keeps that same behaviour.
    #[tokio::test]
    async fn caps_for_a_solver_assigned_indexer_stay_direct_like_its_searches() {
        let origin = caps_origin().await;
        let mut solver = proxy_config(
            "solver",
            scryer_domain::ProxyProviderType::Byparr,
            "http://127.0.0.1:1",
        );
        solver.protocol = Some(scryer_domain::ChallengeSolverProtocol::RequestSolutionV1);
        let config = direct_newznab_config(&origin.uri(), Some("solver"));

        DirectNabCapsSnapshotRefresher::new()
            .fetch_for_config(&config, Some(&solver))
            .await
            .expect("direct caps request")
            .expect("snapshot");

        assert_eq!(
            origin.received_requests().await.unwrap_or_default().len(),
            1
        );
    }
}

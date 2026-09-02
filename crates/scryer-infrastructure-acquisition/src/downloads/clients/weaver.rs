use async_trait::async_trait;
use base64::Engine as _;
use chrono::TimeZone;
use chrono::{DateTime, Utc};
use reqwest::multipart;
use scryer_application::{
    AppError, AppResult, DownloadClient, DownloadClientAddRequest, DownloadGrabResult,
    NullStagedNzbStore, RateLimitCooldownAction, StagedNzbStore,
};
use scryer_domain::{
    CompletedDownload, DownloadClientConfig, DownloadQueueItem, DownloadQueueState,
};
use scryer_outbound_http::{
    OutboundHttpClient, OutboundHttpError, OutboundRequestError, RateLimitRegistry, RequestPolicy,
    generic_reqwest_client,
};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::borrow::Cow;
use std::sync::Arc;
use std::time::Duration;
use tokio::fs::File;
use tokio::sync::Semaphore;
use tokio_util::io::ReaderStream;
use tracing::{debug, warn};

use super::{
    parse_download_client_config_json, read_config_string, resolve_download_client_base_url,
    resolve_staged_nzb_for_request,
};
use crate::graphql::weaver as graphql_docs;

#[derive(Clone)]
pub struct WeaverDownloadClient {
    graphql_url: String,
    api_key: Option<String>,
    outbound_http: OutboundHttpClient,
    staged_nzb_store: Arc<dyn StagedNzbStore>,
    staged_nzb_pipeline_limit: Arc<Semaphore>,
}

#[derive(Debug, Deserialize)]
struct GraphqlResponse<T> {
    data: Option<T>,
    errors: Option<Vec<GraphqlError>>,
}

#[derive(Debug, Deserialize)]
struct GraphqlError {
    message: String,
}

struct GraphqlMultipartUploadRequest<'a> {
    request_label: &'static str,
    query: &'a str,
    variables: Value,
    upload_variable_path: &'a str,
    filename: String,
    upload_path: &'a std::path::Path,
    content_type: &'a str,
    content_length: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WeaverAttribute {
    pub key: String,
    pub value: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WeaverAttention {
    pub message: String,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
pub enum WeaverQueueState {
    #[serde(rename = "QUEUED")]
    Queued,
    #[serde(rename = "DOWNLOADING")]
    Downloading,
    #[serde(rename = "CHECKING")]
    Checking,
    #[serde(rename = "VERIFYING")]
    Verifying,
    #[serde(rename = "QUEUED_REPAIR")]
    QueuedRepair,
    #[serde(rename = "REPAIRING")]
    Repairing,
    #[serde(rename = "QUEUED_EXTRACT")]
    QueuedExtract,
    #[serde(rename = "EXTRACTING")]
    Extracting,
    #[serde(rename = "MOVING")]
    Moving,
    #[serde(rename = "FINALIZING")]
    Finalizing,
    #[serde(rename = "COMPLETE", alias = "COMPLETED")]
    Completed,
    #[serde(rename = "FAILED")]
    Failed,
    #[serde(rename = "PAUSED")]
    Paused,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WeaverQueueItem {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub original_title: Option<String>,
    pub state: WeaverQueueState,
    pub error: Option<String>,
    pub progress_percent: f64,
    pub total_bytes: u64,
    pub category: Option<String>,
    pub attributes: Vec<WeaverAttribute>,
    pub client_request_id: Option<String>,
    pub output_dir: Option<String>,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub attention: Option<WeaverAttention>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SystemMetricsPayload {
    _system_metrics: MinimalMetrics,
}

#[derive(Debug, Deserialize)]
struct VersionPayload {
    #[serde(rename = "version")]
    _version: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MinimalMetrics {
    _bytes_downloaded: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct QueueItemsPayload {
    queue_items: Vec<WeaverQueueItem>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HistoryItemsPayload {
    history_items: Vec<WeaverQueueItem>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HistoryItemPayload {
    history_item: Option<WeaverQueueItem>,
}

const TARGETED_HISTORY_FALLBACK_LIMIT: usize = 100;
const SCRYER_TITLE_ID_ATTRIBUTE_KEY: &str = "*scryer_title_id";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PublishedJobsPayload {
    jobs: Vec<PublishedWeaverJob>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PublishedWeaverJob {
    id: u64,
    name: String,
    #[serde(default)]
    original_title: Option<String>,
    status: WeaverQueueState,
    error: Option<String>,
    progress_percent: f64,
    total_bytes: u64,
    category: Option<String>,
    metadata: Vec<WeaverAttribute>,
    output_dir: Option<String>,
    created_at: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubmissionPayload {
    submit_nzb: SubmissionResultPayload,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubmissionResultPayload {
    accepted: bool,
    /// Weaver's `SubmissionStatus`. Kept as a string rather than an enum so a
    /// Weaver that adds a new variant does not fail deserialization here.
    /// `Option` because older Weavers do not select it (see the compat path).
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    error_code: Option<String>,
    /// Always populated by Weaver alongside a submission outcome, including the
    /// PARKED case that returns no `item`.
    #[serde(default)]
    job_id: Option<u64>,
    item: Option<SubmissionQueueItemPayload>,
}

/// Weaver returns `accepted: false` for EXACTLY ONE status — `IDEMPOTENT_REPLAY`
/// — and that status means the submission already succeeded earlier: the job
/// exists, and Weaver hands back its live `item`/`jobId`. Scryer sends a
/// `clientRequestId` on every submit, so any resubmission of the same release
/// (retry after a blip, RSS re-grab, interactive re-grab) lands here.
///
/// Treating it as a failure loses a grab that actually worked — Scryer never
/// tracks or imports the download while Weaver happily runs it — and the
/// resulting failover can push the same release into a second client as a
/// duplicate.
const WEAVER_STATUS_IDEMPOTENT_REPLAY: &str = "IDEMPOTENT_REPLAY";

/// A semantic-duplicate candidate held pending resolution. Weaver reports
/// `accepted: true` but deliberately returns no `item`, so keying off `item`
/// alone turned this into a hard error too; `jobId` is always populated.
const WEAVER_STATUS_PARKED: &str = "PARKED";

/// Duplicate detection refused the submission outright: an equivalent job
/// (same article layout) already exists on the Weaver side. Terminal for this
/// release — retrying or failing over to another client only duplicates the
/// download.
const WEAVER_STATUS_BLOCKED: &str = "BLOCKED";
const WEAVER_ERROR_DUPLICATE_BLOCKED: &str = "DUPLICATE_BLOCKED";

impl SubmissionResultPayload {
    fn status_is(&self, expected: &str) -> bool {
        self.status
            .as_deref()
            .is_some_and(|status| status.trim().eq_ignore_ascii_case(expected))
    }

    fn error_code_is(&self, expected: &str) -> bool {
        self.error_code
            .as_deref()
            .is_some_and(|code| code.trim().eq_ignore_ascii_case(expected))
    }

    /// The queue item id, falling back to `jobId` for statuses that carry a job
    /// without an item (PARKED).
    fn resolved_job_id(&self) -> Option<u64> {
        self.item.as_ref().map(|item| item.id).or(self.job_id)
    }

    /// Weaver's own reason, for operators. Without this the only thing Scryer
    /// could report was "did not accept the submission", which is why this
    /// defect stayed invisible in logs.
    fn rejection_detail(&self) -> String {
        let mut parts = Vec::new();
        if let Some(status) = self
            .status
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            parts.push(format!("status={status}"));
        }
        if let Some(code) = self
            .error_code
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            parts.push(format!("errorCode={code}"));
        }
        if let Some(message) = self
            .message
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            parts.push(format!("message={message}"));
        }
        if parts.is_empty() {
            "weaver reported no reason".to_string()
        } else {
            parts.join(" ")
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubmissionQueueItemPayload {
    id: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PublishedSubmissionPayload {
    submit_nzb: PublishedSubmissionJobPayload,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PublishedSubmissionJobPayload {
    id: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct QueueCommandAckPayload {
    success: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PauseQueueItemPayload {
    pause_queue_item: QueueCommandAckPayload,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResumeQueueItemPayload {
    resume_queue_item: QueueCommandAckPayload,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CancelQueueItemPayload {
    cancel_queue_item: QueueCommandAckPayload,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoveHistoryItemsPayload {
    remove_history_items: HistoryCommandAckPayload,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HistoryCommandAckPayload {
    success: bool,
}

#[derive(Debug, Deserialize)]
struct PublishedBoolPayload {
    #[serde(default)]
    pause_job: Option<bool>,
    #[serde(default)]
    resume_job: Option<bool>,
    #[serde(default)]
    cancel_job: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct PublishedDeleteHistoryPayload {
    delete_history_batch: Vec<u64>,
}

impl WeaverDownloadClient {
    pub fn new(base_url: String, api_key: Option<String>) -> Self {
        Self::with_staged_nzb_store(
            base_url,
            api_key,
            Arc::new(NullStagedNzbStore),
            Arc::new(Semaphore::new(4)),
        )
    }

    pub fn with_staged_nzb_store(
        base_url: String,
        api_key: Option<String>,
        staged_nzb_store: Arc<dyn StagedNzbStore>,
        staged_nzb_pipeline_limit: Arc<Semaphore>,
    ) -> Self {
        let base = base_url.trim_end_matches('/').to_string();
        let graphql_url = format!("{base}/graphql");
        let http_client = generic_reqwest_client();
        Self {
            graphql_url,
            api_key,
            outbound_http: OutboundHttpClient::new(http_client.clone(), RateLimitRegistry::new()),
            staged_nzb_store,
            staged_nzb_pipeline_limit,
        }
    }

    /// Replace the egress client, so an operator-assigned proxy carries this
    /// client's traffic. Applied after construction because every existing
    /// call site builds the default client and only the router knows whether a
    /// proxy is assigned.
    pub fn with_http_client(mut self, http_client: reqwest::Client) -> Self {
        self.outbound_http = OutboundHttpClient::new(http_client, RateLimitRegistry::new());
        self
    }

    pub fn from_config(config: &DownloadClientConfig) -> AppResult<Self> {
        Self::from_config_with_staged_nzb_store(
            config,
            Arc::new(NullStagedNzbStore),
            Arc::new(Semaphore::new(4)),
        )
    }

    pub fn from_config_with_staged_nzb_store(
        config: &DownloadClientConfig,
        staged_nzb_store: Arc<dyn StagedNzbStore>,
        staged_nzb_pipeline_limit: Arc<Semaphore>,
    ) -> AppResult<Self> {
        let parsed_config = parse_download_client_config_json(&config.config_json)?;
        let base_url = resolve_download_client_base_url(&parsed_config).ok_or_else(|| {
            AppError::Validation(format!(
                "download client {} has no valid base URL",
                config.id
            ))
        })?;
        let api_key = read_config_string(&parsed_config, &["api_key", "apiKey", "apikey"]);
        Ok(Self::with_staged_nzb_store(
            base_url,
            api_key,
            staged_nzb_store,
            staged_nzb_pipeline_limit,
        ))
    }

    pub fn graphql_url(&self) -> &str {
        &self.graphql_url
    }

    pub fn api_key(&self) -> Option<&str> {
        self.api_key.as_deref()
    }

    /// Derive the WebSocket URL from the HTTP GraphQL endpoint.
    pub fn ws_url(&self) -> String {
        let url = self
            .graphql_url
            .replace("https://", "wss://")
            .replace("http://", "ws://");
        format!("{url}/ws")
    }

    fn with_auth_headers(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match self.api_key.as_deref() {
            Some(api_key) => request.header("Authorization", format!("Bearer {api_key}")),
            None => request,
        }
    }

    async fn graphql_request<T>(&self, query: &str, variables: Value) -> AppResult<T>
    where
        T: DeserializeOwned,
    {
        self.graphql_request_with_policy(
            self.read_policy("weaver_graphql_request"),
            query,
            variables,
        )
        .await
    }

    async fn graphql_request_with_policy<T>(
        &self,
        policy: RequestPolicy,
        query: &str,
        variables: Value,
    ) -> AppResult<T>
    where
        T: DeserializeOwned,
    {
        let payload = json!({ "query": query, "variables": variables });

        let response = self
            .outbound_http
            .send(policy, || {
                self.with_auth_headers(
                    self.outbound_http
                        .client()
                        .post(&self.graphql_url)
                        .header("Content-Type", "application/json")
                        .json(&payload),
                )
            })
            .await
            .map_err(|error| map_weaver_outbound_error("weaver request", error))?;

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|err| AppError::Repository(format!("weaver response read failed: {err}")))?;

        Self::parse_graphql_response(status, &body)
    }

    async fn graphql_multipart_request<T>(
        &self,
        request: GraphqlMultipartUploadRequest<'_>,
    ) -> AppResult<T>
    where
        T: DeserializeOwned,
    {
        let GraphqlMultipartUploadRequest {
            request_label,
            query,
            variables,
            upload_variable_path,
            filename,
            upload_path,
            content_type,
            content_length,
        } = request;
        let response = self
            .outbound_http
            .send_async(self.mutation_policy(request_label), || {
                let upload_path = upload_path.to_path_buf();
                let filename = filename.clone();
                let content_type = content_type.to_string();
                let variables = variables.clone();
                async move {
                    let file = File::open(&upload_path).await.map_err(|error| {
                        AppError::Repository(format!(
                            "failed to open weaver upload artifact {}: {error}",
                            upload_path.display()
                        ))
                    })?;
                    let part = multipart::Part::stream_with_length(
                        reqwest::Body::wrap_stream(ReaderStream::new(file)),
                        content_length,
                    )
                    .file_name(filename)
                    .mime_str(&content_type)
                    .map_err(|error| {
                        AppError::Repository(format!(
                            "failed to build weaver multipart file part: {error}"
                        ))
                    })?;
                    let form = multipart::Form::new()
                        .text(
                            "operations",
                            json!({ "query": query, "variables": variables }).to_string(),
                        )
                        .text("map", json!({ "0": [upload_variable_path] }).to_string())
                        .part("0", part);

                    Ok::<reqwest::RequestBuilder, AppError>(
                        self.with_auth_headers(
                            self.outbound_http
                                .client()
                                .post(&self.graphql_url)
                                .multipart(form),
                        ),
                    )
                }
            })
            .await
            .map_err(|error| match error {
                OutboundRequestError::Build(error) => error,
                OutboundRequestError::Http(error) => {
                    map_weaver_outbound_error("weaver multipart request", error)
                }
            })?;

        let status = response.status();
        let body = response.text().await.map_err(|err| {
            AppError::Repository(format!("weaver multipart response read failed: {err}"))
        })?;

        Self::parse_graphql_response(status, &body)
    }

    fn parse_graphql_response<T>(status: reqwest::StatusCode, body: &str) -> AppResult<T>
    where
        T: DeserializeOwned,
    {
        if !status.is_success() {
            let preview: String = body.chars().take(500).collect();
            return Err(AppError::Repository(format!(
                "weaver returned status {status}: {preview}"
            )));
        }

        let json: GraphqlResponse<T> = serde_json::from_str(body).map_err(|err| {
            AppError::Repository(format!("weaver returned non-json response: {err}"))
        })?;

        if let Some(errors) = json.errors
            && let Some(first) = errors.first()
        {
            return Err(AppError::Repository(format!(
                "weaver GraphQL error: {}",
                first.message
            )));
        }

        json.data
            .ok_or_else(|| AppError::Repository("weaver response missing data field".into()))
    }

    fn scope_key(&self) -> String {
        format!("weaver:{}", self.graphql_url)
    }

    fn read_policy(&self, request_label: impl Into<Cow<'static, str>>) -> RequestPolicy {
        RequestPolicy::safe_read(self.scope_key(), request_label)
            .with_max_retries(2)
            .with_backoff(Duration::from_secs(1), Duration::from_secs(15))
    }

    fn mutation_policy(&self, request_label: impl Into<Cow<'static, str>>) -> RequestPolicy {
        RequestPolicy::no_retry(self.scope_key(), request_label)
            .with_backoff(Duration::from_secs(1), Duration::from_secs(15))
    }

    /// Test connectivity by querying metrics.
    pub async fn test_connection(&self) -> AppResult<String> {
        match self
            .graphql_request::<SystemMetricsPayload>(graphql_docs::TEST_CONNECTION_QUERY, json!({}))
            .await
        {
            Ok(_) => {}
            Err(error) if is_weaver_schema_error(&error, "Unknown field \"systemMetrics\"") => {
                let _: VersionPayload = self
                    .graphql_request(graphql_docs::VERSION_COMPAT_QUERY, json!({}))
                    .await?;
            }
            Err(error) => return Err(error),
        }
        Ok("weaver".to_string())
    }

    async fn query_queue_items_once(
        &self,
        title_id: Option<&str>,
        use_title_filter: bool,
    ) -> AppResult<Vec<WeaverQueueItem>> {
        self.graphql_request::<QueueItemsPayload>(
            graphql_docs::QUEUE_ITEMS_QUERY,
            json!({
                "filter": if use_title_filter {
                    title_attribute_filter(title_id)
                } else {
                    None::<serde_json::Value>
                },
            }),
        )
        .await
        .map(|data| data.queue_items)
    }

    async fn query_queue_items(
        &self,
        title_id: Option<&str>,
        use_title_filter: bool,
    ) -> AppResult<Vec<WeaverQueueItem>> {
        match self
            .query_queue_items_once(title_id, use_title_filter)
            .await
        {
            Ok(data) => Ok(data),
            Err(error) if is_weaver_schema_error(&error, "Unknown field \"queueItems\"") => {
                self.query_jobs_compat(
                    Some(&[
                        "QUEUED",
                        "DOWNLOADING",
                        "CHECKING",
                        "VERIFYING",
                        "QUEUED_REPAIR",
                        "REPAIRING",
                        "QUEUED_EXTRACT",
                        "EXTRACTING",
                        "MOVING",
                        "PAUSED",
                    ]),
                    None,
                    None,
                )
                .await
            }
            Err(error)
                if use_title_filter
                    && title_id.is_some()
                    && is_weaver_schema_error(&error, "unknown field \"attributeEquals\"") =>
            {
                self.query_queue_items_once(None, false).await
            }
            Err(error) => Err(error),
        }
    }

    async fn query_history_items_once(
        &self,
        limit: Option<usize>,
        offset: Option<usize>,
        title_id: Option<&str>,
        use_title_filter: bool,
    ) -> AppResult<Vec<WeaverQueueItem>> {
        let after = offset.map(|value| {
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(format!("off:{value}"))
        });
        self.graphql_request::<HistoryItemsPayload>(
            graphql_docs::HISTORY_ITEMS_QUERY,
            json!({
                "filter": if use_title_filter && title_id.is_some() {
                    title_attribute_filter(title_id)
                } else {
                    None::<serde_json::Value>
                },
                "first": limit.and_then(|value| i32::try_from(value).ok()),
                "after": after,
            }),
        )
        .await
        .map(|data| data.history_items)
    }

    async fn query_history_items(
        &self,
        limit: Option<usize>,
        offset: Option<usize>,
        title_id: Option<&str>,
        use_title_filter: bool,
    ) -> AppResult<Vec<WeaverQueueItem>> {
        match self
            .query_history_items_once(limit, offset, title_id, use_title_filter)
            .await
        {
            Ok(data) => Ok(data),
            Err(error)
                if use_title_filter
                    && title_id.is_some()
                    && is_weaver_schema_error(&error, "unknown field \"attributeEquals\"") =>
            {
                self.query_history_items_once(limit, offset, None, false)
                    .await
            }
            Err(error) => Err(error),
        }
    }

    async fn query_history_item(&self, id: i32) -> AppResult<Option<WeaverQueueItem>> {
        self.graphql_request::<HistoryItemPayload>(
            graphql_docs::HISTORY_ITEM_QUERY,
            json!({ "id": id }),
        )
        .await
        .map(|data| data.history_item)
    }

    async fn query_completed_download_recent_fallback(
        &self,
        download_client_item_id: &str,
    ) -> AppResult<Option<CompletedDownload>> {
        let downloads = self
            .list_recent_completed_downloads(TARGETED_HISTORY_FALLBACK_LIMIT)
            .await?;
        Ok(downloads
            .into_iter()
            .find(|download| download.download_client_item_id == download_client_item_id))
    }

    pub async fn get_completed_download(
        &self,
        download_client_item_id: &str,
    ) -> AppResult<Option<CompletedDownload>> {
        let trimmed = download_client_item_id.trim();
        let Ok(job_id) = trimmed.parse::<i32>() else {
            return Ok(None);
        };

        let item = match self.query_history_item(job_id).await {
            Ok(item) => item,
            Err(error) if is_weaver_schema_error(&error, "historyItem") => {
                return self.query_completed_download_recent_fallback(trimmed).await;
            }
            Err(error) => return Err(error),
        };

        Ok(item.as_ref().and_then(weaver_item_to_completed_download))
    }

    async fn query_jobs_compat(
        &self,
        statuses: Option<&[&str]>,
        limit: Option<usize>,
        offset: Option<usize>,
    ) -> AppResult<Vec<WeaverQueueItem>> {
        let data: PublishedJobsPayload = self
            .graphql_request(
                graphql_docs::JOBS_COMPAT_QUERY,
                json!({
                    "status": statuses.map(|values| values.to_vec()),
                    "limit": limit.and_then(|value| i32::try_from(value).ok()),
                    "offset": offset.and_then(|value| i32::try_from(value).ok()),
                }),
            )
            .await?;
        Ok(data
            .jobs
            .into_iter()
            .map(compat_job_to_queue_item)
            .collect())
    }
}

/// Extract scryer metadata from Weaver attributes.
fn parse_scryer_client_request_id(client_request_id: Option<&str>) -> Option<String> {
    let value = client_request_id?.trim();
    let mut parts = value.splitn(3, ':');
    let prefix = parts.next()?;
    let title_id = parts.next()?;
    if prefix.eq_ignore_ascii_case("scryer") && !title_id.trim().is_empty() {
        Some(title_id.trim().to_string())
    } else {
        None
    }
}

fn durable_scryer_download_id(client_request_id: Option<&str>) -> Option<&str> {
    client_request_id
        .map(str::trim)
        .filter(|value| value.starts_with("scryer-download:"))
}

struct ExtractedScryerMetadata {
    title_id: Option<String>,
    facet: Option<String>,
    is_scryer: bool,
    download_id: Option<String>,
}

fn extract_scryer_metadata(
    attributes: &[WeaverAttribute],
    client_request_id: Option<&str>,
) -> ExtractedScryerMetadata {
    let mut title_id = None;
    let mut facet = None;
    let parameters = attributes
        .iter()
        .map(|entry| (entry.key.clone(), entry.value.clone()))
        .collect::<Vec<_>>();
    for entry in attributes {
        let value = entry.value.clone();
        match entry.key.as_str() {
            "*scryer_title_id" => title_id = Some(value),
            "*scryer_facet" => facet = Some(value),
            _ => {}
        }
    }

    if title_id.is_none() {
        title_id = parse_scryer_client_request_id(client_request_id);
    }

    let is_scryer = title_id.is_some()
        || client_request_id
            .map(|value| value.trim_start().starts_with("scryer:"))
            .unwrap_or(false);
    let observed_identity = scryer_application::observed_download_identity(
        scryer_application::ObservedDownloadIdentityInput {
            download_id: durable_scryer_download_id(client_request_id),
            parameters: &parameters,
            info_hash_hint: None,
        },
    );
    ExtractedScryerMetadata {
        title_id,
        facet,
        is_scryer,
        download_id: observed_identity.download_id,
    }
}

/// Map a weaver job status string to scryer's DownloadQueueState.
fn map_weaver_status(status: WeaverQueueState) -> DownloadQueueState {
    match status {
        WeaverQueueState::Queued => DownloadQueueState::Queued,
        WeaverQueueState::Downloading | WeaverQueueState::Checking => {
            DownloadQueueState::Downloading
        }
        WeaverQueueState::Verifying => DownloadQueueState::Verifying,
        WeaverQueueState::QueuedRepair => DownloadQueueState::Downloading,
        WeaverQueueState::Repairing => DownloadQueueState::Repairing,
        WeaverQueueState::QueuedExtract => DownloadQueueState::Repairing,
        WeaverQueueState::Extracting | WeaverQueueState::Moving | WeaverQueueState::Finalizing => {
            DownloadQueueState::Extracting
        }
        WeaverQueueState::Completed => DownloadQueueState::Completed,
        WeaverQueueState::Failed => DownloadQueueState::Failed,
        WeaverQueueState::Paused => DownloadQueueState::Paused,
    }
}

/// Map a Weaver queue/history item to a scryer DownloadQueueItem.
pub fn weaver_item_to_queue_item(job: &WeaverQueueItem) -> DownloadQueueItem {
    let state = map_weaver_status(job.state);

    let attention_reason = if state == DownloadQueueState::Failed {
        job.error
            .clone()
            .or_else(|| job.attention.as_ref().map(|value| value.message.clone()))
    } else {
        job.attention.as_ref().map(|value| value.message.clone())
    };

    let scryer_metadata =
        extract_scryer_metadata(&job.attributes, job.client_request_id.as_deref());
    let title_name = job
        .original_title
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(&job.name)
        .to_string();

    // Calculate remaining seconds from progress and download speed.
    // We don't have per-job speed, so leave it as None.
    DownloadQueueItem {
        id: job.id.to_string(),
        title_id: scryer_metadata.title_id,
        episode_id: None,
        title_name,
        facet: scryer_metadata.facet,
        category: job
            .category
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        client_id: String::new(),
        client_name: String::new(),
        client_type: "weaver".to_string(),
        state,
        progress_percent: if state == DownloadQueueState::Completed {
            100
        } else {
            job.progress_percent.round().clamp(0.0, 100.0) as u8
        },
        import_transfer_phase: None,
        import_transfer_bytes: None,
        import_transfer_total_bytes: None,
        import_transfer_started_at: None,
        import_transfer_updated_at: None,
        size_bytes: Some(job.total_bytes as i64),
        remaining_seconds: None,
        queued_at: Some(job.created_at.to_rfc3339()),
        last_updated_at: None,
        attention_required: job.attention.is_some() || matches!(state, DownloadQueueState::Failed),
        attention_reason,
        download_client_item_id: job.id.to_string(),
        download_id: scryer_metadata.download_id,
        import_status: None,
        import_error_code: None,
        import_error_message: None,
        imported_at: None,
        delete_status: None,
        delete_error_message: None,
        source_provider: None,
        is_scryer_origin: scryer_metadata.is_scryer,
        tracked_state: None,
        tracked_status: None,
        tracked_status_messages: Vec::new(),
        tracked_match_type: None,
        seeding: None,
    }
}

fn filter_items_by_title(items: Vec<DownloadQueueItem>, title_id: &str) -> Vec<DownloadQueueItem> {
    items
        .into_iter()
        .filter(|item| item.title_id.as_deref() == Some(title_id))
        .collect()
}

fn title_attribute_filter(title_id: Option<&str>) -> Option<Value> {
    title_id.map(|title_id| {
        json!({
            "attributeEquals": {
                "key": SCRYER_TITLE_ID_ATTRIBUTE_KEY,
                "value": title_id,
            }
        })
    })
}

fn weaver_item_to_completed_download(job: &WeaverQueueItem) -> Option<CompletedDownload> {
    if job.state != WeaverQueueState::Completed {
        return None;
    }

    let output_dir = job
        .output_dir
        .as_ref()
        .filter(|value| !value.is_empty())?
        .to_string();
    let mut parameters = job
        .attributes
        .iter()
        .map(|entry| (entry.key.clone(), entry.value.clone()))
        .collect::<Vec<_>>();
    if let Some(client_request_id) = job
        .client_request_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        parameters.push((
            scryer_application::DOWNLOAD_ID_PARAMETER.to_string(),
            client_request_id.to_string(),
        ));
    }
    let observed_identity = scryer_application::observed_download_identity(
        scryer_application::ObservedDownloadIdentityInput {
            download_id: durable_scryer_download_id(job.client_request_id.as_deref()),
            parameters: &parameters,
            info_hash_hint: None,
        },
    );

    Some(CompletedDownload {
        client_type: "weaver".to_string(),
        client_id: String::new(),
        download_client_item_id: job.id.to_string(),
        download_id: observed_identity.download_id,
        name: job.name.clone(),
        release_name: job.original_title.clone(),
        dest_dir: output_dir,
        category: job.category.clone(),
        size_bytes: Some(job.total_bytes as i64),
        completed_at: job.completed_at.or(Some(Utc::now())),
        parameters,
    })
}

fn compat_job_to_queue_item(job: PublishedWeaverJob) -> WeaverQueueItem {
    WeaverQueueItem {
        id: job.id,
        name: job.name,
        original_title: job.original_title,
        state: job.status,
        error: job.error,
        progress_percent: job.progress_percent,
        total_bytes: job.total_bytes,
        category: job.category,
        attributes: job.metadata,
        client_request_id: None,
        output_dir: job.output_dir,
        created_at: compat_timestamp_to_utc(job.created_at),
        completed_at: None,
        attention: None,
    }
}

fn compat_timestamp_to_utc(raw: Option<f64>) -> DateTime<Utc> {
    let Some(value) = raw else {
        return Utc::now();
    };
    let millis = if value.abs() >= 1_000_000_000_000.0 {
        value.round() as i64
    } else {
        (value * 1000.0).round() as i64
    };
    Utc.timestamp_millis_opt(millis)
        .single()
        .unwrap_or_else(Utc::now)
}

fn is_weaver_schema_error(error: &AppError, needle: &str) -> bool {
    match error {
        AppError::Repository(message) => message.contains(needle),
        _ => false,
    }
}

fn derive_nzb_filename(source_title: Option<&str>, source_hint: &str, title_name: &str) -> String {
    if let Some(name) = source_title
        && !name.is_empty()
    {
        return if name.ends_with(".nzb") {
            name.to_string()
        } else {
            format!("{name}.nzb")
        };
    }

    let url_filename = source_hint
        .rsplit('/')
        .next()
        .and_then(|segment| segment.split('?').next())
        .filter(|s| !s.is_empty() && s.contains('.'));
    if let Some(filename) = url_filename {
        return filename.to_string();
    }

    format!("{title_name}.nzb")
}

fn map_weaver_outbound_error(operation: &str, error: OutboundHttpError) -> AppError {
    match error {
        OutboundHttpError::RateLimited(rate_limited) => {
            let retry_after = rate_limited.retry_after.filter(|delay| !delay.is_zero());
            AppError::rate_limited_temporary_unavailable(
                match retry_after {
                    Some(delay) => {
                        format!(
                            "{operation} was rate limited; retry after {}s",
                            delay.as_secs()
                        )
                    }
                    None => format!("{operation} was rate limited"),
                },
                retry_after,
                RateLimitCooldownAction::AlreadyRecorded,
            )
        }
        OutboundHttpError::Transport { source, .. } => {
            AppError::Repository(format!("{operation} failed: {source}"))
        }
    }
}

#[async_trait]
impl DownloadClient for WeaverDownloadClient {
    async fn submit_download(
        &self,
        request: &DownloadClientAddRequest,
    ) -> AppResult<DownloadGrabResult> {
        let title = &request.title;
        let source_hint = request
            .source_hint
            .as_deref()
            .map(str::trim)
            .unwrap_or_default()
            .to_string();

        let normalized_source_title = request.source_title.clone().and_then(|v| {
            let t = v.trim().to_string();
            (!t.is_empty()).then_some(t)
        });
        let nzb_filename = derive_nzb_filename(
            normalized_source_title.as_deref(),
            &source_hint,
            &title.name,
        );

        let staged = resolve_staged_nzb_for_request(
            &self.outbound_http,
            &self.staged_nzb_store,
            &self.staged_nzb_pipeline_limit,
            request,
        )
        .await?;

        let password = request
            .source_password
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty() && !v.eq_ignore_ascii_case("0"))
            .map(String::from);

        let category = request.category.clone().and_then(|v| {
            let v = v.trim().to_string();
            (!v.is_empty()).then_some(v)
        });

        let facet_str =
            serde_json::to_string(&title.facet).unwrap_or_else(|_| "\"other\"".to_string());
        let facet_str = facet_str.trim_matches('"');

        let mut attributes = vec![
            json!({"key": "*scryer_title_id", "value": title.id.clone()}),
            json!({"key": "*scryer_facet", "value": facet_str}),
            json!({"key": "*scryer_import_purpose", "value": request.purpose.as_str()}),
        ];
        if let Some(download_id) = request.download_id {
            attributes.push(json!({"key": "*scryer_download_id", "value": download_id.to_wire()}));
        }

        if let Some(imdb_id) = title
            .external_ids
            .iter()
            .find(|id| id.source.eq_ignore_ascii_case("imdb"))
            .map(|id| id.value.trim().to_string())
            .filter(|v| !v.is_empty())
        {
            attributes.push(json!({"key": "*scryer_imdb_id", "value": imdb_id}));
        }

        let client_request_id = request
            .download_id
            .map(|id| id.to_wire())
            .unwrap_or_else(|| {
                format!(
                    "scryer:{}:{}",
                    title.id,
                    request
                        .release_title
                        .clone()
                        .or_else(|| normalized_source_title.clone())
                        .unwrap_or_else(|| title.name.clone())
                )
            });

        let result: AppResult<DownloadGrabResult> = async {
            let variables = json!({
                "input": {
                    "nzbUpload": Value::Null,
                    "filename": nzb_filename,
                    "password": password,
                    "category": category,
                    "attributes": attributes,
                    "clientRequestId": client_request_id,
                }
            });

            debug!(
                endpoint = self.graphql_url.as_str(),
                title = title.name.as_str(),
                filename = nzb_filename.as_str(),
                "weaver submitNzb multipart request"
            );

            match self
                .graphql_multipart_request::<SubmissionPayload>(GraphqlMultipartUploadRequest {
                    request_label: "weaver_submit_nzb",
                    query: graphql_docs::SUBMIT_NZB_MUTATION,
                    variables: variables.clone(),
                    upload_variable_path: "variables.input.nzbUpload",
                    filename: format!("{nzb_filename}.zst"),
                    upload_path: &staged.staged_nzb.compressed_path,
                    content_type: "application/zstd",
                    content_length: tokio::fs::metadata(&staged.staged_nzb.compressed_path)
                        .await
                        .map_err(|error| {
                            AppError::Repository(format!(
                                "failed to stat staged nzb {}: {error}",
                                staged.staged_nzb.compressed_path.display()
                            ))
                        })?
                        .len(),
                })
                .await
            {
                Ok(data) => {
                    let submission = data.submit_nzb;
                    // Branch on STATUS, not on `accepted`. `accepted: false` is
                    // Weaver's idempotent-replay signal, not a rejection — the
                    // job already exists and is returned to us.
                    let replayed = submission.status_is(WEAVER_STATUS_IDEMPOTENT_REPLAY);
                    if !submission.accepted && !replayed {
                        // A duplicate block is a VERDICT about this release —
                        // Weaver already holds an equivalent job — not a client
                        // outage. Mapping it to DownloadSubmitUnavailable made
                        // the router treat it as retryable-with-failover, which
                        // in practice meant every acquisition sweep re-submitted
                        // the same release and got re-blocked: one lost
                        // completion event turned into a submission storm
                        // (7 DUPLICATE_BLOCKED rejections from 3 grabs in one
                        // e2e run). DownloadSubmitRejected stops failover and
                        // records the attempt as a rejection instead.
                        if submission.status_is(WEAVER_STATUS_BLOCKED)
                            || submission.error_code_is(WEAVER_ERROR_DUPLICATE_BLOCKED)
                        {
                            return Err(AppError::DownloadSubmitRejected(format!(
                                "weaver submitNzb blocked the submission as a duplicate ({})",
                                submission.rejection_detail()
                            )));
                        }
                        return Err(AppError::download_submit_unavailable(format!(
                            "weaver submitNzb did not accept the submission ({})",
                            submission.rejection_detail()
                        )));
                    }

                    let job_id = submission.resolved_job_id().ok_or_else(|| {
                        AppError::download_submit_unavailable(format!(
                            "weaver submitNzb returned no queue item or job id ({})",
                            submission.rejection_detail()
                        ))
                    })?;

                    if replayed {
                        debug!(
                            endpoint = self.graphql_url.as_str(),
                            job_id,
                            title = title.name.as_str(),
                            "weaver submitNzb replayed an existing submission; adopting the existing job"
                        );
                    } else if submission.status_is(WEAVER_STATUS_PARKED) {
                        debug!(
                            endpoint = self.graphql_url.as_str(),
                            job_id,
                            title = title.name.as_str(),
                            detail = submission.rejection_detail().as_str(),
                            "weaver submitNzb parked the submission as a duplicate candidate"
                        );
                    }

                    debug!(
                        endpoint = self.graphql_url.as_str(),
                        job_id,
                        title = title.name.as_str(),
                        "weaver submitNzb succeeded"
                    );

                    Ok(DownloadGrabResult {
    download_id: None,
                        job_id: job_id.to_string(),
                        client_id: None,
                        client_type: "weaver".to_string(),
                        info_hash: None,
                        seed_goals: None,
                    })
                }
                Err(error)
                    if is_weaver_schema_error(&error, "Unknown type \"SubmitNzbInput\"")
                        || is_weaver_schema_error(&error, "Unknown argument \"input\"")
                        || is_weaver_schema_error(&error, "Unknown field \"accepted\"")
                        // Selected alongside the idempotent-replay fix. `jobId`
                        // and `errorCode` postdate `accepted`, so a pre-2026-07-10
                        // Weaver lands here and falls back to the legacy mutation
                        // rather than hard-failing the grab.
                        || is_weaver_schema_error(&error, "Unknown field \"status\"")
                        || is_weaver_schema_error(&error, "Unknown field \"message\"")
                        || is_weaver_schema_error(&error, "Unknown field \"jobId\"")
                        || is_weaver_schema_error(&error, "Unknown field \"errorCode\"") =>
                {
                    let compressed_bytes = tokio::fs::read(&staged.staged_nzb.compressed_path)
                        .await
                        .map_err(|read_error| {
                            AppError::Repository(format!(
                                "failed to read staged nzb {}: {read_error}",
                                staged.staged_nzb.compressed_path.display()
                            ))
                        })?;
                    let nzb_bytes = zstd::stream::decode_all(std::io::Cursor::new(compressed_bytes))
                        .map_err(|decode_error| {
                            AppError::Repository(format!(
                                "failed to decode staged nzb {}: {decode_error}",
                                staged.staged_nzb.compressed_path.display()
                            ))
                        })?;
                    let compat_data: PublishedSubmissionPayload = self
                        .graphql_request_with_policy(
                            self.mutation_policy("weaver_submit_nzb_compat"),
                            graphql_docs::SUBMIT_NZB_COMPAT_MUTATION,
                            json!({
                                "source": {
                                    "nzbBase64": base64::engine::general_purpose::STANDARD.encode(nzb_bytes),
                                },
                                "filename": nzb_filename,
                                "password": password,
                                "category": category,
                                "metadata": attributes,
                            }),
                        )
                        .await
                        .map_err(AppError::into_download_submit_unavailable)?;
                    Ok(DownloadGrabResult {
    download_id: None,
                        job_id: compat_data.submit_nzb.id.to_string(),
                        client_id: None,
                        client_type: "weaver".to_string(),
                        info_hash: None,
                        seed_goals: None,
                    })
                }
                Err(error) => Err(error.into_download_submit_unavailable()),
            }
        }
        .await;

        if staged.self_staged
            && let Err(error) = self
                .staged_nzb_store
                .delete_staged_nzb(&staged.staged_nzb)
                .await
        {
            warn!(
                staged_nzb_id = staged.staged_nzb.id.as_str(),
                error = %error,
                "failed to delete self-staged weaver nzb artifact"
            );
        }

        result
    }

    async fn test_connection(&self) -> AppResult<String> {
        WeaverDownloadClient::test_connection(self).await
    }

    async fn list_queue(&self) -> AppResult<Vec<DownloadQueueItem>> {
        let jobs = self.query_queue_items(None, false).await?;
        Ok(jobs.iter().map(weaver_item_to_queue_item).collect())
    }

    async fn list_queue_for_title(&self, title_id: &str) -> AppResult<Vec<DownloadQueueItem>> {
        let jobs = self.query_queue_items(Some(title_id), true).await?;
        Ok(filter_items_by_title(
            jobs.iter().map(weaver_item_to_queue_item).collect(),
            title_id,
        ))
    }

    async fn list_history(&self) -> AppResult<Vec<DownloadQueueItem>> {
        let jobs = match self.query_history_items(None, None, None, false).await {
            Ok(items) => items,
            Err(error) if is_weaver_schema_error(&error, "Unknown field \"historyItems\"") => {
                self.query_jobs_compat(Some(&["COMPLETE", "FAILED"]), Some(200), Some(0))
                    .await?
            }
            Err(error) => return Err(error),
        };
        Ok(jobs.iter().map(weaver_item_to_queue_item).collect())
    }

    async fn list_recent_activity(&self, limit: usize) -> AppResult<Vec<DownloadQueueItem>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let jobs = match self
            .query_history_items(Some(limit), Some(0), None, false)
            .await
        {
            Ok(items) => items,
            Err(error) if is_weaver_schema_error(&error, "Unknown field \"historyItems\"") => {
                self.query_jobs_compat(Some(&["COMPLETE", "FAILED"]), Some(limit), Some(0))
                    .await?
            }
            Err(error) => return Err(error),
        };
        Ok(jobs.iter().map(weaver_item_to_queue_item).collect())
    }

    async fn list_recent_activity_for_title(
        &self,
        title_id: &str,
        limit: usize,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let jobs = match self
            .query_history_items(Some(limit), Some(0), Some(title_id), true)
            .await
        {
            Ok(items) => items,
            Err(error) if is_weaver_schema_error(&error, "Unknown field \"historyItems\"") => {
                self.query_jobs_compat(Some(&["COMPLETE", "FAILED"]), Some(limit), Some(0))
                    .await?
            }
            Err(error) => return Err(error),
        };
        Ok(filter_items_by_title(
            jobs.iter().map(weaver_item_to_queue_item).collect(),
            title_id,
        ))
    }

    async fn list_history_page(
        &self,
        offset: usize,
        limit: usize,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        let jobs = match self
            .query_history_items(Some(limit), Some(offset), None, false)
            .await
        {
            Ok(items) => items,
            Err(error) if is_weaver_schema_error(&error, "Unknown field \"historyItems\"") => {
                self.query_jobs_compat(Some(&["COMPLETE", "FAILED"]), Some(limit), Some(offset))
                    .await?
            }
            Err(error) => return Err(error),
        };
        Ok(jobs.iter().map(weaver_item_to_queue_item).collect())
    }

    async fn list_recent_completed_downloads(
        &self,
        limit: usize,
    ) -> AppResult<Vec<CompletedDownload>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let jobs = match self
            .query_history_items(Some(limit), Some(0), None, false)
            .await
        {
            Ok(items) => items,
            Err(error) if is_weaver_schema_error(&error, "Unknown field \"historyItems\"") => {
                self.query_jobs_compat(Some(&["COMPLETE", "FAILED"]), Some(limit), Some(0))
                    .await?
            }
            Err(error) => return Err(error),
        };
        Ok(jobs
            .iter()
            .filter_map(weaver_item_to_completed_download)
            .collect())
    }

    async fn list_completed_downloads(&self) -> AppResult<Vec<CompletedDownload>> {
        let jobs = match self.query_history_items(None, None, None, false).await {
            Ok(items) => items,
            Err(error) if is_weaver_schema_error(&error, "Unknown field \"historyItems\"") => {
                self.query_jobs_compat(Some(&["COMPLETE", "FAILED"]), Some(200), Some(0))
                    .await?
            }
            Err(error) => return Err(error),
        };
        Ok(jobs
            .iter()
            .filter_map(weaver_item_to_completed_download)
            .collect())
    }

    async fn get_completed_download_for_source(
        &self,
        _client_id: &str,
        _client_type: &str,
        download_client_item_id: &str,
    ) -> AppResult<Option<CompletedDownload>> {
        self.get_completed_download(download_client_item_id).await
    }

    async fn pause_queue_item(&self, id: &str) -> AppResult<()> {
        let job_id: u64 = id
            .parse()
            .map_err(|_| AppError::Validation(format!("invalid weaver job id: {id}")))?;
        match self
            .graphql_request_with_policy::<PauseQueueItemPayload>(
                self.mutation_policy("weaver_pause_queue_item"),
                graphql_docs::PAUSE_QUEUE_ITEM_MUTATION,
                json!({ "id": job_id }),
            )
            .await
        {
            Ok(data) => {
                if !data.pause_queue_item.success {
                    return Err(AppError::Repository(
                        "weaver pauseQueueItem did not succeed".into(),
                    ));
                }
            }
            Err(error) if is_weaver_schema_error(&error, "Unknown field \"pauseQueueItem\"") => {
                let data: PublishedBoolPayload = self
                    .graphql_request_with_policy(
                        self.mutation_policy("weaver_pause_job"),
                        graphql_docs::PAUSE_JOB_MUTATION,
                        json!({ "id": job_id }),
                    )
                    .await?;
                if data.pause_job != Some(true) {
                    return Err(AppError::Repository(
                        "weaver pauseJob did not succeed".into(),
                    ));
                }
            }
            Err(error) => return Err(error),
        }
        Ok(())
    }

    async fn resume_queue_item(&self, id: &str) -> AppResult<()> {
        let job_id: u64 = id
            .parse()
            .map_err(|_| AppError::Validation(format!("invalid weaver job id: {id}")))?;
        match self
            .graphql_request_with_policy::<ResumeQueueItemPayload>(
                self.mutation_policy("weaver_resume_queue_item"),
                graphql_docs::RESUME_QUEUE_ITEM_MUTATION,
                json!({ "id": job_id }),
            )
            .await
        {
            Ok(data) => {
                if !data.resume_queue_item.success {
                    return Err(AppError::Repository(
                        "weaver resumeQueueItem did not succeed".into(),
                    ));
                }
            }
            Err(error) if is_weaver_schema_error(&error, "Unknown field \"resumeQueueItem\"") => {
                let data: PublishedBoolPayload = self
                    .graphql_request_with_policy(
                        self.mutation_policy("weaver_resume_job"),
                        graphql_docs::RESUME_JOB_MUTATION,
                        json!({ "id": job_id }),
                    )
                    .await?;
                if data.resume_job != Some(true) {
                    return Err(AppError::Repository(
                        "weaver resumeJob did not succeed".into(),
                    ));
                }
            }
            Err(error) => return Err(error),
        }
        Ok(())
    }

    /// A history delete requests payload deletion when `remove_data` is true.
    /// Older Weaver versions fall back to removing the history entry only when
    /// they do not support the `deleteFiles` argument.
    async fn delete_queue_item(
        &self,
        id: &str,
        is_history: bool,
        remove_data: bool,
    ) -> AppResult<()> {
        let job_id: u64 = id
            .parse()
            .map_err(|_| AppError::Validation(format!("invalid weaver job id: {id}")))?;
        if is_history {
            let (mutation, variables) = if remove_data {
                (
                    graphql_docs::REMOVE_HISTORY_ITEMS_DELETE_FILES_MUTATION,
                    json!({ "ids": [job_id], "deleteFiles": true }),
                )
            } else {
                (
                    graphql_docs::REMOVE_HISTORY_ITEMS_MUTATION,
                    json!({ "ids": [job_id] }),
                )
            };
            match self
                .graphql_request_with_policy::<RemoveHistoryItemsPayload>(
                    self.mutation_policy(if remove_data {
                        "weaver_remove_history_items_delete_files"
                    } else {
                        "weaver_remove_history_items"
                    }),
                    mutation,
                    variables,
                )
                .await
            {
                Ok(data) => {
                    if !data.remove_history_items.success {
                        return Err(AppError::Repository(
                            "weaver removeHistoryItems did not succeed".into(),
                        ));
                    }
                }
                Err(error)
                    if remove_data
                        && (is_weaver_schema_error(&error, "Unknown argument \"deleteFiles\"")
                            || is_weaver_schema_error(&error, "Variable \"$deleteFiles\"")) =>
                {
                    warn!(
                        error = %error,
                        "weaver: deleteFiles is unsupported; data could not be deleted"
                    );
                    let data: RemoveHistoryItemsPayload = self
                        .graphql_request_with_policy(
                            self.mutation_policy("weaver_remove_history_items"),
                            graphql_docs::REMOVE_HISTORY_ITEMS_MUTATION,
                            json!({ "ids": [job_id] }),
                        )
                        .await?;
                    if !data.remove_history_items.success {
                        return Err(AppError::Repository(
                            "weaver removeHistoryItems did not succeed".into(),
                        ));
                    }
                }
                Err(error)
                    if is_weaver_schema_error(&error, "Unknown field \"removeHistoryItems\"") =>
                {
                    let data: PublishedDeleteHistoryPayload = self
                        .graphql_request_with_policy(
                            self.mutation_policy("weaver_delete_history_batch"),
                            graphql_docs::DELETE_HISTORY_BATCH_MUTATION,
                            json!({ "ids": [job_id], "deleteFiles": remove_data }),
                        )
                        .await?;
                    if !data.delete_history_batch.contains(&job_id) {
                        return Err(AppError::Repository(
                            "weaver deleteHistoryBatch did not remove the requested job".into(),
                        ));
                    }
                }
                Err(error) => return Err(error),
            }
        } else {
            match self
                .graphql_request_with_policy::<CancelQueueItemPayload>(
                    self.mutation_policy("weaver_cancel_queue_item"),
                    graphql_docs::CANCEL_QUEUE_ITEM_MUTATION,
                    json!({ "id": job_id }),
                )
                .await
            {
                Ok(data) => {
                    if !data.cancel_queue_item.success {
                        return Err(AppError::Repository(
                            "weaver cancelQueueItem did not succeed".into(),
                        ));
                    }
                }
                Err(error)
                    if is_weaver_schema_error(&error, "Unknown field \"cancelQueueItem\"") =>
                {
                    let data: PublishedBoolPayload = self
                        .graphql_request_with_policy(
                            self.mutation_policy("weaver_cancel_job"),
                            graphql_docs::CANCEL_JOB_MUTATION,
                            json!({ "id": job_id }),
                        )
                        .await?;
                    if data.cancel_job != Some(true) {
                        return Err(AppError::Repository(
                            "weaver cancelJob did not succeed".into(),
                        ));
                    }
                }
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use serde_json::json;
    use wiremock::matchers::{body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::{
        SubmissionPayload, WeaverDownloadClient, WeaverQueueItem, map_weaver_outbound_error,
        weaver_item_to_queue_item,
    };
    use scryer_application::{
        AppError, DownloadClient, DownloadClientAddRequest, DownloadSubmissionPurpose,
        ResolvedDownloadArtifact,
    };
    use scryer_domain::{DownloadClientConfig, DownloadQueueState, MediaFacet, Title};
    use scryer_outbound_http::OutboundHttpError;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::Semaphore;

    use crate::downloads::staged_nzb_store::FileSystemStagedNzbStore;

    fn test_config(config_json: &str) -> DownloadClientConfig {
        DownloadClientConfig {
            id: "dc-weaver".to_string(),
            name: "Weaver".to_string(),
            client_type: "weaver".to_string(),

            config_json: config_json.to_string(),
            client_priority: 1,
            is_enabled: true,
            status: scryer_domain::DownloadClientStatus::Healthy,
            last_error: None,
            last_seen_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            proxy_config_id: None,
        }
    }

    fn test_add_request(download_id: &str) -> DownloadClientAddRequest {
        let facet = MediaFacet::Movie;
        DownloadClientAddRequest {
            title: Title {
                id: "title-1".to_string(),
                name: "Test Title".to_string(),
                library_id: scryer_domain::default_library_id_for_facet(&facet),
                facet,
                monitored: true,
                tags: vec![],
                canonical_tags: vec![],
                external_ids: vec![],
                root_folder_id: scryer_domain::root_folder_id_for_path("/data/movies"),
                created_by: None,
                created_at: Utc::now(),
                year: None,
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
                aliases: vec![],
                tagged_aliases: vec![],
                metadata_language: None,
                metadata_fetched_at: None,
                min_availability: None,
                digital_release_date: None,
                folder_path: None,
            },
            search_facet: None,
            purpose: DownloadSubmissionPurpose::Standard,
            download_id: Some(
                scryer_domain::download_identity::DownloadId::from_wire(download_id)
                    .expect("test token should be a wire DownloadId"),
            ),
            source_hint: Some("https://example.invalid/release.nzb".to_string()),
            staged_nzb: None,
            resolved_download_artifact: Some(ResolvedDownloadArtifact::Nzb {
                bytes: b"<nzb></nzb>".to_vec(),
                file_name: Some("Test Release.nzb".to_string()),
                content_type: Some("application/x-nzb".to_string()),
            }),
            source_kind: None,
            source_title: Some("Test Release".to_string()),
            source_password: Some("archive-password".to_string()),
            category: Some("movies".to_string()),
            queue_priority: None,
            download_directory: None,
            release_title: None,
            indexer_name: None,
            indexer_id: None,
            info_hash_hint: None,
            seed_goal_ratio: None,
            seed_goal_seconds: None,
            tracker_min_seed_ratio: None,
            tracker_min_seed_time_minutes: None,
            season_pack_seed_ratio: None,
            season_pack_seed_time_minutes: None,
            is_recent: None,
            season_pack: None,
        }
    }

    #[tokio::test]
    async fn submit_nzb_payload_preserves_the_passed_scryer_download_id_attribute() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/release.nzb"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"<nzb></nzb>".to_vec()))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(header("authorization", "Bearer wvr_test"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": {
                    "submitNzb": {
                        "accepted": true,
                        "status": "ACCEPTED",
                        "jobId": 77,
                        "errorCode": null,
                        "message": null,
                        "item": { "id": 77 }
                    }
                }
            })))
            .expect(1)
            .mount(&server)
            .await;

        let download_id = "scryer-download:00000000-0000-4000-8000-000000000022";
        let staged_nzb_dir = tempfile::tempdir().expect("staged nzb directory");
        let staged_nzb_store = Arc::new(
            FileSystemStagedNzbStore::new(staged_nzb_dir.path())
                .await
                .expect("staged nzb store"),
        );
        let client = WeaverDownloadClient::with_staged_nzb_store(
            server.uri(),
            Some("wvr_test".to_string()),
            staged_nzb_store,
            Arc::new(Semaphore::new(1)),
        );
        let mut request = test_add_request(download_id);
        request.source_hint = Some(format!("{}/release.nzb", server.uri()));
        let result = client
            .submit_download(&request)
            .await
            .expect("submitNzb should succeed");
        assert_eq!(result.job_id, "77");

        let requests = server
            .received_requests()
            .await
            .expect("submitNzb request should be recorded");
        let request = requests
            .iter()
            .find(|request| request.url.path() == "/graphql")
            .expect("submitNzb request");
        let body = String::from_utf8_lossy(&request.body);
        let expected_variables = json!({
            "input": {
                "nzbUpload": null,
                "filename": "Test Release.nzb",
                "password": "archive-password",
                "category": "movies",
                "attributes": [
                    { "key": "*scryer_title_id", "value": "title-1" },
                    { "key": "*scryer_facet", "value": "movie" },
                    { "key": "*scryer_import_purpose", "value": "standard" },
                    { "key": "*scryer_download_id", "value": download_id },
                ],
                "clientRequestId": download_id,
            }
        });
        assert!(
            body.contains(&expected_variables.to_string()),
            "submitNzb request did not contain the exact expected input: {body}",
        );
        assert!(body.contains(r#"{"0":["variables.input.nzbUpload"]}"#));
    }

    #[tokio::test]
    async fn delete_history_item_without_data_removal_uses_legacy_mutation() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(header("authorization", "Bearer wvr_test"))
            .and(body_string_contains(
                "mutation RemoveHistoryItems($ids: [Int!]!)",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "removeHistoryItems": { "success": true } }
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client = WeaverDownloadClient::new(server.uri(), Some("wvr_test".to_string()));
        client
            .delete_queue_item("10002", true, false)
            .await
            .expect("legacy history delete should succeed");

        let requests = server
            .received_requests()
            .await
            .expect("requests should be recorded");
        assert!(!String::from_utf8_lossy(&requests[0].body).contains("deleteFiles"));
    }

    #[tokio::test]
    async fn delete_history_item_with_data_removal_uses_delete_files_mutation() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(header("authorization", "Bearer wvr_test"))
            .and(body_string_contains("RemoveHistoryItemsDeleteFiles"))
            .and(body_string_contains("\"deleteFiles\":true"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "removeHistoryItems": { "success": true } }
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client = WeaverDownloadClient::new(server.uri(), Some("wvr_test".to_string()));
        client
            .delete_queue_item("10002", true, true)
            .await
            .expect("history delete with data removal should succeed");
    }

    #[tokio::test]
    async fn delete_history_item_falls_back_when_delete_files_is_unsupported() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(header("authorization", "Bearer wvr_test"))
            .and(body_string_contains("RemoveHistoryItemsDeleteFiles"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "errors": [{
                    "message": "Unknown argument \"deleteFiles\" on field \"Mutation.removeHistoryItems\"."
                }]
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(header("authorization", "Bearer wvr_test"))
            .and(body_string_contains(
                "mutation RemoveHistoryItems($ids: [Int!]!)",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "removeHistoryItems": { "success": true } }
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client = WeaverDownloadClient::new(server.uri(), Some("wvr_test".to_string()));
        client
            .delete_queue_item("10002", true, true)
            .await
            .expect("legacy fallback should remove the history entry");
    }

    #[test]
    fn outbound_rate_limit_preserves_retry_after() {
        let error = OutboundHttpError::RateLimited(scryer_outbound_http::RateLimitedError {
            scope: scryer_outbound_http::RateLimitScopeKey::from("weaver"),
            retry_after: Some(Duration::from_secs(40)),
            attempts: 1,
            retry_after_source: scryer_outbound_http::RetryAfterSource::Seconds,
            request_label: std::borrow::Cow::Borrowed("weaver"),
        });
        let error = map_weaver_outbound_error("weaver queue", error);

        match error {
            AppError::TemporaryUnavailable {
                message,
                retry_after,
                ..
            } => {
                assert!(message.contains("retry after 40s"));
                assert_eq!(retry_after, Some(Duration::from_secs(40)));
            }
            other => panic!("expected temporary unavailable error, got {other:?}"),
        }
    }

    #[test]
    fn rejected_submission_allows_null_queue_item() {
        let payload: SubmissionPayload = WeaverDownloadClient::parse_graphql_response(
            reqwest::StatusCode::OK,
            r#"{"data":{"submitNzb":{"accepted":false,"item":null}}}"#,
        )
        .expect("rejected submission should remain valid GraphQL JSON");

        assert!(!payload.submit_nzb.accepted);
        assert!(payload.submit_nzb.item.is_none());
    }

    #[test]
    fn idempotent_replay_is_a_successful_submission_carrying_the_existing_job() {
        // Weaver sets `accepted: false` for EXACTLY ONE status, IDEMPOTENT_REPLAY,
        // and it means the submission already succeeded: the job exists and the
        // live item comes back with it. Reading `accepted` alone made Scryer
        // discard a grab that had worked and fail over into a duplicate.
        let payload: SubmissionPayload = WeaverDownloadClient::parse_graphql_response(
            reqwest::StatusCode::OK,
            r#"{"data":{"submitNzb":{"accepted":false,"status":"IDEMPOTENT_REPLAY","jobId":4242,"errorCode":null,"message":null,"item":{"id":4242}}}}"#,
        )
        .expect("idempotent replay should remain valid GraphQL JSON");

        let submission = payload.submit_nzb;
        assert!(!submission.accepted);
        assert!(submission.status_is(super::WEAVER_STATUS_IDEMPOTENT_REPLAY));
        assert_eq!(submission.resolved_job_id(), Some(4242));
    }

    #[test]
    fn parked_submission_resolves_its_job_id_without_a_queue_item() {
        // A parked semantic duplicate reports accepted with NO item, which used
        // to trip the "accepted without a queue item" error and lose the grab.
        // jobId is always populated, so the submission stays trackable.
        let payload: SubmissionPayload = WeaverDownloadClient::parse_graphql_response(
            reqwest::StatusCode::OK,
            r#"{"data":{"submitNzb":{"accepted":true,"status":"PARKED","jobId":77,"errorCode":null,"message":"semantic duplicate candidate parked","item":null}}}"#,
        )
        .expect("parked submission should remain valid GraphQL JSON");

        let submission = payload.submit_nzb;
        assert!(submission.item.is_none());
        assert!(submission.status_is(super::WEAVER_STATUS_PARKED));
        assert_eq!(submission.resolved_job_id(), Some(77));
        let detail = submission.rejection_detail();
        assert!(detail.contains("PARKED"));
        assert!(detail.contains("semantic duplicate candidate parked"));
    }

    #[test]
    fn duplicate_blocked_submission_parses_as_a_rejection_verdict() {
        // status=BLOCKED / errorCode=DUPLICATE_BLOCKED means Weaver already
        // holds an equivalent job. The client maps this to
        // DownloadSubmitRejected (no failover, no per-sweep retry); this pins
        // the payload shape and the helpers that branch decision rides on.
        let payload: SubmissionPayload = WeaverDownloadClient::parse_graphql_response(
            reqwest::StatusCode::OK,
            r#"{"data":{"submitNzb":{"accepted":false,"status":"BLOCKED","errorCode":"DUPLICATE_BLOCKED","message":"duplicate submission blocked","item":null}}}"#,
        )
        .expect("blocked submission should remain valid GraphQL JSON");

        let submission = payload.submit_nzb;
        assert!(!submission.accepted);
        assert!(submission.status_is(super::WEAVER_STATUS_BLOCKED));
        assert!(submission.error_code_is(super::WEAVER_ERROR_DUPLICATE_BLOCKED));
        let detail = submission.rejection_detail();
        assert!(detail.contains("BLOCKED"));
        assert!(detail.contains("duplicate submission blocked"));
    }

    #[test]
    fn submission_fields_stay_optional_for_older_weavers() {
        // Older Weavers select neither status nor jobId. Deserialization must
        // still succeed so the compat fallback governs, not a parse error.
        let payload: SubmissionPayload = WeaverDownloadClient::parse_graphql_response(
            reqwest::StatusCode::OK,
            r#"{"data":{"submitNzb":{"accepted":true,"item":{"id":9}}}}"#,
        )
        .expect("legacy submission payload should still parse");

        let submission = payload.submit_nzb;
        assert!(submission.status.is_none());
        assert_eq!(submission.resolved_job_id(), Some(9));
        assert!(!submission.status_is(super::WEAVER_STATUS_IDEMPOTENT_REPLAY));
    }

    #[test]
    fn from_config_reads_api_key_and_base_url() {
        let config = test_config(r#"{"api_key":"wvr_test","host":"weaver.local","port":"9090"}"#);

        let client =
            WeaverDownloadClient::from_config(&config).expect("weaver config should parse");

        assert_eq!(client.graphql_url(), "http://weaver.local:9090/graphql");
        assert_eq!(client.api_key(), Some("wvr_test"));
        assert_eq!(client.ws_url(), "ws://weaver.local:9090/graphql/ws");
    }

    #[test]
    fn weaver_item_to_queue_item_marks_failed_job_attention() {
        let job = json!({
            "id": 42,
            "name": "Example Job",
            "originalTitle": "Example.Release.1080p.WEB-DL-GROUP",
            "state": "FAILED",
            "error": "archive corrupt",
            "progressPercent": 25.0,
            "totalBytes": 4000,
            "downloadedBytes": 1000,
            "failedBytes": 0,
            "health": 800,
            "category": null,
            "outputDir": null,
            "createdAt": "2024-01-01T00:00:00Z",
            "completedAt": null,
            "clientRequestId": null,
            "attributes": [
                { "key": "*scryer_title_id", "value": "title-1" },
                { "key": "*scryer_facet", "value": "anime" }
            ],
            "attention": { "code": "JOB_FAILED", "message": "archive corrupt" }
        });

        let job: WeaverQueueItem = serde_json::from_value(job).expect("job should deserialize");
        let item = weaver_item_to_queue_item(&job);

        assert_eq!(item.state, DownloadQueueState::Failed);
        assert_eq!(item.title_name, "Example.Release.1080p.WEB-DL-GROUP");
        assert_eq!(item.title_id.as_deref(), Some("title-1"));
        assert!(item.is_scryer_origin);
        assert_eq!(item.attention_reason.as_deref(), Some("archive corrupt"));
    }

    #[test]
    fn weaver_item_to_queue_item_uses_client_request_id_as_origin_fallback() {
        let job = json!({
            "id": 77,
            "name": "Origin Fallback",
            "state": "DOWNLOADING",
            "error": null,
            "progressPercent": 10.0,
            "totalBytes": 1000,
            "downloadedBytes": 100,
            "failedBytes": 0,
            "health": 1000,
            "category": null,
            "outputDir": null,
            "createdAt": "2024-01-01T00:00:00Z",
            "completedAt": null,
            "clientRequestId": "scryer:title-77:Origin Fallback",
            "attributes": [],
            "attention": null
        });

        let job: WeaverQueueItem = serde_json::from_value(job).expect("job should deserialize");
        let item = weaver_item_to_queue_item(&job);

        assert_eq!(item.title_name, "Origin Fallback");
        assert_eq!(item.title_id.as_deref(), Some("title-77"));
        assert!(item.is_scryer_origin);
        assert_eq!(item.download_id, None);
    }

    #[test]
    fn weaver_item_to_queue_item_maps_download_id() {
        let job = json!({
            "id": 78,
            "name": "DownloadId",
            "state": "DOWNLOADING",
            "error": null,
            "progressPercent": 10.0,
            "totalBytes": 1000,
            "downloadedBytes": 100,
            "failedBytes": 0,
            "health": 1000,
            "category": "movies",
            "outputDir": null,
            "createdAt": "2024-01-01T00:00:00Z",
            "completedAt": null,
            "clientRequestId": "scryer-download:abc123",
            "attributes": [
                { "key": "*scryer_title_id", "value": "title-1" },
                { "key": "*scryer_facet", "value": "anime" },
                { "key": "*scryer_download_id", "value": "scryer-download:abc123" }
            ],
            "attention": null
        });

        let job: WeaverQueueItem = serde_json::from_value(job).expect("job should deserialize");
        let item = weaver_item_to_queue_item(&job);

        assert_eq!(item.download_id.as_deref(), Some("scryer-download:abc123"));
        assert!(item.is_scryer_origin);
        assert_eq!(item.category.as_deref(), Some("movies"));
    }

    #[tokio::test]
    async fn list_completed_downloads_includes_non_scryer_completed_jobs() {
        let server = MockServer::start().await;
        let client = WeaverDownloadClient::new(server.uri(), Some("wvr_test".to_string()));

        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(header("authorization", "Bearer wvr_test"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": {
                    "historyItems": [
                        {
                            "id": 10000,
                            "name": "8f1d2c3b4a59687766554433221100ff",
                            "state": "COMPLETE",
                            "error": null,
                            "progressPercent": 100.0,
                            "totalBytes": 123456789_u64,
                            "category": "2000",
                            "attributes": [],
                            "clientRequestId": null,
                            "outputDir": "/data/complete/8f1d2c3b4a59687766554433221100ff.#10000",
                            "createdAt": "2024-01-01T00:00:00Z",
                            "completedAt": "2024-01-01T00:10:00Z",
                            "attention": null
                        }
                    ]
                }
            })))
            .mount(&server)
            .await;

        let downloads = client
            .list_completed_downloads()
            .await
            .expect("completed downloads should load");

        assert_eq!(downloads.len(), 1);
        assert_eq!(downloads[0].download_client_item_id, "10000");
        assert_eq!(downloads[0].name, "8f1d2c3b4a59687766554433221100ff");
        assert_eq!(
            downloads[0].dest_dir,
            "/data/complete/8f1d2c3b4a59687766554433221100ff.#10000"
        );
        assert!(downloads[0].parameters.is_empty());
    }

    #[tokio::test]
    async fn list_recent_activity_uses_bounded_history_items_query() {
        let server = MockServer::start().await;
        let client = WeaverDownloadClient::new(server.uri(), Some("wvr_test".to_string()));

        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(header("authorization", "Bearer wvr_test"))
            .and(body_string_contains("\"first\":2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": {
                    "historyItems": [
                        {
                            "id": 10001,
                            "name": "Paper.Lantern.2012.1080p",
                            "state": "COMPLETE",
                            "error": null,
                            "progressPercent": 100.0,
                            "totalBytes": 123456789_u64,
                            "category": "2000",
                            "attributes": [],
                            "clientRequestId": null,
                            "outputDir": "/data/complete/Paper.Lantern.2012.1080p.#10001",
                            "createdAt": "2024-01-01T00:00:00Z",
                            "completedAt": "2024-01-01T00:10:00Z",
                            "attention": null
                        }
                    ]
                }
            })))
            .expect(1)
            .mount(&server)
            .await;

        let items = client
            .list_recent_activity(2)
            .await
            .expect("recent activity should load");

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].download_client_item_id, "10001");
    }

    #[tokio::test]
    async fn list_recent_completed_downloads_uses_bounded_history_items_query() {
        let server = MockServer::start().await;
        let client = WeaverDownloadClient::new(server.uri(), Some("wvr_test".to_string()));

        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(header("authorization", "Bearer wvr_test"))
            .and(body_string_contains("\"first\":3"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": {
                    "historyItems": [
                        {
                            "id": 10002,
                            "name": "8f1d2c3b4a59687766554433221100ff",
                            "state": "COMPLETE",
                            "error": null,
                            "progressPercent": 100.0,
                            "totalBytes": 123456789_u64,
                            "category": "2000",
                            "attributes": [],
                            "clientRequestId": null,
                            "outputDir": "/data/complete/8f1d2c3b4a59687766554433221100ff.#10002",
                            "createdAt": "2024-01-01T00:00:00Z",
                            "completedAt": "2024-01-01T00:10:00Z",
                            "attention": null
                        },
                        {
                            "id": 10003,
                            "name": "ignored.failed.job",
                            "state": "FAILED",
                            "error": "failed",
                            "progressPercent": 100.0,
                            "totalBytes": 123456789_u64,
                            "category": "2000",
                            "attributes": [],
                            "clientRequestId": null,
                            "outputDir": "/data/complete/ignored.failed.job.#10003",
                            "createdAt": "2024-01-01T00:00:00Z",
                            "completedAt": "2024-01-01T00:11:00Z",
                            "attention": null
                        }
                    ]
                }
            })))
            .expect(1)
            .mount(&server)
            .await;

        let downloads = client
            .list_recent_completed_downloads(3)
            .await
            .expect("recent completed downloads should load");

        assert_eq!(downloads.len(), 1);
        assert_eq!(downloads[0].download_client_item_id, "10002");
    }

    #[tokio::test]
    async fn get_completed_download_uses_direct_history_item_query() {
        let server = MockServer::start().await;
        let client = WeaverDownloadClient::new(server.uri(), Some("wvr_test".to_string()));

        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(header("authorization", "Bearer wvr_test"))
            .and(body_string_contains("historyItem(id"))
            .and(body_string_contains("\"id\":10002"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": {
                    "historyItem": {
                        "id": 10002,
                        "name": "8f1d2c3b4a59687766554433221100ff",
                        "state": "COMPLETE",
                        "error": null,
                        "progressPercent": 100.0,
                        "totalBytes": 123456789_u64,
                        "category": "2000",
                        "attributes": [],
                        "clientRequestId": null,
                        "outputDir": "/data/complete/8f1d2c3b4a59687766554433221100ff.#10002",
                        "createdAt": "2024-01-01T00:00:00Z",
                        "completedAt": "2024-01-01T00:10:00Z",
                        "attention": null
                    }
                }
            })))
            .expect(1)
            .mount(&server)
            .await;

        let download = client
            .get_completed_download("10002")
            .await
            .expect("completed download lookup should load")
            .expect("completed download should exist");

        assert_eq!(download.download_client_item_id, "10002");
        assert_eq!(download.name, "8f1d2c3b4a59687766554433221100ff");
    }

    #[tokio::test]
    async fn trait_targeted_lookup_delegates_to_direct_history_item_query() {
        let server = MockServer::start().await;
        let client = WeaverDownloadClient::new(server.uri(), Some("wvr_test".to_string()));

        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(header("authorization", "Bearer wvr_test"))
            .and(body_string_contains("historyItem(id"))
            .and(body_string_contains("\"id\":10002"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": {
                    "historyItem": {
                        "id": 10002,
                        "name": "8f1d2c3b4a59687766554433221100ff",
                        "state": "COMPLETE",
                        "error": null,
                        "progressPercent": 100.0,
                        "totalBytes": 123456789_u64,
                        "category": "2000",
                        "attributes": [],
                        "clientRequestId": null,
                        "outputDir": "/data/complete/8f1d2c3b4a59687766554433221100ff.#10002",
                        "createdAt": "2024-01-01T00:00:00Z",
                        "completedAt": "2024-01-01T00:10:00Z",
                        "attention": null
                    }
                }
            })))
            .expect(1)
            .mount(&server)
            .await;

        let download = scryer_application::DownloadClient::get_completed_download_for_source(
            &client,
            "weaver-client",
            "weaver",
            "10002",
        )
        .await
        .expect("targeted trait lookup should load")
        .expect("targeted trait lookup should find the row");

        assert_eq!(download.download_client_item_id, "10002");
    }

    #[tokio::test]
    async fn get_completed_download_falls_back_to_bounded_recent_history_when_direct_query_missing()
    {
        let server = MockServer::start().await;
        let client = WeaverDownloadClient::new(server.uri(), Some("wvr_test".to_string()));

        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(header("authorization", "Bearer wvr_test"))
            .and(body_string_contains("historyItem(id"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "errors": [
                    { "message": "Cannot query field \"historyItem\" on type \"Query\"." }
                ]
            })))
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(header("authorization", "Bearer wvr_test"))
            .and(body_string_contains("\"first\":100"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": {
                    "historyItems": [
                        {
                            "id": 10002,
                            "name": "8f1d2c3b4a59687766554433221100ff",
                            "state": "COMPLETE",
                            "error": null,
                            "progressPercent": 100.0,
                            "totalBytes": 123456789_u64,
                            "category": "2000",
                            "attributes": [],
                            "clientRequestId": null,
                            "outputDir": "/data/complete/8f1d2c3b4a59687766554433221100ff.#10002",
                            "createdAt": "2024-01-01T00:00:00Z",
                            "completedAt": "2024-01-01T00:10:00Z",
                            "attention": null
                        }
                    ]
                }
            })))
            .expect(1)
            .mount(&server)
            .await;

        let download = client
            .get_completed_download("10002")
            .await
            .expect("completed download fallback should load")
            .expect("completed download should exist");

        assert_eq!(download.download_client_item_id, "10002");
    }

    #[tokio::test]
    async fn list_queue_for_title_uses_exact_attribute_filter() {
        let server = MockServer::start().await;
        let client = WeaverDownloadClient::new(server.uri(), Some("wvr_test".to_string()));

        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(header("authorization", "Bearer wvr_test"))
            .and(body_string_contains(
                "\"attributeEquals\":{\"key\":\"*scryer_title_id\",\"value\":\"title-42\"}",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": {
                    "queueItems": [
                        {
                            "id": 42,
                            "name": "Title Scoped Queue",
                            "state": "DOWNLOADING",
                            "error": null,
                            "progressPercent": 50.0,
                            "totalBytes": 1000,
                            "downloadedBytes": 500,
                            "failedBytes": 0,
                            "health": 1000,
                            "category": null,
                            "outputDir": null,
                            "createdAt": "2024-01-01T00:00:00Z",
                            "clientRequestId": null,
                            "attributes": [
                                { "key": "*scryer_title_id", "value": "title-42" },
                                { "key": "*scryer_facet", "value": "series" }
                            ],
                            "attention": null
                        }
                    ]
                }
            })))
            .expect(1)
            .mount(&server)
            .await;

        let items = client
            .list_queue_for_title("title-42")
            .await
            .expect("title-scoped queue should load");

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title_id.as_deref(), Some("title-42"));
    }

    #[tokio::test]
    async fn list_recent_activity_for_title_uses_exact_attribute_filter() {
        let server = MockServer::start().await;
        let client = WeaverDownloadClient::new(server.uri(), Some("wvr_test".to_string()));

        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(header("authorization", "Bearer wvr_test"))
            .and(body_string_contains("\"first\":2"))
            .and(body_string_contains(
                "\"attributeEquals\":{\"key\":\"*scryer_title_id\",\"value\":\"title-42\"}",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": {
                    "historyItems": [
                        {
                            "id": 142,
                            "name": "Title Scoped History",
                            "state": "COMPLETE",
                            "error": null,
                            "progressPercent": 100.0,
                            "totalBytes": 1000,
                            "downloadedBytes": 1000,
                            "failedBytes": 0,
                            "health": 1000,
                            "category": null,
                            "outputDir": "/downloads/title-42",
                            "createdAt": "2024-01-01T00:00:00Z",
                            "completedAt": "2024-01-01T00:10:00Z",
                            "clientRequestId": null,
                            "attributes": [
                                { "key": "*scryer_title_id", "value": "title-42" },
                                { "key": "*scryer_facet", "value": "series" }
                            ],
                            "attention": null
                        }
                    ]
                }
            })))
            .expect(1)
            .mount(&server)
            .await;

        let items = client
            .list_recent_activity_for_title("title-42", 2)
            .await
            .expect("title-scoped recent activity should load");

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title_id.as_deref(), Some("title-42"));
    }

    #[tokio::test]
    async fn list_queue_for_title_falls_back_when_attribute_filter_is_unsupported() {
        let server = MockServer::start().await;
        let client = WeaverDownloadClient::new(server.uri(), Some("wvr_test".to_string()));

        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(header("authorization", "Bearer wvr_test"))
            .and(body_string_contains(
                "\"attributeEquals\":{\"key\":\"*scryer_title_id\",\"value\":\"title-42\"}",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "errors": [{
                    "message": "Invalid value for argument \"filter\", unknown field \"attributeEquals\" of type \"QueueFilterInput\""
                }]
            })))
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(header("authorization", "Bearer wvr_test"))
            .and(body_string_contains("\"filter\":null"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": {
                    "queueItems": [
                        {
                            "id": 42,
                            "name": "Title Scoped Queue",
                            "state": "DOWNLOADING",
                            "error": null,
                            "progressPercent": 50.0,
                            "totalBytes": 1000,
                            "downloadedBytes": 500,
                            "failedBytes": 0,
                            "health": 1000,
                            "category": null,
                            "outputDir": null,
                            "createdAt": "2024-01-01T00:00:00Z",
                            "clientRequestId": null,
                            "attributes": [
                                { "key": "*scryer_title_id", "value": "title-42" },
                                { "key": "*scryer_facet", "value": "series" }
                            ],
                            "attention": null
                        },
                        {
                            "id": 43,
                            "name": "Other Queue",
                            "state": "DOWNLOADING",
                            "error": null,
                            "progressPercent": 50.0,
                            "totalBytes": 1000,
                            "downloadedBytes": 500,
                            "failedBytes": 0,
                            "health": 1000,
                            "category": null,
                            "outputDir": null,
                            "createdAt": "2024-01-01T00:00:00Z",
                            "clientRequestId": null,
                            "attributes": [
                                { "key": "*scryer_title_id", "value": "title-17" },
                                { "key": "*scryer_facet", "value": "series" }
                            ],
                            "attention": null
                        }
                    ]
                }
            })))
            .expect(1)
            .mount(&server)
            .await;

        let items = client
            .list_queue_for_title("title-42")
            .await
            .expect("title-scoped queue fallback should load");

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title_id.as_deref(), Some("title-42"));
    }

    #[tokio::test]
    async fn list_recent_activity_for_title_falls_back_when_attribute_filter_is_unsupported() {
        let server = MockServer::start().await;
        let client = WeaverDownloadClient::new(server.uri(), Some("wvr_test".to_string()));

        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(header("authorization", "Bearer wvr_test"))
            .and(body_string_contains("\"first\":2"))
            .and(body_string_contains(
                "\"attributeEquals\":{\"key\":\"*scryer_title_id\",\"value\":\"title-42\"}",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "errors": [{
                    "message": "Invalid value for argument \"filter\", unknown field \"attributeEquals\" of type \"QueueFilterInput\""
                }]
            })))
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(header("authorization", "Bearer wvr_test"))
            .and(body_string_contains("\"first\":2"))
            .and(body_string_contains("\"filter\":null"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": {
                    "historyItems": [
                        {
                            "id": 142,
                            "name": "Title Scoped History",
                            "state": "COMPLETE",
                            "error": null,
                            "progressPercent": 100.0,
                            "totalBytes": 1000,
                            "downloadedBytes": 1000,
                            "failedBytes": 0,
                            "health": 1000,
                            "category": null,
                            "outputDir": "/downloads/title-42",
                            "createdAt": "2024-01-01T00:00:00Z",
                            "completedAt": "2024-01-01T00:10:00Z",
                            "clientRequestId": null,
                            "attributes": [
                                { "key": "*scryer_title_id", "value": "title-42" },
                                { "key": "*scryer_facet", "value": "series" }
                            ],
                            "attention": null
                        },
                        {
                            "id": 143,
                            "name": "Other History",
                            "state": "COMPLETE",
                            "error": null,
                            "progressPercent": 100.0,
                            "totalBytes": 1000,
                            "downloadedBytes": 1000,
                            "failedBytes": 0,
                            "health": 1000,
                            "category": null,
                            "outputDir": "/downloads/title-17",
                            "createdAt": "2024-01-01T00:00:00Z",
                            "completedAt": "2024-01-01T00:10:00Z",
                            "clientRequestId": null,
                            "attributes": [
                                { "key": "*scryer_title_id", "value": "title-17" },
                                { "key": "*scryer_facet", "value": "series" }
                            ],
                            "attention": null
                        }
                    ]
                }
            })))
            .expect(1)
            .mount(&server)
            .await;

        let items = client
            .list_recent_activity_for_title("title-42", 2)
            .await
            .expect("title-scoped recent activity fallback should load");

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title_id.as_deref(), Some("title-42"));
    }
}

use async_trait::async_trait;
use base64::Engine as _;
use chrono::TimeZone;
use chrono::{DateTime, Utc};
use reqwest::Client;
use reqwest::multipart;
use scryer_application::{
    AppError, AppResult, DownloadClient, DownloadClientAddRequest, DownloadGrabResult,
    NullStagedNzbStore, StagedNzbStore,
};
use scryer_domain::{
    CompletedDownload, DownloadClientConfig, DownloadQueueItem, DownloadQueueState,
};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::fs::File;
use tokio::sync::Semaphore;
use tokio_util::io::ReaderStream;
use tracing::{debug, warn};

use super::{
    parse_download_client_config_json, read_config_string, resolve_download_client_base_url,
    resolve_staged_nzb_for_request,
};

#[derive(Clone)]
pub struct WeaverDownloadClient {
    graphql_url: String,
    api_key: Option<String>,
    http_client: Client,
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

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WeaverAttribute {
    pub key: String,
    pub value: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WeaverAttention {
    pub message: String,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
pub(crate) enum WeaverQueueState {
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
pub(crate) struct WeaverQueueItem {
    pub id: u64,
    pub name: String,
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
struct PublishedJobsPayload {
    jobs: Vec<PublishedWeaverJob>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PublishedWeaverJob {
    id: u64,
    name: String,
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
    item: SubmissionQueueItemPayload,
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
        Self {
            graphql_url,
            api_key,
            http_client: Client::new(),
            staged_nzb_store,
            staged_nzb_pipeline_limit,
        }
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
        let payload = json!({ "query": query, "variables": variables });

        let response = self
            .with_auth_headers(
                self.http_client
                    .post(&self.graphql_url)
                    .header("Content-Type", "application/json")
                    .json(&payload),
            )
            .send()
            .await
            .map_err(|err| AppError::Repository(format!("weaver request failed: {err}")))?;

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|err| AppError::Repository(format!("weaver response read failed: {err}")))?;

        Self::parse_graphql_response(status, &body)
    }

    async fn graphql_multipart_request<T>(
        &self,
        query: &str,
        variables: Value,
        upload_variable_path: &str,
        filename: String,
        upload_path: &std::path::Path,
        content_type: &str,
        content_length: u64,
    ) -> AppResult<T>
    where
        T: DeserializeOwned,
    {
        let file = File::open(upload_path).await.map_err(|error| {
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
        .mime_str(content_type)
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

        let response = self
            .with_auth_headers(self.http_client.post(&self.graphql_url).multipart(form))
            .send()
            .await
            .map_err(|err| {
                AppError::Repository(format!("weaver multipart request failed: {err}"))
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

    /// Test connectivity by querying metrics.
    pub async fn test_connection(&self) -> AppResult<String> {
        let query = "query { systemMetrics { bytesDownloaded } }";
        match self
            .graphql_request::<SystemMetricsPayload>(query, json!({}))
            .await
        {
            Ok(_) => {}
            Err(error) if is_weaver_schema_error(&error, "Unknown field \"systemMetrics\"") => {
                let compat_query = "query { version }";
                let _: VersionPayload = self.graphql_request(compat_query, json!({})).await?;
            }
            Err(error) => return Err(error),
        }
        Ok("weaver".to_string())
    }

    async fn query_queue_items(&self) -> AppResult<Vec<WeaverQueueItem>> {
        let query = r#"
            query {
                queueItems {
                    id
                    name
                    state
                    error
                    progressPercent
                    totalBytes
                    downloadedBytes
                    failedBytes
                    health
                    category
                    outputDir
                    createdAt
                    clientRequestId
                    attributes { key value }
                    attention { code message }
                }
            }
        "#;
        match self
            .graphql_request::<QueueItemsPayload>(query, json!({}))
            .await
        {
            Ok(data) => Ok(data.queue_items),
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
            Err(error) => Err(error),
        }
    }

    async fn query_history_items(
        &self,
        limit: Option<usize>,
        offset: Option<usize>,
    ) -> AppResult<Vec<WeaverQueueItem>> {
        let query = r#"
            query($first: Int, $after: String) {
                historyItems(first: $first, after: $after) {
                    id
                    name
                    state
                    error
                    progressPercent
                    totalBytes
                    downloadedBytes
                    failedBytes
                    health
                    category
                    outputDir
                    createdAt
                    completedAt
                    clientRequestId
                    attributes { key value }
                    attention { code message }
                }
            }
        "#;
        let after = offset.map(|value| {
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(format!("off:{value}"))
        });
        let data: HistoryItemsPayload = self
            .graphql_request(
                query,
                json!({
                    "first": limit.and_then(|value| i32::try_from(value).ok()),
                    "after": after,
                }),
            )
            .await?;
        Ok(data.history_items)
    }

    async fn query_jobs_compat(
        &self,
        statuses: Option<&[&str]>,
        limit: Option<usize>,
        offset: Option<usize>,
    ) -> AppResult<Vec<WeaverQueueItem>> {
        let query = r#"
            query($status: [JobStatusGql!], $limit: Int, $offset: Int) {
                jobs(status: $status, limit: $limit, offset: $offset) {
                    id
                    name
                    status
                    error
                    progressPercent: progress
                    totalBytes
                    downloadedBytes
                    failedBytes
                    health
                    category
                    metadata { key value }
                    outputDir
                    createdAt
                }
            }
        "#;
        let data: PublishedJobsPayload = self
            .graphql_request(
                query,
                json!({
                    "status": statuses.map(|values| values.iter().copied().collect::<Vec<_>>()),
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

fn extract_scryer_metadata(
    attributes: &[WeaverAttribute],
    client_request_id: Option<&str>,
) -> (Option<String>, Option<String>, bool) {
    let mut title_id = None;
    let mut facet = None;
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
    (title_id, facet, is_scryer)
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
pub(crate) fn weaver_item_to_queue_item(job: &WeaverQueueItem) -> DownloadQueueItem {
    let state = map_weaver_status(job.state);

    let attention_reason = if state == DownloadQueueState::Failed {
        job.error
            .clone()
            .or_else(|| job.attention.as_ref().map(|value| value.message.clone()))
    } else {
        job.attention.as_ref().map(|value| value.message.clone())
    };

    let (title_id, facet, is_scryer) =
        extract_scryer_metadata(&job.attributes, job.client_request_id.as_deref());

    // Calculate remaining seconds from progress and download speed.
    // We don't have per-job speed, so leave it as None.
    DownloadQueueItem {
        id: job.id.to_string(),
        title_id,
        title_name: job.name.clone(),
        facet,
        client_id: String::new(),
        client_name: String::new(),
        client_type: "weaver".to_string(),
        state,
        progress_percent: if state == DownloadQueueState::Completed {
            100
        } else {
            job.progress_percent.round().clamp(0.0, 100.0) as u8
        },
        size_bytes: Some(job.total_bytes as i64),
        remaining_seconds: None,
        queued_at: Some(job.created_at.to_rfc3339()),
        last_updated_at: None,
        attention_required: job.attention.is_some() || matches!(state, DownloadQueueState::Failed),
        attention_reason,
        download_client_item_id: job.id.to_string(),
        import_status: None,
        import_error_message: None,
        imported_at: None,
        is_scryer_origin: is_scryer,
        tracked_state: None,
        tracked_status: None,
        tracked_status_messages: Vec::new(),
        tracked_match_type: None,
    }
}

fn compat_job_to_queue_item(job: PublishedWeaverJob) -> WeaverQueueItem {
    WeaverQueueItem {
        id: job.id,
        name: job.name,
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
            &self.http_client,
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
        ];

        if let Some(imdb_id) = title
            .external_ids
            .iter()
            .find(|id| id.source.eq_ignore_ascii_case("imdb"))
            .map(|id| id.value.trim().to_string())
            .filter(|v| !v.is_empty())
        {
            attributes.push(json!({"key": "*scryer_imdb_id", "value": imdb_id}));
        }

        let client_request_id = format!(
            "scryer:{}:{}",
            title.id,
            request
                .release_title
                .clone()
                .or_else(|| normalized_source_title.clone())
                .unwrap_or_else(|| title.name.clone())
        );

        let result: AppResult<DownloadGrabResult> = async {
            let query = r#"
                mutation($input: SubmitNzbInput!) {
                    submitNzb(input: $input) {
                        accepted
                        clientRequestId
                        item { id name state }
                    }
                }
            "#;

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
                .graphql_multipart_request::<SubmissionPayload>(
                    query,
                    variables.clone(),
                    "variables.input.nzbUpload",
                    format!("{nzb_filename}.zst"),
                    &staged.staged_nzb.compressed_path,
                    "application/zstd",
                    tokio::fs::metadata(&staged.staged_nzb.compressed_path)
                        .await
                        .map_err(|error| {
                            AppError::Repository(format!(
                                "failed to stat staged nzb {}: {error}",
                                staged.staged_nzb.compressed_path.display()
                            ))
                        })?
                        .len(),
                )
                .await
            {
                Ok(data) => {
                    if !data.submit_nzb.accepted {
                        return Err(AppError::Repository(
                            "weaver submitNzb did not accept the submission".into(),
                        ));
                    }
                    let job_id = data.submit_nzb.item.id;

                    debug!(
                        endpoint = self.graphql_url.as_str(),
                        job_id,
                        title = title.name.as_str(),
                        "weaver submitNzb succeeded"
                    );

                    Ok(DownloadGrabResult {
                        job_id: job_id.to_string(),
                        client_type: "weaver".to_string(),
                    })
                }
                Err(error)
                    if is_weaver_schema_error(&error, "Unknown type \"SubmitNzbInput\"")
                        || is_weaver_schema_error(&error, "Unknown argument \"input\"")
                        || is_weaver_schema_error(&error, "Unknown field \"accepted\"") =>
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
                    let compat_query = r#"
                        mutation(
                            $source: NzbSourceInput!
                            $filename: String
                            $password: String
                            $category: String
                            $metadata: [MetadataInput!]
                        ) {
                            submitNzb(
                                source: $source
                                filename: $filename
                                password: $password
                                category: $category
                                metadata: $metadata
                            ) {
                                id
                            }
                        }
                    "#;
                    let compat_data: PublishedSubmissionPayload = self
                        .graphql_request(
                            compat_query,
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
                        .await?;
                    Ok(DownloadGrabResult {
                        job_id: compat_data.submit_nzb.id.to_string(),
                        client_type: "weaver".to_string(),
                    })
                }
                Err(error) => Err(error),
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
        let jobs = self.query_queue_items().await?;
        Ok(jobs.iter().map(weaver_item_to_queue_item).collect())
    }

    async fn list_history(&self) -> AppResult<Vec<DownloadQueueItem>> {
        let jobs = match self.query_history_items(None, None).await {
            Ok(items) => items,
            Err(error) if is_weaver_schema_error(&error, "Unknown field \"historyItems\"") => {
                self.query_jobs_compat(Some(&["COMPLETE", "FAILED"]), Some(200), Some(0))
                    .await?
            }
            Err(error) => return Err(error),
        };
        Ok(jobs.iter().map(weaver_item_to_queue_item).collect())
    }

    async fn list_history_page(
        &self,
        offset: usize,
        limit: usize,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        let jobs = match self.query_history_items(Some(limit), Some(offset)).await {
            Ok(items) => items,
            Err(error) if is_weaver_schema_error(&error, "Unknown field \"historyItems\"") => {
                self.query_jobs_compat(Some(&["COMPLETE", "FAILED"]), Some(limit), Some(offset))
                    .await?
            }
            Err(error) => return Err(error),
        };
        Ok(jobs.iter().map(weaver_item_to_queue_item).collect())
    }

    async fn list_completed_downloads(&self) -> AppResult<Vec<CompletedDownload>> {
        let jobs = match self.query_history_items(None, None).await {
            Ok(items) => items,
            Err(error) if is_weaver_schema_error(&error, "Unknown field \"historyItems\"") => {
                self.query_jobs_compat(Some(&["COMPLETE", "FAILED"]), Some(200), Some(0))
                    .await?
            }
            Err(error) => return Err(error),
        };
        Ok(jobs
            .iter()
            .filter_map(|job| {
                if job.state != WeaverQueueState::Completed {
                    return None;
                }
                let output_dir = job
                    .output_dir
                    .as_ref()
                    .filter(|v| !v.is_empty())?
                    .to_string();
                let parameters = job
                    .attributes
                    .iter()
                    .map(|entry| (entry.key.clone(), entry.value.clone()))
                    .collect::<Vec<_>>();

                // Only return jobs submitted by scryer (have *scryer_title_id metadata).
                let is_scryer = parameters.iter().any(|(k, _)| k == "*scryer_title_id")
                    || job
                        .client_request_id
                        .as_deref()
                        .map(|value| value.trim_start().starts_with("scryer:"))
                        .unwrap_or(false);
                if !is_scryer {
                    return None;
                }

                Some(CompletedDownload {
                    client_type: "weaver".to_string(),
                    client_id: String::new(),
                    download_client_item_id: job.id.to_string(),
                    name: job.name.clone(),
                    dest_dir: output_dir,
                    category: job.category.clone(),
                    size_bytes: Some(job.total_bytes as i64),
                    completed_at: job.completed_at.or(Some(Utc::now())),
                    parameters,
                })
            })
            .collect())
    }

    async fn pause_queue_item(&self, id: &str) -> AppResult<()> {
        let job_id: u64 = id
            .parse()
            .map_err(|_| AppError::Validation(format!("invalid weaver job id: {id}")))?;
        let query = "mutation($id: Int!) { pauseQueueItem(id: $id) { success } }";
        match self
            .graphql_request::<PauseQueueItemPayload>(query, json!({ "id": job_id }))
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
                let compat_query = "mutation($id: Int!) { pauseJob(id: $id) }";
                let data: PublishedBoolPayload = self
                    .graphql_request(compat_query, json!({ "id": job_id }))
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
        let query = "mutation($id: Int!) { resumeQueueItem(id: $id) { success } }";
        match self
            .graphql_request::<ResumeQueueItemPayload>(query, json!({ "id": job_id }))
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
                let compat_query = "mutation($id: Int!) { resumeJob(id: $id) }";
                let data: PublishedBoolPayload = self
                    .graphql_request(compat_query, json!({ "id": job_id }))
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

    async fn delete_queue_item(&self, id: &str, is_history: bool) -> AppResult<()> {
        let job_id: u64 = id
            .parse()
            .map_err(|_| AppError::Validation(format!("invalid weaver job id: {id}")))?;
        let query = if is_history {
            "mutation($ids: [Int!]!) { removeHistoryItems(ids: $ids) { success removedIds } }"
        } else {
            "mutation($id: Int!) { cancelQueueItem(id: $id) { success } }"
        };
        if is_history {
            match self
                .graphql_request::<RemoveHistoryItemsPayload>(query, json!({ "ids": [job_id] }))
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
                    if is_weaver_schema_error(&error, "Unknown field \"removeHistoryItems\"") =>
                {
                    let compat_query = "mutation($ids: [Int!]!, $deleteFiles: Boolean!) { deleteHistoryBatch(ids: $ids, deleteFiles: $deleteFiles) }";
                    let data: PublishedDeleteHistoryPayload = self
                        .graphql_request(
                            compat_query,
                            json!({ "ids": [job_id], "deleteFiles": false }),
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
                .graphql_request::<CancelQueueItemPayload>(query, json!({ "id": job_id }))
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
                    let compat_query = "mutation($id: Int!) { cancelJob(id: $id) }";
                    let data: PublishedBoolPayload = self
                        .graphql_request(compat_query, json!({ "id": job_id }))
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

    use super::{WeaverDownloadClient, WeaverQueueItem, weaver_item_to_queue_item};
    use scryer_domain::{DownloadClientConfig, DownloadQueueState};

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
        }
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

        assert_eq!(item.title_id.as_deref(), Some("title-77"));
        assert!(item.is_scryer_origin);
    }
}

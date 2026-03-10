use async_trait::async_trait;
use base64::{engine::general_purpose, Engine as _};
use chrono::{DateTime, Utc};
use reqwest::Client;
use scryer_application::{
    AppError, AppResult, DownloadClient, DownloadClientAddRequest, DownloadGrabResult,
};
use scryer_domain::{CompletedDownload, DownloadQueueItem, DownloadQueueState};
use serde_json::{json, Value};
use tracing::debug;

use super::is_http_url;

#[derive(Clone)]
pub struct WeaverDownloadClient {
    graphql_url: String,
    http_client: Client,
}

impl WeaverDownloadClient {
    pub fn new(base_url: String) -> Self {
        let base = base_url.trim_end_matches('/').to_string();
        let graphql_url = format!("{base}/graphql");
        Self {
            graphql_url,
            http_client: Client::new(),
        }
    }

    pub fn graphql_url(&self) -> &str {
        &self.graphql_url
    }

    /// Derive the WebSocket URL from the HTTP GraphQL endpoint.
    pub fn ws_url(&self) -> String {
        let url = self
            .graphql_url
            .replace("https://", "wss://")
            .replace("http://", "ws://");
        format!("{url}/ws")
    }

    async fn graphql_request(&self, query: &str, variables: Value) -> AppResult<Value> {
        let payload = json!({ "query": query, "variables": variables });

        let response = self
            .http_client
            .post(&self.graphql_url)
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|err| AppError::Repository(format!("weaver request failed: {err}")))?;

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|err| AppError::Repository(format!("weaver response read failed: {err}")))?;

        if !status.is_success() {
            let preview: String = body.chars().take(500).collect();
            return Err(AppError::Repository(format!(
                "weaver returned status {status}: {preview}"
            )));
        }

        let json: Value = serde_json::from_str(&body).map_err(|err| {
            AppError::Repository(format!("weaver returned non-json response: {err}"))
        })?;

        if let Some(errors) = json.get("errors").and_then(Value::as_array) {
            if let Some(first) = errors.first() {
                let message = first
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown error");
                return Err(AppError::Repository(format!(
                    "weaver GraphQL error: {message}"
                )));
            }
        }

        json.get("data")
            .cloned()
            .ok_or_else(|| AppError::Repository("weaver response missing data field".into()))
    }

    /// Test connectivity by querying metrics.
    pub async fn test_connection(&self) -> AppResult<String> {
        let query = "query { metrics { bytesDownloaded } }";
        self.graphql_request(query, json!({})).await?;
        Ok("weaver".to_string())
    }

    async fn fetch_and_encode_nzb(&self, source_hint: &str) -> AppResult<String> {
        let response = self
            .http_client
            .get(source_hint)
            .header("User-Agent", "scryer/0.1")
            .send()
            .await
            .map_err(|err| AppError::Repository(format!("nzb download request failed: {err}")))?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.map_err(|err| {
                AppError::Repository(format!("nzb download response read failed: {err}"))
            })?;
            let preview: String = body.chars().take(300).collect();
            return Err(AppError::Repository(format!(
                "nzb download failed with status {status}: {preview}"
            )));
        }

        let bytes = response
            .bytes()
            .await
            .map_err(|err| AppError::Repository(format!("nzb download body read failed: {err}")))?;
        if bytes.is_empty() {
            return Err(AppError::Repository(
                "nzb download response body was empty".into(),
            ));
        }

        let text = String::from_utf8_lossy(&bytes);
        let trimmed = text.trim_start();
        if !trimmed.starts_with('<') {
            return Err(AppError::Repository(
                "nzb download payload did not look like xml".into(),
            ));
        }

        Ok(general_purpose::STANDARD.encode(bytes))
    }

    /// Query weaver jobs, optionally filtering by status.
    async fn query_jobs(&self, status_filter: Option<&[&str]>) -> AppResult<Vec<Value>> {
        let query = r#"
            query($status: [JobStatusGql!]) {
                jobs(status: $status) {
                    id name status error progress totalBytes downloadedBytes
                    failedBytes health hasPassword category outputDir createdAt
                    metadata { key value }
                }
            }
        "#;
        let variables = match status_filter {
            Some(statuses) => json!({ "status": statuses }),
            None => json!({}),
        };
        let data = self.graphql_request(query, variables).await?;
        Ok(data
            .get("jobs")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default())
    }
}

/// Extract scryer metadata from weaver job metadata entries.
fn extract_scryer_metadata(job: &Value) -> (Option<String>, Option<String>, bool) {
    let metadata = match job.get("metadata").and_then(Value::as_array) {
        Some(m) => m,
        None => return (None, None, false),
    };

    let mut title_id = None;
    let mut facet = None;
    for entry in metadata {
        let key = entry.get("key").and_then(Value::as_str).unwrap_or("");
        let value = entry
            .get("value")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        match key {
            "*scryer_title_id" => title_id = Some(value),
            "*scryer_facet" => facet = Some(value),
            _ => {}
        }
    }

    let is_scryer = title_id.is_some();
    (title_id, facet, is_scryer)
}

/// Map a weaver job status string to scryer's DownloadQueueState.
fn map_weaver_status(status: &str) -> DownloadQueueState {
    match status {
        "QUEUED" => DownloadQueueState::Queued,
        "DOWNLOADING" | "CHECKING" => DownloadQueueState::Downloading,
        "VERIFYING" => DownloadQueueState::Verifying,
        "REPAIRING" => DownloadQueueState::Repairing,
        "EXTRACTING" => DownloadQueueState::Extracting,
        "COMPLETE" => DownloadQueueState::Completed,
        "FAILED" => DownloadQueueState::Failed,
        "PAUSED" => DownloadQueueState::Paused,
        _ => DownloadQueueState::Queued,
    }
}

/// Map a weaver job JSON object to a scryer DownloadQueueItem.
pub(crate) fn weaver_job_to_queue_item(job: &Value) -> Option<DownloadQueueItem> {
    let id = job.get("id")?.as_u64()?;
    let name = job
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("Unnamed download")
        .to_string();
    let status_str = job
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("QUEUED");
    let state = map_weaver_status(status_str);

    let attention_reason = if state == DownloadQueueState::Failed {
        job.get("error").and_then(Value::as_str).map(String::from)
    } else {
        None
    };

    let progress = job.get("progress").and_then(Value::as_f64).unwrap_or(0.0);
    let total_bytes = job.get("totalBytes").and_then(Value::as_u64).unwrap_or(0);

    let (title_id, facet, is_scryer) = extract_scryer_metadata(job);

    let created_at = job
        .get("createdAt")
        .and_then(Value::as_f64)
        .map(|ms| (ms as i64).to_string());

    // Calculate remaining seconds from progress and download speed.
    // We don't have per-job speed, so leave it as None.
    Some(DownloadQueueItem {
        id: id.to_string(),
        title_id,
        title_name: name,
        facet,
        client_id: String::new(),
        client_name: String::new(),
        client_type: "weaver".to_string(),
        state,
        progress_percent: if state == DownloadQueueState::Completed {
            100
        } else {
            (progress * 100.0).round().clamp(0.0, 100.0) as u8
        },
        size_bytes: Some(total_bytes as i64),
        remaining_seconds: None,
        queued_at: created_at,
        last_updated_at: None,
        attention_required: matches!(state, DownloadQueueState::Failed),
        attention_reason,
        download_client_item_id: id.to_string(),
        import_status: None,
        import_error_message: None,
        imported_at: None,
        is_scryer_origin: is_scryer,
    })
}

fn derive_nzb_filename(source_title: Option<&str>, source_hint: &str, title_name: &str) -> String {
    if let Some(name) = source_title {
        if !name.is_empty() {
            return if name.ends_with(".nzb") {
                name.to_string()
            } else {
                format!("{name}.nzb")
            };
        }
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
            .clone()
            .and_then(|v| {
                let v = v.trim().to_string();
                (!v.is_empty()).then_some(v)
            })
            .ok_or_else(|| {
                AppError::Validation("source hint is required to queue a download".into())
            })?;

        if !is_http_url(&source_hint) {
            return Err(AppError::Validation(format!(
                "source hint must be an NZB URL; got {source_hint}"
            )));
        }

        let normalized_source_title = request.source_title.clone().and_then(|v| {
            let t = v.trim().to_string();
            (!t.is_empty()).then_some(t)
        });
        let nzb_filename = derive_nzb_filename(
            normalized_source_title.as_deref(),
            &source_hint,
            &title.name,
        );

        let nzb_base64 = self.fetch_and_encode_nzb(&source_hint).await?;

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

        let mut metadata = vec![
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
            metadata.push(json!({"key": "*scryer_imdb_id", "value": imdb_id}));
        }

        let query = r#"
            mutation($nzbBase64: String!, $filename: String, $password: String, $category: String, $metadata: [MetadataInput!]) {
                submitNzb(nzbBase64: $nzbBase64, filename: $filename, password: $password, category: $category, metadata: $metadata) {
                    id name status
                }
            }
        "#;

        let variables = json!({
            "nzbBase64": nzb_base64,
            "filename": nzb_filename,
            "password": password,
            "category": category,
            "metadata": metadata,
        });

        debug!(
            endpoint = self.graphql_url.as_str(),
            title = title.name.as_str(),
            filename = nzb_filename.as_str(),
            "weaver submitNzb request"
        );

        let data = self.graphql_request(query, variables).await?;
        let job = data
            .get("submitNzb")
            .ok_or_else(|| AppError::Repository("weaver submitNzb response missing job".into()))?;
        let job_id = job
            .get("id")
            .and_then(Value::as_u64)
            .ok_or_else(|| AppError::Repository("weaver submitNzb returned no job id".into()))?;

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

    async fn test_connection(&self) -> AppResult<String> {
        WeaverDownloadClient::test_connection(self).await
    }

    async fn list_queue(&self) -> AppResult<Vec<DownloadQueueItem>> {
        let jobs = self.query_jobs(None).await?;
        Ok(jobs.iter().filter_map(weaver_job_to_queue_item).collect())
    }

    async fn list_history(&self) -> AppResult<Vec<DownloadQueueItem>> {
        let jobs = self.query_jobs(Some(&["COMPLETE", "FAILED"])).await?;
        Ok(jobs.iter().filter_map(weaver_job_to_queue_item).collect())
    }

    async fn list_completed_downloads(&self) -> AppResult<Vec<CompletedDownload>> {
        let jobs = self.query_jobs(Some(&["COMPLETE"])).await?;
        Ok(jobs
            .iter()
            .filter_map(|job| {
                let id = job.get("id")?.as_u64()?;
                let name = job
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("Unnamed")
                    .to_string();
                let output_dir = job
                    .get("outputDir")
                    .and_then(Value::as_str)
                    .filter(|v| !v.is_empty())?
                    .to_string();
                let total_bytes = job.get("totalBytes").and_then(Value::as_u64).unwrap_or(0);
                let category = job
                    .get("category")
                    .and_then(Value::as_str)
                    .filter(|v| !v.is_empty())
                    .map(String::from);

                let parameters = job
                    .get("metadata")
                    .and_then(Value::as_array)
                    .map(|entries| {
                        entries
                            .iter()
                            .filter_map(|e| {
                                let key = e.get("key").and_then(Value::as_str)?.to_string();
                                let value = e.get("value").and_then(Value::as_str)?.to_string();
                                Some((key, value))
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();

                let completed_at = job
                    .get("createdAt")
                    .and_then(Value::as_f64)
                    .and_then(|ms| DateTime::from_timestamp_millis(ms as i64))
                    .or_else(|| Some(Utc::now()));

                Some(CompletedDownload {
                    client_type: "weaver".to_string(),
                    client_id: String::new(),
                    download_client_item_id: id.to_string(),
                    name,
                    dest_dir: output_dir,
                    category,
                    size_bytes: Some(total_bytes as i64),
                    completed_at,
                    parameters,
                })
            })
            .collect())
    }

    async fn pause_queue_item(&self, id: &str) -> AppResult<()> {
        let job_id: u64 = id
            .parse()
            .map_err(|_| AppError::Validation(format!("invalid weaver job id: {id}")))?;
        let query = "mutation($id: Int!) { pauseJob(id: $id) }";
        self.graphql_request(query, json!({ "id": job_id })).await?;
        Ok(())
    }

    async fn resume_queue_item(&self, id: &str) -> AppResult<()> {
        let job_id: u64 = id
            .parse()
            .map_err(|_| AppError::Validation(format!("invalid weaver job id: {id}")))?;
        let query = "mutation($id: Int!) { resumeJob(id: $id) }";
        self.graphql_request(query, json!({ "id": job_id })).await?;
        Ok(())
    }

    async fn delete_queue_item(&self, id: &str, is_history: bool) -> AppResult<()> {
        let job_id: u64 = id
            .parse()
            .map_err(|_| AppError::Validation(format!("invalid weaver job id: {id}")))?;
        let query = if is_history {
            "mutation($id: Int!) { deleteHistory(id: $id) { id } }"
        } else {
            "mutation($id: Int!) { cancelJob(id: $id) }"
        };
        self.graphql_request(query, json!({ "id": job_id })).await?;
        Ok(())
    }
}

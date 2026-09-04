use std::borrow::Cow;
use std::collections::HashMap;
use std::time::Duration;

use async_compression::tokio::bufread::ZstdDecoder;
use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose};
use chrono::{DateTime, Utc};
use futures_util::stream;
use scryer_application::{
    AppError, AppResult, DownloadClient, DownloadClientAddRequest, DownloadGrabResult,
    NullStagedNzbStore, RateLimitCooldownAction, StagedNzbRef, StagedNzbStore,
};
use scryer_domain::{DownloadQueueItem, DownloadQueueState};
use scryer_outbound_http::{
    OutboundHttpClient, OutboundHttpError, OutboundRequestError, RateLimitRegistry, RequestPolicy,
    generic_reqwest_client,
};
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, BufReader};
use tokio::sync::Semaphore;
use tracing::{debug, info, trace, warn};

use super::{
    extract_f64_value, extract_i64_value, parse_duration_seconds, progress_percent_from_sizes,
    resolve_staged_nzb_for_request, size_to_bytes,
};

// NZBGet can take a while to serialize large queue and history responses. Safe reads may make
// three attempts, so 90 seconds per attempt still fits beneath the default 300-second client gate.
const NZBGET_HTTP_REQUEST_TIMEOUT: Duration = scryer_outbound_http::DOWNLOAD_CLIENT_HTTP_TIMEOUT;

#[derive(Clone)]
pub struct NzbgetDownloadClient {
    rpc_url: String,
    username: Option<String>,
    password: Option<String>,
    dupe_mode: String,
    outbound_http: OutboundHttpClient,
    staged_nzb_store: Arc<dyn StagedNzbStore>,
    staged_nzb_pipeline_limit: Arc<Semaphore>,
}

#[derive(Clone, Copy)]
struct NzbgetAppendRequest<'a> {
    /// JSON-RPC request correlation ID (not the NZBGet queue ID).
    request_id: &'a str,
    title_name: &'a str,
    nzb_filename: &'a str,
    source_for_payload: &'a str,
    category: &'a str,
    queue_priority: i32,
    parameters: &'a [Value],
    use_auto_category: bool,
}

impl NzbgetDownloadClient {
    pub fn new(
        rpc_url: String,
        username: Option<String>,
        password: Option<String>,
        dupe_mode: String,
    ) -> Self {
        Self::with_staged_nzb_store(
            rpc_url,
            username,
            password,
            dupe_mode,
            Arc::new(NullStagedNzbStore),
            Arc::new(Semaphore::new(4)),
        )
    }

    pub fn with_staged_nzb_store(
        rpc_url: String,
        username: Option<String>,
        password: Option<String>,
        dupe_mode: String,
        staged_nzb_store: Arc<dyn StagedNzbStore>,
        staged_nzb_pipeline_limit: Arc<Semaphore>,
    ) -> Self {
        let dupe_mode = match dupe_mode.to_uppercase().as_str() {
            "ALL" | "FORCE" => dupe_mode.to_uppercase(),
            _ => "SCORE".to_string(),
        };
        let http_client = generic_reqwest_client();
        Self {
            rpc_url: rpc_url.trim_end_matches('/').to_string(),
            username,
            password,
            dupe_mode,
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

    pub fn endpoint(&self) -> String {
        if self.rpc_url.is_empty() {
            "http://127.0.0.1:6789/jsonrpc".to_string()
        } else if self.rpc_url.ends_with("/jsonrpc") {
            self.rpc_url.clone()
        } else {
            format!("{}/jsonrpc", self.rpc_url)
        }
    }

    async fn rpc_call(&self, method: &str, params: Vec<Value>) -> AppResult<Value> {
        self.rpc_call_with_policy(
            method,
            params,
            self.read_policy(format!("nzbget_rpc_{method}")),
        )
        .await
    }

    async fn rpc_call_with_policy(
        &self,
        method: &str,
        params: Vec<Value>,
        policy: RequestPolicy,
    ) -> AppResult<Value> {
        let payload = json!({
            "version": "2.0",
            "method": method,
            "params": params,
            "id": "scryer-rpc",
        });

        let endpoint = self.endpoint();
        let response = self
            .outbound_http
            .send(policy, || {
                let mut request = self
                    .outbound_http
                    .client()
                    .post(&endpoint)
                    .timeout(NZBGET_HTTP_REQUEST_TIMEOUT)
                    .header("Content-Type", "application/json")
                    .json(&payload);

                if let Some(username) = self.username.as_ref() {
                    request = request.basic_auth(username, self.password.as_deref());
                }

                request
            })
            .await
            .map_err(|error| {
                map_nzbget_outbound_error(&format!("nzbget rpc call {method}"), error)
            })?;

        let status = response.status();
        let response_text = response.text().await.map_err(|err| {
            AppError::Repository(format!("nzbget rpc call response read failed: {err}"))
        })?;

        if !status.is_success() {
            let preview = response_text.chars().take(600).collect::<String>();
            return Err(AppError::Repository(format!(
                "nzbget rpc call {method} returned status {status}: {preview}"
            )));
        }

        let response_json: Value = serde_json::from_str(&response_text).map_err(|err| {
            AppError::Repository(format!("nzbget rpc call returned non-json response: {err}"))
        })?;

        if let Some(error) = response_json.get("error") {
            let code = error
                .get("code")
                .and_then(Value::as_i64)
                .unwrap_or_default();
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            return Err(AppError::Repository(format!(
                "nzbget rpc call {method} failed with error code {code}: {message}"
            )));
        }

        response_json
            .get("result")
            .cloned()
            .ok_or_else(|| AppError::Repository(format!("nzbget rpc call {method} missing result")))
    }

    pub async fn test_connection(&self) -> AppResult<String> {
        let payload = json!({
            "version": "2.0",
            "method": "version",
            "params": [],
            "id": "scryer-test",
        });
        let endpoint = self.endpoint();
        let response = self
            .outbound_http
            .send(self.read_policy("nzbget_test_connection"), || {
                let mut request = self
                    .outbound_http
                    .client()
                    .post(&endpoint)
                    .timeout(NZBGET_HTTP_REQUEST_TIMEOUT)
                    .header("Content-Type", "application/json")
                    .json(&payload);

                if let Some(username) = self.username.as_ref() {
                    request = request.basic_auth(username, self.password.as_deref());
                }

                request
            })
            .await
            .map_err(|error| map_nzbget_outbound_error("nzbget test call", error))?;
        let status = response.status();

        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(AppError::Repository(
                "nzbget authentication failed: check username and password".into(),
            ));
        }
        if !status.is_success() {
            return Err(AppError::Repository(format!(
                "nzbget test call returned status {status}"
            )));
        }

        let response_text = response.text().await.map_err(|err| {
            AppError::Repository(format!("nzbget test call response read failed: {err}"))
        })?;

        let response_json: Value = serde_json::from_str(&response_text).map_err(|err| {
            AppError::Repository(format!(
                "nzbget test call returned non-json response: {err}"
            ))
        })?;
        if let Some(error) = response_json.get("error") {
            let code = error
                .get("code")
                .and_then(Value::as_i64)
                .unwrap_or_default();
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            return Err(AppError::Repository(format!(
                "nzbget test call failed with error code {code}: {message}"
            )));
        }
        let result = response_json.get("result");
        if result.is_none() {
            return Err(AppError::Repository(
                "nzbget test call response missing result".to_string(),
            ));
        }

        let version = response_json
            .get("result")
            .map_or("nzbget", |result| match result {
                Value::String(value) => value.as_str(),
                _ => "nzbget",
            });
        let version = version.to_string();

        // Validate minimum version (v12+ required for append API)
        if let Some(major) = parse_nzbget_major_version(&version)
            && major < 12
        {
            return Err(AppError::Repository(format!(
                "nzbget {version} is not supported; version 12.0+ is required"
            )));
        }

        // Check KeepHistory config — if 0, completed downloads are immediately
        // purged and auto-import will never see them.
        match self.rpc_call("config", vec![]).await {
            Ok(config_result) => {
                if let Some(entries) = config_result.as_array() {
                    let keep_history = entries.iter().find_map(|entry| {
                        let obj = entry.as_object()?;
                        let name = obj.get("Name").and_then(Value::as_str)?;
                        if name.eq_ignore_ascii_case("KeepHistory") {
                            obj.get("Value")
                                .and_then(Value::as_str)
                                .map(|v| v.to_string())
                        } else {
                            None
                        }
                    });
                    if let Some(kh) = keep_history
                        && let Ok(kh_val) = kh.parse::<i64>()
                        && kh_val == 0
                    {
                        return Err(AppError::Repository(
                            "nzbget KeepHistory is set to 0 — completed downloads are \
                                     immediately purged and cannot be auto-imported. Set \
                                     KeepHistory to at least 1 in NZBGet settings."
                                .into(),
                        ));
                    }
                }
            }
            Err(err) => {
                warn!(error = %err, "failed to read nzbget config for KeepHistory check");
            }
        }

        Ok(version)
    }

    async fn append_requires_auto_category(&self) -> bool {
        match self.rpc_call("version", vec![]).await {
            Ok(Value::String(version)) => supports_nzbget_append_auto_category(&version),
            Ok(other) => {
                warn!(result = %other, "nzbget version call returned non-string result");
                false
            }
            Err(err) => {
                warn!(error = %err, "failed to determine nzbget version before append");
                false
            }
        }
    }

    async fn send_append_request(
        &self,
        append_request: &NzbgetAppendRequest<'_>,
        staged_nzb: &StagedNzbRef,
    ) -> AppResult<i64> {
        const STREAM_PLACEHOLDER: &str = "__SCRYER_STREAMED_NZB_BASE64__";
        let placeholder_request = NzbgetAppendRequest {
            source_for_payload: STREAM_PLACEHOLDER,
            ..*append_request
        };
        let request_payload = build_nzbget_append_payload(&placeholder_request, &self.dupe_mode);

        let mut request_payload_for_log =
            build_nzbget_append_payload(append_request, &self.dupe_mode);
        if let Some(params) = request_payload_for_log
            .get_mut("params")
            .and_then(Value::as_array_mut)
            && params.len() > 1
        {
            params[1] = Value::String("<omitted base64 nzb content>".to_string());
        }

        let endpoint = self.endpoint();
        tracing::info!(
            endpoint = endpoint.as_str(),
            auto_category = append_request.use_auto_category,
            payload = %request_payload_for_log,
            "nzbget append request payload"
        );
        let (prefix, suffix) =
            split_streaming_payload(&request_payload, STREAM_PLACEHOLDER.as_bytes())?;
        let content_length = prefix.len() as u64
            + base64_encoded_len(staged_nzb.raw_size_bytes)
            + suffix.len() as u64;
        let compressed_path = staged_nzb.compressed_path.clone();
        let response = self
            .outbound_http
            .send_async(self.mutation_policy("nzbget_append"), || {
                let endpoint = endpoint.clone();
                let compressed_path = compressed_path.clone();
                let prefix = prefix.clone();
                let suffix = suffix.clone();
                async move {
                    let staged_file = File::open(&compressed_path).await.map_err(|error| {
                        AppError::Repository(format!(
                            "failed to open staged nzb {}: {error}",
                            compressed_path.display()
                        ))
                    })?;
                    let mut request = self
                        .outbound_http
                        .client()
                        .post(&endpoint)
                        .timeout(NZBGET_HTTP_REQUEST_TIMEOUT)
                        .header("Content-Type", "application/json")
                        .header(reqwest::header::CONTENT_LENGTH, content_length.to_string())
                        .body(reqwest::Body::wrap_stream(
                            Self::build_streaming_append_body(staged_file, prefix, suffix),
                        ));

                    if let Some(username) = self.username.as_ref() {
                        request = request.basic_auth(username, self.password.as_deref());
                    }

                    Ok::<reqwest::RequestBuilder, AppError>(request)
                }
            })
            .await
            .map_err(|error| match error {
                OutboundRequestError::Build(error) => error,
                OutboundRequestError::Http(error) => {
                    map_nzbget_append_outbound_error(append_request, error)
                }
            })?;
        let status = response.status();
        let body_text = response
            .text()
            .await
            .map_err(|err| AppError::Repository(format!("nzbget response read failed: {err}")))?;

        if !status.is_success() {
            let preview = body_text.chars().take(800).collect::<String>();
            warn!(
                endpoint = endpoint.as_str(),
                status = status.to_string().as_str(),
                preview = preview.as_str(),
                title = append_request.title_name,
                auto_category = append_request.use_auto_category,
                "nzbget request failed"
            );
            return Err(AppError::Repository(format!(
                "nzbget rejected request with status {status}"
            )));
        }

        let response_json: Value = serde_json::from_str(&body_text).map_err(|err| {
            AppError::Repository(format!("nzbget returned non-json response: {err}"))
        })?;

        if let Some(error) = response_json.get("error") {
            let code = error
                .get("code")
                .and_then(Value::as_i64)
                .unwrap_or_default();
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            return Err(AppError::Repository(format!(
                "nzbget API error {code}: {message}"
            )));
        }

        let result = response_json
            .get("result")
            .and_then(Value::as_i64)
            .unwrap_or(-1);
        if result <= 0 {
            return Err(AppError::Repository(format!(
                "nzbget returned non-positive queue id {result}"
            )));
        }

        debug!(
            endpoint = endpoint.as_str(),
            nzb_id = result,
            job_id = append_request.request_id,
            title = append_request.title_name,
            category = append_request.category,
            auto_category = append_request.use_auto_category,
            "nzbget append succeeded"
        );

        Ok(result)
    }

    fn build_streaming_append_body(
        staged_file: File,
        prefix: Vec<u8>,
        suffix: Vec<u8>,
    ) -> impl futures_util::Stream<Item = Result<Vec<u8>, std::io::Error>> + Send {
        struct AppendBodyState {
            prefix: Option<Vec<u8>>,
            suffix: Option<Vec<u8>>,
            decoder: ZstdDecoder<BufReader<File>>,
            read_buf: [u8; 16 * 1024],
            remainder: Vec<u8>,
            finished_source: bool,
        }

        impl AppendBodyState {
            async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, std::io::Error> {
                if let Some(prefix) = self.prefix.take() {
                    return Ok(Some(prefix));
                }

                loop {
                    if self.finished_source {
                        if !self.remainder.is_empty() {
                            let encoded = general_purpose::STANDARD.encode(&self.remainder);
                            self.remainder.clear();
                            return Ok(Some(encoded.into_bytes()));
                        }
                        return Ok(self.suffix.take());
                    }

                    let bytes_read = self.decoder.read(&mut self.read_buf).await?;
                    if bytes_read == 0 {
                        self.finished_source = true;
                        continue;
                    }

                    let mut combined = Vec::with_capacity(self.remainder.len() + bytes_read);
                    combined.extend_from_slice(&self.remainder);
                    combined.extend_from_slice(&self.read_buf[..bytes_read]);

                    let remainder_len = combined.len() % 3;
                    let emit_len = combined.len() - remainder_len;
                    if emit_len == 0 {
                        self.remainder = combined;
                        continue;
                    }

                    let encoded = general_purpose::STANDARD.encode(&combined[..emit_len]);
                    self.remainder = combined[emit_len..].to_vec();
                    return Ok(Some(encoded.into_bytes()));
                }
            }
        }

        let state = AppendBodyState {
            prefix: Some(prefix),
            suffix: Some(suffix),
            decoder: ZstdDecoder::new(BufReader::new(staged_file)),
            read_buf: [0u8; 16 * 1024],
            remainder: Vec::new(),
            finished_source: false,
        };

        stream::try_unfold(state, |mut state| async move {
            match state.next_chunk().await? {
                Some(chunk) => Ok(Some((chunk, state))),
                None => Ok(None),
            }
        })
    }

    async fn reconcile_append_after_transport_error(
        &self,
        append_request: &NzbgetAppendRequest<'_>,
        title_id: &str,
        error: &AppError,
    ) -> Option<i64> {
        const RECONCILE_DELAYS: [Duration; 5] = [
            Duration::from_millis(0),
            Duration::from_millis(100),
            Duration::from_millis(250),
            Duration::from_millis(500),
            Duration::from_millis(1000),
        ];

        warn!(
            error = %error,
            title_id,
            title = append_request.title_name,
            nzb_filename = append_request.nzb_filename,
            "nzbget append response was ambiguous; reconciling queue and history before returning failure"
        );

        for delay in RECONCILE_DELAYS {
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }

            match self.list_queue_for_client().await {
                Ok(items) => {
                    if let Some(job_id) =
                        find_reconciled_nzbget_append_job(&items, title_id, append_request)
                    {
                        info!(
                            nzb_id = job_id,
                            title_id,
                            nzb_filename = append_request.nzb_filename,
                            "reconciled ambiguous nzbget append from queue"
                        );
                        return Some(job_id);
                    }
                }
                Err(error) => {
                    warn!(
                        error = %error,
                        title_id,
                        nzb_filename = append_request.nzb_filename,
                        "failed to read nzbget queue while reconciling ambiguous append"
                    );
                }
            }

            match self.list_history_for_client().await {
                Ok(items) => {
                    if let Some(job_id) =
                        find_reconciled_nzbget_append_job(&items, title_id, append_request)
                    {
                        info!(
                            nzb_id = job_id,
                            title_id,
                            nzb_filename = append_request.nzb_filename,
                            "reconciled ambiguous nzbget append from history"
                        );
                        return Some(job_id);
                    }
                }
                Err(error) => {
                    warn!(
                        error = %error,
                        title_id,
                        nzb_filename = append_request.nzb_filename,
                        "failed to read nzbget history while reconciling ambiguous append"
                    );
                }
            }
        }

        warn!(
            title_id,
            nzb_filename = append_request.nzb_filename,
            "ambiguous nzbget append was not found in queue or history"
        );
        None
    }

    async fn edit_queue(&self, command: &str, ids: Vec<i64>) -> AppResult<()> {
        if self.try_edit_queue(command, ids).await? {
            Ok(())
        } else {
            Err(AppError::Repository(format!(
                "nzbget editqueue {command} matched no items"
            )))
        }
    }

    /// Runs one `editqueue` command and reports whether NZBGet applied it.
    /// NZBGet answers `false` when none of the ids matched the list the
    /// command targets (queue vs history), which callers may want to recover
    /// from rather than treat as a failure.
    async fn try_edit_queue(&self, command: &str, ids: Vec<i64>) -> AppResult<bool> {
        let result = self
            .rpc_call_with_policy(
                "editqueue",
                vec![json!(command), json!(""), json!(ids)],
                self.mutation_policy(format!("nzbget_editqueue_{command}")),
            )
            .await?;
        match result.as_bool() {
            Some(applied) => Ok(applied),
            None => Err(AppError::Repository(format!(
                "nzbget editqueue {command} returned unexpected result: {result}"
            ))),
        }
    }

    async fn list_queue_for_client(&self) -> AppResult<Vec<DownloadQueueItem>> {
        let result = self.rpc_call("listgroups", vec![]).await?;
        let groups = extract_result_array(result, "Groups").unwrap_or_default();

        let mut items: Vec<DownloadQueueItem> = groups
            .into_iter()
            .filter_map(|group| {
                let group = group.as_object()?;
                let nzb_id = extract_i64_value(group.get("NZBID"))
                    .or_else(|| extract_i64_value(group.get("nzbId")))
                    .or_else(|| extract_i64_value(group.get("ID")))
                    .filter(|value| *value > 0)?;

                let status = group
                    .get("Status")
                    .or_else(|| group.get("status"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let pp_stage = extract_postprocessing_stage_from_entry(group);
                let state = queue_state_from_status(status);

                let size_mb =
                    extract_f64_value(group.get("FileSizeMB").or_else(|| group.get("fileSizeMB")))
                        .unwrap_or(0.0);
                let remaining_mb = extract_f64_value(
                    group
                        .get("RemainingSizeMB")
                        .or_else(|| group.get("remainingSizeMB")),
                )
                .unwrap_or(0.0);

                let title_name = group
                    .get("NZBName")
                    .or_else(|| group.get("NzbName"))
                    .or_else(|| group.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or("Unnamed download")
                    .to_string();
                let category = extract_nzbget_category(group);

                let scryer_parameters = extract_nzbget_parameters(group);
                let queue_progress = progress_percent_from_sizes(size_mb, remaining_mb);
                let remaining_seconds = extract_remaining_seconds_from_entry(group);

                Some(DownloadQueueItem {
                    id: nzb_id.to_string(),
                    title_id: scryer_parameters.title_id,
                    episode_id: None,
                    title_name,
                    facet: scryer_parameters.facet,
                    category,
                    client_id: String::new(),
                    client_name: String::new(),
                    client_type: "nzbget".to_string(),
                    state,
                    progress_percent: if state == DownloadQueueState::Downloading
                        && pp_stage.is_some()
                    {
                        extract_postprocessing_progress_from_entry(group).unwrap_or(0)
                    } else {
                        queue_progress
                    },
                    import_transfer_phase: None,
                    import_transfer_bytes: None,
                    import_transfer_total_bytes: None,
                    import_transfer_started_at: None,
                    import_transfer_updated_at: None,
                    size_bytes: size_to_bytes(size_mb),
                    remaining_seconds,
                    queued_at: extract_i64_value(
                        group
                            .get("MinPostTime")
                            .or_else(|| group.get("minPostTime")),
                    )
                    .map(|value| value.to_string()),
                    last_updated_at: None,
                    attention_required: false,
                    attention_reason: if state == DownloadQueueState::Downloading {
                        pp_stage
                    } else {
                        None
                    },
                    download_client_item_id: nzb_id.to_string(),
                    download_id: scryer_parameters.download_id,
                    import_status: None,
                    import_error_code: None,
                    import_error_message: None,
                    imported_at: None,
                    delete_status: None,
                    delete_error_message: None,
                    source_provider: None,
                    is_scryer_origin: scryer_parameters.is_scryer,
                    tracked_state: None,
                    tracked_status: None,
                    tracked_status_messages: Vec::new(),
                    tracked_match_type: None,
                    seeding: None,
                })
            })
            .collect();

        // NZBGet keeps post-processing jobs in a separate postqueue list.
        // Merge those jobs so Activity shows UNPACK/VERIFY/REPAIR stages too.
        let mut item_index_by_id: HashMap<String, usize> = items
            .iter()
            .enumerate()
            .map(|(index, item)| (item.download_client_item_id.clone(), index))
            .collect();
        if let Ok(postqueue_result) = self.rpc_call("postqueue", vec![]).await {
            let postqueue_entries =
                extract_result_array(postqueue_result, "PostQueue").unwrap_or_default();
            for entry in postqueue_entries {
                let Some(entry) = entry.as_object() else {
                    continue;
                };

                let nzb_id = extract_i64_value(entry.get("NZBID"))
                    .or_else(|| extract_i64_value(entry.get("nzbId")))
                    .or_else(|| extract_i64_value(entry.get("ID")))
                    .filter(|value| *value > 0);
                let Some(nzb_id) = nzb_id else {
                    continue;
                };
                let id = nzb_id.to_string();

                let status = entry
                    .get("Status")
                    .or_else(|| entry.get("status"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let stage = entry
                    .get("Stage")
                    .or_else(|| entry.get("stage"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let state = if !status.trim().is_empty() {
                    queue_state_from_status(status)
                } else {
                    queue_state_from_status(stage)
                };
                let pp_stage = extract_postprocessing_stage_from_entry(entry).or_else(|| {
                    let fallback = format!("{status} {stage}");
                    is_nzbget_postprocessing_status(&fallback).then(|| "POSTPROCESSING".to_string())
                });

                let progress_percent =
                    extract_postprocessing_progress_from_entry(entry).unwrap_or(0);
                let remaining_seconds = extract_remaining_seconds_from_entry(entry);

                let size_mb = extract_f64_value(
                    entry
                        .get("FileSizeMB")
                        .or_else(|| entry.get("fileSizeMB"))
                        .or_else(|| entry.get("TotalSizeMB"))
                        .or_else(|| entry.get("totalSizeMB")),
                )
                .unwrap_or(0.0);

                let title_name = entry
                    .get("NZBName")
                    .or_else(|| entry.get("NzbName"))
                    .or_else(|| entry.get("Name"))
                    .or_else(|| entry.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or("Unnamed download")
                    .to_string();
                let category = extract_nzbget_category(entry);

                let scryer_parameters = extract_nzbget_parameters(entry);
                let updated_at = extract_i64_value(
                    entry
                        .get("PostTime")
                        .or_else(|| entry.get("postTime"))
                        .or_else(|| entry.get("Time"))
                        .or_else(|| entry.get("time")),
                )
                .map(|value| value.to_string());

                if let Some(existing_index) = item_index_by_id.get(&id).copied() {
                    let existing = &mut items[existing_index];
                    existing.state = state;
                    existing.progress_percent = progress_percent;
                    if existing.last_updated_at.is_none() {
                        existing.last_updated_at = updated_at;
                    }
                    if existing.title_id.is_none() {
                        existing.title_id = scryer_parameters.title_id.clone();
                    }
                    if existing.facet.is_none() {
                        existing.facet = scryer_parameters.facet.clone();
                    }
                    if existing.download_id.is_none() {
                        existing.download_id = scryer_parameters.download_id.clone();
                    }
                    if existing.size_bytes.is_none() || existing.size_bytes == Some(0) {
                        existing.size_bytes = size_to_bytes(size_mb);
                    }
                    if existing.category.is_none() {
                        existing.category = category.clone();
                    }
                    if remaining_seconds.is_some() {
                        existing.remaining_seconds = remaining_seconds;
                    }
                    if existing.title_name == "Unnamed download" && title_name != "Unnamed download"
                    {
                        existing.title_name = title_name;
                    }
                    if state == DownloadQueueState::Downloading && pp_stage.is_some() {
                        existing.attention_reason = pp_stage.clone();
                    } else if existing
                        .attention_reason
                        .as_deref()
                        .is_some_and(is_nzbget_postprocessing_status)
                    {
                        existing.attention_reason = None;
                    }
                    existing.is_scryer_origin =
                        existing.is_scryer_origin || scryer_parameters.is_scryer;
                    continue;
                }

                items.push(DownloadQueueItem {
                    id: id.clone(),
                    title_id: scryer_parameters.title_id,
                    episode_id: None,
                    title_name,
                    facet: scryer_parameters.facet,
                    category,
                    client_id: String::new(),
                    client_name: String::new(),
                    client_type: "nzbget".to_string(),
                    state,
                    progress_percent,
                    import_transfer_phase: None,
                    import_transfer_bytes: None,
                    import_transfer_total_bytes: None,
                    import_transfer_started_at: None,
                    import_transfer_updated_at: None,
                    size_bytes: size_to_bytes(size_mb),
                    remaining_seconds,
                    queued_at: None,
                    last_updated_at: updated_at,
                    attention_required: false,
                    attention_reason: if state == DownloadQueueState::Downloading {
                        pp_stage
                    } else {
                        None
                    },
                    download_client_item_id: id.clone(),
                    download_id: scryer_parameters.download_id,
                    import_status: None,
                    import_error_code: None,
                    import_error_message: None,
                    imported_at: None,
                    delete_status: None,
                    delete_error_message: None,
                    source_provider: None,
                    is_scryer_origin: scryer_parameters.is_scryer,
                    tracked_state: None,
                    tracked_status: None,
                    tracked_status_messages: Vec::new(),
                    tracked_match_type: None,
                    seeding: None,
                });
                let next_index = items.len() - 1;
                item_index_by_id.insert(id, next_index);
            }
        } else {
            debug!("nzbget postqueue endpoint unavailable; skipping pp queue merge");
        }

        // Global pause detection: when NZBGet's download queue is globally paused,
        // all non-completed items should show as Paused.
        if !items.is_empty()
            && let Ok(status_result) = self.rpc_call("status", vec![]).await
        {
            let download_paused = status_result
                .get("DownloadPaused")
                .or_else(|| status_result.get("downloadPaused"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if download_paused {
                for item in &mut items {
                    if matches!(
                        item.state,
                        DownloadQueueState::Queued | DownloadQueueState::Downloading
                    ) {
                        item.state = DownloadQueueState::Paused;
                    }
                }
            }
        }

        Ok(items)
    }

    async fn list_history_for_client(&self) -> AppResult<Vec<DownloadQueueItem>> {
        let result = self.rpc_call("history", vec![json!(false)]).await?;
        let entries = extract_result_array(result, "History").unwrap_or_default();
        let cutoff_ts = Utc::now().timestamp() - (7 * 24 * 60 * 60);

        Ok(entries
            .into_iter()
            .filter_map(|entry| {
                let entry = entry.as_object()?;
                let nzb_id = extract_i64_value(entry.get("NZBID"))
                    .or_else(|| extract_i64_value(entry.get("nzbId")))
                    .or_else(|| extract_i64_value(entry.get("ID")))
                    .filter(|value| *value > 0)?;
                let status = entry
                    .get("Status")
                    .or_else(|| entry.get("status"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let status_upper = status.to_ascii_uppercase();

                if status_upper.starts_with("DELETED") {
                    return None;
                }

                let (state, attention_reason) = map_history_state(&status_upper, entry);
                let history_ts =
                    extract_i64_value(entry.get("HistoryTime").or_else(|| entry.get("time")));
                if let Some(ts) = history_ts
                    && ts < cutoff_ts
                {
                    return None;
                }

                let title_name = entry
                    .get("Name")
                    .or_else(|| entry.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or("Unnamed download")
                    .to_string();
                let category = extract_nzbget_category(entry);
                let size_mb =
                    extract_f64_value(entry.get("FileSizeMB").or_else(|| entry.get("fileSizeMB")))
                        .unwrap_or(0.0);

                let scryer_parameters = extract_nzbget_parameters(entry);

                Some(DownloadQueueItem {
                    id: nzb_id.to_string(),
                    title_id: scryer_parameters.title_id,
                    episode_id: None,
                    title_name,
                    facet: scryer_parameters.facet,
                    category,
                    client_id: String::new(),
                    client_name: String::new(),
                    client_type: "nzbget".to_string(),
                    state,
                    progress_percent: if state == DownloadQueueState::Completed {
                        100
                    } else {
                        0
                    },
                    import_transfer_phase: None,
                    import_transfer_bytes: None,
                    import_transfer_total_bytes: None,
                    import_transfer_started_at: None,
                    import_transfer_updated_at: None,
                    size_bytes: size_to_bytes(size_mb),
                    remaining_seconds: None,
                    queued_at: None,
                    last_updated_at: history_ts.map(|value| value.to_string()),
                    attention_required: matches!(state, DownloadQueueState::Failed),
                    attention_reason,
                    download_client_item_id: nzb_id.to_string(),
                    download_id: scryer_parameters.download_id,
                    import_status: None,
                    import_error_code: None,
                    import_error_message: None,
                    imported_at: None,
                    delete_status: None,
                    delete_error_message: None,
                    source_provider: None,
                    is_scryer_origin: scryer_parameters.is_scryer,
                    tracked_state: None,
                    tracked_status: None,
                    tracked_status_messages: Vec::new(),
                    tracked_match_type: None,
                    seeding: None,
                })
            })
            .collect())
    }

    fn scope_key(&self) -> String {
        format!("nzbget:{}", self.endpoint())
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
}

fn map_nzbget_outbound_error(operation: &str, error: OutboundHttpError) -> AppError {
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

fn map_nzbget_append_outbound_error(
    append_request: &NzbgetAppendRequest<'_>,
    error: OutboundHttpError,
) -> AppError {
    match error {
        OutboundHttpError::RateLimited(rate_limited) => {
            let retry_after = rate_limited.retry_after.filter(|delay| !delay.is_zero());
            AppError::rate_limited_temporary_unavailable(
                match retry_after {
                    Some(delay) => {
                        format!(
                            "nzbget append request was rate limited; retry after {}s",
                            delay.as_secs()
                        )
                    }
                    None => "nzbget append request was rate limited".to_string(),
                },
                retry_after,
                RateLimitCooldownAction::AlreadyRecorded,
            )
        }
        OutboundHttpError::Transport {
            attempts, source, ..
        } => {
            warn!(
                attempts,
                error = %source,
                error_debug = ?source,
                is_timeout = source.is_timeout(),
                is_connect = source.is_connect(),
                is_request = source.is_request(),
                is_body = source.is_body(),
                is_decode = source.is_decode(),
                title = append_request.title_name,
                nzb_filename = append_request.nzb_filename,
                "nzbget append transport failed after sending mutation request"
            );
            AppError::DownloadSubmitAmbiguous(format!(
                "nzbget append request transport failed after the request may have been accepted: {source}"
            ))
        }
    }
}

fn find_reconciled_nzbget_append_job(
    items: &[DownloadQueueItem],
    title_id: &str,
    append_request: &NzbgetAppendRequest<'_>,
) -> Option<i64> {
    let expected_name = normalize_nzbget_append_match_name(append_request.nzb_filename);
    items.iter().find_map(|item| {
        let item_title_id = item.title_id.as_deref()?.trim();
        if item_title_id != title_id {
            return None;
        }

        let item_name = normalize_nzbget_append_match_name(&item.title_name);
        if item_name != expected_name {
            return None;
        }

        item.download_client_item_id.trim().parse::<i64>().ok()
    })
}

fn normalize_nzbget_append_match_name(value: &str) -> String {
    let trimmed = value.trim();
    let without_ext = if trimmed.to_ascii_lowercase().ends_with(".nzb") {
        &trimmed[..trimmed.len().saturating_sub(4)]
    } else {
        trimmed
    };
    without_ext.to_ascii_lowercase()
}

#[async_trait]
impl DownloadClient for NzbgetDownloadClient {
    async fn submit_download(
        &self,
        request: &DownloadClientAddRequest,
    ) -> AppResult<DownloadGrabResult> {
        let title = &request.title;
        let request_id = scryer_domain::Id::new().0;
        let source_hint = request
            .source_hint
            .as_deref()
            .map(str::trim)
            .unwrap_or_default()
            .to_string();

        let normalized_source_title = request.source_title.clone().and_then(|value| {
            let trimmed = value.trim().to_string();
            (!trimmed.is_empty()).then_some(trimmed)
        });
        let nzb_filename = derive_nzb_filename(
            normalized_source_title.as_deref(),
            &source_hint,
            &title.name,
        );
        let category = request
            .category
            .clone()
            .and_then(|value| {
                let value = value.trim().to_string();
                (!value.is_empty()).then_some(value)
            })
            .unwrap_or_default();

        let staged = resolve_staged_nzb_for_request(
            &self.outbound_http,
            &self.staged_nzb_store,
            &self.staged_nzb_pipeline_limit,
            request,
        )
        .await?;
        let source_password = request
            .source_password
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("0"))
            .map(str::to_string);

        let facet_str =
            serde_json::to_string(&title.facet).unwrap_or_else(|_| "\"other\"".to_string());
        let facet_str = facet_str.trim_matches('"');
        // NZBGet append PPParameters: array of single-key objects where the
        // key is the parameter name and the value is the parameter value.
        // NZBGet stores them as {"Name":…,"Value":…} in responses, but
        // accepts {"*key": "val"} in the append request.
        let mut parameters: Vec<Value> = vec![
            json!({"*scryer_title_id": title.id.clone()}),
            json!({"*scryer_facet": facet_str}),
            json!({"*scryer_import_purpose": request.purpose.as_str()}),
        ];
        if let Some(download_id) = request.download_id {
            parameters.push(json!({"*scryer_download_id": download_id.to_wire()}));
        }

        if let Some(imdb_id) = title
            .external_ids
            .iter()
            .find(|id| id.source.eq_ignore_ascii_case("imdb"))
            .map(|id| id.value.trim().to_string())
            .filter(|value| !value.is_empty())
        {
            parameters.push(json!({"*scryer_imdb_id": imdb_id}));
        }

        if let Some(password) = source_password {
            parameters.push(json!({"*Unpack:Password": password}));
        }

        let result: AppResult<DownloadGrabResult> = async {
            let use_auto_category = self.append_requires_auto_category().await;
            let queue_priority = nzbget_queue_priority(request.queue_priority.as_deref());
            let append_request = NzbgetAppendRequest {
                request_id: &request_id,
                title_name: title.name.as_str(),
                nzb_filename: &nzb_filename,
                source_for_payload: "",
                category: &category,
                queue_priority,
                parameters: &parameters,
                use_auto_category,
            };

            let nzbget_id = match self
                .send_append_request(&append_request, &staged.staged_nzb)
                .await
            {
                Ok(queue_id) => queue_id,
                Err(err) if is_nzbget_invalid_procedure_error(&err) => {
                    let retry_use_auto_category = !append_request.use_auto_category;
                    warn!(
                        error = %err,
                        retry_auto_category = retry_use_auto_category,
                        title = title.name.as_str(),
                        "nzbget append rejected payload shape; retrying alternate append signature"
                    );
                    let retry_request = NzbgetAppendRequest {
                        use_auto_category: retry_use_auto_category,
                        ..append_request
                    };
                    self.send_append_request(&retry_request, &staged.staged_nzb)
                        .await
                        .map_err(AppError::into_download_submit_unavailable)?
                }
                Err(err @ AppError::DownloadSubmitAmbiguous(_)) => {
                    if let Some(queue_id) = self
                        .reconcile_append_after_transport_error(
                            &append_request,
                            title.id.as_str(),
                            &err,
                        )
                        .await
                    {
                        queue_id
                    } else {
                        return Err(err);
                    }
                }
                Err(err) => return Err(err.into_download_submit_unavailable()),
            };

            // Use the NZBGet queue ID (integer) as the job_id so it matches
            // the NZBID in NZBGet's history — required for failure detection
            // in check_grabbed_for_failures.
            Ok(DownloadGrabResult {
                download_id: None,
                job_id: nzbget_id.to_string(),
                client_id: None,
                client_type: "nzbget".to_string(),
                info_hash: None,
                seed_goals: None,
            })
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
                "failed to delete self-staged nzbget nzb artifact"
            );
        }

        result
    }

    async fn test_connection(&self) -> AppResult<String> {
        NzbgetDownloadClient::test_connection(self).await
    }

    async fn list_queue(&self) -> AppResult<Vec<DownloadQueueItem>> {
        self.list_queue_for_client().await
    }

    async fn list_history(&self) -> AppResult<Vec<DownloadQueueItem>> {
        self.list_history_for_client().await
    }

    async fn pause_queue_item(&self, id: &str) -> AppResult<()> {
        let nzb_id: i64 = id
            .parse()
            .map_err(|_| AppError::Validation(format!("invalid nzbget queue id: {id}")))?;
        self.edit_queue("GroupPause", vec![nzb_id]).await
    }

    async fn resume_queue_item(&self, id: &str) -> AppResult<()> {
        let nzb_id: i64 = id
            .parse()
            .map_err(|_| AppError::Validation(format!("invalid nzbget queue id: {id}")))?;
        self.edit_queue("GroupResume", vec![nzb_id]).await
    }

    /// NZBGet has no per-call file-deletion option for history entries.
    /// Scryer deletes a burned download's job data itself (see
    /// `import_workflow::delete_burned_download_data`) before issuing this delete.
    ///
    /// `is_history` is only a hint derived from the last polled state. NZBGet
    /// answers `false` when the id is not in the list the command targets, so
    /// a wrong hint in either direction falls through to the other command
    /// instead of leaving the item behind to be re-adopted on the next poll.
    async fn delete_queue_item(
        &self,
        id: &str,
        is_history: bool,
        _remove_data: bool,
    ) -> AppResult<()> {
        let nzb_id: i64 = id
            .parse()
            .map_err(|_| AppError::Validation(format!("invalid nzbget queue id: {id}")))?;
        let (hinted, fallback) = nzbget_delete_commands(is_history);
        if self.try_edit_queue(hinted, vec![nzb_id]).await? {
            return Ok(());
        }
        debug!(
            nzb_id,
            hinted_history = is_history,
            "nzbget {hinted} matched nothing; retrying with {fallback}"
        );
        if self.try_edit_queue(fallback, vec![nzb_id]).await? {
            return Ok(());
        }
        Err(AppError::Repository(format!(
            "nzbget editqueue {hinted} and {fallback} both matched no items for id {nzb_id}"
        )))
    }

    async fn list_completed_downloads(&self) -> AppResult<Vec<scryer_domain::CompletedDownload>> {
        let result = self.rpc_call("history", vec![json!(false)]).await?;
        let entries = extract_result_array(result, "History").unwrap_or_default();
        let cutoff_ts = Utc::now().timestamp() - (7 * 24 * 60 * 60);

        info!(
            total_history_entries = entries.len(),
            "nzbget: fetched history for completed downloads"
        );

        Ok(entries
            .into_iter()
            .filter_map(|entry| {
                completed_download_from_nzbget_history_entry(&entry, Some(cutoff_ts))
            })
            .collect())
    }

    async fn get_completed_download_for_source(
        &self,
        _client_id: &str,
        _client_type: &str,
        download_client_item_id: &str,
    ) -> AppResult<Option<scryer_domain::CompletedDownload>> {
        let Ok(nzb_id) = download_client_item_id.trim().parse::<i64>() else {
            return Ok(None);
        };
        let result = self.rpc_call("history", vec![json!(false)]).await?;
        let entry = extract_result_array(result, "History")
            .unwrap_or_default()
            .into_iter()
            .find(|entry| {
                entry.as_object().is_some_and(|entry| {
                    extract_i64_value(entry.get("NZBID").or_else(|| entry.get("nzbId")))
                        == Some(nzb_id)
                })
            });
        Ok(entry.and_then(|entry| completed_download_from_nzbget_history_entry(&entry, None)))
    }
}

fn completed_download_from_nzbget_history_entry(
    entry: &Value,
    cutoff_ts: Option<i64>,
) -> Option<scryer_domain::CompletedDownload> {
    let entry = entry.as_object()?;
    let nzb_id = extract_i64_value(entry.get("NZBID").or_else(|| entry.get("nzbId")))
        .filter(|value| *value > 0)?;
    let name = history_entry_name(entry);
    let status = entry
        .get("Status")
        .or_else(|| entry.get("status"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let status_upper = status.to_ascii_uppercase();

    if !status_upper.starts_with("SUCCESS") {
        debug!(
            nzb_id,
            name = name.as_str(),
            status,
            "nzbget: skipping non-SUCCESS history entry"
        );
        return None;
    }

    // Multi-field failure detection (mirrors Sonarr's cascade):
    // Even when top-level Status is SUCCESS, individual stages may
    // indicate problems that make the download unusable.
    if let Some(reason) = check_history_stage_failure(entry) {
        warn!(
            nzb_id,
            name = name.as_str(),
            reason = reason.as_str(),
            "skipping completed download due to stage failure"
        );
        return None;
    }

    let history_ts = extract_i64_value(entry.get("HistoryTime").or_else(|| entry.get("time")));
    if let Some(cutoff_ts) = cutoff_ts
        && let Some(ts) = history_ts
        && ts < cutoff_ts
    {
        trace!(
            nzb_id,
            name = name.as_str(),
            "nzbget: skipping history entry older than 7 days"
        );
        return None;
    }

    // Prefer FinalDir (post-move location) over DestDir, mirroring Sonarr.
    let dest_dir = entry
        .get("FinalDir")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            entry
                .get("DestDir")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
        })
        .unwrap_or("")
        .to_string();

    if dest_dir.is_empty() {
        debug!(
            nzb_id,
            name = name.as_str(),
            "nzbget: skipping history entry with empty dest_dir"
        );
        return None;
    }

    let category = extract_nzbget_category(entry);
    let size_mb = extract_f64_value(entry.get("FileSizeMB").or_else(|| entry.get("fileSizeMB")))
        .unwrap_or(0.0);
    let parameters = entry
        .get("Parameters")
        .or_else(|| entry.get("parameters"))
        .and_then(Value::as_array)
        .map(|params| {
            params
                .iter()
                .filter_map(|parameter| {
                    let parameter = parameter.as_object()?;
                    let key = parameter
                        .get("Name")
                        .or_else(|| parameter.get("name"))
                        .and_then(Value::as_str)?
                        .to_string();
                    let value = parameter
                        .get("Value")
                        .or_else(|| parameter.get("value"))
                        .and_then(Value::as_str)?
                        .to_string();
                    Some((key, value))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let completed_at =
        history_ts.map(|timestamp| DateTime::from_timestamp(timestamp, 0).unwrap_or_else(Utc::now));
    let observed_identity = scryer_application::observed_download_identity(
        scryer_application::ObservedDownloadIdentityInput {
            download_id: None,
            parameters: &parameters,
            info_hash_hint: None,
        },
    );

    Some(scryer_domain::CompletedDownload {
        client_type: "nzbget".to_string(),
        client_id: String::new(),
        download_client_item_id: nzb_id.to_string(),
        download_id: observed_identity.download_id,
        name,
        release_name: history_entry_release_name(entry),
        dest_dir,
        category,
        size_bytes: size_to_bytes(size_mb),
        completed_at,
        parameters,
    })
}

struct ExtractedNzbgetParameters {
    title_id: Option<String>,
    facet: Option<String>,
    is_scryer: bool,
    download_id: Option<String>,
}

fn extract_nzbget_category(entry: &serde_json::Map<String, Value>) -> Option<String> {
    entry
        .get("Category")
        .or_else(|| entry.get("category"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn extract_nzbget_parameters(entry: &serde_json::Map<String, Value>) -> ExtractedNzbgetParameters {
    let params = entry
        .get("Parameters")
        .or_else(|| entry.get("parameters"))
        .and_then(Value::as_array);
    let params = match params {
        Some(params) => params,
        None => {
            return ExtractedNzbgetParameters {
                title_id: None,
                facet: None,
                is_scryer: false,
                download_id: None,
            };
        }
    };
    let mut title_id: Option<String> = None;
    let mut facet: Option<String> = None;
    let mut is_scryer = false;
    let mut parameters = Vec::new();
    for p in params {
        let obj = match p.as_object() {
            Some(obj) => obj,
            None => continue,
        };
        let key = obj
            .get("Name")
            .or_else(|| obj.get("name"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let value = obj
            .get("Value")
            .or_else(|| obj.get("value"))
            .and_then(Value::as_str)
            .unwrap_or("");
        if !key.is_empty() {
            parameters.push((key.to_string(), value.to_string()));
        }
        match key {
            "*scryer_title_id" => {
                is_scryer = true;
                if !value.is_empty() {
                    title_id = Some(value.to_string());
                }
            }
            "*scryer_facet" if !value.is_empty() => {
                facet = Some(value.to_string());
            }
            _ => {}
        }
    }
    let observed_identity = scryer_application::observed_download_identity(
        scryer_application::ObservedDownloadIdentityInput {
            download_id: None,
            parameters: &parameters,
            info_hash_hint: None,
        },
    );
    ExtractedNzbgetParameters {
        title_id,
        facet,
        is_scryer,
        download_id: observed_identity.download_id,
    }
}

fn extract_result_array(value: Value, preferred_key: &str) -> Option<Vec<Value>> {
    match value {
        Value::Array(items) => Some(items),
        Value::Object(container) => container
            .get(preferred_key)
            .and_then(Value::as_array)
            .cloned()
            .or_else(|| {
                container
                    .get(&preferred_key.to_ascii_lowercase())
                    .and_then(Value::as_array)
                    .cloned()
            })
            .or_else(|| container.get("items").and_then(Value::as_array).cloned()),
        _ => None,
    }
}

/// The `editqueue` command a delete should try first for the hinted list, and
/// the one to fall back to when NZBGet reports that nothing matched.
fn nzbget_delete_commands(is_history: bool) -> (&'static str, &'static str) {
    if is_history {
        ("HistoryDelete", "GroupDelete")
    } else {
        ("GroupDelete", "HistoryDelete")
    }
}

fn queue_state_from_status(status: &str) -> DownloadQueueState {
    let normalized = status.to_ascii_uppercase();
    match normalized.as_str() {
        "DOWNLOADING" | "PP_QUEUED" | "LOADING_PARS" | "VERIFYING_SOURCES" | "REPAIRING"
        | "VERIFYING_REPAIRED" | "RENAMING" | "UNPACKING" | "MOVING" | "EXECUTING_SCRIPT"
        | "POSTPROCESSING" => DownloadQueueState::Downloading,
        "QUEUED" => DownloadQueueState::Queued,
        "PAUSED" | "PAUSED_DOWNLOAD" => DownloadQueueState::Paused,
        _ => DownloadQueueState::Queued,
    }
}

fn is_nzbget_postprocessing_status(status: &str) -> bool {
    find_nzbget_postprocessing_token(status).is_some()
}

fn find_nzbget_postprocessing_token(value: &str) -> Option<&'static str> {
    let normalized = value.to_ascii_uppercase().replace([' ', '-'], "_");
    const TOKENS: [&str; 10] = [
        "PP_QUEUED",
        "LOADING_PARS",
        "VERIFYING_SOURCES",
        "REPAIRING",
        "VERIFYING_REPAIRED",
        "RENAMING",
        "UNPACKING",
        "MOVING",
        "EXECUTING_SCRIPT",
        "POSTPROCESSING",
    ];
    TOKENS
        .iter()
        .copied()
        .find(|token| normalized.contains(token))
}

fn extract_postprocessing_stage_from_entry(
    entry: &serde_json::Map<String, Value>,
) -> Option<String> {
    let candidates = [
        entry.get("Status"),
        entry.get("status"),
        entry.get("Stage"),
        entry.get("stage"),
        entry.get("PostInfoText"),
        entry.get("postInfoText"),
        entry.get("PostInfo"),
        entry.get("postInfo"),
    ];

    for candidate in candidates.into_iter().flatten() {
        if let Some(text) = candidate.as_str()
            && let Some(token) = find_nzbget_postprocessing_token(text)
        {
            return Some(token.to_string());
        }
    }

    // NZBGet includes PostInfoText/PostStageProgress/PostTotalTimeSec on every
    // group entry — these can contain stale values from a previous PP attempt
    // even for items that are actively downloading. We intentionally do NOT
    // use these fields as evidence of post-processing here. Instead, real PP
    // items are detected via:
    //   1. The Status/Stage tokens above (PP_QUEUED, UNPACKING, etc.)
    //   2. The postqueue API merge in list_queue_for_client
    //   3. The remaining=0 heuristic below (download done, PP likely starting)

    let status = entry
        .get("Status")
        .or_else(|| entry.get("status"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let size_mb = extract_f64_value(entry.get("FileSizeMB").or_else(|| entry.get("fileSizeMB")))
        .unwrap_or(0.0);
    let remaining_mb = extract_f64_value(
        entry
            .get("RemainingSizeMB")
            .or_else(|| entry.get("remainingSizeMB")),
    )
    .unwrap_or(0.0);
    if status.eq_ignore_ascii_case("downloading") && size_mb > 0.0 && remaining_mb <= 0.0 {
        return Some("POSTPROCESSING".to_string());
    }

    None
}

fn extract_postprocessing_progress_from_entry(
    entry: &serde_json::Map<String, Value>,
) -> Option<u8> {
    extract_postprocessing_progress_permille(entry)
        .map(|value| (value / 10.0).round().clamp(0.0, 100.0) as u8)
}

fn extract_postprocessing_progress_permille(entry: &serde_json::Map<String, Value>) -> Option<f64> {
    extract_f64_value(
        entry
            .get("PostStageProgress")
            .or_else(|| entry.get("postStageProgress"))
            .or_else(|| entry.get("StageProgress"))
            .or_else(|| entry.get("stageProgress"))
            .or_else(|| entry.get("FileProgress"))
            .or_else(|| entry.get("fileProgress"))
            .or_else(|| entry.get("Progress"))
            .or_else(|| entry.get("progress")),
    )
}

fn extract_remaining_seconds_from_entry(entry: &serde_json::Map<String, Value>) -> Option<i64> {
    let direct_seconds = extract_i64_value(
        entry
            .get("RemainingSec")
            .or_else(|| entry.get("remainingSec"))
            .or_else(|| entry.get("RemainingSeconds"))
            .or_else(|| entry.get("remainingSeconds"))
            .or_else(|| entry.get("RemainingTimeSec"))
            .or_else(|| entry.get("remainingTimeSec"))
            .or_else(|| entry.get("PostRemainingSec"))
            .or_else(|| entry.get("postRemainingSec"))
            .or_else(|| entry.get("PostRemainingTimeSec"))
            .or_else(|| entry.get("postRemainingTimeSec")),
    );

    if let Some(seconds) = direct_seconds {
        return Some(seconds.max(0));
    }

    let text_seconds = entry
        .get("RemainingTime")
        .or_else(|| entry.get("remainingTime"))
        .or_else(|| entry.get("PostRemainingTime"))
        .or_else(|| entry.get("postRemainingTime"))
        .and_then(Value::as_str)
        .and_then(parse_duration_seconds);

    if text_seconds.is_some() {
        return text_seconds;
    }

    // Match NZBGet WebUI estimate math for post-processing:
    // PostStageTimeSec / PostStageProgress * (1000 - PostStageProgress)
    // where progress is in permille.
    let post_stage_progress = extract_postprocessing_progress_permille(entry);
    let post_stage_time = extract_i64_value(
        entry
            .get("PostStageTimeSec")
            .or_else(|| entry.get("postStageTimeSec"))
            .or_else(|| entry.get("StageTimeSec"))
            .or_else(|| entry.get("stageTimeSec")),
    );
    if let (Some(progress_permille), Some(stage_time_sec)) = (post_stage_progress, post_stage_time)
        && progress_permille > 0.0
        && stage_time_sec >= 0
    {
        let remaining = ((stage_time_sec as f64 / progress_permille) * (1000.0 - progress_permille))
            .round() as i64;
        return Some(remaining.max(0));
    }

    let total_seconds = extract_i64_value(
        entry
            .get("PostTotalTimeSec")
            .or_else(|| entry.get("postTotalTimeSec"))
            .or_else(|| entry.get("TotalTimeSec"))
            .or_else(|| entry.get("totalTimeSec")),
    );
    let elapsed_seconds = extract_i64_value(
        entry
            .get("PostStageTimeSec")
            .or_else(|| entry.get("postStageTimeSec"))
            .or_else(|| entry.get("StageTimeSec"))
            .or_else(|| entry.get("stageTimeSec")),
    );
    if let (Some(total), Some(elapsed)) = (total_seconds, elapsed_seconds)
        && total > 0
        && elapsed >= 0
    {
        return Some((total - elapsed).max(0));
    }

    let progress = extract_postprocessing_progress_from_entry(entry)?;
    if progress >= 100 {
        return Some(0);
    }
    if progress == 0 {
        return None;
    }

    let elapsed = extract_i64_value(
        entry
            .get("StageTimeSec")
            .or_else(|| entry.get("stageTimeSec"))
            .or_else(|| entry.get("PostTotalTimeSec"))
            .or_else(|| entry.get("postTotalTimeSec"))
            .or_else(|| entry.get("TotalTimeSec"))
            .or_else(|| entry.get("totalTimeSec")),
    )?;
    if elapsed <= 0 {
        return None;
    }

    let remaining =
        ((elapsed as f64) * f64::from(100 - progress) / f64::from(progress)).round() as i64;
    Some(remaining.max(0))
}

fn map_history_state(
    status_upper: &str,
    entry: &serde_json::Map<String, Value>,
) -> (DownloadQueueState, Option<String>) {
    if status_upper.starts_with("SUCCESS") {
        // Even with SUCCESS status, check individual stage fields for failures
        if let Some(reason) = check_history_stage_failure(entry) {
            return (DownloadQueueState::Failed, Some(reason));
        }
        (DownloadQueueState::Completed, None)
    } else if status_upper.starts_with("FAILURE") {
        let reason = check_history_stage_failure(entry).or_else(|| {
            status_upper.split_once('/').and_then(|(_, detail)| {
                let detail = detail.trim();
                (!detail.is_empty()).then_some(detail.to_string())
            })
        });
        (DownloadQueueState::Failed, reason)
    } else if status_upper.starts_with("UNKNOWN") {
        (
            DownloadQueueState::Failed,
            Some("unknown failure".to_string()),
        )
    } else {
        (DownloadQueueState::Completed, None)
    }
}

/// Extracts the downloader display Name field from a history entry.
fn history_entry_name(entry: &serde_json::Map<String, Value>) -> String {
    entry
        .get("Name")
        .or_else(|| entry.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("Unnamed download")
        .to_string()
}

/// Extracts NZBGet's `NZBName` for a history entry.
///
/// NZBGet has no immutable release-name field: `NZBName`, the deprecated
/// `NZBNicename`, and the history `Name` are all formatted from the same
/// `NzbInfo::m_name` (`XmlRpc.cpp` `AppendNzbInfoFields` /
/// `HistoryInfo::GetName`), which starts as the NZB filename minus `.nzb` and
/// is rewritten by `GroupSetName` (UI rename), scan/PP-script `NZBNAME`
/// directives, and URL-fetch flows. This is therefore only as canonical as
/// `Name`; the durable release identity for a Scryer grab is the persisted
/// submission title, not this field.
fn history_entry_release_name(entry: &serde_json::Map<String, Value>) -> Option<String> {
    entry
        .get("NZBName")
        .or_else(|| entry.get("NzbName"))
        .or_else(|| entry.get("nzbName"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// Reads a string field from an NZBGet history entry, trying PascalCase then camelCase.
fn history_field_str<'a>(
    entry: &'a serde_json::Map<String, Value>,
    pascal: &str,
    camel: &str,
) -> Option<&'a str> {
    entry
        .get(pascal)
        .or_else(|| entry.get(camel))
        .and_then(Value::as_str)
}

/// Checks NZBGet's individual post-processing stage fields for failures.
/// Returns Some(reason) if any stage indicates a problem, None if all stages passed.
///
/// Mirrors Sonarr's multi-field cascade:
///   DeleteStatus → MarkStatus → ParStatus → UnpackStatus → ScriptStatus
///
/// Success values are "SUCCESS" and "NONE" (step was skipped/not applicable).
fn check_history_stage_failure(entry: &serde_json::Map<String, Value>) -> Option<String> {
    let delete_status = history_field_str(entry, "DeleteStatus", "deleteStatus").unwrap_or("NONE");
    let mark_status = history_field_str(entry, "MarkStatus", "markStatus").unwrap_or("NONE");

    // Manual deletion: user removed from NZBGet UI
    if delete_status.eq_ignore_ascii_case("MANUAL") {
        if mark_status.eq_ignore_ascii_case("BAD") {
            return Some("marked bad and manually deleted".to_string());
        }
        // User-deleted but not marked bad — skip entirely (handled by caller via DELETED status)
        return None;
    }
    if delete_status.eq_ignore_ascii_case("HEALTH") {
        return Some("deleted due to health check failure".to_string());
    }
    if delete_status.eq_ignore_ascii_case("DUPE") {
        return Some("deleted as duplicate".to_string());
    }
    if !is_nzbget_success_value(delete_status) {
        return Some(format!("delete failed: {delete_status}"));
    }

    let par_status = history_field_str(entry, "ParStatus", "parStatus").unwrap_or("NONE");
    if !is_nzbget_success_value(par_status) {
        return Some(format!("par repair failed: {par_status}"));
    }

    let unpack_status = history_field_str(entry, "UnpackStatus", "unpackStatus").unwrap_or("NONE");
    if unpack_status.eq_ignore_ascii_case("SPACE") {
        return Some("unpack failed: disk space".to_string());
    }
    if !is_nzbget_success_value(unpack_status) {
        return Some(format!("unpack failed: {unpack_status}"));
    }

    let move_status = history_field_str(entry, "MoveStatus", "moveStatus").unwrap_or("NONE");
    if !is_nzbget_success_value(move_status) {
        return Some(format!("move failed: {move_status}"));
    }

    let script_status = history_field_str(entry, "ScriptStatus", "scriptStatus").unwrap_or("NONE");
    if !is_nzbget_success_value(script_status) {
        return Some(format!("script failed: {script_status}"));
    }

    None
}

/// NZBGet considers "SUCCESS" and "NONE" (step skipped) as passing values.
fn is_nzbget_success_value(value: &str) -> bool {
    value.eq_ignore_ascii_case("SUCCESS") || value.eq_ignore_ascii_case("NONE")
}

/// Parses the major version number from an NZBGet version string.
/// Handles formats like "24.3", "nzbget-24.3", "24.3-testing".
fn parse_nzbget_major_version(version: &str) -> Option<u32> {
    parse_nzbget_version(version).map(|version| version.major)
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct NzbgetVersion {
    major: u32,
    minor: u32,
}

fn parse_nzbget_version(version: &str) -> Option<NzbgetVersion> {
    let cleaned = version
        .trim()
        .trim_start_matches("nzbget-")
        .trim_start_matches("nzbget")
        .trim();

    let mut segments = cleaned.split('.');
    let major = parse_numeric_prefix(segments.next()?)?;
    let minor = segments.next().and_then(parse_numeric_prefix).unwrap_or(0);

    Some(NzbgetVersion { major, minor })
}

fn parse_numeric_prefix(segment: &str) -> Option<u32> {
    let digits: String = segment
        .trim()
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect();

    if digits.is_empty() {
        None
    } else {
        digits.parse::<u32>().ok()
    }
}

fn supports_nzbget_append_auto_category(version: &str) -> bool {
    parse_nzbget_version(version)
        .map(|parsed| {
            parsed
                >= NzbgetVersion {
                    major: 25,
                    minor: 3,
                }
        })
        .unwrap_or(false)
}

fn build_nzbget_append_payload(append_request: &NzbgetAppendRequest<'_>, dupe_mode: &str) -> Value {
    let mut params = vec![
        json!(append_request.nzb_filename),
        json!(append_request.source_for_payload),
        json!(append_request.category),
        json!(append_request.queue_priority),
        json!(false),
        json!(false),
        json!(""),
        json!(0),
        json!(dupe_mode),
    ];
    if append_request.use_auto_category {
        params.push(json!(false));
    }
    params.push(json!(append_request.parameters));

    json!({
        "version": "2.0",
        "method": "append",
        "params": params,
        "id": append_request.request_id,
    })
}

fn base64_encoded_len(raw_len: u64) -> u64 {
    raw_len.div_ceil(3) * 4
}

fn split_streaming_payload(payload: &Value, placeholder: &[u8]) -> AppResult<(Vec<u8>, Vec<u8>)> {
    let payload_json = serde_json::to_vec(payload).map_err(|error| {
        AppError::Repository(format!("failed to encode nzbget append payload: {error}"))
    })?;
    let Some(position) = payload_json
        .windows(placeholder.len())
        .position(|window| window == placeholder)
    else {
        return Err(AppError::Repository(
            "failed to locate streaming payload placeholder in nzbget request".into(),
        ));
    };

    let prefix = payload_json[..position].to_vec();
    let suffix = payload_json[position + placeholder.len()..].to_vec();
    Ok((prefix, suffix))
}

fn nzbget_queue_priority(raw_priority: Option<&str>) -> i32 {
    match raw_priority
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
        .as_deref()
    {
        Some("force") => 900,
        Some("very high") => 100,
        Some("high") => 50,
        Some("normal") => 0,
        Some("low") => -50,
        Some("very low") => -100,
        _ => 0,
    }
}

fn is_nzbget_invalid_procedure_error(err: &AppError) -> bool {
    matches!(err, AppError::Repository(message) if message.contains("Invalid procedure"))
}

fn sanitize_filename_with_nzb_ext(name: &str) -> String {
    let mut sanitized = String::new();
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, ' ' | '_' | '-' | '.' | '(' | ')') {
            sanitized.push(ch);
        } else {
            sanitized.push('_');
        }
    }

    let trimmed = sanitized.trim();
    if trimmed.is_empty() {
        "download.nzb".to_string()
    } else {
        format!("{trimmed}.nzb")
    }
}

fn sanitize_nzb_name(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return "download.nzb".to_string();
    }

    let without_ext = if trimmed.to_ascii_lowercase().ends_with(".nzb") {
        &trimmed[..trimmed.len().saturating_sub(4)]
    } else {
        trimmed
    };

    sanitize_filename_with_nzb_ext(without_ext)
}

fn derive_nzb_filename(
    source_title: Option<&str>,
    source_hint: &str,
    fallback_title: &str,
) -> String {
    if let Some(title) = source_title {
        return sanitize_nzb_name(title);
    }

    if let Ok(url) = reqwest::Url::parse(source_hint) {
        if let Some(query_title) = url.query_pairs().find_map(|(key, value)| {
            (key.eq_ignore_ascii_case("title")
                || key.eq_ignore_ascii_case("dn")
                || key.eq_ignore_ascii_case("name"))
            .then(|| value.into_owned())
        }) {
            return sanitize_nzb_name(&query_title);
        }

        if let Some(path_segment) = url.path_segments().and_then(|segments| {
            let mut segments = segments;
            segments.rfind(|segment| !segment.is_empty())
        }) && path_segment.to_ascii_lowercase().ends_with(".nzb")
        {
            return sanitize_nzb_name(path_segment);
        }
    }

    sanitize_filename_with_nzb_ext(fallback_title)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_partial_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn mount_editqueue(
        server: &MockServer,
        command: &str,
        nzb_id: i64,
        result: bool,
        expected: u64,
    ) {
        Mock::given(method("POST"))
            .and(path("/jsonrpc"))
            .and(body_partial_json(json!({
                "method": "editqueue",
                "params": [command, "", [nzb_id]],
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "version": "2.0",
                "result": result,
                "id": "scryer-rpc",
            })))
            .expect(expected)
            .mount(server)
            .await;
    }

    #[test]
    fn nzbget_delete_commands_fall_back_to_the_other_list() {
        assert_eq!(
            nzbget_delete_commands(false),
            ("GroupDelete", "HistoryDelete")
        );
        assert_eq!(
            nzbget_delete_commands(true),
            ("HistoryDelete", "GroupDelete")
        );
    }

    #[tokio::test]
    async fn delete_queue_hint_falls_back_to_history_when_group_delete_matches_nothing() {
        let server = MockServer::start().await;
        mount_editqueue(&server, "GroupDelete", 42, false, 1).await;
        mount_editqueue(&server, "HistoryDelete", 42, true, 1).await;

        let client = NzbgetDownloadClient::new(server.uri(), None, None, "SCORE".to_string());
        client
            .delete_queue_item("42", false, false)
            .await
            .expect("history fallback should remove the item");
    }

    #[tokio::test]
    async fn delete_history_hint_falls_back_to_queue_when_history_delete_matches_nothing() {
        let server = MockServer::start().await;
        mount_editqueue(&server, "HistoryDelete", 42, false, 1).await;
        mount_editqueue(&server, "GroupDelete", 42, true, 1).await;

        let client = NzbgetDownloadClient::new(server.uri(), None, None, "SCORE".to_string());
        client
            .delete_queue_item("42", true, false)
            .await
            .expect("queue fallback should remove the item");
    }

    #[tokio::test]
    async fn delete_with_correct_hint_issues_exactly_one_command() {
        let server = MockServer::start().await;
        mount_editqueue(&server, "GroupDelete", 42, true, 1).await;
        mount_editqueue(&server, "HistoryDelete", 42, true, 0).await;

        let client = NzbgetDownloadClient::new(server.uri(), None, None, "SCORE".to_string());
        client
            .delete_queue_item("42", false, false)
            .await
            .expect("queue delete should succeed on the first command");
    }

    #[tokio::test]
    async fn delete_fails_when_neither_list_holds_the_item() {
        let server = MockServer::start().await;
        mount_editqueue(&server, "GroupDelete", 42, false, 1).await;
        mount_editqueue(&server, "HistoryDelete", 42, false, 1).await;

        let client = NzbgetDownloadClient::new(server.uri(), None, None, "SCORE".to_string());
        let error = client
            .delete_queue_item("42", false, false)
            .await
            .expect_err("an item in neither list should surface as an error");
        assert!(
            error.to_string().contains("matched no items"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn outbound_rate_limit_preserves_retry_after() {
        let error = OutboundHttpError::RateLimited(scryer_outbound_http::RateLimitedError {
            scope: scryer_outbound_http::RateLimitScopeKey::from("nzbget"),
            retry_after: Some(Duration::from_secs(30)),
            attempts: 1,
            retry_after_source: scryer_outbound_http::RetryAfterSource::Seconds,
            request_label: std::borrow::Cow::Borrowed("nzbget"),
        });
        let error = map_nzbget_outbound_error("nzbget status", error);

        match error {
            AppError::TemporaryUnavailable {
                message,
                retry_after,
                ..
            } => {
                assert!(message.contains("retry after 30s"));
                assert_eq!(retry_after, Some(Duration::from_secs(30)));
            }
            other => panic!("expected temporary unavailable error, got {other:?}"),
        }
    }

    #[test]
    fn exact_completed_lookup_keeps_old_retained_history() {
        let entry = serde_json::json!({
            "NZBID": 42,
            "Status": "SUCCESS/ALL",
            "HistoryTime": 0,
            "FinalDir": "/downloads/complete/retained",
            "FileSizeMB": 1,
        });

        assert!(
            completed_download_from_nzbget_history_entry(
                &entry,
                Some(Utc::now().timestamp() - (7 * 24 * 60 * 60)),
            )
            .is_none()
        );
        assert_eq!(
            completed_download_from_nzbget_history_entry(&entry, None)
                .expect("exact lookup should retain the old history entry")
                .download_client_item_id,
            "42"
        );
    }

    #[test]
    fn parse_nzbget_version_handles_common_formats() {
        assert_eq!(
            parse_nzbget_version("25.3"),
            Some(NzbgetVersion {
                major: 25,
                minor: 3
            })
        );
        assert_eq!(
            parse_nzbget_version("nzbget-24.3"),
            Some(NzbgetVersion {
                major: 24,
                minor: 3
            })
        );
        assert_eq!(
            parse_nzbget_version("nzbget 25.3-testing-r2"),
            Some(NzbgetVersion {
                major: 25,
                minor: 3
            })
        );
        assert_eq!(
            parse_nzbget_version("26"),
            Some(NzbgetVersion {
                major: 26,
                minor: 0
            })
        );
        assert_eq!(parse_nzbget_version("unknown"), None);
    }

    #[test]
    fn append_auto_category_support_starts_at_v25_3() {
        assert!(!supports_nzbget_append_auto_category("25.2"));
        assert!(supports_nzbget_append_auto_category("25.3"));
        assert!(supports_nzbget_append_auto_category("26.0"));
    }

    #[test]
    fn reconciled_append_job_requires_matching_title_id_and_release_name() {
        let parameters = Vec::new();
        let append_request = NzbgetAppendRequest {
            request_id: "req-1",
            title_name: "Bluey",
            nzb_filename: "Bluey.S01.720p.WEB-DL.AV1.AAC2.0-NTb.nzb",
            source_for_payload: "",
            category: "series",
            queue_priority: 0,
            parameters: &parameters,
            use_auto_category: true,
        };

        let matching = DownloadQueueItem {
            id: "queue-1".to_string(),
            title_id: Some("title-1".to_string()),
            episode_id: None,
            title_name: "Bluey.S01.720p.WEB-DL.AV1.AAC2.0-NTb".to_string(),
            facet: None,
            category: None,
            client_id: String::new(),
            client_name: String::new(),
            client_type: "nzbget".to_string(),
            state: DownloadQueueState::Queued,
            progress_percent: 0,
            import_transfer_phase: None,
            import_transfer_bytes: None,
            import_transfer_total_bytes: None,
            import_transfer_started_at: None,
            import_transfer_updated_at: None,
            size_bytes: None,
            remaining_seconds: None,
            queued_at: None,
            last_updated_at: None,
            attention_required: false,
            attention_reason: None,
            download_client_item_id: "42".to_string(),
            download_id: None,
            import_status: None,
            import_error_code: None,
            import_error_message: None,
            imported_at: None,
            delete_status: None,
            delete_error_message: None,
            source_provider: None,
            is_scryer_origin: true,
            tracked_state: None,
            tracked_status: None,
            tracked_status_messages: Vec::new(),
            tracked_match_type: None,
            seeding: None,
        };
        let wrong_release = DownloadQueueItem {
            title_name: "Bluey.S02.720p.WEB-DL.AV1.AAC2.0-NTb".to_string(),
            download_client_item_id: "43".to_string(),
            ..matching.clone()
        };
        let wrong_title = DownloadQueueItem {
            title_id: Some("title-2".to_string()),
            download_client_item_id: "44".to_string(),
            ..matching.clone()
        };

        assert_eq!(
            find_reconciled_nzbget_append_job(
                &[wrong_release, wrong_title, matching],
                "title-1",
                &append_request,
            ),
            Some(42)
        );
    }

    #[test]
    fn extract_nzbget_category_trims_empty_values() {
        let entry = json!({
            "Category": " movies "
        });
        let entry = entry.as_object().expect("object");
        assert_eq!(extract_nzbget_category(entry).as_deref(), Some("movies"));

        let entry = json!({
            "category": " "
        });
        let entry = entry.as_object().expect("object");
        assert_eq!(extract_nzbget_category(entry), None);
    }

    #[test]
    fn extract_nzbget_parameters_maps_download_id() {
        let entry = json!({
            "Parameters": [
                { "Name": "*scryer_title_id", "Value": "title-1" },
                { "Name": "*scryer_facet", "Value": "anime" },
                { "Name": "*scryer_download_id", "Value": " scryer-download:nzbget-1 " }
            ]
        });
        let params = extract_nzbget_parameters(entry.as_object().expect("object"));

        assert_eq!(params.title_id.as_deref(), Some("title-1"));
        assert_eq!(params.facet.as_deref(), Some("anime"));
        assert_eq!(
            params.download_id.as_deref(),
            Some("scryer-download:nzbget-1")
        );
        assert!(params.is_scryer);
    }

    #[test]
    fn build_append_payload_preserves_the_passed_scryer_download_id() {
        let parameters = vec![
            json!({"*scryer_title_id": "title-1"}),
            json!({"*scryer_facet": "movie"}),
            json!({"*scryer_import_purpose": "standard"}),
            json!({"*scryer_download_id": "scryer-download:nzbget-token"}),
        ];
        let append_request = NzbgetAppendRequest {
            request_id: "request-1",
            title_name: "Example",
            nzb_filename: "Example.nzb",
            source_for_payload: "YmFzZTY0LWRhdGE=",
            category: "movies",
            queue_priority: 50,
            parameters: &parameters,
            use_auto_category: true,
        };

        assert_eq!(
            build_nzbget_append_payload(&append_request, "SCORE"),
            json!({
                "version": "2.0",
                "method": "append",
                "params": [
                    "Example.nzb",
                    "YmFzZTY0LWRhdGE=",
                    "movies",
                    50,
                    false,
                    false,
                    "",
                    0,
                    "SCORE",
                    false,
                    parameters,
                ],
                "id": "request-1",
            }),
        );
    }

    #[test]
    fn build_append_payload_uses_legacy_signature_for_older_servers() {
        let parameters = vec![json!({"*scryer_title_id": "title-1"})];
        let append_request = NzbgetAppendRequest {
            request_id: "req-1",
            title_name: "Example",
            nzb_filename: "Example.nzb",
            source_for_payload: "base64-data",
            category: "movies",
            queue_priority: 0,
            parameters: &parameters,
            use_auto_category: false,
        };
        let payload = build_nzbget_append_payload(&append_request, "SCORE");

        let params = payload
            .get("params")
            .and_then(Value::as_array)
            .expect("append payload should include params");
        assert_eq!(params.len(), 10);
        assert_eq!(params[8], json!("SCORE"));
        assert_eq!(params[9], json!(parameters));
    }

    #[test]
    fn build_append_payload_includes_auto_category_for_newer_servers() {
        let parameters = vec![json!({"*scryer_title_id": "title-1"})];
        let append_request = NzbgetAppendRequest {
            request_id: "req-1",
            title_name: "Example",
            nzb_filename: "Example.nzb",
            source_for_payload: "base64-data",
            category: "movies",
            queue_priority: 0,
            parameters: &parameters,
            use_auto_category: true,
        };
        let payload = build_nzbget_append_payload(&append_request, "SCORE");

        let params = payload
            .get("params")
            .and_then(Value::as_array)
            .expect("append payload should include params");
        assert_eq!(params.len(), 11);
        assert_eq!(params[8], json!("SCORE"));
        assert_eq!(params[9], json!(false));
        assert_eq!(params[10], json!(parameters));
    }

    #[test]
    fn split_streaming_payload_preserves_json_string_quotes() {
        let placeholder = "__SCRYER_STREAMED_NZB_BASE64__";
        let parameters = vec![json!({"*scryer_title_id": "title-1"})];
        let append_request = NzbgetAppendRequest {
            request_id: "req-1",
            title_name: "Example",
            nzb_filename: "Example.nzb",
            source_for_payload: placeholder,
            category: "movies",
            queue_priority: 0,
            parameters: &parameters,
            use_auto_category: false,
        };
        let payload = build_nzbget_append_payload(&append_request, "SCORE");
        let (prefix, suffix) =
            split_streaming_payload(&payload, placeholder.as_bytes()).expect("split should work");
        let body = [prefix, b"YmFzZTY0LWRhdGE=".to_vec(), suffix].concat();
        let body_json: Value =
            serde_json::from_slice(&body).expect("body should remain valid json");
        let params = body_json
            .get("params")
            .and_then(Value::as_array)
            .expect("append payload should include params");
        assert_eq!(params[1], json!("YmFzZTY0LWRhdGE="));
    }

    #[test]
    fn nzbget_queue_priority_maps_supported_values() {
        assert_eq!(nzbget_queue_priority(Some("force")), 900);
        assert_eq!(nzbget_queue_priority(Some("very high")), 100);
        assert_eq!(nzbget_queue_priority(Some("high")), 50);
        assert_eq!(nzbget_queue_priority(Some("normal")), 0);
        assert_eq!(nzbget_queue_priority(Some("low")), -50);
        assert_eq!(nzbget_queue_priority(Some("very low")), -100);
        assert_eq!(nzbget_queue_priority(None), 0);
    }
}

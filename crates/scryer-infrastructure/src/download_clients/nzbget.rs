use std::collections::HashMap;
use std::io::Write;

use async_trait::async_trait;
use base64::{engine::general_purpose, Engine as _};
use chrono::{DateTime, Utc};
use flate2::write::GzEncoder;
use flate2::Compression;
use reqwest::Client;
use scryer_application::{
    AppError, AppResult, DownloadClient, DownloadClientAddRequest, DownloadGrabResult,
};
use scryer_domain::{DownloadQueueItem, DownloadQueueState};
use serde_json::{json, Value};
use tracing::{debug, warn};

use super::{
    extract_f64_value, extract_i64_value, is_http_url, parse_duration_seconds,
    progress_percent_from_sizes, size_to_bytes,
};

#[derive(Clone)]
pub struct NzbgetDownloadClient {
    rpc_url: String,
    username: Option<String>,
    password: Option<String>,
    dupe_mode: String,
    http_client: Client,
}

impl NzbgetDownloadClient {
    pub fn new(
        rpc_url: String,
        username: Option<String>,
        password: Option<String>,
        dupe_mode: String,
    ) -> Self {
        let dupe_mode = match dupe_mode.to_uppercase().as_str() {
            "ALL" | "FORCE" => dupe_mode.to_uppercase(),
            _ => "SCORE".to_string(),
        };
        Self {
            rpc_url: rpc_url.trim_end_matches('/').to_string(),
            username,
            password,
            dupe_mode,
            http_client: Client::new(),
        }
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
        let payload = json!({
            "version": "2.0",
            "method": method,
            "params": params,
            "id": "scryer-rpc",
        });

        let endpoint = self.endpoint();
        let mut request = self
            .http_client
            .post(&endpoint)
            .header("Content-Type", "application/json")
            .json(&payload);

        if let Some(username) = self.username.clone() {
            request = request.basic_auth(username, self.password.as_deref());
        }

        let response = request
            .send()
            .await
            .map_err(|err| AppError::Repository(format!("nzbget rpc call failed: {err}")))?;

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
            .http_client
            .post(&endpoint)
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|err| AppError::Repository(format!("nzbget test call failed: {err}")))?;
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
        if let Some(major) = parse_nzbget_major_version(&version) {
            if major < 12 {
                return Err(AppError::Repository(format!(
                    "nzbget {version} is not supported; version 12.0+ is required"
                )));
            }
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
                    if let Some(kh) = keep_history {
                        if let Ok(kh_val) = kh.parse::<i64>() {
                            if kh_val == 0 {
                                return Err(AppError::Repository(
                                    "nzbget KeepHistory is set to 0 — completed downloads are \
                                     immediately purged and cannot be auto-imported. Set \
                                     KeepHistory to at least 1 in NZBGet settings."
                                        .into(),
                                ));
                            }
                        }
                    }
                }
            }
            Err(err) => {
                warn!(error = %err, "failed to read nzbget config for KeepHistory check");
            }
        }

        Ok(version)
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
            let preview = body.chars().take(300).collect::<String>();
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
        job_id: &str,
        title_name: &str,
        nzb_filename: &str,
        source_for_payload: &str,
        category: &str,
        queue_priority: i32,
        parameters: &[Value],
        use_auto_category: bool,
    ) -> AppResult<i64> {
        let request_payload = build_nzbget_append_payload(
            job_id,
            nzb_filename,
            source_for_payload,
            category,
            queue_priority,
            &self.dupe_mode,
            parameters,
            use_auto_category,
        );

        let mut request_payload_for_log = request_payload.clone();
        if let Some(params) = request_payload_for_log
            .get_mut("params")
            .and_then(Value::as_array_mut)
        {
            if params.len() > 1 {
                params[1] = Value::String("<omitted base64 nzb content>".to_string());
            }
        }

        let endpoint = self.endpoint();
        tracing::info!(
            endpoint = endpoint.as_str(),
            auto_category = use_auto_category,
            payload = %request_payload_for_log,
            "nzbget append request payload"
        );
        let json_bytes = serde_json::to_vec(&request_payload).map_err(|err| {
            AppError::Repository(format!("nzbget append payload serialization failed: {err}"))
        })?;
        let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
        encoder.write_all(&json_bytes).map_err(|err| {
            AppError::Repository(format!("nzbget append gzip compression failed: {err}"))
        })?;
        let compressed = encoder.finish().map_err(|err| {
            AppError::Repository(format!("nzbget append gzip finalization failed: {err}"))
        })?;

        let mut request = self
            .http_client
            .post(&endpoint)
            .header("Content-Type", "application/json")
            .header("Content-Encoding", "gzip")
            .body(compressed);

        if let Some(username) = self.username.clone() {
            request = request.basic_auth(username, self.password.as_deref());
        }

        let response = request
            .send()
            .await
            .map_err(|err| AppError::Repository(format!("nzbget request failed: {err}")))?;
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
                title = title_name,
                auto_category = use_auto_category,
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
            job_id,
            title = title_name,
            category,
            auto_category = use_auto_category,
            "nzbget append succeeded"
        );

        Ok(result)
    }

    async fn edit_queue(&self, command: &str, ids: Vec<i64>) -> AppResult<()> {
        let result = self
            .rpc_call("editqueue", vec![json!(command), json!(""), json!(ids)])
            .await?;
        if result.as_bool() == Some(true) {
            Ok(())
        } else {
            Err(AppError::Repository(format!(
                "nzbget editqueue {command} returned unexpected result: {result}"
            )))
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

                let (param_title_id, param_facet, is_scryer) = extract_nzbget_parameters(group);
                let queue_progress = progress_percent_from_sizes(size_mb, remaining_mb);
                let remaining_seconds = extract_remaining_seconds_from_entry(group);

                Some(DownloadQueueItem {
                    id: nzb_id.to_string(),
                    title_id: param_title_id,
                    title_name,
                    facet: param_facet.clone(),
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
                    import_status: None,
                    import_error_message: None,
                    imported_at: None,
                    is_scryer_origin: is_scryer,
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

                let (param_title_id, param_facet, is_scryer) = extract_nzbget_parameters(entry);
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
                        existing.title_id = param_title_id;
                    }
                    if existing.facet.is_none() {
                        existing.facet = param_facet;
                    }
                    if existing.size_bytes.is_none() || existing.size_bytes == Some(0) {
                        existing.size_bytes = size_to_bytes(size_mb);
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
                    existing.is_scryer_origin = existing.is_scryer_origin || is_scryer;
                    continue;
                }

                items.push(DownloadQueueItem {
                    id: id.clone(),
                    title_id: param_title_id,
                    title_name,
                    facet: param_facet.clone(),
                    client_id: String::new(),
                    client_name: String::new(),
                    client_type: "nzbget".to_string(),
                    state,
                    progress_percent,
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
                    import_status: None,
                    import_error_message: None,
                    imported_at: None,
                    is_scryer_origin: is_scryer,
                });
                let next_index = items.len() - 1;
                item_index_by_id.insert(id, next_index);
            }
        } else {
            debug!("nzbget postqueue endpoint unavailable; skipping pp queue merge");
        }

        // Global pause detection: when NZBGet's download queue is globally paused,
        // all non-completed items should show as Paused.
        if !items.is_empty() {
            if let Ok(status_result) = self.rpc_call("status", vec![]).await {
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
                if let Some(ts) = history_ts {
                    if ts < cutoff_ts {
                        return None;
                    }
                }

                let title_name = entry
                    .get("Name")
                    .or_else(|| entry.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or("Unnamed download")
                    .to_string();
                let size_mb =
                    extract_f64_value(entry.get("FileSizeMB").or_else(|| entry.get("fileSizeMB")))
                        .unwrap_or(0.0);

                let (param_title_id, param_facet, is_scryer) = extract_nzbget_parameters(entry);

                Some(DownloadQueueItem {
                    id: nzb_id.to_string(),
                    title_id: param_title_id,
                    title_name,
                    facet: param_facet.clone(),
                    client_id: String::new(),
                    client_name: String::new(),
                    client_type: "nzbget".to_string(),
                    state,
                    progress_percent: if state == DownloadQueueState::Completed {
                        100
                    } else {
                        0
                    },
                    size_bytes: size_to_bytes(size_mb),
                    remaining_seconds: None,
                    queued_at: None,
                    last_updated_at: history_ts.map(|value| value.to_string()),
                    attention_required: matches!(state, DownloadQueueState::Failed),
                    attention_reason,
                    download_client_item_id: nzb_id.to_string(),
                    import_status: None,
                    import_error_message: None,
                    imported_at: None,
                    is_scryer_origin: is_scryer,
                })
            })
            .collect())
    }
}

#[async_trait]
impl DownloadClient for NzbgetDownloadClient {
    async fn submit_download(
        &self,
        request: &DownloadClientAddRequest,
    ) -> AppResult<DownloadGrabResult> {
        let title = &request.title;
        let job_id = scryer_domain::Id::new().0;
        let source_hint = request
            .source_hint
            .clone()
            .and_then(|value| {
                let value = value.trim().to_string();
                (!value.is_empty()).then_some(value)
            })
            .ok_or_else(|| {
                AppError::Validation("source hint is required to queue a download".into())
            })?;

        if !is_http_url(&source_hint) {
            return Err(AppError::Validation(format!(
                "source hint must be an NZB URL; got {source_hint}"
            )));
        }

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

        let source_for_payload = self.fetch_and_encode_nzb(&source_hint).await?;
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
        ];

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

        let use_auto_category = self.append_requires_auto_category().await;
        let queue_priority = nzbget_queue_priority(request.queue_priority.as_deref());
        match self
            .send_append_request(
                &job_id,
                title.name.as_str(),
                &nzb_filename,
                &source_for_payload,
                &category,
                queue_priority,
                &parameters,
                use_auto_category,
            )
            .await
        {
            Ok(queue_id) => queue_id,
            Err(err) if is_nzbget_invalid_procedure_error(&err) => {
                let retry_use_auto_category = !use_auto_category;
                warn!(
                    error = %err,
                    retry_auto_category = retry_use_auto_category,
                    title = title.name.as_str(),
                    "nzbget append rejected payload shape; retrying alternate append signature"
                );
                self.send_append_request(
                    &job_id,
                    title.name.as_str(),
                    &nzb_filename,
                    &source_for_payload,
                    &category,
                    queue_priority,
                    &parameters,
                    retry_use_auto_category,
                )
                .await?
            }
            Err(err) => return Err(err),
        };

        Ok(DownloadGrabResult {
            job_id,
            client_type: "nzbget".to_string(),
        })
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

    async fn delete_queue_item(&self, id: &str, is_history: bool) -> AppResult<()> {
        let nzb_id: i64 = id
            .parse()
            .map_err(|_| AppError::Validation(format!("invalid nzbget queue id: {id}")))?;
        let command = if is_history {
            "HistoryDelete"
        } else {
            "GroupDelete"
        };
        self.edit_queue(command, vec![nzb_id]).await
    }

    async fn list_completed_downloads(&self) -> AppResult<Vec<scryer_domain::CompletedDownload>> {
        let result = self.rpc_call("history", vec![json!(false)]).await?;
        let entries = extract_result_array(result, "History").unwrap_or_default();
        let cutoff_ts = Utc::now().timestamp() - (7 * 24 * 60 * 60);

        Ok(entries
            .into_iter()
            .filter_map(|entry| {
                let entry = entry.as_object()?;
                let nzb_id = extract_i64_value(entry.get("NZBID").or_else(|| entry.get("nzbId")))
                    .filter(|value| *value > 0)?;
                let status = entry
                    .get("Status")
                    .or_else(|| entry.get("status"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let status_upper = status.to_ascii_uppercase();

                if !status_upper.starts_with("SUCCESS") {
                    return None;
                }

                // Multi-field failure detection (mirrors Sonarr's cascade):
                // Even when top-level Status is SUCCESS, individual stages may
                // indicate problems that make the download unusable.
                if let Some(reason) = check_history_stage_failure(entry) {
                    let name = history_entry_name(entry);
                    warn!(
                        nzb_id,
                        name = name.as_str(),
                        reason = reason.as_str(),
                        "skipping completed download due to stage failure"
                    );
                    return None;
                }

                let history_ts =
                    extract_i64_value(entry.get("HistoryTime").or_else(|| entry.get("time")));
                if let Some(ts) = history_ts {
                    if ts < cutoff_ts {
                        return None;
                    }
                }

                // FinalDir is where files end up after NZBGet's move step;
                // DestDir may point to a now-empty intermediate directory.
                let dest_dir = entry
                    .get("FinalDir")
                    .or_else(|| entry.get("finalDir"))
                    .or_else(|| entry.get("DestDir"))
                    .or_else(|| entry.get("destDir"))
                    .and_then(Value::as_str)
                    .filter(|v| !v.is_empty())
                    .unwrap_or("")
                    .to_string();

                if dest_dir.is_empty() {
                    return None;
                }

                let name = history_entry_name(entry);

                let category = entry
                    .get("Category")
                    .or_else(|| entry.get("category"))
                    .and_then(Value::as_str)
                    .map(|v| v.to_string())
                    .filter(|v| !v.is_empty());

                let size_mb =
                    extract_f64_value(entry.get("FileSizeMB").or_else(|| entry.get("fileSizeMB")))
                        .unwrap_or(0.0);

                let parameters = entry
                    .get("Parameters")
                    .or_else(|| entry.get("parameters"))
                    .and_then(Value::as_array)
                    .map(|params| {
                        params
                            .iter()
                            .filter_map(|p| {
                                let obj = p.as_object()?;
                                let key = obj
                                    .get("Name")
                                    .or_else(|| obj.get("name"))
                                    .and_then(Value::as_str)?
                                    .to_string();
                                let value = obj
                                    .get("Value")
                                    .or_else(|| obj.get("value"))
                                    .and_then(Value::as_str)?
                                    .to_string();
                                Some((key, value))
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();

                let completed_at =
                    history_ts.map(|ts| DateTime::from_timestamp(ts, 0).unwrap_or_else(Utc::now));

                Some(scryer_domain::CompletedDownload {
                    client_type: "nzbget".to_string(),
                    client_id: String::new(),
                    download_client_item_id: nzb_id.to_string(),
                    name,
                    dest_dir,
                    category,
                    size_bytes: size_to_bytes(size_mb),
                    completed_at,
                    parameters,
                })
            })
            .collect())
    }
}

fn extract_nzbget_parameters(
    entry: &serde_json::Map<String, Value>,
) -> (Option<String>, Option<String>, bool) {
    let params = entry
        .get("Parameters")
        .or_else(|| entry.get("parameters"))
        .and_then(Value::as_array);
    let params = match params {
        Some(params) => params,
        None => return (None, None, false),
    };
    let mut title_id: Option<String> = None;
    let mut facet: Option<String> = None;
    let mut is_scryer = false;
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
        match key {
            "*scryer_title_id" => {
                is_scryer = true;
                if !value.is_empty() {
                    title_id = Some(value.to_string());
                }
            }
            "*scryer_facet" => {
                if !value.is_empty() {
                    facet = Some(value.to_string());
                }
            }
            _ => {}
        }
    }
    (title_id, facet, is_scryer)
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
        if let Some(text) = candidate.as_str() {
            if let Some(token) = find_nzbget_postprocessing_token(text) {
                return Some(token.to_string());
            }
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
    {
        if progress_permille > 0.0 && stage_time_sec >= 0 {
            let remaining = ((stage_time_sec as f64 / progress_permille)
                * (1000.0 - progress_permille))
                .round() as i64;
            return Some(remaining.max(0));
        }
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
    if let (Some(total), Some(elapsed)) = (total_seconds, elapsed_seconds) {
        if total > 0 && elapsed >= 0 {
            return Some((total - elapsed).max(0));
        }
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

/// Extracts the Name field from a history entry.
fn history_entry_name(entry: &serde_json::Map<String, Value>) -> String {
    entry
        .get("Name")
        .or_else(|| entry.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("Unnamed download")
        .to_string()
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

fn build_nzbget_append_payload(
    job_id: &str,
    nzb_filename: &str,
    source_for_payload: &str,
    category: &str,
    queue_priority: i32,
    dupe_mode: &str,
    parameters: &[Value],
    use_auto_category: bool,
) -> Value {
    let mut params = vec![
        json!(nzb_filename),
        json!(source_for_payload),
        json!(category),
        json!(queue_priority),
        json!(false),
        json!(false),
        json!(""),
        json!(0),
        json!(dupe_mode),
    ];
    if use_auto_category {
        params.push(json!(false));
    }
    params.push(json!(parameters));

    json!({
        "version": "2.0",
        "method": "append",
        "params": params,
        "id": job_id,
    })
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn build_append_payload_uses_legacy_signature_for_older_servers() {
        let parameters = vec![json!({"*scryer_title_id": "title-1"})];
        let payload = build_nzbget_append_payload(
            "job-1",
            "Example.nzb",
            "base64-data",
            "movies",
            0,
            "SCORE",
            &parameters,
            false,
        );

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
        let payload = build_nzbget_append_payload(
            "job-1",
            "Example.nzb",
            "base64-data",
            "movies",
            0,
            "SCORE",
            &parameters,
            true,
        );

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
        }) {
            if path_segment.to_ascii_lowercase().ends_with(".nzb") {
                return sanitize_nzb_name(path_segment);
            }
        }
    }

    sanitize_filename_with_nzb_ext(fallback_title)
}

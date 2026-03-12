use std::io::Write;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use flate2::write::GzEncoder;
use flate2::Compression;
use reqwest::multipart;
use reqwest::Client;
use scryer_application::{
    AppError, AppResult, DownloadClient, DownloadClientAddRequest, DownloadGrabResult,
};
use scryer_domain::{CompletedDownload, DownloadQueueItem, DownloadQueueState};
use serde_json::Value;
use tracing::debug;

use super::{extract_f64_value, extract_i64_value, is_http_url, parse_duration_seconds};

#[derive(Clone)]
pub struct SabnzbdDownloadClient {
    base_url: String,
    api_key: String,
    http_client: Client,
}

impl SabnzbdDownloadClient {
    pub fn new(base_url: String, api_key: String) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
            http_client: Client::new(),
        }
    }

    fn api_url(&self) -> String {
        format!("{}/api", self.base_url)
    }

    async fn api_get(&self, params: &[(&str, &str)]) -> AppResult<Value> {
        let url = self.api_url();
        let mut query: Vec<(&str, &str)> = vec![("apikey", &self.api_key), ("output", "json")];
        query.extend_from_slice(params);

        let response = self
            .http_client
            .get(&url)
            .query(&query)
            .send()
            .await
            .map_err(|err| AppError::Repository(format!("sabnzbd api call failed: {err}")))?;

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|err| AppError::Repository(format!("sabnzbd response read failed: {err}")))?;

        if !status.is_success() {
            let preview = body.chars().take(600).collect::<String>();
            return Err(AppError::Repository(format!(
                "sabnzbd api returned status {status}: {preview}"
            )));
        }

        let json: Value = serde_json::from_str(&body).map_err(|err| {
            AppError::Repository(format!("sabnzbd returned non-json response: {err}"))
        })?;

        if let Some(false) = json.get("status").and_then(Value::as_bool) {
            let error_msg = json
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("unknown error");
            return Err(AppError::Repository(format!(
                "sabnzbd api error: {error_msg}"
            )));
        }

        Ok(json)
    }

    async fn fetch_nzb(&self, url: &str) -> AppResult<Vec<u8>> {
        let response = self
            .http_client
            .get(url)
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

        Ok(bytes.to_vec())
    }

    pub async fn test_connection(&self) -> AppResult<String> {
        // First check connectivity with unauthenticated version call
        let url = self.api_url();
        let response = self
            .http_client
            .get(&url)
            .query(&[("mode", "version"), ("output", "json")])
            .send()
            .await
            .map_err(|err| AppError::Repository(format!("sabnzbd test call failed: {err}")))?;

        let status = response.status();
        if !status.is_success() {
            return Err(AppError::Repository(format!(
                "sabnzbd test call returned status {status}"
            )));
        }

        let body = response.text().await.map_err(|err| {
            AppError::Repository(format!("sabnzbd test response read failed: {err}"))
        })?;

        let json: Value = serde_json::from_str(&body).map_err(|err| {
            AppError::Repository(format!(
                "sabnzbd test call returned non-json response: {err}"
            ))
        })?;

        let version = json
            .get("version")
            .and_then(Value::as_str)
            .unwrap_or("sabnzbd")
            .to_string();

        // Check version >= 3.0.0
        let mut warnings = Vec::new();
        let version_parts: Vec<u32> = version.split('.').filter_map(|p| p.parse().ok()).collect();
        if version_parts.len() >= 2 && version_parts[0] < 3 {
            warnings.push(format!(
                "SABnzbd {version} is outdated; version 3.0.0+ is recommended"
            ));
        }

        // Validate the API key by making an authenticated request
        self.api_get(&[("mode", "queue"), ("limit", "0")])
            .await
            .map_err(|err| {
                AppError::Repository(format!("sabnzbd api key validation failed: {err}"))
            })?;

        if warnings.is_empty() {
            Ok(version)
        } else {
            Ok(format!("{version} ({})", warnings.join("; ")))
        }
    }
}

#[async_trait]
impl DownloadClient for SabnzbdDownloadClient {
    async fn submit_download(
        &self,
        request: &DownloadClientAddRequest,
    ) -> AppResult<DownloadGrabResult> {
        let title = &request.title;
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

        let nzb_name = request
            .source_title
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .unwrap_or(title.name.as_str());

        let nzb_bytes = self.fetch_nzb(&source_hint).await?;

        let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
        encoder.write_all(&nzb_bytes).map_err(|err| {
            AppError::Repository(format!("sabnzbd nzb gzip compression failed: {err}"))
        })?;
        let compressed = encoder.finish().map_err(|err| {
            AppError::Repository(format!("sabnzbd nzb gzip finalization failed: {err}"))
        })?;

        let nzb_filename = if nzb_name.to_ascii_lowercase().ends_with(".nzb") {
            format!("{nzb_name}.gz")
        } else {
            format!("{nzb_name}.nzb.gz")
        };

        let nzb_part = multipart::Part::bytes(compressed)
            .file_name(nzb_filename)
            .mime_str("application/gzip")
            .map_err(|err| {
                AppError::Repository(format!("sabnzbd multipart build failed: {err}"))
            })?;

        let mut form = multipart::Form::new()
            .text("apikey", self.api_key.clone())
            .text("output", "json")
            .text("mode", "addfile")
            .text("nzbname", nzb_name.to_string())
            .text(
                "priority",
                sabnzbd_queue_priority(request.queue_priority.as_deref()).to_string(),
            )
            .part("nzbfile", nzb_part);

        if let Some(cat) = request.category.as_deref() {
            let trimmed = cat.trim();
            if !trimmed.is_empty() {
                form = form.text("cat", trimmed.to_string());
            }
        }

        if let Some(pw) = request.source_password.as_deref() {
            let trimmed = pw.trim();
            if !trimmed.is_empty() && trimmed != "0" {
                form = form.text("password", trimmed.to_string());
            }
        }

        let url = self.api_url();
        let response = self
            .http_client
            .post(&url)
            .multipart(form)
            .send()
            .await
            .map_err(|err| AppError::Repository(format!("sabnzbd addfile call failed: {err}")))?;

        let status = response.status();
        let body = response.text().await.map_err(|err| {
            AppError::Repository(format!("sabnzbd addfile response read failed: {err}"))
        })?;

        if !status.is_success() {
            let preview = body.chars().take(600).collect::<String>();
            return Err(AppError::Repository(format!(
                "sabnzbd addfile returned status {status}: {preview}"
            )));
        }

        let json: Value = serde_json::from_str(&body).map_err(|err| {
            AppError::Repository(format!("sabnzbd addfile returned non-json response: {err}"))
        })?;

        if let Some(false) = json.get("status").and_then(Value::as_bool) {
            let error_msg = json
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("unknown error");
            return Err(AppError::Repository(format!(
                "sabnzbd addfile error: {error_msg}"
            )));
        }

        let nzo_id = json
            .get("nzo_ids")
            .and_then(Value::as_array)
            .and_then(|ids| ids.first())
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| {
                AppError::Repository("sabnzbd addfile did not return an nzo_id".into())
            })?;

        debug!(
            nzo_id = nzo_id.as_str(),
            title = title.name.as_str(),
            nzb_name = nzb_name,
            "sabnzbd addfile succeeded"
        );

        Ok(DownloadGrabResult {
            job_id: nzo_id,
            client_type: "sabnzbd".to_string(),
        })
    }

    async fn test_connection(&self) -> AppResult<String> {
        SabnzbdDownloadClient::test_connection(self).await
    }

    async fn list_queue(&self) -> AppResult<Vec<DownloadQueueItem>> {
        let json = self.api_get(&[("mode", "queue")]).await?;

        let slots = json
            .get("queue")
            .and_then(|q| q.get("slots"))
            .and_then(Value::as_array);

        let slots = match slots {
            Some(s) => s,
            None => return Ok(Vec::new()),
        };

        Ok(slots
            .iter()
            .filter_map(|slot| {
                let slot = slot.as_object()?;

                let nzo_id = slot.get("nzo_id").and_then(Value::as_str)?.to_string();

                let raw_filename = slot
                    .get("filename")
                    .and_then(Value::as_str)
                    .unwrap_or("Unnamed download");
                let (title_name, is_encrypted) =
                    if let Some(stripped) = raw_filename.strip_prefix("ENCRYPTED / ") {
                        (stripped.to_string(), true)
                    } else {
                        (raw_filename.to_string(), false)
                    };

                let status = slot.get("status").and_then(Value::as_str).unwrap_or("");
                let state = sabnzbd_queue_state(status);

                let percentage = slot
                    .get("percentage")
                    .and_then(|v| v.as_str().or_else(|| v.as_u64().map(|_| "")))
                    .and_then(|s| {
                        if s.is_empty() {
                            slot.get("percentage")
                                .and_then(Value::as_u64)
                                .map(|v| v as u8)
                        } else {
                            s.parse::<u8>().ok()
                        }
                    })
                    .unwrap_or(0);

                let size_bytes = extract_f64_value(slot.get("mb")).map(|mb| {
                    if !mb.is_finite() || mb <= 0.0 {
                        0
                    } else {
                        (mb * 1_048_576f64).round() as i64
                    }
                });

                let remaining_seconds = slot
                    .get("timeleft")
                    .and_then(Value::as_str)
                    .and_then(parse_duration_seconds);

                let pp_status = if state == DownloadQueueState::Downloading {
                    sabnzbd_postprocessing_stage(status)
                } else {
                    None
                };

                let attention_required = is_encrypted;
                let attention_reason = if is_encrypted {
                    Some("ENCRYPTED".to_string())
                } else {
                    pp_status
                };

                Some(DownloadQueueItem {
                    id: nzo_id.clone(),
                    title_id: None,
                    title_name,
                    facet: None,
                    client_id: String::new(),
                    client_name: String::new(),
                    client_type: "sabnzbd".to_string(),
                    state,
                    progress_percent: percentage,
                    size_bytes,
                    remaining_seconds,
                    queued_at: None,
                    last_updated_at: None,
                    attention_required,
                    attention_reason,
                    download_client_item_id: nzo_id,
                    import_status: None,
                    import_error_message: None,
                    imported_at: None,
                    is_scryer_origin: false,
                })
            })
            .collect())
    }

    async fn list_history(&self) -> AppResult<Vec<DownloadQueueItem>> {
        let json = self
            .api_get(&[("mode", "history"), ("limit", "50")])
            .await?;

        let slots = json
            .get("history")
            .and_then(|h| h.get("slots"))
            .and_then(Value::as_array);

        let slots = match slots {
            Some(s) => s,
            None => return Ok(Vec::new()),
        };

        let cutoff_ts = Utc::now().timestamp() - (7 * 24 * 60 * 60);

        Ok(slots
            .iter()
            .filter_map(|slot| {
                let slot = slot.as_object()?;

                let nzo_id = slot.get("nzo_id").and_then(Value::as_str)?.to_string();

                let completed_ts = extract_i64_value(slot.get("completed"));
                if let Some(ts) = completed_ts {
                    if ts < cutoff_ts {
                        return None;
                    }
                }

                let title_name = slot
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("Unnamed download")
                    .to_string();

                let status = slot.get("status").and_then(Value::as_str).unwrap_or("");
                let (state, attention_reason) = sabnzbd_history_state(status);

                Some(DownloadQueueItem {
                    id: nzo_id.clone(),
                    title_id: None,
                    title_name,
                    facet: None,
                    client_id: String::new(),
                    client_name: String::new(),
                    client_type: "sabnzbd".to_string(),
                    state,
                    progress_percent: if state == DownloadQueueState::Completed {
                        100
                    } else {
                        0
                    },
                    size_bytes: extract_i64_value(slot.get("bytes")),
                    remaining_seconds: None,
                    queued_at: extract_i64_value(slot.get("time_added")).map(|v| v.to_string()),
                    last_updated_at: completed_ts.map(|v| v.to_string()),
                    attention_required: matches!(state, DownloadQueueState::Failed),
                    attention_reason,
                    download_client_item_id: nzo_id,
                    import_status: None,
                    import_error_message: None,
                    imported_at: None,
                    is_scryer_origin: false,
                })
            })
            .collect())
    }

    async fn list_completed_downloads(&self) -> AppResult<Vec<CompletedDownload>> {
        let json = self
            .api_get(&[("mode", "history"), ("limit", "50")])
            .await?;

        let slots = json
            .get("history")
            .and_then(|h| h.get("slots"))
            .and_then(Value::as_array);

        let slots = match slots {
            Some(s) => s,
            None => return Ok(Vec::new()),
        };

        let cutoff_ts = Utc::now().timestamp() - (7 * 24 * 60 * 60);

        Ok(slots
            .iter()
            .filter_map(|slot| {
                let slot = slot.as_object()?;

                let status = slot.get("status").and_then(Value::as_str).unwrap_or("");
                if !status.eq_ignore_ascii_case("Completed") {
                    return None;
                }

                let nzo_id = slot.get("nzo_id").and_then(Value::as_str)?.to_string();

                let completed_ts = extract_i64_value(slot.get("completed"));
                if let Some(ts) = completed_ts {
                    if ts < cutoff_ts {
                        return None;
                    }
                }

                let dest_dir = slot
                    .get("storage")
                    .or_else(|| slot.get("path"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();

                if dest_dir.is_empty() {
                    return None;
                }

                let name = slot
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("Unnamed download")
                    .to_string();

                let category = slot
                    .get("category")
                    .and_then(Value::as_str)
                    .filter(|c| !c.is_empty() && *c != "*")
                    .map(str::to_string);

                let size_bytes = extract_i64_value(slot.get("bytes"));

                let completed_at =
                    completed_ts.map(|ts| DateTime::from_timestamp(ts, 0).unwrap_or_else(Utc::now));

                Some(CompletedDownload {
                    client_type: "sabnzbd".to_string(),
                    client_id: String::new(),
                    download_client_item_id: nzo_id,
                    name,
                    dest_dir,
                    category,
                    size_bytes,
                    completed_at,
                    parameters: Vec::new(),
                })
            })
            .collect())
    }

    async fn pause_queue_item(&self, id: &str) -> AppResult<()> {
        self.api_get(&[("mode", "queue"), ("name", "pause"), ("value", id)])
            .await?;
        Ok(())
    }

    async fn resume_queue_item(&self, id: &str) -> AppResult<()> {
        self.api_get(&[("mode", "queue"), ("name", "resume"), ("value", id)])
            .await?;
        Ok(())
    }

    async fn delete_queue_item(&self, id: &str, is_history: bool) -> AppResult<()> {
        if is_history {
            self.api_get(&[("mode", "history"), ("name", "delete"), ("value", id)])
                .await?;
        } else {
            self.api_get(&[
                ("mode", "queue"),
                ("name", "delete"),
                ("value", id),
                ("del_files", "1"),
            ])
            .await?;
        }
        Ok(())
    }
}

fn sabnzbd_queue_priority(raw_priority: Option<&str>) -> i32 {
    match raw_priority
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
        .as_deref()
    {
        Some("force") => 2,
        Some("very high") | Some("high") => 1,
        Some("normal") => 0,
        Some("low") | Some("very low") => -1,
        _ => -1,
    }
}

fn sabnzbd_queue_state(status: &str) -> DownloadQueueState {
    let normalized = status.to_ascii_uppercase();
    match normalized.as_str() {
        "DOWNLOADING" => DownloadQueueState::Downloading,
        "QUEUED" | "FETCHING" | "PROPAGATING" | "GRABBING" => DownloadQueueState::Queued,
        "PAUSED" => DownloadQueueState::Paused,
        // Post-processing stages reported in queue (SABnzbd 4.x can show these)
        "VERIFYING" | "QUICKCHECK" => DownloadQueueState::Verifying,
        "REPAIRING" => DownloadQueueState::Repairing,
        "EXTRACTING" => DownloadQueueState::Extracting,
        "MOVING" | "RUNNING" => DownloadQueueState::Downloading,
        _ => DownloadQueueState::Queued,
    }
}

fn sabnzbd_postprocessing_stage(status: &str) -> Option<String> {
    let normalized = status.to_ascii_uppercase();
    match normalized.as_str() {
        "VERIFYING" | "QUICKCHECK" => Some("VERIFYING".to_string()),
        "REPAIRING" => Some("REPAIRING".to_string()),
        "EXTRACTING" => Some("UNPACKING".to_string()),
        "MOVING" => Some("MOVING".to_string()),
        "RUNNING" => Some("EXECUTING_SCRIPT".to_string()),
        _ => None,
    }
}

fn sabnzbd_history_state(status: &str) -> (DownloadQueueState, Option<String>) {
    let normalized = status.to_ascii_uppercase();
    match normalized.as_str() {
        "COMPLETED" => (DownloadQueueState::Completed, None),
        "FAILED" => (DownloadQueueState::Failed, None),
        "QUEUED" => (DownloadQueueState::Queued, None),
        // Active post-processing stages in history
        "VERIFYING" | "QUICKCHECK" => (DownloadQueueState::Verifying, None),
        "REPAIRING" => (DownloadQueueState::Repairing, None),
        "EXTRACTING" => (DownloadQueueState::Extracting, None),
        "MOVING" | "RUNNING" => (DownloadQueueState::Downloading, None),
        _ => {
            if normalized.starts_with("FAILED") {
                let reason = status
                    .split_once(" - ")
                    .map(|(_, detail)| detail.trim().to_string())
                    .filter(|d| !d.is_empty());
                (DownloadQueueState::Failed, reason)
            } else {
                (DownloadQueueState::Completed, None)
            }
        }
    }
}

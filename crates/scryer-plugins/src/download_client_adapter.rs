use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use scryer_application::{
    AppError, AppResult, DownloadClient, DownloadClientAddRequest,
    DownloadClientMarkImportedRequest, DownloadClientStatus, DownloadGrabResult,
};
use scryer_domain::{CompletedDownload, DownloadQueueItem, DownloadQueueState};
use tracing::warn;

use crate::types::{
    PluginCompletedDownload, PluginDescriptor, PluginDownloadClientAddRequest,
    PluginDownloadClientAddResponse, PluginDownloadClientControlRequest,
    PluginDownloadClientMarkImportedRequest, PluginDownloadClientStatus, PluginDownloadItem,
    PluginDownloadRelease, PluginDownloadRouting, PluginDownloadSource, PluginDownloadTitle,
};

pub struct WasmDownloadClient {
    plugin: Arc<Mutex<extism::Plugin>>,
    descriptor: PluginDescriptor,
    client_name: String,
    client_id: String,
}

impl WasmDownloadClient {
    pub fn new(
        plugin: extism::Plugin,
        descriptor: PluginDescriptor,
        client_id: String,
        client_name: String,
    ) -> Self {
        Self {
            plugin: Arc::new(Mutex::new(plugin)),
            descriptor,
            client_name,
            client_id,
        }
    }
}

fn parse_timestamp(raw: Option<String>) -> Option<DateTime<Utc>> {
    raw.and_then(|value| chrono::DateTime::parse_from_rfc3339(&value).ok())
        .map(|value| value.with_timezone(&Utc))
}

fn map_state(raw: &str) -> DownloadQueueState {
    match raw.trim().to_ascii_lowercase().as_str() {
        "queued" => DownloadQueueState::Queued,
        "downloading" => DownloadQueueState::Downloading,
        "verifying" => DownloadQueueState::Verifying,
        "repairing" => DownloadQueueState::Repairing,
        "extracting" => DownloadQueueState::Extracting,
        "paused" => DownloadQueueState::Paused,
        "completed" => DownloadQueueState::Completed,
        "import_pending" => DownloadQueueState::ImportPending,
        "failed" | "error" => DownloadQueueState::Failed,
        "seeding" => DownloadQueueState::Completed,
        _ => DownloadQueueState::Queued,
    }
}

fn attention_required(item: &PluginDownloadItem) -> bool {
    matches!(
        item.state.trim().to_ascii_lowercase().as_str(),
        "failed" | "error" | "warning"
    )
}

fn map_queue_item(
    item: PluginDownloadItem,
    client_id: &str,
    client_name: &str,
    client_type: &str,
) -> DownloadQueueItem {
    let attention = attention_required(&item);
    let attention_reason = item.message.clone();
    DownloadQueueItem {
        id: format!("{client_type}:{}", item.client_item_id),
        title_id: None,
        title_name: item.title,
        facet: None,
        client_id: client_id.to_string(),
        client_name: client_name.to_string(),
        client_type: client_type.to_string(),
        state: map_state(&item.state),
        progress_percent: item.progress_percent.unwrap_or(0),
        size_bytes: item.total_size_bytes,
        remaining_seconds: item.eta_seconds,
        queued_at: None,
        last_updated_at: None,
        attention_required: attention,
        attention_reason,
        download_client_item_id: item.client_item_id,
        import_status: None,
        import_error_message: None,
        imported_at: None,
        is_scryer_origin: false,
    }
}

fn map_completed_download(
    item: PluginCompletedDownload,
    client_id: &str,
    client_type: &str,
) -> CompletedDownload {
    CompletedDownload {
        client_type: client_type.to_string(),
        client_id: client_id.to_string(),
        download_client_item_id: item.client_item_id,
        name: item.name,
        dest_dir: item.dest_dir,
        category: item.category,
        size_bytes: item.size_bytes,
        completed_at: parse_timestamp(item.completed_at),
        parameters: item.parameters,
    }
}

#[async_trait]
impl DownloadClient for WasmDownloadClient {
    async fn submit_download(
        &self,
        request: &DownloadClientAddRequest,
    ) -> AppResult<DownloadGrabResult> {
        let source_hint = request.source_hint.clone();
        let source_kind = source_hint
            .as_deref()
            .map(|value| {
                if value.starts_with("magnet:") {
                    "magnet_uri"
                } else {
                    "torrent_file"
                }
            })
            .unwrap_or("torrent_file")
            .to_string();

        let plugin_request = PluginDownloadClientAddRequest {
            source: PluginDownloadSource {
                kind: source_kind,
                download_url: source_hint.clone(),
                magnet_uri: source_hint
                    .as_ref()
                    .filter(|value| value.starts_with("magnet:"))
                    .cloned(),
                torrent_bytes_base64: None,
                source_title: request.source_title.clone(),
                source_password: request.source_password.clone(),
            },
            release: PluginDownloadRelease {
                release_title: request
                    .release_title
                    .clone()
                    .or_else(|| request.source_title.clone()),
                is_recent: request.is_recent,
                season_pack: request.season_pack,
                indexer_name: request.indexer_name.clone(),
                info_hash_hint: request.info_hash_hint.clone(),
                seed_goal_ratio: request.seed_goal_ratio,
                seed_goal_seconds: request.seed_goal_seconds,
            },
            title: PluginDownloadTitle {
                title_id: Some(request.title.id.clone()),
                title_name: request.title.name.clone(),
                media_facet: match request.title.facet {
                    scryer_domain::MediaFacet::Movie => "movie",
                    scryer_domain::MediaFacet::Tv => "tv",
                    scryer_domain::MediaFacet::Anime => "anime",
                    scryer_domain::MediaFacet::Other => "other",
                }
                .to_string(),
                tags: request.title.tags.clone(),
            },
            routing: PluginDownloadRouting {
                isolation_value: request.category.clone(),
                download_directory: request.download_directory.clone(),
            },
        };

        let input = serde_json::to_string(&plugin_request).map_err(|e| {
            AppError::Repository(format!("failed to serialize plugin request: {e}"))
        })?;

        let plugin_name = self.descriptor.name.clone();
        let plugin = Arc::clone(&self.plugin);
        let output = tokio::task::spawn_blocking(move || {
            let mut guard = plugin
                .lock()
                .map_err(|e| AppError::Repository(format!("plugin mutex poisoned: {e}")))?;
            guard
                .call::<&str, String>("add_download", &input)
                .map_err(|e| AppError::Repository(format!("plugin add_download() failed: {e}")))
        })
        .await
        .map_err(|e| AppError::Repository(format!("plugin task panicked: {e}")))??;

        let response: PluginDownloadClientAddResponse = serde_json::from_str(&output).map_err(|e| {
            warn!(plugin = plugin_name.as_str(), error = %e, "plugin returned invalid add_download response JSON");
            AppError::Repository(format!("plugin returned invalid JSON: {e}"))
        })?;

        Ok(DownloadGrabResult {
            job_id: response.client_item_id,
            client_type: self.descriptor.provider_type.clone(),
        })
    }

    async fn list_queue(&self) -> AppResult<Vec<DownloadQueueItem>> {
        let plugin_name = self.descriptor.name.clone();
        let plugin = Arc::clone(&self.plugin);
        let output = tokio::task::spawn_blocking(move || {
            let mut guard = plugin
                .lock()
                .map_err(|e| AppError::Repository(format!("plugin mutex poisoned: {e}")))?;
            guard
                .call::<(), String>("list_downloads", ())
                .map_err(|e| AppError::Repository(format!("plugin list_downloads() failed: {e}")))
        })
        .await
        .map_err(|e| AppError::Repository(format!("plugin task panicked: {e}")))??;

        let items: Vec<PluginDownloadItem> = serde_json::from_str(&output).map_err(|e| {
            warn!(plugin = plugin_name.as_str(), error = %e, "plugin returned invalid list_downloads JSON");
            AppError::Repository(format!("plugin returned invalid JSON: {e}"))
        })?;

        Ok(items
            .into_iter()
            .filter(|item| {
                !matches!(
                    item.state.trim().to_ascii_lowercase().as_str(),
                    "completed" | "seeding" | "failed" | "error"
                )
            })
            .map(|item| {
                map_queue_item(
                    item,
                    &self.client_id,
                    &self.client_name,
                    &self.descriptor.provider_type,
                )
            })
            .collect())
    }

    async fn list_history(&self) -> AppResult<Vec<DownloadQueueItem>> {
        let plugin_name = self.descriptor.name.clone();
        let plugin = Arc::clone(&self.plugin);
        let output = tokio::task::spawn_blocking(move || {
            let mut guard = plugin
                .lock()
                .map_err(|e| AppError::Repository(format!("plugin mutex poisoned: {e}")))?;
            guard
                .call::<(), String>("list_downloads", ())
                .map_err(|e| AppError::Repository(format!("plugin list_downloads() failed: {e}")))
        })
        .await
        .map_err(|e| AppError::Repository(format!("plugin task panicked: {e}")))??;

        let items: Vec<PluginDownloadItem> = serde_json::from_str(&output).map_err(|e| {
            warn!(plugin = plugin_name.as_str(), error = %e, "plugin returned invalid list_downloads JSON");
            AppError::Repository(format!("plugin returned invalid JSON: {e}"))
        })?;

        Ok(items
            .into_iter()
            .filter(|item| {
                matches!(
                    item.state.trim().to_ascii_lowercase().as_str(),
                    "completed" | "seeding" | "failed" | "error"
                )
            })
            .map(|item| {
                map_queue_item(
                    item,
                    &self.client_id,
                    &self.client_name,
                    &self.descriptor.provider_type,
                )
            })
            .collect())
    }

    async fn list_completed_downloads(&self) -> AppResult<Vec<CompletedDownload>> {
        let plugin_name = self.descriptor.name.clone();
        let plugin = Arc::clone(&self.plugin);
        let output = tokio::task::spawn_blocking(move || {
            let mut guard = plugin
                .lock()
                .map_err(|e| AppError::Repository(format!("plugin mutex poisoned: {e}")))?;
            guard
                .call::<(), String>("list_completed_downloads", ())
                .map_err(|e| {
                    AppError::Repository(format!("plugin list_completed_downloads() failed: {e}"))
                })
        })
        .await
        .map_err(|e| AppError::Repository(format!("plugin task panicked: {e}")))??;

        let items: Vec<PluginCompletedDownload> = serde_json::from_str(&output).map_err(|e| {
            warn!(plugin = plugin_name.as_str(), error = %e, "plugin returned invalid completed download JSON");
            AppError::Repository(format!("plugin returned invalid JSON: {e}"))
        })?;

        Ok(items
            .into_iter()
            .map(|item| {
                map_completed_download(item, &self.client_id, &self.descriptor.provider_type)
            })
            .collect())
    }

    async fn pause_queue_item(&self, id: &str) -> AppResult<()> {
        let request = PluginDownloadClientControlRequest {
            action: "pause".to_string(),
            client_item_id: id.to_string(),
            remove_data: false,
            is_history: false,
        };
        let input = serde_json::to_string(&request).map_err(|e| {
            AppError::Repository(format!("failed to serialize control request: {e}"))
        })?;
        let plugin = Arc::clone(&self.plugin);
        tokio::task::spawn_blocking(move || {
            let mut guard = plugin
                .lock()
                .map_err(|e| AppError::Repository(format!("plugin mutex poisoned: {e}")))?;
            guard
                .call::<&str, String>("control", &input)
                .map_err(|e| AppError::Repository(format!("plugin control() failed: {e}")))
                .map(|_| ())
        })
        .await
        .map_err(|e| AppError::Repository(format!("plugin task panicked: {e}")))?
    }

    async fn resume_queue_item(&self, id: &str) -> AppResult<()> {
        let request = PluginDownloadClientControlRequest {
            action: "resume".to_string(),
            client_item_id: id.to_string(),
            remove_data: false,
            is_history: false,
        };
        let input = serde_json::to_string(&request).map_err(|e| {
            AppError::Repository(format!("failed to serialize control request: {e}"))
        })?;
        let plugin = Arc::clone(&self.plugin);
        tokio::task::spawn_blocking(move || {
            let mut guard = plugin
                .lock()
                .map_err(|e| AppError::Repository(format!("plugin mutex poisoned: {e}")))?;
            guard
                .call::<&str, String>("control", &input)
                .map_err(|e| AppError::Repository(format!("plugin control() failed: {e}")))
                .map(|_| ())
        })
        .await
        .map_err(|e| AppError::Repository(format!("plugin task panicked: {e}")))?
    }

    async fn delete_queue_item(&self, id: &str, is_history: bool) -> AppResult<()> {
        let request = PluginDownloadClientControlRequest {
            action: "remove".to_string(),
            client_item_id: id.to_string(),
            remove_data: false,
            is_history,
        };
        let input = serde_json::to_string(&request).map_err(|e| {
            AppError::Repository(format!("failed to serialize control request: {e}"))
        })?;
        let plugin = Arc::clone(&self.plugin);
        tokio::task::spawn_blocking(move || {
            let mut guard = plugin
                .lock()
                .map_err(|e| AppError::Repository(format!("plugin mutex poisoned: {e}")))?;
            guard
                .call::<&str, String>("control", &input)
                .map_err(|e| AppError::Repository(format!("plugin control() failed: {e}")))
                .map(|_| ())
        })
        .await
        .map_err(|e| AppError::Repository(format!("plugin task panicked: {e}")))?
    }

    async fn mark_imported(&self, request: &DownloadClientMarkImportedRequest) -> AppResult<()> {
        let input = serde_json::to_string(&PluginDownloadClientMarkImportedRequest {
            client_item_id: request.client_item_id.clone(),
            info_hash: request.info_hash.clone(),
            title_id: request.title_id.clone(),
            title_name: request.title_name.clone(),
            category: request.category.clone(),
            imported_path: request.imported_path.clone(),
            download_path: request.download_path.clone(),
        })
        .map_err(|e| {
            AppError::Repository(format!("failed to serialize mark_imported request: {e}"))
        })?;
        let plugin = Arc::clone(&self.plugin);
        tokio::task::spawn_blocking(move || {
            let mut guard = plugin
                .lock()
                .map_err(|e| AppError::Repository(format!("plugin mutex poisoned: {e}")))?;
            guard
                .call::<&str, String>("mark_imported", &input)
                .map_err(|e| AppError::Repository(format!("plugin mark_imported() failed: {e}")))
                .map(|_| ())
        })
        .await
        .map_err(|e| AppError::Repository(format!("plugin task panicked: {e}")))?
    }

    async fn get_client_status(&self) -> AppResult<DownloadClientStatus> {
        let plugin_name = self.descriptor.name.clone();
        let plugin = Arc::clone(&self.plugin);
        let output = tokio::task::spawn_blocking(move || {
            let mut guard = plugin
                .lock()
                .map_err(|e| AppError::Repository(format!("plugin mutex poisoned: {e}")))?;
            guard
                .call::<(), String>("get_client_status", ())
                .map_err(|e| {
                    AppError::Repository(format!("plugin get_client_status() failed: {e}"))
                })
        })
        .await
        .map_err(|e| AppError::Repository(format!("plugin task panicked: {e}")))??;

        let status: PluginDownloadClientStatus = serde_json::from_str(&output).map_err(|e| {
            warn!(plugin = plugin_name.as_str(), error = %e, "plugin returned invalid client status JSON");
            AppError::Repository(format!("plugin returned invalid JSON: {e}"))
        })?;

        Ok(DownloadClientStatus {
            version: status.version,
            is_localhost: status.is_localhost,
            remote_output_roots: status.remote_output_roots,
            removes_completed_downloads: status.removes_completed_downloads,
            sorting_mode: status.sorting_mode,
            warnings: status.warnings,
        })
    }

    async fn test_connection(&self) -> AppResult<String> {
        let plugin = Arc::clone(&self.plugin);
        tokio::task::spawn_blocking(move || {
            let mut guard = plugin
                .lock()
                .map_err(|e| AppError::Repository(format!("plugin mutex poisoned: {e}")))?;
            guard
                .call::<(), String>("test_connection", ())
                .map_err(|e| AppError::Repository(format!("plugin test_connection() failed: {e}")))
        })
        .await
        .map_err(|e| AppError::Repository(format!("plugin task panicked: {e}")))?
    }
}

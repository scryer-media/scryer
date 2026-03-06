use std::sync::Arc;

use async_trait::async_trait;
use scryer_application::{AppError, AppResult, DownloadClient, DownloadClientConfigRepository};
use scryer_domain::{DownloadClientConfig, DownloadQueueItem, Title};
use tracing::warn;

use super::nzbget::NzbgetDownloadClient;
use super::sabnzbd::SabnzbdDownloadClient;
use super::{parse_download_client_config_json, read_config_string, resolve_download_client_base_url};

#[derive(Clone)]
pub struct PrioritizedDownloadClientRouter {
    download_client_configs: Arc<dyn DownloadClientConfigRepository>,
    fallback_client: Arc<dyn DownloadClient>,
}

impl PrioritizedDownloadClientRouter {
    pub fn new(
        download_client_configs: Arc<dyn DownloadClientConfigRepository>,
        fallback_client: Arc<dyn DownloadClient>,
    ) -> Self {
        Self {
            download_client_configs,
            fallback_client,
        }
    }

    async fn list_enabled_clients_by_priority(&self) -> AppResult<Vec<DownloadClientConfig>> {
        let mut clients = self
            .download_client_configs
            .list(None)
            .await?
            .into_iter()
            .filter(|config| config.is_enabled)
            .collect::<Vec<_>>();
        clients.sort_by_key(|config| config.client_priority);
        Ok(clients)
    }

    fn client_from_config(config: &DownloadClientConfig) -> AppResult<Arc<dyn DownloadClient>> {
        let client_type = config.client_type.trim().to_ascii_lowercase();
        match client_type.as_str() {
            "nzbget" => {
                let parsed_config = parse_download_client_config_json(&config.config_json)?;
                let base_url = resolve_download_client_base_url(config, &parsed_config)
                    .ok_or_else(|| {
                        AppError::Validation(format!(
                            "download client {} has no valid base URL",
                            config.id
                        ))
                    })?;
                let username = read_config_string(&parsed_config, &["username"]);
                let password = read_config_string(&parsed_config, &["password"]);
                let dupe_mode = read_config_string(&parsed_config, &["dupe_mode", "dupeMode"])
                    .unwrap_or_else(|| "SCORE".to_string());
                let client = NzbgetDownloadClient::new(base_url, username, password, dupe_mode);
                Ok(Arc::new(client))
            }
            "sabnzbd" => {
                let parsed_config = parse_download_client_config_json(&config.config_json)?;
                let base_url = resolve_download_client_base_url(config, &parsed_config)
                    .ok_or_else(|| {
                        AppError::Validation(format!(
                            "download client {} has no valid base URL",
                            config.id
                        ))
                    })?;
                let api_key = read_config_string(&parsed_config, &["api_key", "apiKey", "apikey"])
                    .ok_or_else(|| {
                        AppError::Validation(format!(
                            "download client {} (sabnzbd) requires an API key",
                            config.id
                        ))
                    })?;
                let client = SabnzbdDownloadClient::new(base_url, api_key);
                Ok(Arc::new(client))
            }
            _ => Err(AppError::Validation(format!(
                "unsupported download client type '{}' for config {}",
                config.client_type, config.id
            ))),
        }
    }
}

#[async_trait]
impl DownloadClient for PrioritizedDownloadClientRouter {
    async fn submit_to_download_queue(
        &self,
        title: &Title,
        source_hint: Option<String>,
        source_title: Option<String>,
        source_password: Option<String>,
        category: Option<String>,
    ) -> AppResult<String> {
        let clients = match self.list_enabled_clients_by_priority().await {
            Ok(configs) => configs,
            Err(error) => {
                warn!(
                    error = %error,
                    title = title.name.as_str(),
                    "failed to load prioritized download clients; falling back to default client"
                );
                return self
                    .fallback_client
                    .submit_to_download_queue(
                        title,
                        source_hint,
                        source_title,
                        source_password,
                        category,
                    )
                    .await;
            }
        };

        if clients.is_empty() {
            return self
                .fallback_client
                .submit_to_download_queue(
                    title,
                    source_hint,
                    source_title,
                    source_password,
                    category,
                )
                .await;
        }

        let mut last_error: Option<AppError> = None;
        for config in clients {
            let client = match Self::client_from_config(&config) {
                Ok(client) => client,
                Err(error) => {
                    warn!(
                        client_id = config.id.as_str(),
                        client_name = config.name.as_str(),
                        client_type = config.client_type.as_str(),
                        error = %error,
                        "download client skipped due to invalid configuration"
                    );
                    last_error = Some(error);
                    continue;
                }
            };

            match client
                .submit_to_download_queue(
                    title,
                    source_hint.clone(),
                    source_title.clone(),
                    source_password.clone(),
                    category.clone(),
                )
                .await
            {
                Ok(job_id) => return Ok(job_id),
                Err(error) => {
                    let should_failover = matches!(error, AppError::Repository(_));
                    warn!(
                        client_id = config.id.as_str(),
                        client_name = config.name.as_str(),
                        client_type = config.client_type.as_str(),
                        error = %error,
                        failover = should_failover,
                        "download client enqueue failed"
                    );
                    if should_failover {
                        last_error = Some(error);
                        continue;
                    }
                    return Err(error);
                }
            }
        }

        Err(last_error.unwrap_or_else(|| {
            AppError::Repository(
                "all prioritized download clients failed to enqueue this release".to_string(),
            )
        }))
    }

    async fn list_queue(&self) -> AppResult<Vec<DownloadQueueItem>> {
        let clients = self.list_enabled_clients_by_priority().await?;
        let mut last_error: Option<AppError> = None;
        for config in clients {
            let client = match Self::client_from_config(&config) {
                Ok(client) => client,
                Err(error) => {
                    last_error = Some(error);
                    continue;
                }
            };
            match client.list_queue().await {
                Ok(items) => return Ok(items),
                Err(error) => {
                    last_error = Some(error);
                }
            }
        }
        if let Some(error) = last_error {
            return Err(error);
        }
        self.fallback_client.list_queue().await
    }

    async fn list_history(&self) -> AppResult<Vec<DownloadQueueItem>> {
        let clients = self.list_enabled_clients_by_priority().await?;
        let mut last_error: Option<AppError> = None;
        for config in clients {
            let client = match Self::client_from_config(&config) {
                Ok(client) => client,
                Err(error) => {
                    last_error = Some(error);
                    continue;
                }
            };
            match client.list_history().await {
                Ok(items) => return Ok(items),
                Err(error) => {
                    last_error = Some(error);
                }
            }
        }
        if let Some(error) = last_error {
            return Err(error);
        }
        self.fallback_client.list_history().await
    }

    async fn list_completed_downloads(&self) -> AppResult<Vec<scryer_domain::CompletedDownload>> {
        let clients = self.list_enabled_clients_by_priority().await?;
        let mut last_error: Option<AppError> = None;
        for config in clients {
            let client = match Self::client_from_config(&config) {
                Ok(client) => client,
                Err(error) => {
                    last_error = Some(error);
                    continue;
                }
            };
            match client.list_completed_downloads().await {
                Ok(mut items) => {
                    for item in &mut items {
                        item.client_id = config.id.clone();
                    }
                    return Ok(items);
                }
                Err(error) => {
                    last_error = Some(error);
                }
            }
        }
        if let Some(error) = last_error {
            return Err(error);
        }
        self.fallback_client.list_completed_downloads().await
    }

    async fn pause_queue_item(&self, id: &str) -> AppResult<()> {
        let clients = self.list_enabled_clients_by_priority().await?;
        for config in clients {
            if let Ok(client) = Self::client_from_config(&config) {
                return client.pause_queue_item(id).await;
            }
        }
        self.fallback_client.pause_queue_item(id).await
    }

    async fn resume_queue_item(&self, id: &str) -> AppResult<()> {
        let clients = self.list_enabled_clients_by_priority().await?;
        for config in clients {
            if let Ok(client) = Self::client_from_config(&config) {
                return client.resume_queue_item(id).await;
            }
        }
        self.fallback_client.resume_queue_item(id).await
    }

    async fn delete_queue_item(&self, id: &str, is_history: bool) -> AppResult<()> {
        let clients = self.list_enabled_clients_by_priority().await?;
        for config in clients {
            if let Ok(client) = Self::client_from_config(&config) {
                return client.delete_queue_item(id, is_history).await;
            }
        }
        self.fallback_client.delete_queue_item(id, is_history).await
    }
}

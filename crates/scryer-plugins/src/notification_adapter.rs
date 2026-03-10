use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use scryer_application::{AppError, AppResult, NotificationClient};
use tracing::warn;

use crate::types::{PluginDescriptor, PluginNotificationRequest, PluginNotificationResponse};

pub struct WasmNotificationClient {
    plugin: Arc<Mutex<extism::Plugin>>,
    descriptor: PluginDescriptor,
    channel_name: String,
}

impl WasmNotificationClient {
    pub fn new(plugin: extism::Plugin, descriptor: PluginDescriptor, channel_name: String) -> Self {
        Self {
            plugin: Arc::new(Mutex::new(plugin)),
            descriptor,
            channel_name,
        }
    }
}

#[async_trait]
impl NotificationClient for WasmNotificationClient {
    async fn send_notification(
        &self,
        event_type: &str,
        title: &str,
        message: &str,
        metadata: &std::collections::HashMap<String, serde_json::Value>,
    ) -> AppResult<()> {
        let request = PluginNotificationRequest {
            event_type: event_type.to_string(),
            title: title.to_string(),
            message: message.to_string(),
            title_name: metadata
                .get("title_name")
                .and_then(|v| v.as_str())
                .map(String::from),
            title_year: metadata
                .get("title_year")
                .and_then(|v| v.as_i64())
                .map(|v| v as i32),
            title_facet: metadata
                .get("title_facet")
                .and_then(|v| v.as_str())
                .map(String::from),
            poster_url: metadata
                .get("poster_url")
                .and_then(|v| v.as_str())
                .map(String::from),
            episode_info: metadata
                .get("episode_info")
                .and_then(|v| v.as_str())
                .map(String::from),
            quality: metadata
                .get("quality")
                .and_then(|v| v.as_str())
                .map(String::from),
            release_title: metadata
                .get("release_title")
                .and_then(|v| v.as_str())
                .map(String::from),
            download_client: metadata
                .get("download_client")
                .and_then(|v| v.as_str())
                .map(String::from),
            file_path: metadata
                .get("file_path")
                .and_then(|v| v.as_str())
                .map(String::from),
            health_message: metadata
                .get("health_message")
                .and_then(|v| v.as_str())
                .map(String::from),
            application_version: metadata
                .get("application_version")
                .and_then(|v| v.as_str())
                .map(String::from),
            metadata: metadata.clone(),
        };

        let input = serde_json::to_string(&request).map_err(|e| {
            AppError::Repository(format!("failed to serialize notification request: {e}"))
        })?;

        let plugin_name = self.descriptor.name.clone();
        let channel_name = self.channel_name.clone();

        let plugin = Arc::clone(&self.plugin);
        let output = tokio::task::spawn_blocking(move || {
            let mut guard = plugin
                .lock()
                .map_err(|e| AppError::Repository(format!("plugin mutex poisoned: {e}")))?;
            guard
                .call::<&str, String>("send_notification", &input)
                .map_err(|e| {
                    AppError::Repository(format!("plugin send_notification() failed: {e}"))
                })
        })
        .await
        .map_err(|e| AppError::Repository(format!("notification plugin task panicked: {e}")))??;

        let response: PluginNotificationResponse = serde_json::from_str(&output).map_err(|e| {
            warn!(
                plugin = plugin_name.as_str(),
                channel = channel_name.as_str(),
                error = %e,
                "notification plugin returned invalid response JSON"
            );
            AppError::Repository(format!("notification plugin returned invalid JSON: {e}"))
        })?;

        if !response.success {
            let err_msg = response
                .error
                .unwrap_or_else(|| "unknown error".to_string());
            warn!(
                plugin = plugin_name.as_str(),
                channel = channel_name.as_str(),
                error = err_msg.as_str(),
                "notification plugin reported failure"
            );
            return Err(AppError::Repository(format!(
                "notification failed: {err_msg}"
            )));
        }

        Ok(())
    }
}

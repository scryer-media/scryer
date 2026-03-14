use std::collections::HashMap;
use std::time::Duration;

use reqwest::Client;
use scryer_application::{AppError, AppResult};
use serde_json::Value;

/// Root folder discovered from a Sonarr/Radarr instance.
#[derive(Debug, Clone)]
pub struct ArrRootFolder {
    pub id: i64,
    pub path: String,
}

/// Download client discovered from a Sonarr/Radarr instance.
#[derive(Debug, Clone)]
pub struct ArrDownloadClient {
    pub id: i64,
    pub name: String,
    pub implementation: String,
    pub fields: HashMap<String, Value>,
}

/// Indexer discovered from a Sonarr/Radarr instance.
#[derive(Debug, Clone)]
pub struct ArrIndexer {
    pub id: i64,
    pub name: String,
    pub implementation: String,
    pub fields: HashMap<String, Value>,
}

/// HTTP client for Sonarr/Radarr v3 API.
#[derive(Clone)]
pub struct ExternalArrClient {
    base_url: String,
    api_key: String,
    http_client: Client,
}

impl ExternalArrClient {
    pub fn new(base_url: String, api_key: String) -> Self {
        let http_client = Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .unwrap_or_else(|_| Client::new());
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
            http_client,
        }
    }

    /// Test connectivity and return (app_name, version).
    pub async fn test_connection(&self) -> AppResult<(String, String)> {
        let json = self.api_get("system/status").await?;
        let app_name = json
            .get("appName")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let version = json
            .get("version")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        Ok((app_name, version))
    }

    /// Fetch root folders (media library paths).
    pub async fn list_root_folders(&self) -> AppResult<Vec<ArrRootFolder>> {
        let json = self.api_get("rootfolder").await?;
        let arr = json
            .as_array()
            .ok_or_else(|| AppError::Repository("rootfolder response was not an array".into()))?;
        Ok(arr
            .iter()
            .filter_map(|item| {
                let id = item.get("id")?.as_i64()?;
                let path = item.get("path")?.as_str()?.to_string();
                if path.is_empty() {
                    return None;
                }
                Some(ArrRootFolder { id, path })
            })
            .collect())
    }

    /// Fetch download client configurations.
    ///
    /// Fetches the list first, then re-fetches each client individually because
    /// Sonarr v4+ / Radarr v5+ mask sensitive field values (e.g. `apiKey`) in the
    /// list endpoint response.
    pub async fn list_download_clients(&self) -> AppResult<Vec<ArrDownloadClient>> {
        let json = self.api_get("downloadclient").await?;
        let arr = json.as_array().ok_or_else(|| {
            AppError::Repository("downloadclient response was not an array".into())
        })?;

        let mut results = Vec::new();
        for item in arr {
            let id = match item.get("id").and_then(Value::as_i64) {
                Some(id) => id,
                None => continue,
            };
            let name = match item.get("name").and_then(Value::as_str) {
                Some(n) => n.to_string(),
                None => continue,
            };
            let implementation = match item.get("implementation").and_then(Value::as_str) {
                Some(i) => i.to_string(),
                None => continue,
            };

            // Re-fetch individually to get unmasked sensitive fields.
            let fields = match self.api_get(&format!("downloadclient/{id}")).await {
                Ok(detail) => detail
                    .get("fields")
                    .and_then(Value::as_array)
                    .map(|f| flatten_arr_fields(f))
                    .unwrap_or_default(),
                Err(_) => item
                    .get("fields")
                    .and_then(Value::as_array)
                    .map(|f| flatten_arr_fields(f))
                    .unwrap_or_default(),
            };

            results.push(ArrDownloadClient {
                id,
                name,
                implementation,
                fields,
            });
        }
        Ok(results)
    }

    /// Fetch indexer configurations.
    ///
    /// Like `list_download_clients`, re-fetches each indexer individually to get
    /// unmasked sensitive fields (e.g. `apiKey`).
    pub async fn list_indexers(&self) -> AppResult<Vec<ArrIndexer>> {
        let json = self.api_get("indexer").await?;
        let arr = json
            .as_array()
            .ok_or_else(|| AppError::Repository("indexer response was not an array".into()))?;

        let mut results = Vec::new();
        for item in arr {
            let id = match item.get("id").and_then(Value::as_i64) {
                Some(id) => id,
                None => continue,
            };
            let name = match item.get("name").and_then(Value::as_str) {
                Some(n) => n.to_string(),
                None => continue,
            };
            let implementation = match item.get("implementation").and_then(Value::as_str) {
                Some(i) => i.to_string(),
                None => continue,
            };

            // Re-fetch individually to get unmasked sensitive fields.
            let fields = match self.api_get(&format!("indexer/{id}")).await {
                Ok(detail) => detail
                    .get("fields")
                    .and_then(Value::as_array)
                    .map(|f| flatten_arr_fields(f))
                    .unwrap_or_default(),
                Err(_) => item
                    .get("fields")
                    .and_then(Value::as_array)
                    .map(|f| flatten_arr_fields(f))
                    .unwrap_or_default(),
            };

            results.push(ArrIndexer {
                id,
                name,
                implementation,
                fields,
            });
        }
        Ok(results)
    }

    async fn api_get(&self, path: &str) -> AppResult<Value> {
        let url = format!("{}/api/v3/{}", self.base_url, path);
        let response = self
            .http_client
            .get(&url)
            .header("X-Api-Key", &self.api_key)
            .send()
            .await
            .map_err(|err| {
                AppError::Repository(format!("external api call to {path} failed: {err}"))
            })?;

        let status = response.status();
        let body = response.text().await.map_err(|err| {
            AppError::Repository(format!("external api response read failed: {err}"))
        })?;

        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(AppError::Repository("invalid API key".into()));
        }
        if !status.is_success() {
            let preview = body.chars().take(400).collect::<String>();
            return Err(AppError::Repository(format!(
                "external api returned status {status}: {preview}"
            )));
        }

        serde_json::from_str(&body)
            .map_err(|err| AppError::Repository(format!("external api returned non-json: {err}")))
    }
}

/// Map the Sonarr/Radarr implementation name to a Scryer download client type.
pub fn map_download_client_type(implementation: &str) -> Option<&'static str> {
    match implementation {
        "Nzbget" => Some("nzbget"),
        "Sabnzbd" => Some("sabnzbd"),
        // QBittorrent: mapped but not yet implemented in Scryer
        _ => None,
    }
}

/// Map the Sonarr/Radarr indexer implementation to a Scryer provider type.
///
/// For Newznab indexers, checks the base URL to identify known services that have
/// native Scryer plugins (e.g. NZBGeek, AnimeTosho) rather than falling back to
/// the generic newznab plugin.
pub fn map_indexer_provider_type(
    implementation: &str,
    fields: &HashMap<String, Value>,
) -> Option<&'static str> {
    match implementation {
        "Newznab" => {
            let base_url = field_str(fields, "baseUrl")
                .unwrap_or_default()
                .to_lowercase();
            if base_url.contains("nzbgeek.info") {
                Some("nzbgeek")
            } else if base_url.contains("animetosho.org") {
                Some("animetosho")
            } else {
                Some("newznab")
            }
        }
        _ => None,
    }
}

/// Extract a string value from the flattened fields map.
pub fn field_str(fields: &HashMap<String, Value>, key: &str) -> Option<String> {
    fields.get(key).and_then(|v| match v {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        _ => None,
    })
}

/// Extract a string from the fields map, falling back to empty string for numeric values.
pub fn field_str_or_number(fields: &HashMap<String, Value>, key: &str) -> Option<String> {
    fields.get(key).and_then(|v| match v {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    })
}

/// Extract a boolean from the fields map.
pub fn field_bool(fields: &HashMap<String, Value>, key: &str) -> Option<bool> {
    fields.get(key).and_then(|v| match v {
        Value::Bool(b) => Some(*b),
        Value::String(s) => match s.to_ascii_lowercase().as_str() {
            "true" | "1" => Some(true),
            "false" | "0" => Some(false),
            _ => None,
        },
        _ => None,
    })
}

fn flatten_arr_fields(fields: &[Value]) -> HashMap<String, Value> {
    fields
        .iter()
        .filter_map(|f| {
            let name = f.get("name")?.as_str()?.to_string();
            let value = f.get("value")?.clone();
            Some((name, value))
        })
        .collect()
}

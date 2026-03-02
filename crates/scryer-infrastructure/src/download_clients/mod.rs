mod multi_indexer;
mod nzbget;
mod nzbgeek;
mod router;

use scryer_application::{AppError, AppResult};
use scryer_domain::DownloadClientConfig;
use serde_json::{json, Value};

pub use multi_indexer::MultiIndexerSearchClient;
pub use nzbgeek::{
    NzbGeekSearchClient, NZBGEEK_BASE_BACKOFF_SECONDS, NZBGEEK_MAX_BACKOFF_SECONDS,
    NZBGEEK_MIN_REQUEST_INTERVAL_MS,
};
pub(crate) use nzbgeek::parse_retry_after;
pub use nzbget::NzbgetDownloadClient;
pub use router::PrioritizedDownloadClientRouter;

fn parse_download_client_config_json(raw: &str) -> AppResult<Value> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_str::<Value>(trimmed).map_err(|error| {
        AppError::Validation(format!("invalid download client config JSON: {error}"))
    })
}

fn read_config_string(config: &Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(value) = config.get(*key).and_then(Value::as_str) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn read_config_bool(config: &Value, keys: &[&str], default_value: bool) -> bool {
    for key in keys {
        if let Some(value) = config.get(*key) {
            if let Some(bool_value) = value.as_bool() {
                return bool_value;
            }
            if let Some(raw_string) = value.as_str() {
                let normalized = raw_string.trim().to_ascii_lowercase();
                if normalized == "true" || normalized == "1" || normalized == "yes" {
                    return true;
                }
                if normalized == "false" || normalized == "0" || normalized == "no" {
                    return false;
                }
            }
        }
    }
    default_value
}

fn resolve_download_client_base_url(config: &DownloadClientConfig, json_config: &Value) -> Option<String> {
    if let Some(value) = config.base_url.as_deref() {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    let host = read_config_string(json_config, &["host"])?;
    let port = read_config_string(json_config, &["port"]);
    let use_ssl = read_config_bool(json_config, &["use_ssl", "useSsl"], false);
    let url_base = read_config_string(json_config, &["url_base", "urlBase"]);

    let mut value = String::new();
    if use_ssl {
        value.push_str("https://");
    } else {
        value.push_str("http://");
    }
    value.push_str(host.trim());

    if let Some(port_value) = port {
        let port_trimmed = port_value.trim();
        if !port_trimmed.is_empty() {
            value.push(':');
            value.push_str(port_trimmed);
        }
    }

    if let Some(path_value) = url_base {
        let normalized_path = path_value.trim().trim_start_matches('/');
        if !normalized_path.is_empty() {
            value.push('/');
            value.push_str(normalized_path);
        }
    }

    Some(value)
}

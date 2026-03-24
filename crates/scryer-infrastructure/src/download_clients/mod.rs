mod multi_indexer;
mod nzbget;
mod router;
mod sabnzbd;
pub(crate) mod weaver;
pub mod weaver_subscription;

use scryer_application::{AppError, AppResult};
use serde_json::{Value, json};

pub use multi_indexer::MultiIndexerSearchClient;
pub use nzbget::NzbgetDownloadClient;
pub use router::PrioritizedDownloadClientRouter;
pub use sabnzbd::SabnzbdDownloadClient;
pub use weaver::WeaverDownloadClient;
pub use weaver_subscription::start_weaver_subscription_bridge;

/// Compute a base URL from host/port/use_ssl/url_base in a config_json string.
/// Public for use by the GraphQL mapper layer.
pub fn resolve_base_url_from_config_json(config_json: &str) -> Option<String> {
    let parsed = parse_download_client_config_json(config_json).ok()?;
    resolve_download_client_base_url(&parsed)
}

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

/// Build a base URL from config_json component parts (host, port, use_ssl, url_base).
pub fn resolve_download_client_base_url(json_config: &Value) -> Option<String> {
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
    value.push_str(&host);

    if let Some(port_value) = port
        && !port_value.is_empty()
    {
        value.push(':');
        value.push_str(&port_value);
    }

    if let Some(path_value) = url_base {
        let normalized_path = path_value.trim_start_matches('/');
        if !normalized_path.is_empty() {
            value.push('/');
            value.push_str(normalized_path);
        }
    }

    Some(value)
}

// ---------------------------------------------------------------------------
// Shared helpers used by multiple download client implementations
// ---------------------------------------------------------------------------

pub(crate) fn extract_i64_value(value: Option<&Value>) -> Option<i64> {
    value.and_then(|value| {
        value.as_i64().or_else(|| {
            value
                .as_str()
                .and_then(|raw| raw.trim().parse::<i64>().ok())
        })
    })
}

pub(crate) fn extract_f64_value(value: Option<&Value>) -> Option<f64> {
    value.and_then(|value| {
        value.as_f64().or_else(|| {
            value
                .as_str()
                .and_then(|raw| raw.trim().parse::<f64>().ok())
        })
    })
}

pub(crate) fn size_to_bytes(size_mb: f64) -> Option<i64> {
    if !size_mb.is_finite() {
        return None;
    }
    if size_mb <= 0.0 {
        return Some(0);
    }
    let bytes = (size_mb * 1_048_576f64).round() as i64;
    Some(bytes.max(0))
}

pub(crate) fn progress_percent_from_sizes(size_mb: f64, remaining_mb: f64) -> u8 {
    if size_mb <= 0.0 || !size_mb.is_finite() || !remaining_mb.is_finite() {
        return 0;
    }

    let completed_mb = (size_mb - remaining_mb).clamp(0.0, size_mb);
    if completed_mb <= 0.0 {
        return 0;
    }

    let percent = ((completed_mb / size_mb) * 100.0).round();
    let clamped = if percent.is_nan() {
        0.0
    } else {
        percent.clamp(0.0, 100.0)
    };
    clamped as u8
}

pub(crate) fn parse_duration_seconds(raw: &str) -> Option<i64> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Ok(seconds) = trimmed.parse::<i64>() {
        return Some(seconds.max(0));
    }

    let mut parts = trimmed.split(':');
    let first = parts.next()?;
    let second = parts.next()?;
    let third = parts.next();
    if parts.next().is_some() {
        return None;
    }

    let (hours, minutes, seconds) = if let Some(third_part) = third {
        let hours = first.parse::<i64>().ok()?;
        let minutes = second.parse::<i64>().ok()?;
        let seconds = third_part.parse::<i64>().ok()?;
        (hours, minutes, seconds)
    } else {
        let minutes = first.parse::<i64>().ok()?;
        let seconds = second.parse::<i64>().ok()?;
        (0, minutes, seconds)
    };

    if hours < 0 || minutes < 0 || seconds < 0 || minutes >= 60 || seconds >= 60 {
        return None;
    }

    Some(hours * 3600 + minutes * 60 + seconds)
}

pub(crate) fn is_http_url(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("https://")
}

/// Fetch an NZB file from a URL, validate it looks like XML, and return the raw bytes.
pub(crate) async fn fetch_nzb_bytes(client: &reqwest::Client, url: &str) -> AppResult<Vec<u8>> {
    let response = client
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

    Ok(bytes.to_vec())
}

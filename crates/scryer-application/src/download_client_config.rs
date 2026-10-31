use serde_json::{Value, json};

use crate::{AppError, AppResult};

/// Compute a base URL from host/port/use_ssl/url_base in a config_json string.
pub fn resolve_download_client_base_url_from_config_json(config_json: &str) -> Option<String> {
    let parsed = parse_download_client_config_json(config_json).ok()?;
    resolve_download_client_base_url(&parsed)
}

fn parse_download_client_config_json(raw: &str) -> AppResult<Value> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_str::<Value>(trimmed).map_err(|error| {
        AppError::validation_refused(
            crate::error_reasons::download_client_config::JSON_INVALID,
            format!("invalid download client config JSON: {error}"),
        )
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

fn resolve_download_client_base_url(json_config: &Value) -> Option<String> {
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

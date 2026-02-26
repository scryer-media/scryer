use scryer_application::AppUseCase;
use scryer_domain::{NewDownloadClientConfig, NewIndexerConfig};
use scryer_infrastructure::SqliteServices;
use serde::Deserialize;

use crate::admin_routes::normalize_base_url;

#[derive(Deserialize)]
struct SeedConfig {
    admin: Option<SeedAdmin>,
    #[serde(default)]
    indexers: Vec<SeedIndexer>,
    #[serde(default)]
    download_clients: Vec<SeedDownloadClient>,
    #[serde(default)]
    settings: Vec<SeedSetting>,
}

#[derive(Deserialize)]
struct SeedAdmin {
    password: String,
}

#[derive(Deserialize)]
struct SeedIndexer {
    name: String,
    provider_type: String,
    base_url: String,
    api_key: Option<String>,
    rate_limit_seconds: Option<i64>,
    rate_limit_burst: Option<i64>,
    #[serde(default = "default_true")]
    enabled: bool,
}

#[derive(Deserialize)]
struct SeedDownloadClient {
    name: String,
    client_type: String,
    base_url: Option<String>,
    config: toml::Value,
    #[serde(default = "default_true")]
    enabled: bool,
}

#[derive(Deserialize)]
struct SeedSetting {
    scope: String,
    key: String,
    value: toml::Value,
    scope_id: Option<String>,
}

fn default_true() -> bool {
    true
}

/// Convert a TOML value to a JSON string for storage.
fn toml_value_to_json_string(value: &toml::Value) -> String {
    match value {
        toml::Value::String(s) => serde_json::to_string(s).unwrap(),
        toml::Value::Integer(n) => serde_json::to_string(n).unwrap(),
        toml::Value::Float(f) => serde_json::to_string(f).unwrap(),
        toml::Value::Boolean(b) => serde_json::to_string(b).unwrap(),
        toml::Value::Array(arr) => {
            let json_values: Vec<serde_json::Value> = arr
                .iter()
                .map(|v| serde_json::from_str(&toml_value_to_json_string(v)).unwrap())
                .collect();
            serde_json::to_string(&json_values).unwrap()
        }
        toml::Value::Table(table) => {
            let mut map = serde_json::Map::new();
            for (k, v) in table {
                let json_val: serde_json::Value =
                    serde_json::from_str(&toml_value_to_json_string(v)).unwrap();
                map.insert(k.clone(), json_val);
            }
            serde_json::to_string(&serde_json::Value::Object(map)).unwrap()
        }
        toml::Value::Datetime(dt) => serde_json::to_string(&dt.to_string()).unwrap(),
    }
}

pub(crate) async fn apply_dev_seed(
    app: &AppUseCase,
    db: &SqliteServices,
) -> Result<(), String> {
    let seed_path = match std::env::var("SCRYER_DEV_SEED_FILE") {
        Ok(path) if !path.trim().is_empty() => path.trim().to_string(),
        _ => return Ok(()),
    };

    let contents = std::fs::read_to_string(&seed_path)
        .map_err(|e| format!("failed to read seed file {seed_path}: {e}"))?;

    let config: SeedConfig = toml::from_str(&contents)
        .map_err(|e| format!("failed to parse seed file {seed_path}: {e}"))?;

    let actor = app
        .find_or_create_default_user()
        .await
        .map_err(|e| format!("failed to get admin user for seeding: {e}"))?;

    // Admin password
    if let Some(admin) = &config.admin {
        if !admin.password.is_empty() {
            match app
                .bootstrap_user_password(&actor.id, &admin.password)
                .await
            {
                Ok(_) => tracing::info!("dev seed: admin password set"),
                Err(e) => tracing::warn!(error = %e, "dev seed: failed to set admin password"),
            }
        }
    }

    // Indexers
    for seed_indexer in &config.indexers {
        let provider_type = seed_indexer.provider_type.to_lowercase();
        let normalized_base = normalize_base_url(&seed_indexer.base_url);

        let existing = app
            .list_indexer_configs(&actor, Some(provider_type.clone()))
            .await
            .map_err(|e| format!("dev seed: failed to list indexer configs: {e}"))?;

        let already_exists = existing.iter().any(|c| {
            c.provider_type.eq_ignore_ascii_case(&provider_type)
                && normalize_base_url(&c.base_url) == normalized_base
        });

        if already_exists {
            tracing::info!(
                name = %seed_indexer.name,
                provider = %provider_type,
                "dev seed: indexer already exists, skipping"
            );
            continue;
        }

        let input = NewIndexerConfig {
            name: seed_indexer.name.clone(),
            provider_type,
            base_url: seed_indexer.base_url.trim_end_matches('/').to_string(),
            api_key_encrypted: seed_indexer.api_key.clone(),
            rate_limit_seconds: seed_indexer.rate_limit_seconds,
            rate_limit_burst: seed_indexer.rate_limit_burst,
            is_enabled: seed_indexer.enabled,
            enable_interactive_search: true,
            enable_auto_search: true,
        };

        app.create_indexer_config(&actor, input)
            .await
            .map_err(|e| format!("dev seed: failed to create indexer '{}': {e}", seed_indexer.name))?;

        tracing::info!(name = %seed_indexer.name, "dev seed: created indexer");
    }

    // Download clients
    for seed_client in &config.download_clients {
        let existing = app
            .list_download_client_configs(&actor, None)
            .await
            .map_err(|e| format!("dev seed: failed to list download clients: {e}"))?;

        let already_exists = existing
            .iter()
            .any(|c| c.name.eq_ignore_ascii_case(&seed_client.name));

        if already_exists {
            tracing::info!(
                name = %seed_client.name,
                "dev seed: download client already exists, skipping"
            );
            continue;
        }

        let config_json = toml_value_to_json_string(&seed_client.config);

        let input = NewDownloadClientConfig {
            name: seed_client.name.clone(),
            client_type: seed_client.client_type.clone(),
            base_url: seed_client.base_url.clone(),
            config_json,
            client_priority: 0, // auto-calculated by create_download_client_config
            is_enabled: seed_client.enabled,
        };

        app.create_download_client_config(&actor, input)
            .await
            .map_err(|e| {
                format!(
                    "dev seed: failed to create download client '{}': {e}",
                    seed_client.name
                )
            })?;

        tracing::info!(name = %seed_client.name, "dev seed: created download client");
    }

    // Settings
    for seed_setting in &config.settings {
        let value_json = toml_value_to_json_string(&seed_setting.value);

        db.upsert_setting_value(
            &seed_setting.scope,
            &seed_setting.key,
            seed_setting.scope_id.clone(),
            value_json,
            "dev-seed",
            None,
        )
        .await
        .map_err(|e| {
            format!(
                "dev seed: failed to upsert setting {}.{}: {e}",
                seed_setting.scope, seed_setting.key
            )
        })?;

        tracing::info!(
            scope = %seed_setting.scope,
            key = %seed_setting.key,
            scope_id = ?seed_setting.scope_id,
            "dev seed: upserted setting"
        );
    }

    tracing::info!(seed_file = %seed_path, "dev seed: completed");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_example_seed_file() {
        let contents = include_str!("../../../dev-seed.example.toml");
        let config: SeedConfig = toml::from_str(contents).expect("example seed file should parse");
        assert!(config.admin.is_some());
        assert_eq!(config.admin.unwrap().password, "admin");
        assert_eq!(config.indexers.len(), 1);
        assert_eq!(config.indexers[0].provider_type, "nzbgeek");
        assert_eq!(config.download_clients.len(), 1);
        assert_eq!(config.download_clients[0].client_type, "nzbget");
        assert!(config.settings.len() >= 2);
    }

    #[test]
    fn parse_nested_table_settings() {
        let toml_str = r#"
[[settings]]
scope = "media"
key = "movies.path"
value = "/media/movies"

[[settings]]
scope = "system"
key = "nzbget.client_routing"
scope_id = "movie"

[settings.value.home-nzbget]
category = "Movies"
recentPriority = "normal"
"#;
        let config: SeedConfig = toml::from_str(toml_str).expect("nested tables should parse");
        assert_eq!(config.settings.len(), 2);

        // First: simple string value
        let json0 = toml_value_to_json_string(&config.settings[0].value);
        assert_eq!(json0, r#""/media/movies""#);

        // Second: nested table value
        let json1 = toml_value_to_json_string(&config.settings[1].value);
        let parsed: serde_json::Value = serde_json::from_str(&json1).unwrap();
        assert_eq!(parsed["home-nzbget"]["category"], "Movies");
    }

    #[test]
    fn toml_value_converts_to_json() {
        let toml_str = r#"
value = { host = "127.0.0.1", port = "6789", use_ssl = false, tags = [] }
"#;
        let table: toml::Value = toml::from_str(toml_str).unwrap();
        let value = table.get("value").unwrap();
        let json = toml_value_to_json_string(value);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["host"], "127.0.0.1");
        assert_eq!(parsed["use_ssl"], false);
        assert!(parsed["tags"].is_array());
    }
}

use async_graphql::{Context, Error, Object, Result as GqlResult};
use scryer_domain::{Entitlement, NewDownloadClientConfig, NewIndexerConfig};
use serde_json::{Map, Value, json};

use crate::context::{actor_from_ctx, app_from_ctx, settings_db_from_ctx, to_gql_error};
use crate::mappers::{
    from_download_client_config, from_housekeeping_report, from_indexer_config,
    from_rss_sync_report,
};
use crate::types::*;

const SETTINGS_SCOPE_SYSTEM: &str = "system";
const DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY: &str = "download_client.routing";
const LEGACY_NZBGET_CLIENT_ROUTING_SETTINGS_KEY: &str = "nzbget.client_routing";
const DOWNLOAD_CLIENT_ROUTING_SCOPE_IDS: [&str; 3] = ["movie", "series", "anime"];

fn should_seed_download_client_routing(client_type: &str) -> bool {
    matches!(
        client_type.trim().to_ascii_lowercase().as_str(),
        "nzbget" | "sabnzbd" | "weaver"
    )
}

fn parse_download_client_routing_priority(raw_priority: &Value) -> Option<i64> {
    match raw_priority {
        Value::Number(number) => number.as_i64(),
        Value::String(value) => value.parse::<i64>().ok(),
        _ => None,
    }
}

fn next_download_client_routing_priority(routing_by_client: &Map<String, Value>) -> i64 {
    let max_explicit_priority = routing_by_client
        .values()
        .filter_map(|value| value.get("priority"))
        .filter_map(parse_download_client_routing_priority)
        .max();

    match max_explicit_priority {
        Some(max_priority) => max_priority + 1,
        None => i64::try_from(routing_by_client.len()).unwrap_or(0) + 1,
    }
}

fn default_download_client_routing_entry(priority: i64) -> Value {
    json!({
        "enabled": true,
        "category": "",
        "recentQueuePriority": "",
        "olderQueuePriority": "",
        "removeCompleted": true,
        "removeFailed": false,
        "priority": priority,
    })
}

fn parse_download_client_routing_object(raw_payload: &str) -> Map<String, Value> {
    serde_json::from_str::<Value>(raw_payload)
        .ok()
        .and_then(|value| value.as_object().map(std::borrow::ToOwned::to_owned))
        .unwrap_or_default()
}

async fn load_download_client_routing_payload(
    db: &scryer_infrastructure::SqliteServices,
    scope_id: &str,
) -> GqlResult<Map<String, Value>> {
    let current = db
        .get_setting_with_defaults(
            SETTINGS_SCOPE_SYSTEM,
            DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
            Some(scope_id.to_string()),
        )
        .await
        .map_err(to_gql_error)?;

    if let Some(record) = current.as_ref()
        && record.value_json.is_some()
    {
        return Ok(parse_download_client_routing_object(
            &record.effective_value_json,
        ));
    }

    let legacy = db
        .get_setting_with_defaults(
            SETTINGS_SCOPE_SYSTEM,
            LEGACY_NZBGET_CLIENT_ROUTING_SETTINGS_KEY,
            Some(scope_id.to_string()),
        )
        .await
        .map_err(to_gql_error)?;

    if let Some(record) = legacy.as_ref()
        && record.value_json.is_some()
    {
        return Ok(parse_download_client_routing_object(
            &record.effective_value_json,
        ));
    }

    Ok(current
        .as_ref()
        .map(|record| parse_download_client_routing_object(&record.effective_value_json))
        .unwrap_or_default())
}

pub(crate) async fn ensure_download_client_routing_entry_for_client(
    db: &scryer_infrastructure::SqliteServices,
    client_id: &str,
    actor_id: &str,
) -> GqlResult<()> {
    for scope_id in DOWNLOAD_CLIENT_ROUTING_SCOPE_IDS {
        let mut payload = load_download_client_routing_payload(db, scope_id).await?;

        if payload.contains_key(client_id) {
            continue;
        }

        let next_priority = next_download_client_routing_priority(&payload);
        payload.insert(
            client_id.to_string(),
            default_download_client_routing_entry(next_priority),
        );

        db.upsert_setting_value(
            SETTINGS_SCOPE_SYSTEM,
            DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
            Some(scope_id.to_string()),
            Value::Object(payload).to_string(),
            "admin_graphql",
            Some(actor_id.to_string()),
        )
        .await
        .map_err(to_gql_error)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::default_download_client_routing_entry;

    #[test]
    fn default_download_client_routing_entry_seeds_enabled_true() {
        let entry = default_download_client_routing_entry(4);

        assert_eq!(entry["enabled"], true);
        assert_eq!(entry["priority"], 4);
    }
}

#[derive(Default)]
pub(crate) struct ConfigMutations;

#[Object]
impl ConfigMutations {
    async fn create_indexer_config(
        &self,
        ctx: &Context<'_>,
        input: CreateIndexerConfigInput,
    ) -> GqlResult<IndexerConfigPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let config = app
            .create_indexer_config(
                &actor,
                NewIndexerConfig {
                    name: input.name,
                    provider_type: input.provider_type,
                    base_url: input.base_url,
                    api_key_encrypted: input.api_key,
                    rate_limit_seconds: input.rate_limit_seconds,
                    rate_limit_burst: input.rate_limit_burst,
                    is_enabled: input.is_enabled.unwrap_or(true),
                    enable_interactive_search: input.enable_interactive_search.unwrap_or(true),
                    enable_auto_search: input.enable_auto_search.unwrap_or(true),
                    config_json: input.config_json,
                },
            )
            .await
            .map_err(to_gql_error)?;
        Ok(from_indexer_config(config))
    }

    async fn update_indexer_config(
        &self,
        ctx: &Context<'_>,
        input: UpdateIndexerConfigInput,
    ) -> GqlResult<IndexerConfigPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let config = app
            .update_indexer_config(
                &actor,
                &input.id,
                input.name,
                input.provider_type,
                input.base_url,
                input.api_key,
                input.rate_limit_seconds,
                input.rate_limit_burst,
                input.is_enabled,
                input.enable_interactive_search,
                input.enable_auto_search,
                input.config_json,
            )
            .await
            .map_err(to_gql_error)?;
        Ok(from_indexer_config(config))
    }

    async fn delete_indexer_config(
        &self,
        ctx: &Context<'_>,
        input: DeleteIndexerConfigInput,
    ) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.delete_indexer_config(&actor, &input.id)
            .await
            .map_err(to_gql_error)
            .map(|_| true)
    }

    async fn create_download_client_config(
        &self,
        ctx: &Context<'_>,
        input: CreateDownloadClientConfigInput,
    ) -> GqlResult<DownloadClientConfigPayload> {
        let app = app_from_ctx(ctx)?;
        let db = settings_db_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let config = app
            .create_download_client_config(
                &actor,
                NewDownloadClientConfig {
                    name: input.name,
                    client_type: input.client_type,
                    base_url: input.base_url,
                    config_json: input.config_json,
                    client_priority: 0,
                    is_enabled: input.is_enabled.unwrap_or(true),
                },
            )
            .await
            .map_err(to_gql_error)?;

        if should_seed_download_client_routing(&config.client_type) {
            ensure_download_client_routing_entry_for_client(&db, &config.id, &actor.id).await?;
        }

        Ok(from_download_client_config(config))
    }

    async fn update_download_client_config(
        &self,
        ctx: &Context<'_>,
        input: UpdateDownloadClientConfigInput,
    ) -> GqlResult<DownloadClientConfigPayload> {
        let app = app_from_ctx(ctx)?;
        let db = settings_db_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let config = app
            .update_download_client_config(
                &actor,
                &input.id,
                input.name,
                input.client_type,
                input.base_url,
                input.config_json,
                input.is_enabled,
            )
            .await
            .map_err(to_gql_error)?;

        if should_seed_download_client_routing(&config.client_type) {
            ensure_download_client_routing_entry_for_client(&db, &config.id, &actor.id).await?;
        }

        Ok(from_download_client_config(config))
    }

    async fn delete_download_client_config(
        &self,
        ctx: &Context<'_>,
        input: DeleteDownloadClientConfigInput,
    ) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.delete_download_client_config(&actor, &input.id)
            .await
            .map_err(to_gql_error)
            .map(|_| true)
    }

    async fn reorder_download_client_configs(
        &self,
        ctx: &Context<'_>,
        input: ReorderDownloadClientConfigsInput,
    ) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.reorder_download_clients(&actor, input.ids)
            .await
            .map_err(to_gql_error)
            .map(|_| true)
    }

    async fn test_download_client_connection(
        &self,
        ctx: &Context<'_>,
        input: TestDownloadClientConnectionInput,
    ) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }

        let client_type = input.client_type.trim().to_lowercase();

        let base_url = input.base_url.trim().to_string();
        if base_url.is_empty() {
            return Err(Error::new("base_url is required"));
        }

        let config_json = input.config_json.trim().to_string();
        let config: Value = if config_json.is_empty() {
            json!({})
        } else {
            serde_json::from_str(&config_json)
                .map_err(|error| Error::new(format!("invalid client config_json: {error}")))?
        };

        match client_type.as_str() {
            "nzbget" => {
                let username = config
                    .get("username")
                    .and_then(Value::as_str)
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty());
                let password = config
                    .get("password")
                    .and_then(Value::as_str)
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty());

                scryer_infrastructure::NzbgetDownloadClient::new(
                    base_url,
                    username,
                    password,
                    "SCORE".to_string(),
                )
                .test_connection()
                .await
                .map_err(to_gql_error)?;
            }
            "sabnzbd" => {
                let api_key = config
                    .get("api_key")
                    .or_else(|| config.get("apiKey"))
                    .or_else(|| config.get("apikey"))
                    .and_then(Value::as_str)
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| Error::new("sabnzbd requires an API key"))?;

                scryer_infrastructure::SabnzbdDownloadClient::new(base_url, api_key)
                    .test_connection()
                    .await
                    .map_err(to_gql_error)?;
            }
            "weaver" => {
                let api_key = config
                    .get("api_key")
                    .or_else(|| config.get("apiKey"))
                    .or_else(|| config.get("apikey"))
                    .and_then(Value::as_str)
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty());

                scryer_infrastructure::WeaverDownloadClient::new(base_url, api_key)
                    .test_connection()
                    .await
                    .map_err(to_gql_error)?;
            }
            _ => {
                let provider = app
                    .services
                    .download_client_plugin_provider
                    .as_ref()
                    .ok_or_else(|| {
                        Error::new(format!(
                            "test connection is not supported for client type '{client_type}'"
                        ))
                    })?;
                let plugin_config = scryer_domain::DownloadClientConfig {
                    id: "test-download-client".to_string(),
                    name: "Test Download Client".to_string(),
                    client_type: client_type.clone(),
                    base_url: Some(base_url),
                    config_json: serde_json::to_string(&config).map_err(|error| {
                        Error::new(format!("invalid client config_json: {error}"))
                    })?,
                    client_priority: 0,
                    is_enabled: true,
                    status: "unknown".to_string(),
                    last_error: None,
                    last_seen_at: None,
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                };
                let client = provider.client_for_config(&plugin_config).ok_or_else(|| {
                    Error::new(format!(
                        "test connection is not supported for client type '{client_type}'"
                    ))
                })?;
                client.test_connection().await.map_err(to_gql_error)?;
            }
        }

        Ok(true)
    }

    async fn test_indexer_connection(
        &self,
        ctx: &Context<'_>,
        input: TestIndexerConnectionInput,
    ) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.test_indexer_connection(
            &actor,
            &input.provider_type,
            &input.base_url,
            input.api_key.as_deref(),
            input.config_json.as_deref(),
        )
        .await
        .map_err(to_gql_error)?;
        Ok(true)
    }

    async fn run_housekeeping(&self, ctx: &Context<'_>) -> GqlResult<HousekeepingReportPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
        let report = app.run_housekeeping().await.map_err(to_gql_error)?;
        Ok(from_housekeeping_report(report))
    }

    async fn trigger_rss_sync(&self, ctx: &Context<'_>) -> GqlResult<RssSyncReportPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
        let report = app.run_rss_sync().await.map_err(to_gql_error)?;
        Ok(from_rss_sync_report(report))
    }
}

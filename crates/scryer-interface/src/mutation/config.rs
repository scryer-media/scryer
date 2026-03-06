use async_graphql::{Context, Error, Object, Result as GqlResult};
use scryer_domain::{Entitlement, NewDownloadClientConfig, NewIndexerConfig};
use serde_json::{json, Map, Value};

use crate::context::{actor_from_ctx, app_from_ctx, settings_db_from_ctx, to_gql_error};
use crate::mappers::{from_download_client_config, from_indexer_config};
use crate::types::*;

const SETTINGS_SCOPE_SYSTEM: &str = "system";
const NZBGET_CLIENT_ROUTING_SETTINGS_KEY: &str = "nzbget.client_routing";
const NZBGET_CLIENT_RANKING_SCOPE_IDS: [&str; 3] = ["movie", "series", "anime"];

fn parse_nzbget_priority(raw_priority: &Value) -> Option<i64> {
    match raw_priority {
        Value::Number(number) => number.as_i64(),
        Value::String(value) => value.parse::<i64>().ok(),
        _ => None,
    }
}

fn next_nzbget_routing_priority(routing_by_client: &Map<String, Value>) -> i64 {
    let max_explicit_priority = routing_by_client
        .values()
        .filter_map(|value| value.get("priority"))
        .filter_map(parse_nzbget_priority)
        .max()
        ;

    match max_explicit_priority {
        Some(max_priority) => max_priority + 1,
        None => i64::try_from(routing_by_client.len()).unwrap_or(0) + 1,
    }
}

async fn ensure_nzbget_routing_entry_for_client(
    db: &scryer_infrastructure::SqliteServices,
    client_id: &str,
    actor_id: &str,
) -> GqlResult<()> {
    for scope_id in NZBGET_CLIENT_RANKING_SCOPE_IDS {
        let existing = db
            .get_setting_with_defaults(
                SETTINGS_SCOPE_SYSTEM,
                NZBGET_CLIENT_ROUTING_SETTINGS_KEY,
                Some(scope_id.to_string()),
            )
            .await
            .map_err(to_gql_error)?;

        let Some(existing) = existing else {
            continue;
        };

        let raw_payload = existing.effective_value_json;
        let mut payload = serde_json::from_str::<Value>(&raw_payload)
            .ok()
            .and_then(|value| value.as_object().map(std::borrow::ToOwned::to_owned))
            .unwrap_or_default();

        if payload.contains_key(client_id) {
            continue;
        }

        let next_priority = next_nzbget_routing_priority(&payload);
        payload.insert(
            client_id.to_string(),
            json!({
                "category": "",
                "recentPriority": "",
                "olderPriority": "",
                "removeCompleted": false,
                "removeFailed": false,
                "tags": [],
                "priority": next_priority,
            }),
        );

        db.upsert_setting_value(
            SETTINGS_SCOPE_SYSTEM,
            NZBGET_CLIENT_ROUTING_SETTINGS_KEY,
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

        if config.client_type == "nzbget" || config.client_type == "sabnzbd" {
            ensure_nzbget_routing_entry_for_client(&db, &config.id, &actor.id).await?;
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

        if config.client_type == "nzbget" || config.client_type == "sabnzbd" {
            ensure_nzbget_routing_entry_for_client(&db, &config.id, &actor.id).await?;
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

    async fn test_download_client_connection(
        &self,
        ctx: &Context<'_>,
        input: TestDownloadClientConnectionInput,
    ) -> GqlResult<bool> {
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
            serde_json::from_str(&config_json).map_err(|error| {
                Error::new(format!("invalid client config_json: {error}"))
            })?
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
            _ => {
                return Err(Error::new(format!(
                    "test connection is not supported for client type '{client_type}'"
                )));
            }
        }

        Ok(true)
    }
}

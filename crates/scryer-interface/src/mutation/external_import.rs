use std::collections::HashSet;

use async_graphql::{Context, Object, Result as GqlResult};
use scryer_domain::{Entitlement, NewDownloadClientConfig, NewIndexerConfig};
use scryer_infrastructure::external_import::{
    self, ArrDownloadClient, ArrIndexer, ExternalArrClient,
};

use crate::context::{actor_from_ctx, app_from_ctx, settings_db_from_ctx};
use crate::types::*;

use super::config::ensure_download_client_routing_entry_for_client;

const SETTINGS_SCOPE_MEDIA: &str = "media";

#[derive(Default)]
pub(crate) struct ExternalImportMutations;

#[Object]
impl ExternalImportMutations {
    /// Connect to Sonarr and/or Radarr, fetch their configs, return a preview.
    async fn preview_external_import(
        &self,
        ctx: &Context<'_>,
        input: PreviewExternalImportInput,
    ) -> GqlResult<ExternalImportPreviewPayload> {
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&Entitlement::ManageConfig) {
            return Err(async_graphql::Error::new("insufficient entitlements"));
        }

        if input.sonarr.is_none() && input.radarr.is_none() {
            return Err(async_graphql::Error::new(
                "at least one of sonarr or radarr must be provided",
            ));
        }

        let mut payload = ExternalImportPreviewPayload {
            sonarr_connected: false,
            radarr_connected: false,
            sonarr_version: None,
            radarr_version: None,
            root_folders: Vec::new(),
            download_clients: Vec::new(),
            indexers: Vec::new(),
        };

        // Map from dedup_key → index in payload vecs, so duplicates merge sources.
        let mut dc_key_idx: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        let mut idx_key_idx: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();

        for (conn_opt, source) in [(&input.sonarr, "sonarr"), (&input.radarr, "radarr")] {
            let Some(conn) = conn_opt else { continue };
            let client = ExternalArrClient::new(conn.base_url.clone(), conn.api_key.clone());
            match client.test_connection().await {
                Ok((_app_name, version)) => {
                    if source == "sonarr" {
                        payload.sonarr_connected = true;
                        payload.sonarr_version = Some(version);
                    } else {
                        payload.radarr_connected = true;
                        payload.radarr_version = Some(version);
                    }

                    if let Ok(folders) = client.list_root_folders().await {
                        for folder in folders {
                            payload.root_folders.push(ExternalImportRootFolderPayload {
                                source: source.to_string(),
                                path: folder.path,
                            });
                        }
                    }

                    if let Ok(clients) = client.list_download_clients().await {
                        for dc in clients {
                            let mapped = map_download_client(&dc, source);
                            if let Some(&existing) = dc_key_idx.get(&mapped.dedup_key) {
                                payload.download_clients[existing]
                                    .sources
                                    .push(source.to_string());
                            } else {
                                dc_key_idx.insert(
                                    mapped.dedup_key.clone(),
                                    payload.download_clients.len(),
                                );
                                payload.download_clients.push(mapped);
                            }
                        }
                    }

                    if let Ok(indexers) = client.list_indexers().await {
                        for idx in indexers {
                            let mapped = map_indexer(&idx, source);
                            if let Some(&existing) = idx_key_idx.get(&mapped.dedup_key) {
                                payload.indexers[existing].sources.push(source.to_string());
                            } else {
                                idx_key_idx
                                    .insert(mapped.dedup_key.clone(), payload.indexers.len());
                                payload.indexers.push(mapped);
                            }
                        }
                    }
                }
                Err(_) => {
                    if source == "sonarr" {
                        payload.sonarr_connected = false;
                    } else {
                        payload.radarr_connected = false;
                    }
                }
            }
        }

        Ok(payload)
    }

    /// Re-connect to Sonarr/Radarr, fetch configs, and create selected items in Scryer.
    async fn execute_external_import(
        &self,
        ctx: &Context<'_>,
        input: ExecuteExternalImportInput,
    ) -> GqlResult<ExternalImportResultPayload> {
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&Entitlement::ManageConfig) {
            return Err(async_graphql::Error::new("insufficient entitlements"));
        }

        let app = app_from_ctx(ctx)?;
        let db = settings_db_from_ctx(ctx)?;

        let selected_dc_keys: HashSet<String> = input
            .selected_download_client_dedup_keys
            .into_iter()
            .collect();
        let selected_idx_keys: HashSet<String> =
            input.selected_indexer_dedup_keys.into_iter().collect();
        let dc_api_key_overrides: std::collections::HashMap<String, String> = input
            .download_client_api_key_overrides
            .into_iter()
            .map(|o| (o.dedup_key, o.api_key))
            .collect();

        let mut result = ExternalImportResultPayload {
            media_paths_saved: false,
            download_clients_created: 0,
            indexers_created: 0,
            plugins_installed: Vec::new(),
            errors: Vec::new(),
        };

        // ── Save media paths ──────────────────────────────────────────────
        let mut paths_saved = false;
        if let Some(movies_path) = &input.selected_movies_path {
            if let Err(err) = db
                .upsert_setting_value(
                    SETTINGS_SCOPE_MEDIA,
                    "movies.path",
                    None,
                    &format!("\"{}\"", movies_path.replace('"', "\\\"")),
                    "external-import",
                    Some(actor.id.clone()),
                )
                .await
            {
                result
                    .errors
                    .push(format!("failed to save movies path: {err}"));
            } else {
                paths_saved = true;
            }
        }
        if let Some(series_path) = &input.selected_series_path {
            if let Err(err) = db
                .upsert_setting_value(
                    SETTINGS_SCOPE_MEDIA,
                    "series.path",
                    None,
                    &format!("\"{}\"", series_path.replace('"', "\\\"")),
                    "external-import",
                    Some(actor.id.clone()),
                )
                .await
            {
                result
                    .errors
                    .push(format!("failed to save series path: {err}"));
            } else {
                paths_saved = true;
            }
        }
        if let Some(anime_path) = &input.selected_anime_path {
            if let Err(err) = db
                .upsert_setting_value(
                    SETTINGS_SCOPE_MEDIA,
                    "anime.path",
                    None,
                    &format!("\"{}\"", anime_path.replace('"', "\\\"")),
                    "external-import",
                    Some(actor.id.clone()),
                )
                .await
            {
                result
                    .errors
                    .push(format!("failed to save anime path: {err}"));
            } else {
                paths_saved = true;
            }
        }
        result.media_paths_saved = paths_saved;

        // ── Collect download clients + indexers from external apps ─────────
        let mut all_download_clients: Vec<(ArrDownloadClient, String)> = Vec::new();
        let mut all_indexers: Vec<(ArrIndexer, String)> = Vec::new();
        let mut seen_dc_keys: HashSet<String> = HashSet::new();
        let mut seen_idx_keys: HashSet<String> = HashSet::new();

        for (conn_opt, source) in [(&input.sonarr, "sonarr"), (&input.radarr, "radarr")] {
            let Some(conn) = conn_opt else { continue };
            let client = ExternalArrClient::new(conn.base_url.clone(), conn.api_key.clone());

            if client.test_connection().await.is_err() {
                result.errors.push(format!("failed to connect to {source}"));
                continue;
            }

            if let Ok(clients) = client.list_download_clients().await {
                for dc in clients {
                    let mapped = map_download_client(&dc, source);
                    if mapped.supported
                        && seen_dc_keys.insert(mapped.dedup_key.clone())
                        && selected_dc_keys.contains(&mapped.dedup_key)
                    {
                        all_download_clients.push((dc, source.to_string()));
                    }
                }
            }

            if let Ok(indexers) = client.list_indexers().await {
                for idx in indexers {
                    let mapped = map_indexer(&idx, source);
                    if mapped.supported
                        && seen_idx_keys.insert(mapped.dedup_key.clone())
                        && selected_idx_keys.contains(&mapped.dedup_key)
                    {
                        all_indexers.push((idx, source.to_string()));
                    }
                }
            }
        }

        // ── Create download clients ───────────────────────────────────────
        for (dc, _source) in &all_download_clients {
            let Some(scryer_type) = external_import::map_download_client_type(&dc.implementation)
            else {
                continue;
            };

            let host = external_import::field_str(&dc.fields, "host").unwrap_or_default();
            let port = external_import::field_str_or_number(&dc.fields, "port").unwrap_or_default();
            let use_ssl = external_import::field_bool(&dc.fields, "useSsl").unwrap_or(false);
            let url_base = external_import::field_str(&dc.fields, "urlBase").unwrap_or_default();

            let protocol = if use_ssl { "https" } else { "http" };
            let port_part = if port.is_empty() {
                String::new()
            } else {
                format!(":{port}")
            };
            let path_part = if url_base.is_empty() {
                String::new()
            } else {
                format!("/{}", url_base.trim_start_matches('/'))
            };
            let base_url = format!("{protocol}://{host}{port_part}{path_part}");

            let mut config_obj = serde_json::Map::new();
            config_obj.insert("host".into(), serde_json::Value::String(host.clone()));
            config_obj.insert("port".into(), serde_json::Value::String(port.clone()));
            config_obj.insert("use_ssl".into(), serde_json::Value::Bool(use_ssl));
            config_obj.insert("url_base".into(), serde_json::Value::String(url_base));
            config_obj.insert(
                "client_type".into(),
                serde_json::Value::String(scryer_type.to_string()),
            );

            if scryer_type == "sabnzbd" || scryer_type == "weaver" {
                // Prefer a user-supplied override (needed when Sonarr/Radarr masked
                // the key), then fall back to the value fetched from the arr API.
                let dedup_key = format!("{}:{}:{}", scryer_type, host, port);
                let api_key = dc_api_key_overrides
                    .get(&dedup_key)
                    .cloned()
                    .or_else(|| external_import::field_str_sensitive(&dc.fields, "apiKey"));
                if let Some(api_key) = api_key {
                    config_obj.insert("api_key".into(), serde_json::Value::String(api_key));
                }
            } else {
                if let Some(username) = external_import::field_str(&dc.fields, "username") {
                    config_obj.insert("username".into(), serde_json::Value::String(username));
                }
                if let Some(password) = external_import::field_str(&dc.fields, "password") {
                    config_obj.insert("password".into(), serde_json::Value::String(password));
                }
            }

            let config_json = serde_json::Value::Object(config_obj).to_string();

            match app
                .create_download_client_config(
                    &actor,
                    NewDownloadClientConfig {
                        name: dc.name.clone(),
                        client_type: scryer_type.to_string(),
                        base_url: Some(base_url),
                        config_json,
                        client_priority: 0,
                        is_enabled: true,
                    },
                )
                .await
            {
                Ok(config) => {
                    result.download_clients_created += 1;
                    if scryer_type == "nzbget"
                        || scryer_type == "sabnzbd"
                        || scryer_type == "weaver"
                    {
                        let _ = ensure_download_client_routing_entry_for_client(
                            &db, &config.id, &actor.id,
                        )
                        .await;
                    }
                }
                Err(err) => {
                    result.errors.push(format!(
                        "failed to create download client '{}': {err}",
                        dc.name
                    ));
                }
            }
        }

        // ── Auto-install non-builtin plugins needed by selected indexers ──
        let available_providers: HashSet<String> = app
            .available_indexer_provider_types()
            .iter()
            .map(|(pt, _, _, _)| pt.clone())
            .collect();

        let mut auto_installed: HashSet<String> = HashSet::new();
        for (idx, _) in &all_indexers {
            let Some(scryer_type) =
                external_import::map_indexer_provider_type(&idx.implementation, &idx.fields)
            else {
                continue;
            };
            if available_providers.contains(scryer_type) || auto_installed.contains(scryer_type) {
                continue;
            }
            // Plugin not loaded — try to install from registry
            let install_result = match app.install_plugin(&actor, scryer_type).await {
                Ok(inst) => Ok(inst),
                Err(_) => {
                    // Registry might not be cached yet — refresh and retry
                    let _ = app.refresh_plugin_registry_internal().await;
                    app.install_plugin(&actor, scryer_type).await
                }
            };
            match install_result {
                Ok(inst) => {
                    auto_installed.insert(scryer_type.to_string());
                    result.plugins_installed.push(inst.name);
                }
                Err(err) => {
                    result
                        .errors
                        .push(format!("failed to install {} plugin: {err}", scryer_type));
                }
            }
        }

        // ── Create indexers ───────────────────────────────────────────────
        for (idx, _source) in &all_indexers {
            let Some(scryer_type) =
                external_import::map_indexer_provider_type(&idx.implementation, &idx.fields)
            else {
                continue;
            };

            let mut base_url =
                external_import::field_str(&idx.fields, "baseUrl").unwrap_or_default();
            let api_path = external_import::field_str(&idx.fields, "apiPath");
            if let Some(path) = &api_path {
                if !path.is_empty() && !base_url.is_empty() {
                    base_url = format!(
                        "{}/{}",
                        base_url.trim_end_matches('/'),
                        path.trim_start_matches('/')
                    );
                }
            }

            let api_key = external_import::field_str(&idx.fields, "apiKey");

            // If the plugin was just auto-installed, it may have auto-created a
            // default IndexerConfig. Update that config instead of creating a
            // duplicate. Once claimed, further indexers of the same type create
            // new configs normally.
            if auto_installed.remove(scryer_type) {
                let existing = app
                    .list_indexer_configs(&actor, Some(scryer_type.to_string()))
                    .await
                    .unwrap_or_default();
                if let Some(existing_config) = existing.first() {
                    if api_key.is_some() || existing_config.base_url != base_url {
                        let _ = app
                            .update_indexer_config(
                                &actor,
                                &existing_config.id,
                                Some(idx.name.clone()),
                                None,
                                Some(base_url),
                                api_key,
                                None,
                                None,
                                None,
                                None,
                                None,
                                None,
                            )
                            .await;
                    }
                    result.indexers_created += 1;
                    continue;
                }
            }

            match app
                .create_indexer_config(
                    &actor,
                    NewIndexerConfig {
                        name: idx.name.clone(),
                        provider_type: scryer_type.to_string(),
                        base_url,
                        api_key_encrypted: api_key,
                        rate_limit_seconds: None,
                        rate_limit_burst: None,
                        is_enabled: true,
                        enable_interactive_search: true,
                        enable_auto_search: true,
                        config_json: None,
                    },
                )
                .await
            {
                Ok(_) => {
                    result.indexers_created += 1;
                }
                Err(err) => {
                    result
                        .errors
                        .push(format!("failed to create indexer '{}': {err}", idx.name));
                }
            }
        }

        Ok(result)
    }
}

fn map_download_client(
    dc: &ArrDownloadClient,
    source: &str,
) -> ExternalImportDownloadClientPayload {
    let scryer_type = external_import::map_download_client_type(&dc.implementation);
    let host = external_import::field_str(&dc.fields, "host");
    let port = external_import::field_str_or_number(&dc.fields, "port");
    let use_ssl = external_import::field_bool(&dc.fields, "useSsl").unwrap_or(false);
    let url_base = external_import::field_str(&dc.fields, "urlBase");
    let username = external_import::field_str(&dc.fields, "username");
    // Use field_str_sensitive so that Sonarr/Radarr's "********" mask becomes
    // None — callers can then detect that the key must be entered manually.
    let api_key = external_import::field_str_sensitive(&dc.fields, "apiKey");

    let dedup_key = format!(
        "{}:{}:{}",
        scryer_type.unwrap_or("unsupported"),
        host.as_deref().unwrap_or(""),
        port.as_deref().unwrap_or("")
    );

    ExternalImportDownloadClientPayload {
        sources: vec![source.to_string()],
        name: dc.name.clone(),
        implementation: dc.implementation.clone(),
        scryer_client_type: scryer_type.map(str::to_string),
        host,
        port,
        use_ssl,
        url_base,
        username,
        api_key,
        dedup_key,
        supported: scryer_type.is_some(),
    }
}

fn map_indexer(idx: &ArrIndexer, source: &str) -> ExternalImportIndexerPayload {
    let scryer_type = external_import::map_indexer_provider_type(&idx.implementation, &idx.fields);
    let base_url = external_import::field_str(&idx.fields, "baseUrl");
    let api_key = external_import::field_str_sensitive(&idx.fields, "apiKey");

    let dedup_key = format!(
        "{}:{}",
        scryer_type.unwrap_or("unsupported"),
        base_url.as_deref().unwrap_or("")
    );

    ExternalImportIndexerPayload {
        sources: vec![source.to_string()],
        name: idx.name.clone(),
        implementation: idx.implementation.clone(),
        scryer_provider_type: scryer_type.map(str::to_string),
        base_url,
        api_key,
        dedup_key,
        supported: scryer_type.is_some(),
    }
}

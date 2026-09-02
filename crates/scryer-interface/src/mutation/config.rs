use async_graphql::{Context, ID, MaybeUndefined, Object, Result as GqlResult};
use chrono::{DateTime, Utc};
use scryer_application::{
    AppError, DownloadClientConfigUpdate, IndexerConfigUpdate, NewProxyConfig, NewSeedingProfile,
    ProxyConfigUpdate, SeedingProfileUpdate, SubtitleProviderConfigUpdate,
};
use scryer_domain::{AppPermission, NewDownloadClientConfig, NewIndexerConfig, ProxyProviderType};

use crate::context::{actor_from_ctx, app_from_ctx, require_config_app_permission, to_gql_error};
use crate::mappers::{
    from_download_client_config_with_fields, from_indexer_config_sync_result,
    from_indexer_config_with_fields, from_proxy_config, from_proxy_test_result,
    from_rss_sync_report, from_seeding_profile, from_subtitle_provider_config,
    post_import_tracking_from_value, provider_config_values_to_json,
    season_pack_seed_mode_from_value, seed_goal_met_action_from_value,
};
use crate::types::*;

fn should_seed_download_client_routing(client_type: &str) -> bool {
    matches!(client_type, "nzbget" | "sabnzbd" | "weaver")
}

fn optional_datetime_input(
    value: MaybeUndefined<DateTime<Utc>>,
    _field_name: &str,
) -> GqlResult<Option<Option<DateTime<Utc>>>> {
    match value {
        MaybeUndefined::Undefined => Ok(None),
        MaybeUndefined::Null => Ok(Some(None)),
        MaybeUndefined::Value(value) => Ok(Some(Some(value))),
    }
}

fn optional_id_input(value: MaybeUndefined<ID>) -> Option<Option<String>> {
    match value {
        MaybeUndefined::Undefined => None,
        MaybeUndefined::Null => Some(None),
        MaybeUndefined::Value(value) => Some(Some(value.to_string())),
    }
}

fn optional_scalar_input<T>(value: MaybeUndefined<T>) -> Option<Option<T>> {
    match value {
        MaybeUndefined::Undefined => None,
        MaybeUndefined::Null => Some(None),
        MaybeUndefined::Value(value) => Some(Some(value)),
    }
}

/// Narrow a GraphQL `Int` onto the `u16` a WireGuard link setting is.
///
/// GraphQL has no unsigned type, so a negative or oversized MTU or keepalive
/// arrives as a perfectly valid `Int` and has to be refused here. The range
/// itself belongs to the workflow; this only rejects what cannot be a link
/// setting at all.
fn proxy_link_setting(value: Option<i32>, field: &str) -> GqlResult<Option<u16>> {
    value
        .map(|value| {
            u16::try_from(value).map_err(|_| {
                to_gql_error(AppError::Validation(format!(
                    "{field} must be between 0 and {}",
                    u16::MAX
                )))
            })
        })
        .transpose()
}

/// The patch form of [`proxy_link_setting`]: omission preserves, null restores
/// the engine's default.
fn optional_proxy_link_setting(
    value: MaybeUndefined<i32>,
    field: &str,
) -> GqlResult<Option<Option<u16>>> {
    match value {
        MaybeUndefined::Undefined => Ok(None),
        MaybeUndefined::Null => Ok(Some(None)),
        MaybeUndefined::Value(value) => proxy_link_setting(Some(value), field).map(Some),
    }
}

/// Parse an optional challenge-solver protocol from GraphQL input.
///
/// Whether a protocol is *allowed* for the provider is the workflow's call —
/// this only rejects a value that names no protocol at all.
fn parse_challenge_solver_protocol(
    raw: Option<&str>,
) -> GqlResult<Option<scryer_domain::ChallengeSolverProtocol>> {
    raw.map(|value| {
        scryer_domain::ChallengeSolverProtocol::parse(value).ok_or_else(|| {
            to_gql_error(AppError::Validation(format!(
                "unsupported proxy protocol '{value}'"
            )))
        })
    })
    .transpose()
}

async fn enrich_download_client_config_json(
    _client_type: &str,
    config_json: String,
) -> GqlResult<String> {
    Ok(config_json)
}

fn download_client_config_fields(
    app: &scryer_application::AppUseCase,
    client_type: &str,
) -> Vec<scryer_domain::ConfigFieldDef> {
    app.available_download_client_provider_types()
        .into_iter()
        .find_map(|(provider_type, _, fields, _)| {
            provider_type
                .eq_ignore_ascii_case(client_type)
                .then_some(fields)
        })
        .unwrap_or_default()
}

fn provider_config_key_looks_secret(key: &str) -> bool {
    let normalized = key.trim().to_ascii_lowercase();
    normalized.contains("password")
        || normalized.contains("secret")
        || normalized.contains("token")
        || normalized == "username"
        || normalized == "user_name"
        || normalized == "api_key"
        || normalized == "apikey"
        || normalized.contains("api_key")
}

fn merge_omitted_provider_secrets(
    incoming_json: String,
    existing_json: &str,
    config_fields: &[scryer_domain::ConfigFieldDef],
) -> scryer_application::AppResult<String> {
    let mut incoming = serde_json::from_str::<serde_json::Value>(&incoming_json)
        .map_err(|error| scryer_application::AppError::Validation(error.to_string()))?;
    let existing = serde_json::from_str::<serde_json::Value>(existing_json)
        .map_err(|error| scryer_application::AppError::Validation(error.to_string()))?;
    let Some(incoming_object) = incoming.as_object_mut() else {
        return Ok(incoming_json);
    };
    let Some(existing_object) = existing.as_object() else {
        return Ok(incoming_json);
    };
    let configured_secret_keys = config_fields
        .iter()
        .filter(|field| field.field_type == scryer_domain::ConfigFieldType::Password)
        .map(|field| field.key.as_str())
        .collect::<std::collections::HashSet<_>>();

    for (key, value) in existing_object {
        let is_secret =
            configured_secret_keys.contains(key.as_str()) || provider_config_key_looks_secret(key);
        if is_secret && !incoming_object.contains_key(key) {
            incoming_object.insert(key.clone(), value.clone());
        }
    }

    serde_json::to_string(&incoming)
        .map_err(|error| scryer_application::AppError::Validation(error.to_string()))
}

fn prepare_download_client_connection_test_config(
    id: &str,
    requested_client_type: &str,
    incoming_json: String,
    existing: Option<(&str, &str)>,
    config_fields: &[scryer_domain::ConfigFieldDef],
) -> scryer_application::AppResult<String> {
    let Some((existing_client_type, existing_json)) = existing else {
        return Err(AppError::NotFound(format!("download client {id}")));
    };
    if !existing_client_type.eq_ignore_ascii_case(requested_client_type) {
        return Err(AppError::Validation(format!(
            "download client {id} uses provider type '{existing_client_type}', not '{requested_client_type}'"
        )));
    }

    merge_omitted_provider_secrets(incoming_json, existing_json, config_fields)
}

#[derive(Default)]
pub(crate) struct ConfigMutations;

#[Object]
impl ConfigMutations {
    /// Create an indexer configuration with provider, routing, and search defaults.
    async fn create_indexer_config(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Indexer provider configuration, optional proxy and download-client identities, and search defaults."
        )]
        input: CreateIndexerConfigInput,
    ) -> GqlResult<IndexerConfigPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let config_json = input
            .config
            .map(provider_config_values_to_json)
            .transpose()
            .map_err(to_gql_error)?;
        let config = app
            .create_indexer_config(
                &actor,
                NewIndexerConfig {
                    name: input.name,
                    provider_type: input.provider_type,
                    rate_limit_seconds: input.rate_limit_seconds,
                    rate_limit_burst: input.rate_limit_burst,
                    is_enabled: input.is_enabled.unwrap_or(true),
                    enable_interactive_search: input.enable_interactive_search.unwrap_or(true),
                    enable_auto_search: input.enable_auto_search.unwrap_or(true),
                    proxy_config_id: input.proxy_config_id.map(|id| id.to_string()),
                    download_client_id: input.download_client_id.map(|id| id.to_string()),
                    config_json,
                },
            )
            .await
            .map_err(to_gql_error)?;
        let config_fields = app
            .indexer_config_fields_for_provider_type(&config.provider_type)
            .unwrap_or_default();

        Ok(from_indexer_config_with_fields(config, &config_fields))
    }

    /// Patch an indexer configuration while preserving omitted fields and stored secrets.
    async fn update_indexer_config(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Indexer configuration identity and optional replacement fields; omitted provider secrets remain stored."
        )]
        input: UpdateIndexerConfigInput,
    ) -> GqlResult<IndexerConfigPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let config_json = input
            .config
            .map(provider_config_values_to_json)
            .transpose()
            .map_err(to_gql_error)?;
        let config = app
            .update_indexer_config(
                &actor,
                IndexerConfigUpdate {
                    id: input.id.to_string(),
                    name: input.name,
                    provider_type: input.provider_type,
                    derived_base_url: None,
                    rate_limit_seconds: input.rate_limit_seconds,
                    rate_limit_burst: input.rate_limit_burst,
                    is_enabled: input.is_enabled,
                    enable_interactive_search: input.enable_interactive_search,
                    enable_auto_search: input.enable_auto_search,
                    proxy_config_id: optional_id_input(input.proxy_config_id),
                    download_client_id: optional_id_input(input.download_client_id),
                    // Seeding profiles are assigned through setIndexerSeedingProfile
                    // so the torrent-capability check stays single-sourced.
                    seeding_profile_id: None,
                    managed_parent_config_id: None,
                    managed_child_key: None,
                    managed_metadata_json: None,
                    caps_snapshot_json: None,
                    config_json,
                },
            )
            .await
            .map_err(to_gql_error)?;
        let config_fields = app
            .indexer_config_fields_for_provider_type(&config.provider_type)
            .unwrap_or_default();
        Ok(from_indexer_config_with_fields(config, &config_fields))
    }

    /// Set or clear the download client associated with an indexer.
    async fn set_indexer_download_client_mapping(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Indexer identity and optional download-client identity; null clears the mapping."
        )]
        input: SetIndexerDownloadClientMappingInput,
    ) -> GqlResult<IndexerConfigPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let download_client_id = input.download_client_id.map(|id| id.to_string());
        let config = app
            .set_indexer_download_client_mapping(
                &actor,
                input.indexer_id.as_ref(),
                download_client_id.as_deref(),
            )
            .await
            .map_err(to_gql_error)?;
        let config_fields = app
            .indexer_config_fields_for_provider_type(&config.provider_type)
            .unwrap_or_default();
        Ok(from_indexer_config_with_fields(config, &config_fields))
    }

    /// Set or clear the seeding profile applied to torrents from an indexer.
    async fn set_indexer_seeding_profile(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Indexer identity and optional seeding profile identity; null clears the assignment."
        )]
        input: SetIndexerSeedingProfileInput,
    ) -> GqlResult<IndexerConfigPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let seeding_profile_id = input.seeding_profile_id.map(|id| id.to_string());
        let config = app
            .set_indexer_seeding_profile(
                &actor,
                input.indexer_id.as_ref(),
                seeding_profile_id.as_deref(),
            )
            .await
            .map_err(to_gql_error)?;
        let config_fields = app
            .indexer_config_fields_for_provider_type(&config.provider_type)
            .unwrap_or_default();
        Ok(from_indexer_config_with_fields(config, &config_fields))
    }

    /// Create a torrent seeding profile.
    async fn create_seeding_profile(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Seeding profile name, goals, and removal behavior.")]
        input: CreateSeedingProfileInput,
    ) -> GqlResult<SeedingProfilePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let profile = app
            .create_seeding_profile(
                &actor,
                NewSeedingProfile {
                    name: input.name,
                    ratio: input.ratio,
                    seed_time_minutes: input.seed_time_minutes,
                    season_pack_mode: input
                        .season_pack_mode
                        .map(season_pack_seed_mode_from_value)
                        .unwrap_or_default(),
                    season_pack_ratio: input.season_pack_ratio,
                    season_pack_seed_time_minutes: input.season_pack_seed_time_minutes,
                    honor_tracker_minimums: input.honor_tracker_minimums.unwrap_or(true),
                    goal_met_action: input
                        .goal_met_action
                        .map(seed_goal_met_action_from_value)
                        .unwrap_or_default(),
                    never_remove: input.never_remove.unwrap_or(false),
                    minimum_seeders: input.minimum_seeders,
                    post_import_tracking: input
                        .post_import_tracking
                        .map(post_import_tracking_from_value)
                        .unwrap_or_default(),
                },
            )
            .await
            .map_err(to_gql_error)?;
        Ok(from_seeding_profile(profile))
    }

    /// Patch a torrent seeding profile.
    async fn update_seeding_profile(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Seeding profile identity and optional replacement fields; explicit nulls clear goals."
        )]
        input: UpdateSeedingProfileInput,
    ) -> GqlResult<SeedingProfilePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let profile = app
            .update_seeding_profile(
                &actor,
                SeedingProfileUpdate {
                    id: input.id.to_string(),
                    name: input.name,
                    ratio: optional_scalar_input(input.ratio),
                    seed_time_minutes: optional_scalar_input(input.seed_time_minutes),
                    season_pack_mode: input.season_pack_mode.map(season_pack_seed_mode_from_value),
                    season_pack_ratio: optional_scalar_input(input.season_pack_ratio),
                    season_pack_seed_time_minutes: optional_scalar_input(
                        input.season_pack_seed_time_minutes,
                    ),
                    honor_tracker_minimums: input.honor_tracker_minimums,
                    goal_met_action: input.goal_met_action.map(seed_goal_met_action_from_value),
                    never_remove: input.never_remove,
                    minimum_seeders: optional_scalar_input(input.minimum_seeders),
                    post_import_tracking: input
                        .post_import_tracking
                        .map(post_import_tracking_from_value),
                },
            )
            .await
            .map_err(to_gql_error)?;
        Ok(from_seeding_profile(profile))
    }

    /// Delete a seeding profile that nothing references.
    async fn delete_seeding_profile(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Seeding profile identity to delete.")] id: ID,
    ) -> GqlResult<DeleteSeedingProfilePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let id = id.to_string();
        app.delete_seeding_profile(&actor, &id)
            .await
            .map_err(to_gql_error)?;
        Ok(DeleteSeedingProfilePayload { id: ID::from(id) })
    }

    /// Set or clear the seeding profile applied when nothing more specific matches.
    async fn set_default_seeding_profile(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Seeding profile identity to use as the default, or null to clear it.")]
        input: SetDefaultSeedingProfileInput,
    ) -> GqlResult<DefaultSeedingProfilePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let seeding_profile_id = input.seeding_profile_id.map(|id| id.to_string());
        let seeding_profile_id = app
            .set_default_seeding_profile(&actor, seeding_profile_id.as_deref())
            .await
            .map_err(to_gql_error)?
            .map(Into::into);
        Ok(DefaultSeedingProfilePayload {
            seeding_profile_id,
            minimum_seeders_floor: app
                .get_minimum_seeders_floor(&actor)
                .await
                .map_err(to_gql_error)?,
        })
    }

    /// Set the minimum-seeder floor applied when no seeding profile resolves.
    async fn set_minimum_seeders_floor(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Fewest seeders a torrent candidate may report when no seeding profile resolves; 0 disables the check."
        )]
        input: SetMinimumSeedersFloorInput,
    ) -> GqlResult<DefaultSeedingProfilePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let minimum_seeders_floor = app
            .set_minimum_seeders_floor(&actor, input.minimum_seeders_floor)
            .await
            .map_err(to_gql_error)?;
        Ok(DefaultSeedingProfilePayload {
            seeding_profile_id: app
                .get_default_seeding_profile_id(&actor)
                .await
                .map_err(to_gql_error)?
                .map(Into::into),
            minimum_seeders_floor,
        })
    }

    /// Create a proxy configuration.
    async fn create_proxy_config(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Proxy provider, base URL, timeout, and enabled state.")]
        input: CreateProxyConfigInput,
    ) -> GqlResult<ProxyConfigPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let provider_type = ProxyProviderType::parse(&input.provider_type).ok_or_else(|| {
            to_gql_error(AppError::Validation(format!(
                "unsupported proxy provider '{}'",
                input.provider_type
            )))
        })?;
        let request_timeout_seconds = input
            .request_timeout_seconds
            .map(|value| {
                u32::try_from(value).map_err(|_| {
                    to_gql_error(AppError::Validation(
                        "request timeout seconds must be positive".into(),
                    ))
                })
            })
            .transpose()?;
        let protocol = parse_challenge_solver_protocol(input.protocol.as_deref())?;
        let config = app
            .create_proxy_config(
                &actor,
                NewProxyConfig {
                    name: input.name,
                    provider_type,
                    protocol,
                    base_url: input.base_url,
                    request_timeout_seconds,
                    is_enabled: input.is_enabled.unwrap_or(true),
                    // Write-only: plaintext in, never echoed back out. The
                    // workflow validates them per provider kind and the store
                    // encrypts at rest.
                    username: input.username,
                    password: input.password,
                    remote_dns: input.remote_dns,
                    private_key: input.private_key,
                    private_key_passphrase: input.private_key_passphrase,
                    peer_public_key: input.peer_public_key,
                    preshared_key: input.preshared_key,
                    tunnel_addresses: input.tunnel_addresses,
                    tunnel_dns_servers: input.tunnel_dns_servers,
                    tunnel_mtu: proxy_link_setting(input.tunnel_mtu, "tunnel MTU")?,
                    tunnel_keepalive_seconds: proxy_link_setting(
                        input.tunnel_keepalive_seconds,
                        "tunnel keepalive seconds",
                    )?,
                },
            )
            .await
            .map_err(to_gql_error)?;
        Ok(from_proxy_config(config))
    }

    /// Patch a proxy configuration while preserving omitted fields.
    async fn update_proxy_config(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Proxy configuration identity and optional replacement fields.")]
        input: UpdateProxyConfigInput,
    ) -> GqlResult<ProxyConfigPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let request_timeout_seconds = input
            .request_timeout_seconds
            .map(|value| {
                u32::try_from(value).map_err(|_| {
                    to_gql_error(AppError::Validation(
                        "request timeout seconds must be positive".into(),
                    ))
                })
            })
            .transpose()?;
        let config = app
            .update_proxy_config(
                &actor,
                ProxyConfigUpdate {
                    id: input.id.to_string(),
                    name: input.name,
                    base_url: input.base_url,
                    request_timeout_seconds,
                    is_enabled: input.is_enabled,
                    // Write-only credentials: an omitted field keeps the stored
                    // secret, an explicit null clears it. Nothing is read back.
                    username: optional_scalar_input(input.username),
                    password: optional_scalar_input(input.password),
                    remote_dns: input.remote_dns,
                    private_key: optional_scalar_input(input.private_key),
                    private_key_passphrase: optional_scalar_input(input.private_key_passphrase),
                    // The peer key is public and not optional, so it has no
                    // cleared state: omission keeps what is stored.
                    peer_public_key: input.peer_public_key,
                    preshared_key: optional_scalar_input(input.preshared_key),
                    tunnel_addresses: input.tunnel_addresses,
                    tunnel_dns_servers: input.tunnel_dns_servers,
                    // Tri-state, but the inner value is a link setting rather
                    // than a secret: null restores the engine's default.
                    tunnel_mtu: optional_proxy_link_setting(input.tunnel_mtu, "tunnel MTU")?,
                    tunnel_keepalive_seconds: optional_proxy_link_setting(
                        input.tunnel_keepalive_seconds,
                        "tunnel keepalive seconds",
                    )?,
                },
            )
            .await
            .map_err(to_gql_error)?;
        Ok(from_proxy_config(config))
    }

    /// Delete a proxy configuration.
    async fn delete_proxy_config(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Proxy configuration identity to delete.")] id: ID,
    ) -> GqlResult<DeleteProxyConfigPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        app.delete_proxy_config(&actor, id.as_ref())
            .await
            .map_err(to_gql_error)?;
        Ok(DeleteProxyConfigPayload { id })
    }

    /// Test a proxy connection and return provider validation details.
    async fn test_proxy_config(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Proxy configuration identity to test.")] id: ID,
    ) -> GqlResult<ProxyTestResultPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let result = app
            .test_proxy_config(&actor, id.as_ref())
            .await
            .map_err(to_gql_error)?;
        Ok(from_proxy_test_result(result))
    }

    /// Forget the pinned tunnel host key so the next connection trusts the server again.
    async fn reset_proxy_host_key(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Tunnel proxy configuration identity whose pinned host key to forget.")]
        id: ID,
    ) -> GqlResult<ProxyConfigPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let config = app
            .reset_proxy_host_key(&actor, id.as_ref())
            .await
            .map_err(to_gql_error)?;
        Ok(from_proxy_config(config))
    }

    /// Delete an indexer configuration.
    async fn delete_indexer_config(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Indexer configuration identity to delete.")] id: ID,
    ) -> GqlResult<DeleteIndexerConfigPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let id = id.to_string();
        app.delete_indexer_config(&actor, &id)
            .await
            .map_err(to_gql_error)?;
        Ok(DeleteIndexerConfigPayload { id: ID::from(id) })
    }

    /// Create a download-client configuration and seed supported routing defaults.
    async fn create_download_client_config(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Download-client provider, configuration values, and enabled state.")]
        input: CreateDownloadClientConfigInput,
    ) -> GqlResult<DownloadClientConfigPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let config_json = provider_config_values_to_json(input.config).map_err(to_gql_error)?;
        let config_json =
            enrich_download_client_config_json(&input.client_type, config_json).await?;
        let config = app
            .create_download_client_config(
                &actor,
                NewDownloadClientConfig {
                    name: input.name,
                    client_type: input.client_type,
                    config_json,
                    client_priority: 0,
                    is_enabled: input.is_enabled.unwrap_or(true),
                    proxy_config_id: input.proxy_config_id.map(|id| id.to_string()),
                },
            )
            .await
            .map_err(to_gql_error)?;

        if should_seed_download_client_routing(&config.client_type) {
            app.ensure_download_client_routing_entry_for_client(&actor, &config.id)
                .await
                .map_err(to_gql_error)?;
        }

        let config_fields = download_client_config_fields(&app, &config.client_type);
        Ok(from_download_client_config_with_fields(
            config,
            &config_fields,
        ))
    }

    /// Patch a download-client configuration while preserving omitted provider secrets.
    async fn update_download_client_config(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Download-client identity and optional replacement fields; omitted provider secrets remain stored."
        )]
        input: UpdateDownloadClientConfigInput,
    ) -> GqlResult<DownloadClientConfigPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let existing = app
            .get_download_client_config(&actor, input.id.as_ref())
            .await
            .map_err(to_gql_error)?
            .ok_or_else(|| {
                to_gql_error(AppError::NotFound(format!(
                    "download client {}",
                    input.id.as_ref()
                )))
            })?;
        let effective_client_type = input
            .client_type
            .as_deref()
            .unwrap_or(existing.client_type.as_str())
            .to_string();
        let effective_config_json = match input.config {
            Some(config) => {
                let config_json = provider_config_values_to_json(config).map_err(to_gql_error)?;
                let config_fields = download_client_config_fields(&app, &effective_client_type);
                let config_json = merge_omitted_provider_secrets(
                    config_json,
                    &existing.config_json,
                    &config_fields,
                )
                .map_err(to_gql_error)?;
                Some(enrich_download_client_config_json(&effective_client_type, config_json).await?)
            }
            None if input
                .client_type
                .as_deref()
                .is_some_and(|value| value.eq_ignore_ascii_case("sabnzbd")) =>
            {
                Some(
                    enrich_download_client_config_json(
                        &effective_client_type,
                        existing.config_json.clone(),
                    )
                    .await?,
                )
            }
            None => None,
        };
        let config = app
            .update_download_client_config(
                &actor,
                DownloadClientConfigUpdate {
                    id: input.id.to_string(),
                    name: input.name,
                    client_type: input.client_type,
                    config_json: effective_config_json,
                    is_enabled: input.is_enabled,
                    proxy_config_id: optional_scalar_input(input.proxy_config_id)
                        .map(|id| id.map(|id| id.to_string())),
                },
            )
            .await
            .map_err(to_gql_error)?;

        if should_seed_download_client_routing(&config.client_type) {
            app.ensure_download_client_routing_entry_for_client(&actor, &config.id)
                .await
                .map_err(to_gql_error)?;
        }

        let config_fields = download_client_config_fields(&app, &config.client_type);
        Ok(from_download_client_config_with_fields(
            config,
            &config_fields,
        ))
    }

    /// Delete a download-client configuration and clear dependent indexer mappings.
    async fn delete_download_client_config(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Download-client configuration identity to delete.")] id: ID,
    ) -> GqlResult<DeleteDownloadClientConfigPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let id = id.to_string();
        let cleared_indexer_mapping_count = app
            .delete_download_client_config(&actor, &id)
            .await
            .map_err(to_gql_error)?;
        Ok(DeleteDownloadClientConfigPayload {
            id: ID::from(id),
            cleared_indexer_mapping_count: i32::try_from(cleared_indexer_mapping_count)
                .unwrap_or(i32::MAX),
        })
    }

    /// Persist the ordering of download-client configurations.
    async fn reorder_download_client_configs(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Download-client identities in their desired order.")]
        input: ReorderDownloadClientConfigsInput,
    ) -> GqlResult<ReorderDownloadClientConfigsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let ids = input.ids;
        let id_strings = ids.iter().map(|id| id.to_string()).collect::<Vec<_>>();
        app.reorder_download_clients(&actor, id_strings)
            .await
            .map_err(to_gql_error)?;
        Ok(ReorderDownloadClientConfigsPayload { ids })
    }

    /// Test a download-client connection using stored or supplied configuration values.
    async fn test_download_client_connection(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Optional stored client identity, provider type, and connection values to test."
        )]
        input: TestDownloadClientConnectionInput,
    ) -> GqlResult<ProviderValidationPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;

        let client_type = input.client_type.trim().to_lowercase();
        let config_json = provider_config_values_to_json(input.config).map_err(to_gql_error)?;
        let config_json = if let Some(id) = input.id {
            let existing = app
                .get_download_client_config(&actor, id.as_ref())
                .await
                .map_err(to_gql_error)?;
            let config_fields = download_client_config_fields(&app, &client_type);
            prepare_download_client_connection_test_config(
                id.as_ref(),
                &client_type,
                config_json,
                existing
                    .as_ref()
                    .map(|config| (config.client_type.as_str(), config.config_json.as_str())),
                &config_fields,
            )
            .map_err(to_gql_error)?
        } else {
            config_json
        };
        app.test_download_client_connection(
            &actor,
            &client_type,
            &config_json,
            input.proxy_config_id.as_ref().map(|id| id.as_str()),
        )
        .await
        .map_err(to_gql_error)?;
        Ok(ProviderValidationPayload {
            status: "ok".to_string(),
            message: None,
            retry_after_seconds: None,
        })
    }

    /// Create a subtitle-provider configuration.
    async fn create_subtitle_provider_config(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Subtitle provider, configuration values, enabled facets, and enabled state."
        )]
        input: CreateSubtitleProviderConfigInput,
    ) -> GqlResult<SubtitleProviderConfigPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let config_json = provider_config_values_to_json(input.config).map_err(to_gql_error)?;
        let config = app
            .create_subtitle_provider_config(
                &actor,
                input.name,
                input.provider_type,
                config_json,
                input.enabled_facets.map(|facets| {
                    facets
                        .into_iter()
                        .map(|facet| facet.as_scope_id().to_string())
                        .collect()
                }),
                input.is_enabled.unwrap_or(true),
            )
            .await
            .map_err(to_gql_error)?;
        let config_fields = app.subtitle_provider_config_fields(&config.provider_type);
        Ok(from_subtitle_provider_config(config, &config_fields))
    }

    /// Patch a subtitle-provider configuration while preserving omitted fields and secrets.
    async fn update_subtitle_provider_config(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Subtitle-provider identity and optional replacement fields; omitted secrets remain stored."
        )]
        input: UpdateSubtitleProviderConfigInput,
    ) -> GqlResult<SubtitleProviderConfigPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let disabled_until = optional_datetime_input(input.disabled_until, "disabled_until")?;
        let config_json = input
            .config
            .map(provider_config_values_to_json)
            .transpose()
            .map_err(to_gql_error)?;
        let config = app
            .update_subtitle_provider_config(
                &actor,
                SubtitleProviderConfigUpdate {
                    id: input.id.to_string(),
                    name: input.name,
                    provider_type: input.provider_type,
                    config_json,
                    enabled_facets: input.enabled_facets.map(|facets| {
                        facets
                            .into_iter()
                            .map(|facet| facet.as_scope_id().to_string())
                            .collect()
                    }),
                    is_enabled: input.is_enabled,
                    last_health_status: None,
                    last_error: None,
                    last_error_at: None,
                    disabled_until,
                },
            )
            .await
            .map_err(to_gql_error)?;
        let config_fields = app.subtitle_provider_config_fields(&config.provider_type);
        Ok(from_subtitle_provider_config(config, &config_fields))
    }

    /// Delete a subtitle-provider configuration.
    async fn delete_subtitle_provider_config(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Subtitle-provider configuration identity to delete.")] id: ID,
    ) -> GqlResult<DeleteSubtitleProviderConfigPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let id = id.to_string();
        app.delete_subtitle_provider_config(&actor, &id)
            .await
            .map_err(to_gql_error)?;
        Ok(DeleteSubtitleProviderConfigPayload { id: ID::from(id) })
    }

    /// Test a subtitle-provider connection and return provider validation details.
    async fn test_subtitle_provider_connection(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Optional stored provider identity, provider type, and connection values to test."
        )]
        input: TestSubtitleProviderConnectionInput,
    ) -> GqlResult<ProviderValidationPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let result = app
            .test_subtitle_provider_connection(
                &actor,
                input.id.as_ref().map(|id| id.as_ref()),
                input.provider_type,
                provider_config_values_to_json(input.config).map_err(to_gql_error)?,
            )
            .await
            .map_err(to_gql_error)?;
        Ok(ProviderValidationPayload {
            status: result.status,
            message: result.message,
            retry_after_seconds: result.retry_after_seconds,
        })
    }

    /// Test an indexer connection and return provider validation details.
    async fn test_indexer_connection(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Provider type, optional values, optional indexer identity, and tri-state proxy identity."
        )]
        input: TestIndexerConnectionInput,
    ) -> GqlResult<ProviderValidationPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let config_json = input
            .config
            .map(provider_config_values_to_json)
            .transpose()
            .map_err(to_gql_error)?;

        app.test_indexer_connection(
            &actor,
            &input.provider_type,
            config_json.as_deref(),
            input.indexer_id.as_ref().map(|id| id.as_ref()),
            match &input.proxy_config_id {
                async_graphql::MaybeUndefined::Undefined => None,
                async_graphql::MaybeUndefined::Null => Some(None),
                async_graphql::MaybeUndefined::Value(id) => Some(Some(id.as_ref())),
            },
        )
        .await
        .map_err(to_gql_error)?;
        Ok(ProviderValidationPayload {
            status: "ok".to_string(),
            message: None,
            retry_after_seconds: None,
        })
    }

    /// Synchronize an indexer configuration with its provider and return the sync result.
    async fn sync_indexer_config(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Indexer configuration identity to synchronize.")] id: ID,
    ) -> GqlResult<IndexerConfigSyncPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let id = id.to_string();
        let result = app
            .sync_indexer_config(&actor, &id)
            .await
            .map_err(to_gql_error)?;
        Ok(from_indexer_config_sync_result(result))
    }

    /// Run RSS synchronization for accessible indexers and return its report.
    async fn trigger_rss_sync(&self, ctx: &Context<'_>) -> GqlResult<RssSyncReportPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let report = app.run_rss_sync(&actor).await.map_err(to_gql_error)?;
        Ok(from_rss_sync_report(report))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn enrich_download_client_config_json_leaves_sab_config_unchanged() {
        let config_json = enrich_download_client_config_json(
            "sabnzbd",
            r#"{"host":"127.0.0.1","port":"8080","use_ssl":false,"api_key":"test-api-key"}"#
                .to_string(),
        )
        .await
        .expect("config enrichment should succeed");

        assert_eq!(
            config_json,
            r#"{"host":"127.0.0.1","port":"8080","use_ssl":false,"api_key":"test-api-key"}"#
        );
    }

    #[tokio::test]
    async fn enrich_download_client_config_json_leaves_other_client_config_unchanged() {
        let config_json = enrich_download_client_config_json(
            "weaver",
            r#"{"host":"127.0.0.1","port":"8081"}"#.to_string(),
        )
        .await
        .expect("config enrichment should succeed");

        assert_eq!(config_json, r#"{"host":"127.0.0.1","port":"8081"}"#);
    }

    fn secret_field(key: &str) -> scryer_domain::ConfigFieldDef {
        scryer_domain::ConfigFieldDef {
            key: key.to_string(),
            label: key.to_string(),
            field_type: scryer_domain::ConfigFieldType::Password,
            required: false,
            default_value: None,
            value_source: Default::default(),
            role: None,
            host_binding: None,
            options: vec![],
            help_text: None,
        }
    }

    #[test]
    fn proxy_protocol_input_parses_or_names_the_bad_value() {
        assert_eq!(parse_challenge_solver_protocol(None).unwrap(), None);
        assert_eq!(
            parse_challenge_solver_protocol(Some("request_solution_v1")).unwrap(),
            Some(scryer_domain::ChallengeSolverProtocol::RequestSolutionV1)
        );
        // Casing and dashes normalize, matching the domain parser.
        assert_eq!(
            parse_challenge_solver_protocol(Some("Request-Solution-V1")).unwrap(),
            Some(scryer_domain::ChallengeSolverProtocol::RequestSolutionV1)
        );

        let error = parse_challenge_solver_protocol(Some("socks5"))
            .expect_err("an unknown protocol must be rejected");
        assert!(
            error.message.contains("socks5"),
            "error should name the rejected value: {}",
            error.message
        );
    }

    #[test]
    fn proxy_credential_patches_distinguish_omitted_from_cleared() {
        // Write-only convention: omitted keeps the stored secret, explicit null
        // clears it, a value replaces it.
        assert_eq!(
            optional_scalar_input(MaybeUndefined::<String>::Undefined),
            None
        );
        assert_eq!(
            optional_scalar_input(MaybeUndefined::<String>::Null),
            Some(None)
        );
        assert_eq!(
            optional_scalar_input(MaybeUndefined::Value("operator".to_string())),
            Some(Some("operator".to_string()))
        );
    }

    #[test]
    fn connection_test_merge_reuses_omitted_saved_secrets() {
        let merged = merge_omitted_provider_secrets(
            r#"{"host":"localhost"}"#.to_string(),
            r#"{"host":"localhost","api_key":"saved-key","password":"saved-password"}"#,
            &[secret_field("api_key"), secret_field("password")],
        )
        .expect("merge saved secrets");
        let merged: serde_json::Value = serde_json::from_str(&merged).unwrap();

        assert_eq!(merged["api_key"], "saved-key");
        assert_eq!(merged["password"], "saved-password");
    }

    #[test]
    fn connection_test_merge_respects_explicit_secret_clear() {
        let merged = merge_omitted_provider_secrets(
            r#"{"api_key":null}"#.to_string(),
            r#"{"api_key":"saved-key"}"#,
            &[secret_field("api_key")],
        )
        .expect("merge explicit clear");
        let merged: serde_json::Value = serde_json::from_str(&merged).unwrap();

        assert!(merged["api_key"].is_null());
    }

    #[test]
    fn connection_test_with_saved_client_reuses_omitted_secrets() {
        let merged = prepare_download_client_connection_test_config(
            "client-1",
            "qbittorrent",
            r#"{"host":"localhost"}"#.to_string(),
            Some((
                "qBittorrent",
                r#"{"host":"localhost","api_key":"saved-key"}"#,
            )),
            &[secret_field("api_key")],
        )
        .expect("saved client config should merge");
        let merged: serde_json::Value = serde_json::from_str(&merged).unwrap();

        assert_eq!(merged["api_key"], "saved-key");
    }

    #[test]
    fn connection_test_with_missing_client_id_is_rejected() {
        let error = prepare_download_client_connection_test_config(
            "missing",
            "qbittorrent",
            "{}".to_string(),
            None,
            &[secret_field("api_key")],
        )
        .expect_err("missing client should fail");

        assert!(
            matches!(error, AppError::NotFound(message) if message == "download client missing")
        );
    }

    #[test]
    fn connection_test_with_mismatched_client_type_is_rejected() {
        let error = prepare_download_client_connection_test_config(
            "client-1",
            "qbittorrent",
            "{}".to_string(),
            Some(("sabnzbd", "{}")),
            &[secret_field("api_key")],
        )
        .expect_err("mismatched client should fail");

        assert!(
            matches!(error, AppError::Validation(message) if message.contains("uses provider type 'sabnzbd', not 'qbittorrent'"))
        );
    }
}

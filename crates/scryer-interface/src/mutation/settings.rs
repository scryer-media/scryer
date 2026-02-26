use async_graphql::{Context, Error, Object, Result as GqlResult};
use chrono::Utc;
use scryer_application::QUALITY_PROFILE_CATALOG_KEY;
use scryer_domain::Entitlement;
use serde_json::json;

use crate::context::{actor_from_ctx, app_from_ctx, settings_db_from_ctx, to_gql_error};
use crate::mappers::{from_tvdb_scan_operation, from_user, map_admin_setting};
use crate::quality_profiles::{merge_quality_profiles, parse_profile_catalog_from_json};
use crate::types::*;

#[derive(Default)]
pub(crate) struct SettingsMutations;

#[Object]
impl SettingsMutations {
    async fn save_admin_settings(
        &self,
        ctx: &Context<'_>,
        input: AdminSettingsUpdateInput,
    ) -> GqlResult<AdminSettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
        let db = settings_db_from_ctx(ctx)?;

        let scope = input.scope.trim();
        if scope.is_empty() {
            return Err(Error::new("scope is required"));
        }
        if input.items.is_empty() {
            return Err(Error::new("at least one setting update item is required"));
        }

        let mut profile_catalog_update: Option<(String, Option<String>, Vec<scryer_application::QualityProfile>)> = None;
        let mut updated_keys = Vec::with_capacity(input.items.len());
        let mut quality_profiles_json: Option<String> = None;
        for item in input.items {
            let key_name = item.key_name.trim();
            if key_name.is_empty() {
                return Err(Error::new("key_name is required"));
            }
            if !updated_keys.iter().any(|key| key == key_name) {
                updated_keys.push(key_name.to_string());
            }

            if key_name == QUALITY_PROFILE_CATALOG_KEY {
                let parsed_profiles = parse_profile_catalog_from_json(&item.value).map_err(|error| {
                    Error::new(format!(
                        "invalid quality profile catalog JSON for {QUALITY_PROFILE_CATALOG_KEY}: {error}"
                    ))
                })?;
                profile_catalog_update =
                    Some((scope.to_string(), input.scope_id.clone(), parsed_profiles));
                continue;
            }

            db.upsert_setting_value(
                scope.to_string(),
                key_name.to_string(),
                input.scope_id.clone(),
                item.value,
                "admin_graphql",
                Some(actor.id.clone()),
            )
            .await
            .map_err(to_gql_error)?;
        }

        if let Some((profile_scope, profile_scope_id, profiles)) = profile_catalog_update {
            let existing_profiles = app
                .services
                .quality_profiles
                .list_quality_profiles(profile_scope.as_str(), profile_scope_id.clone())
                .await
                .map_err(to_gql_error)?;
            let merged_profiles = merge_quality_profiles(existing_profiles, profiles.clone());
            let profile_catalog_text =
                serde_json::to_string(&merged_profiles).map_err(|error| {
                    Error::new(format!("failed to encode quality profiles: {error}"))
                })?;
            quality_profiles_json = Some(profile_catalog_text.clone());

            db.upsert_quality_profiles(&profile_scope, profile_scope_id.clone(), profiles)
                .await
                .map_err(|error| {
                    Error::new(format!("failed to persist quality profiles: {error}"))
                })?;
            db.upsert_setting_value(
                profile_scope,
                QUALITY_PROFILE_CATALOG_KEY,
                profile_scope_id,
                profile_catalog_text,
                "admin_graphql",
                Some(actor.id.clone()),
            )
            .await
            .map_err(|error| {
                Error::new(format!(
                    "failed to persist quality profile catalog setting {QUALITY_PROFILE_CATALOG_KEY}: {error}"
                ))
            })?;
        }

        let _ = app
            .services
            .record_activity_event(
                Some(actor.id.clone()),
                None,
                scryer_application::ActivityKind::SettingSaved,
                format!(
                    "settings saved in scope '{scope}' ({})",
                    updated_keys.join(", ")
                ),
                scryer_application::ActivitySeverity::Success,
                vec![
                    scryer_application::ActivityChannel::Toast,
                    scryer_application::ActivityChannel::WebUi,
                ],
            )
            .await;

        let scope_name = scope.to_string();
        let items = db
            .list_settings_with_defaults(scope_name.clone(), input.scope_id.clone())
            .await
            .map_err(to_gql_error)?
            .into_iter()
            .map(map_admin_setting)
            .collect();

        Ok(AdminSettingsPayload {
            scope: scope_name,
            scope_id: input.scope_id,
            items,
            quality_profiles: quality_profiles_json,
        })
    }

    async fn queue_tvdb_movies_scan(
        &self,
        ctx: &Context<'_>,
        input: QueueTvdbMoviesScanInput,
    ) -> GqlResult<TvdbScanOperationPayload> {
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
        let db = settings_db_from_ctx(ctx)?;

        let limit = if input.limit > 0 {
            input.limit
        } else {
            return Err(Error::new("limit is required and must be greater than zero"));
        };

        let source = input.source.trim();
        if source.is_empty() {
            return Err(Error::new("source is required"));
        }

        let progress_json = json!({
            "type": "tvdb_movies_scan",
            "limit": limit,
            "source": source,
        })
        .to_string();

        let operation = db
            .create_workflow_operation(
                "tvdb_movies_scan",
                "queued",
                Some(actor.id),
                Some(progress_json),
                None,
                None,
            )
            .await
            .map_err(to_gql_error)?;

        Ok(from_tvdb_scan_operation(
            operation,
            limit,
            source.to_string(),
        ))
    }

    async fn login(
        &self,
        ctx: &Context<'_>,
        input: LoginInput,
    ) -> GqlResult<LoginPayload> {
        let app = app_from_ctx(ctx)?;
        let user = app
            .authenticate_credentials(&input.username, &input.password)
            .await
            .map_err(to_gql_error)?;
        let token = app.issue_access_token(&user).map_err(to_gql_error)?;
        let expires_at = (Utc::now() + chrono::Duration::seconds(app.token_lifetime()))
            .to_rfc3339();
        Ok(LoginPayload {
            token,
            user: from_user(user),
            expires_at,
        })
    }

    /// Issue a JWT for the default admin user without credentials.
    /// Only available when SCRYER_DEV_AUTO_LOGIN=true.
    async fn dev_auto_login(&self, ctx: &Context<'_>) -> GqlResult<LoginPayload> {
        let api_ctx = ctx.data_unchecked::<crate::context::ApiContext>();
        if !api_ctx.dev_auto_login {
            return Err(Error::new("dev auto-login is not enabled"));
        }
        let app = &api_ctx.app;
        let user = app
            .find_or_create_default_user()
            .await
            .map_err(to_gql_error)?;
        let token = app.issue_access_token(&user).map_err(to_gql_error)?;
        let expires_at = (Utc::now() + chrono::Duration::seconds(app.token_lifetime()))
            .to_rfc3339();
        Ok(LoginPayload {
            token,
            user: from_user(user),
            expires_at,
        })
    }

}

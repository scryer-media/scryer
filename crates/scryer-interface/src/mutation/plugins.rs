use async_graphql::{Context, Object, Result as GqlResult};

use crate::context::{actor_from_ctx, app_from_ctx, to_gql_error};
use crate::mappers::{from_plugin_installation, from_registry_plugin};
use crate::types::*;

#[derive(Default)]
pub(crate) struct PluginMutations;

#[Object]
impl PluginMutations {
    async fn refresh_plugin_registry(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Vec<RegistryPluginPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let plugins = app
            .refresh_plugin_registry(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(plugins.into_iter().map(from_registry_plugin).collect())
    }

    async fn install_plugin(
        &self,
        ctx: &Context<'_>,
        input: InstallPluginInput,
    ) -> GqlResult<PluginInstallationPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let installation = app
            .install_plugin(&actor, &input.plugin_id)
            .await
            .map_err(to_gql_error)?;
        Ok(from_plugin_installation(installation))
    }

    async fn uninstall_plugin(
        &self,
        ctx: &Context<'_>,
        input: UninstallPluginInput,
    ) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.uninstall_plugin(&actor, &input.plugin_id)
            .await
            .map_err(to_gql_error)?;
        Ok(true)
    }

    async fn toggle_plugin(
        &self,
        ctx: &Context<'_>,
        input: TogglePluginInput,
    ) -> GqlResult<PluginInstallationPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let installation = app
            .toggle_plugin(&actor, &input.plugin_id, input.enabled)
            .await
            .map_err(to_gql_error)?;
        Ok(from_plugin_installation(installation))
    }

    async fn upgrade_plugin(
        &self,
        ctx: &Context<'_>,
        input: UpgradePluginInput,
    ) -> GqlResult<PluginInstallationPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let installation = app
            .upgrade_plugin(&actor, &input.plugin_id)
            .await
            .map_err(to_gql_error)?;
        Ok(from_plugin_installation(installation))
    }
}

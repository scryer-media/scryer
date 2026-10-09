use async_graphql::{Context, ID, Object, Result as GqlResult};
use scryer_domain::AppPermission;

use scryer_interface_core::{app_from_ctx, require_config_app_permission, to_gql_error};
use scryer_interface_media::mappers::{
    from_manual_plugin_preview, from_plugin_install_progress, from_plugin_installation,
    from_registry_plugin,
};
use scryer_interface_media::types::*;

#[derive(Default)]
pub struct PluginMutations;

#[Object]
impl PluginMutations {
    /// Replace or clear explicitly supplied installation-wide settings.
    async fn update_installed_plugin_settings(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Installed plugin ID whose settings are changed.")] plugin_id: ID,
        #[graphql(
            desc = "Object mapping declared setting keys to a new value, or null to clear the stored value; omitted keys are unchanged and unknown keys are rejected."
        )]
        changes: async_graphql::Json<std::collections::BTreeMap<String, Option<String>>>,
    ) -> GqlResult<async_graphql::Json<scryer_application::PluginSettingsView>> {
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        app_from_ctx(ctx)?
            .update_installed_plugin_settings(&actor, plugin_id.as_str(), changes.0)
            .await
            .map(async_graphql::Json)
            .map_err(to_gql_error)
    }

    /// Refresh the configured plugin registry catalogs and return the available plugins.
    async fn refresh_plugin_catalog(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Vec<RegistryPluginPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let plugins = app
            .refresh_plugin_catalog(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(plugins.into_iter().map(from_registry_plugin).collect())
    }

    /// Start background installation of a catalog plugin and return the initial progress snapshot.
    async fn begin_install_plugin(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Catalog plugin ID to install.")] plugin_id: ID,
    ) -> GqlResult<PluginInstallProgressPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let plugin_id = String::from(plugin_id);
        let snapshot = app
            .begin_install_plugin(&actor, &plugin_id)
            .await
            .map_err(to_gql_error)?;
        Ok(from_plugin_install_progress(snapshot))
    }

    /// Uninstall a plugin and remove its installation record.
    async fn uninstall_plugin(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Installed plugin ID to remove.")] plugin_id: ID,
    ) -> GqlResult<UninstallPluginPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let plugin_id = String::from(plugin_id);
        app.uninstall_plugin(&actor, &plugin_id)
            .await
            .map_err(to_gql_error)?;
        Ok(UninstallPluginPayload {
            plugin_id: ID::from(plugin_id),
        })
    }

    /// Enable or disable an installed plugin.
    async fn toggle_plugin(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Installed plugin ID and desired enabled state.")]
        input: TogglePluginInput,
    ) -> GqlResult<PluginInstallationPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let installation = app
            .toggle_plugin(&actor, input.plugin_id.as_ref(), input.enabled)
            .await
            .map_err(to_gql_error)?;
        Ok(from_plugin_installation(installation))
    }

    /// Start background upgrade of an installed catalog plugin and return the initial progress snapshot.
    async fn begin_upgrade_plugin(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Installed plugin ID to upgrade.")] plugin_id: ID,
    ) -> GqlResult<PluginInstallProgressPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let plugin_id = String::from(plugin_id);
        let snapshot = app
            .begin_upgrade_plugin(&actor, &plugin_id)
            .await
            .map_err(to_gql_error)?;
        Ok(from_plugin_install_progress(snapshot))
    }

    /// Inspect a GitHub plugin repository without installing its artifact.
    async fn inspect_manual_plugin_repo(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "GitHub repository URL containing the plugin manifest and artifact.")]
        input: ManualPluginRepoInput,
    ) -> GqlResult<ManualPluginPreviewPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let preview = app
            .inspect_manual_plugin_repo(&actor, &input.github_repo_url)
            .await
            .map_err(to_gql_error)?;
        Ok(from_manual_plugin_preview(preview))
    }

    /// Install a plugin directly from an inspected GitHub repository.
    async fn install_manual_plugin(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "GitHub repository URL containing the plugin manifest and artifact.")]
        input: ManualPluginRepoInput,
    ) -> GqlResult<PluginInstallationPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let installation = app
            .install_manual_plugin(&actor, &input.github_repo_url)
            .await
            .map_err(to_gql_error)?;
        Ok(from_plugin_installation(installation))
    }

    /// Install a base64-encoded WebAssembly plugin after explicit risk acknowledgement.
    async fn install_uploaded_plugin(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Plugin filename, base64-encoded WebAssembly bytes, and required risk acknowledgement."
        )]
        input: ManualPluginUploadInput,
    ) -> GqlResult<PluginInstallationPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let installation = app
            .install_uploaded_plugin(
                &actor,
                &input.file_name,
                &input.wasm_base64,
                input.acknowledge_risk,
            )
            .await
            .map_err(to_gql_error)?;
        Ok(from_plugin_installation(installation))
    }
}

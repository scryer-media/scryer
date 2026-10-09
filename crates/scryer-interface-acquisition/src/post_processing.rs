use async_graphql::{Context, ID, MaybeUndefined, Object, Result as GqlResult};
use chrono::Utc;
use scryer_application::{AppError, UpdateScriptInterpreterSettings};
use scryer_domain::{AppPermission, ExecutionMode, Id, PostProcessingScript, ScriptType};
use std::path::Path;

use scryer_interface_core::{app_from_ctx, require_config_app_permission, to_gql_error};
use scryer_interface_media::{mappers, types::*};

#[derive(Default)]
pub struct PostProcessingMutations;

fn parse_script_type(value: &str) -> GqlResult<ScriptType> {
    ScriptType::parse(value).ok_or_else(|| {
        to_gql_error(AppError::Validation(format!(
            "invalid post-processing script type: {value}"
        )))
    })
}

fn parse_execution_mode(value: Option<ExecutionModeValue>) -> ExecutionMode {
    value.map(ExecutionMode::from).unwrap_or_default()
}

fn require_inline_shell_acknowledgement(acknowledged: Option<bool>) -> GqlResult<()> {
    if acknowledged == Some(true) {
        return Ok(());
    }
    Err(to_gql_error(AppError::Validation(
        "inline shell scripts require explicit acknowledgement".into(),
    )))
}

fn validate_file_script_content(script_content: &str) -> GqlResult<()> {
    let path = script_content.trim();
    if path.is_empty() {
        return Err(to_gql_error(AppError::Validation(
            "file post-processing script path is required".into(),
        )));
    }
    if !Path::new(path).is_absolute() {
        return Err(to_gql_error(AppError::Validation(
            "file post-processing script path must be absolute".into(),
        )));
    }
    Ok(())
}

#[Object]
impl PostProcessingMutations {
    /// Create an enabled post-processing script after validating its type and content.
    async fn create_post_processing_script(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Script definition, execution settings, and required acknowledgement for inline shell code."
        )]
        input: CreatePostProcessingScriptInput,
    ) -> GqlResult<PostProcessingScriptPayload> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, AppPermission::ManageCatalogSettings).await?;

        let script_type = parse_script_type(&input.script_type)?;
        let script_content = input.script_content.unwrap_or_default();
        if script_type == ScriptType::Inline {
            require_inline_shell_acknowledgement(input.inline_shell_acknowledged)?;
        } else {
            validate_file_script_content(&script_content)?;
        }
        let execution_mode = parse_execution_mode(input.execution_mode);

        let now = Utc::now();
        let script = PostProcessingScript {
            id: Id::new().0,
            name: input.name,
            description: input.description.unwrap_or_default(),
            script_type,
            script_content,
            applied_facets: input.applied_facets.unwrap_or_default(),
            execution_mode,
            timeout_secs: input.timeout_secs.map(|v| v as i64).unwrap_or(300),
            priority: input.priority.unwrap_or(0),
            enabled: true,
            debug: input.debug.unwrap_or(false),
            created_at: now,
            updated_at: now,
        };

        let created = app
            .create_post_processing_script(&actor, script)
            .await
            .map_err(to_gql_error)?;

        Ok(mappers::from_pp_script(created))
    }

    /// Update selected fields of a post-processing script and revalidate changed executable content.
    async fn update_post_processing_script(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Script identity and optional fields to replace; omitted fields retain their values."
        )]
        input: UpdatePostProcessingScriptInput,
    ) -> GqlResult<PostProcessingScriptPayload> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, AppPermission::ManageCatalogSettings).await?;
        let script_id = input.id.to_string();

        let mut script = app
            .get_post_processing_script(&actor, &script_id)
            .await
            .map_err(to_gql_error)?
            .ok_or_else(|| to_gql_error(AppError::NotFound(format!("script {script_id}"))))?;

        let previous_script_type = script.script_type;
        let previous_script_content = script.script_content.clone();
        let previous_enabled = script.enabled;
        let next_script_type = match input.script_type.as_deref() {
            Some(value) => Some(parse_script_type(value)?),
            None => None,
        };
        let next_execution_mode = input.execution_mode.map(ExecutionMode::from);

        if let Some(name) = input.name {
            script.name = name;
        }
        if let Some(description) = input.description {
            script.description = description;
        }
        if let Some(script_type) = next_script_type {
            script.script_type = script_type;
        }
        if let Some(script_content) = input.script_content {
            script.script_content = script_content;
        }
        if let Some(applied_facets) = input.applied_facets {
            script.applied_facets = applied_facets;
        }
        if let Some(execution_mode) = next_execution_mode {
            script.execution_mode = execution_mode;
        }
        if let Some(timeout_secs) = input.timeout_secs {
            script.timeout_secs = timeout_secs as i64;
        }
        if let Some(priority) = input.priority {
            script.priority = priority;
        }
        if let Some(enabled) = input.enabled {
            script.enabled = enabled;
        }
        if let Some(debug) = input.debug {
            script.debug = debug;
        }

        let inline_transition =
            previous_script_type != ScriptType::Inline && script.script_type == ScriptType::Inline;
        let inline_content_changed = script.script_type == ScriptType::Inline
            && script.script_content != previous_script_content;
        let inline_enabled =
            script.script_type == ScriptType::Inline && !previous_enabled && script.enabled;
        if inline_transition || inline_content_changed || inline_enabled {
            require_inline_shell_acknowledgement(input.inline_shell_acknowledged)?;
        }
        if script.script_type == ScriptType::File {
            validate_file_script_content(&script.script_content)?;
        }

        script.updated_at = Utc::now();

        let updated = app
            .update_post_processing_script(&actor, script)
            .await
            .map_err(to_gql_error)?;

        Ok(mappers::from_pp_script(updated))
    }

    /// Delete a post-processing script; this requires catalog-settings permission.
    async fn delete_post_processing_script(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Script identity to delete.")] id: ID,
    ) -> GqlResult<DeletePostProcessingScriptPayload> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, AppPermission::ManageCatalogSettings).await?;

        let id_string = id.to_string();
        app.delete_post_processing_script(&actor, &id_string)
            .await
            .map_err(to_gql_error)?;

        Ok(DeletePostProcessingScriptPayload { id })
    }

    /// Enable or disable a post-processing script, requiring acknowledgement when enabling inline shell code.
    async fn toggle_post_processing_script(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Script identity to toggle.")] id: ID,
        #[graphql(desc = "Must be true when enabling an inline shell script; ignored otherwise.")]
        inline_shell_acknowledged: Option<bool>,
    ) -> GqlResult<PostProcessingScriptPayload> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, AppPermission::ManageCatalogSettings).await?;
        let id = id.to_string();
        let script = app
            .get_post_processing_script(&actor, &id)
            .await
            .map_err(to_gql_error)?
            .ok_or_else(|| to_gql_error(AppError::NotFound(format!("script {id}"))))?;
        if script.script_type == ScriptType::Inline && !script.enabled {
            require_inline_shell_acknowledgement(inline_shell_acknowledged)?;
        }

        let updated = app
            .toggle_post_processing_script(&actor, &id)
            .await
            .map_err(to_gql_error)?;

        Ok(mappers::from_pp_script(updated))
    }

    /// Replace the interpreters operator scripts are launched with; null or an empty string restores the default command.
    async fn update_script_interpreter_settings(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Interpreter path or command for each script language.")]
        input: ScriptInterpreterSettingsInput,
    ) -> GqlResult<ScriptInterpreterSettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;

        let settings = app
            .update_script_interpreter_settings(
                &actor,
                UpdateScriptInterpreterSettings {
                    python: pin_change(input.python),
                    powershell: pin_change(input.powershell),
                    batch: pin_change(input.batch),
                    go: pin_change(input.go),
                },
            )
            .await
            .map_err(to_gql_error)?;

        Ok(mappers::from_script_interpreter_config(settings))
    }
}

/// Undefined keeps the current pin, null clears it, a value replaces it.
fn pin_change(value: MaybeUndefined<String>) -> Option<Option<String>> {
    match value {
        MaybeUndefined::Undefined => None,
        MaybeUndefined::Null => Some(None),
        MaybeUndefined::Value(value) => Some(Some(value)),
    }
}

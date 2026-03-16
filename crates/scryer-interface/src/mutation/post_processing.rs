use async_graphql::{Context, Object, Result as GqlResult};
use chrono::Utc;
use scryer_domain::{Id, PostProcessingScript};

use crate::context::{app_from_ctx, to_gql_error};
use crate::types::*;

#[derive(Default)]
pub(crate) struct PostProcessingMutations;

#[Object]
impl PostProcessingMutations {
    async fn create_post_processing_script(
        &self,
        ctx: &Context<'_>,
        input: CreatePostProcessingScriptInput,
    ) -> GqlResult<PostProcessingScriptPayload> {
        let app = app_from_ctx(ctx)?;

        let now = Utc::now();
        let script = PostProcessingScript {
            id: Id::new().0,
            name: input.name,
            description: input.description.unwrap_or_default(),
            script_type: input.script_type.unwrap_or_else(|| "inline".to_string()),
            script_content: input.script_content.unwrap_or_default(),
            applied_facets: input.applied_facets.unwrap_or_default(),
            execution_mode: input
                .execution_mode
                .unwrap_or_else(|| "blocking".to_string()),
            timeout_secs: input.timeout_secs.map(|v| v as i64).unwrap_or(300),
            priority: input.priority.unwrap_or(0),
            enabled: true,
            debug: input.debug.unwrap_or(false),
            created_at: now,
            updated_at: now,
        };

        let created = app
            .services
            .pp_scripts
            .create_script(script)
            .await
            .map_err(to_gql_error)?;

        Ok(crate::mappers::from_pp_script(created))
    }

    async fn update_post_processing_script(
        &self,
        ctx: &Context<'_>,
        input: UpdatePostProcessingScriptInput,
    ) -> GqlResult<PostProcessingScriptPayload> {
        let app = app_from_ctx(ctx)?;

        let mut script = app
            .services
            .pp_scripts
            .get_script(&input.id)
            .await
            .map_err(to_gql_error)?
            .ok_or_else(|| async_graphql::Error::new(format!("script {} not found", input.id)))?;

        if let Some(name) = input.name {
            script.name = name;
        }
        if let Some(description) = input.description {
            script.description = description;
        }
        if let Some(script_type) = input.script_type {
            script.script_type = script_type;
        }
        if let Some(script_content) = input.script_content {
            script.script_content = script_content;
        }
        if let Some(applied_facets) = input.applied_facets {
            script.applied_facets = applied_facets;
        }
        if let Some(execution_mode) = input.execution_mode {
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
        script.updated_at = Utc::now();

        let updated = app
            .services
            .pp_scripts
            .update_script(script)
            .await
            .map_err(to_gql_error)?;

        Ok(crate::mappers::from_pp_script(updated))
    }

    async fn delete_post_processing_script(
        &self,
        ctx: &Context<'_>,
        id: String,
    ) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;

        app.services
            .pp_scripts
            .delete_script(&id)
            .await
            .map_err(to_gql_error)?;

        Ok(true)
    }

    async fn toggle_post_processing_script(
        &self,
        ctx: &Context<'_>,
        id: String,
    ) -> GqlResult<PostProcessingScriptPayload> {
        let app = app_from_ctx(ctx)?;

        let mut script = app
            .services
            .pp_scripts
            .get_script(&id)
            .await
            .map_err(to_gql_error)?
            .ok_or_else(|| async_graphql::Error::new(format!("script {id} not found")))?;

        script.enabled = !script.enabled;
        script.updated_at = Utc::now();

        let updated = app
            .services
            .pp_scripts
            .update_script(script)
            .await
            .map_err(to_gql_error)?;

        Ok(crate::mappers::from_pp_script(updated))
    }
}

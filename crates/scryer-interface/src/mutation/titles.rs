use async_graphql::{Context, Object, Result as GqlResult};

use crate::context::{actor_from_ctx, app_from_ctx, to_gql_error};
use crate::mappers::from_title;
use crate::types::*;
use crate::utils::{map_add_input, parse_download_source_kind, parse_facet};

#[derive(Default)]
pub(crate) struct TitleMutations;

#[Object]
impl TitleMutations {
    async fn add_title(
        &self,
        ctx: &Context<'_>,
        input: AddTitleInput,
    ) -> GqlResult<AddTitleResult> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let domain_title = app
            .add_title(&actor, map_add_input(input))
            .await
            .map_err(to_gql_error)?;

        Ok(AddTitleResult {
            title: from_title(domain_title),
            download_job_id: String::new(),
        })
    }

    async fn add_title_and_queue_download(
        &self,
        ctx: &Context<'_>,
        input: AddTitleInput,
    ) -> GqlResult<AddTitleResult> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let source_hint = input.source_hint.clone();
        let source_kind = parse_download_source_kind(input.source_kind.clone());
        let source_title = input.source_title.clone();
        let (title, job_id) = app
            .add_title_and_queue_download(
                &actor,
                map_add_input(input),
                source_hint,
                source_kind,
                source_title,
            )
            .await
            .map_err(to_gql_error)?;

        Ok(AddTitleResult {
            title: from_title(title),
            download_job_id: job_id,
        })
    }

    async fn update_title(
        &self,
        ctx: &Context<'_>,
        input: UpdateTitleInput,
    ) -> GqlResult<TitlePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let facet = input.facet.and_then(|value| parse_facet(Some(value)));
        let tags = input.tags.map(|tags| {
            tags.into_iter()
                .map(|tag| {
                    let trimmed = tag.trim().to_string();
                    // Preserve case for structured scryer: tags (they may contain paths)
                    if trimmed.starts_with("scryer:") {
                        trimmed
                    } else {
                        trimmed.to_lowercase()
                    }
                })
                .filter(|tag| !tag.is_empty())
                .collect::<Vec<_>>()
        });

        let title = app
            .update_title_metadata(&actor, &input.title_id, input.name, facet, tags)
            .await
            .map_err(to_gql_error)?;
        Ok(from_title(title))
    }

    async fn delete_title(&self, ctx: &Context<'_>, input: DeleteTitleInput) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.delete_title(
            &actor,
            &input.title_id,
            input.delete_files_on_disk.unwrap_or(false),
        )
        .await
        .map(|_| true)
        .map_err(to_gql_error)
    }

    async fn set_title_monitored(
        &self,
        ctx: &Context<'_>,
        input: SetTitleMonitoredInput,
    ) -> GqlResult<TitlePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let title = app
            .set_title_monitored(&actor, &input.title_id, input.monitored)
            .await
            .map_err(to_gql_error)?;
        Ok(from_title(title))
    }
}

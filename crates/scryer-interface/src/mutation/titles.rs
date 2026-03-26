use async_graphql::{Context, Error, Object, Result as GqlResult};

use crate::context::{actor_from_ctx, app_from_ctx, to_gql_error};
use crate::mappers::from_title;
use crate::types::*;
use crate::utils::{
    map_add_input, merge_title_option_tags, normalize_title_tags, parse_download_source_kind,
};

#[derive(Default)]
pub(crate) struct TitleMutations;

fn queued_download_payload(
    title: &scryer_domain::Title,
    job_id: String,
    source_title: Option<String>,
    source_kind: Option<scryer_application::DownloadSourceKind>,
) -> QueueDownloadPayload {
    QueueDownloadPayload {
        job_id,
        title_id: title.id.clone(),
        title_name: title.name.clone(),
        source_title,
        source_kind: source_kind.map(DownloadSourceKindValue::from_application),
    }
}

#[Object]
impl TitleMutations {
    async fn add_title(
        &self,
        ctx: &Context<'_>,
        input: AddTitleInput,
    ) -> GqlResult<AddTitleResult> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let request = map_add_input(input)?;
        let domain_title = app
            .add_title(&actor, request)
            .await
            .map_err(to_gql_error)?;

        Ok(AddTitleResult {
            title: from_title(domain_title),
            download_job_id: None,
            queued_download: None,
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
        let request = map_add_input(input)?;
        let (title, job_id) = app
            .add_title_and_queue_download(
                &actor,
                request,
                source_hint,
                source_kind,
                source_title.clone(),
            )
            .await
            .map_err(to_gql_error)?;
        let queued_download = queued_download_payload(&title, job_id.clone(), source_title, source_kind);

        Ok(AddTitleResult {
            title: from_title(title),
            download_job_id: Some(job_id),
            queued_download: Some(queued_download),
        })
    }

    async fn update_title(
        &self,
        ctx: &Context<'_>,
        input: UpdateTitleInput,
    ) -> GqlResult<TitlePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let UpdateTitleInput {
            title_id,
            name,
            facet,
            tags,
            options,
        } = input;
        let facet = facet.map(MediaFacetValue::into_domain);
        let mut tags = tags.map(normalize_title_tags);

        if let Some(options) = options {
            let base_tags = match tags.take() {
                Some(tags) => tags,
                None => app
                    .services
                    .titles
                    .get_by_id(&title_id)
                    .await
                    .map_err(to_gql_error)?
                    .map(|title| title.tags)
                    .ok_or_else(|| Error::new(format!("title not found: {title_id}")))?,
            };
            tags = Some(merge_title_option_tags(base_tags, options));
        }

        let title = app
            .update_title_metadata(&actor, &title_id, name, facet, tags)
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

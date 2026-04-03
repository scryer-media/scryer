use async_graphql::{Context, Object, Result as GqlResult};

use crate::context::{actor_from_ctx, app_from_ctx, to_gql_error};
use crate::mappers::from_job_run;
use crate::types::{JobKeyValue, JobRunPayload};

#[derive(Default)]
pub(crate) struct JobMutations;

#[Object]
impl JobMutations {
    async fn trigger_job(
        &self,
        ctx: &Context<'_>,
        job_key: JobKeyValue,
    ) -> GqlResult<JobRunPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let run = app
            .trigger_job(&actor, job_key.into_application())
            .await
            .map_err(to_gql_error)?;
        Ok(from_job_run(run))
    }
}

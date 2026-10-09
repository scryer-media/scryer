use async_graphql::{Context, ID, Object, Result as GqlResult};
use scryer_application::AppError;

use scryer_interface_core::{actor_from_ctx, app_from_ctx, to_gql_error};
use scryer_interface_media::mappers::from_job_run;
use scryer_interface_media::types::{IntoApplication, JobKeyValue, JobRunPayload};

#[derive(Default)]
pub struct JobMutations;

#[Object]
impl JobMutations {
    /// Start the configured background job, or a scheduled script when the key is
    /// `CUSTOM_JOB`, and return its accepted run snapshot.
    async fn trigger_job(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Job key identifying the server-side job to start.")] job_key: JobKeyValue,
        #[graphql(
            desc = "Scheduled script to run; required with `CUSTOM_JOB` and rejected with any other key."
        )]
        custom_job_id: Option<ID>,
    ) -> GqlResult<JobRunPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let run = match (job_key, custom_job_id) {
            (JobKeyValue::CustomJob, Some(custom_job_id)) => {
                app.trigger_custom_job(&actor, custom_job_id.as_str()).await
            }
            (JobKeyValue::CustomJob, None) => Err(AppError::Validation(
                "customJobId is required for CUSTOM_JOB".to_string(),
            )),
            (_, Some(_)) => Err(AppError::Validation(
                "customJobId is only accepted with CUSTOM_JOB".to_string(),
            )),
            (job_key, None) => app.trigger_job(&actor, job_key.into_application()).await,
        }
        .map_err(to_gql_error)?;
        Ok(from_job_run(run))
    }
}

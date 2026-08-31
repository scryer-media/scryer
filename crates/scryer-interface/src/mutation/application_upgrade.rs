use async_graphql::{Context, Object, Result as GqlResult};
use scryer_application::{
    AppError,
    application_upgrade::{ApplicationUpgradeJobRequest, InstallationKind},
};
use scryer_domain::AppPermission;
use scryer_interface_core::{
    app_from_ctx, application_upgrade_assessment_from_ctx, require_config_app_permission,
    to_gql_error,
};
use scryer_interface_media::mappers::from_job_run;
use scryer_interface_media::types::{ApplicationUpgradeStartPayload, StartApplicationUpgradeInput};

#[derive(Default)]
pub(crate) struct ApplicationUpgradeMutations;

#[Object]
impl ApplicationUpgradeMutations {
    /// Begin a signed in-application upgrade for the currently advertised release.
    async fn start_application_upgrade(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "The update tag and version currently advertised by SMG.")]
        input: StartApplicationUpgradeInput,
    ) -> GqlResult<ApplicationUpgradeStartPayload> {
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let assessment = application_upgrade_assessment_from_ctx(ctx);
        if !assessment.eligible
            || !matches!(
                assessment.kind,
                InstallationKind::Portable | InstallationKind::DirectMsi
            )
        {
            return Err(to_gql_error(AppError::Validation(format!(
                "application upgrade is not eligible: {}",
                assessment.reason.as_str()
            ))));
        }
        let accepted = app_from_ctx(ctx)?
            .start_application_upgrade_job(
                &actor,
                ApplicationUpgradeJobRequest {
                    expected_tag: input.expected_tag,
                    expected_version: input.expected_version,
                    installation_kind: assessment.kind,
                    executable_path: None,
                    tray_supervised: assessment.tray_supervised,
                },
            )
            .await
            .map_err(to_gql_error)?;
        Ok(ApplicationUpgradeStartPayload {
            job_run: from_job_run(accepted.job_run),
        })
    }
}

use async_graphql::{Context, Object, Result as GqlResult};

use crate::context::{actor_from_ctx, app_from_ctx, to_gql_error};
use crate::types::DirectoryEntryPayload;

#[derive(Default)]
pub struct FilesystemMutations;

#[Object]
impl FilesystemMutations {
    /// Create one folder inside an absolute directory for the folder picker;
    /// requires library-settings read permission, the same as browsePath. The
    /// folder gets the server's umask and is owned by the user Scryer runs as.
    /// Refusals carry a FOLDER_CREATE_* reason.
    async fn create_folder(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Absolute path of the directory to create the folder in.")]
        parent_path: String,
        #[graphql(desc = "Name of the new folder; surrounding spaces are trimmed.")] name: String,
    ) -> GqlResult<DirectoryEntryPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let created = app
            .create_folder(&actor, &parent_path, &name)
            .await
            .map_err(to_gql_error)?;
        Ok(DirectoryEntryPayload {
            name: created
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            path: created.to_string_lossy().into_owned(),
            is_directory: true,
        })
    }
}

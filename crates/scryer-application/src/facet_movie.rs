use std::collections::HashSet;

use async_trait::async_trait;
use scryer_domain::{Collection, MediaFacet, Title};

use crate::facet_handler::{FacetHandler, HydrationResult, movie_to_hydration_result};
use crate::{
    ActivityKind, AppResult, MetadataGateway, RenameCollisionPolicy, RenameMissingMetadataPolicy,
    RenamePlanItem,
};

pub struct MovieFacetHandler;

#[async_trait]
impl FacetHandler for MovieFacetHandler {
    fn facet(&self) -> MediaFacet {
        MediaFacet::Movie
    }

    fn facet_id(&self) -> &str {
        "movie"
    }

    fn download_category(&self) -> &str {
        "movie"
    }

    fn library_path_key(&self) -> &str {
        "movies.path"
    }

    fn root_folders_key(&self) -> &str {
        "movies.root_folders"
    }

    fn rename_template_key(&self) -> &str {
        "rename.template.movie.global"
    }

    fn collision_policy_key(&self) -> &str {
        "rename.collision_policy.movie.global"
    }

    fn missing_metadata_policy_key(&self) -> &str {
        "rename.missing_metadata_policy.movie.global"
    }

    fn default_rename_template(&self) -> &str {
        "{title} ({year}) - {quality}.{ext}"
    }

    fn default_library_path(&self) -> &str {
        "/data/movies"
    }

    fn has_episodes(&self) -> bool {
        false
    }

    fn title_added_activity_kind(&self) -> Option<ActivityKind> {
        Some(ActivityKind::MovieAdded)
    }

    fn search_category(&self) -> &str {
        "movie"
    }

    fn rename_scope_id(&self) -> &str {
        "movie"
    }

    async fn hydrate_metadata(
        &self,
        gateway: &dyn MetadataGateway,
        tvdb_id: i64,
        language: &str,
    ) -> AppResult<HydrationResult> {
        let movie = gateway.get_movie(tvdb_id, language).await?;
        Ok(movie_to_hydration_result(movie, language))
    }

    fn build_rename_plan_item(
        &self,
        title: &Title,
        collection: &Collection,
        template: &str,
        collision_policy: &RenameCollisionPolicy,
        missing_metadata_policy: &RenameMissingMetadataPolicy,
        planned_targets: &mut HashSet<String>,
    ) -> RenamePlanItem {
        crate::app_usecase_library::build_movie_rename_plan_item(
            title,
            collection,
            template,
            collision_policy,
            missing_metadata_policy,
            planned_targets,
        )
    }
}

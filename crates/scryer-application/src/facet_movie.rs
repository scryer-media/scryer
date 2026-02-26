use std::collections::HashSet;

use async_trait::async_trait;
use chrono::Utc;
use scryer_domain::{Collection, MediaFacet, Title};

use crate::facet_handler::{FacetHandler, HydrationResult};
use crate::{
    ActivityKind, AppResult, MetadataGateway, RenameCollisionPolicy,
    RenameMissingMetadataPolicy, RenamePlanItem, TitleMetadataUpdate,
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
        "/media/movies"
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
        let update = TitleMetadataUpdate {
            year: movie.year,
            overview: Some(movie.overview),
            poster_url: Some(movie.poster_url),
            sort_title: Some(movie.sort_title),
            slug: Some(movie.slug),
            imdb_id: Some(movie.imdb_id),
            runtime_minutes: Some(movie.runtime_minutes),
            genres: movie.genres,
            content_status: Some(movie.content_status),
            language: Some(movie.language),
            first_aired: None,
            network: None,
            studio: Some(movie.studio),
            country: None,
            aliases: vec![],
            metadata_language: Some(language.to_string()),
            metadata_fetched_at: Some(Utc::now().to_rfc3339()),
            ..Default::default()
        };
        Ok(HydrationResult {
            metadata_update: update,
            seasons: vec![],
            episodes: vec![],
            anime_mappings: vec![],
        })
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

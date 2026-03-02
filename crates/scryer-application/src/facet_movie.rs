use std::collections::HashSet;

use async_trait::async_trait;
use chrono::Utc;
use scryer_domain::{Collection, MediaFacet, Title};

use crate::facet_handler::{FacetHandler, HydrationResult};
use crate::{
    ActivityKind, AppResult, MetadataGateway, RenameCollisionPolicy,
    RenameMissingMetadataPolicy, RenamePlanItem, TitleMetadataUpdate,
};

fn non_empty(s: String) -> Option<String> {
    if s.trim().is_empty() { None } else { Some(s) }
}

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
            year: movie.year.filter(|&y| y > 0),
            overview: non_empty(movie.overview),
            poster_url: non_empty(movie.poster_url),
            sort_title: non_empty(movie.sort_title),
            slug: non_empty(movie.slug),
            imdb_id: non_empty(movie.imdb_id),
            runtime_minutes: if movie.runtime_minutes > 0 { Some(movie.runtime_minutes) } else { None },
            genres: movie.genres,
            content_status: non_empty(movie.content_status),
            language: non_empty(movie.language),
            first_aired: None,
            network: None,
            studio: non_empty(movie.studio),
            country: None,
            aliases: vec![],
            metadata_language: Some(language.to_string()),
            metadata_fetched_at: Some(Utc::now().to_rfc3339()),
            digital_release_date: movie.tmdb_release_date,
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

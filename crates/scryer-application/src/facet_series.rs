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

/// Handles both TV and Anime facets (they share series behavior
/// with different scope IDs and rename templates).
pub struct SeriesFacetHandler {
    media_facet: MediaFacet,
}

impl SeriesFacetHandler {
    pub fn new(media_facet: MediaFacet) -> Self {
        Self { media_facet }
    }
}

#[async_trait]
impl FacetHandler for SeriesFacetHandler {
    fn facet(&self) -> MediaFacet {
        self.media_facet.clone()
    }

    fn facet_id(&self) -> &str {
        match self.media_facet {
            MediaFacet::Tv => "tv",
            MediaFacet::Anime => "anime",
            _ => "tv",
        }
    }

    fn download_category(&self) -> &str {
        match self.media_facet {
            MediaFacet::Tv => "tv",
            MediaFacet::Anime => "anime",
            _ => "tv",
        }
    }

    fn library_path_key(&self) -> &str {
        match self.media_facet {
            MediaFacet::Anime => "anime.path",
            _ => "series.path",
        }
    }

    fn rename_template_key(&self) -> &str {
        match self.media_facet {
            MediaFacet::Anime => "rename.template.anime.global",
            _ => "rename.template.series.global",
        }
    }

    fn collision_policy_key(&self) -> &str {
        "rename.collision_policy.series.global"
    }

    fn missing_metadata_policy_key(&self) -> &str {
        "rename.missing_metadata_policy.series.global"
    }

    fn default_rename_template(&self) -> &str {
        match self.media_facet {
            MediaFacet::Anime => "{title} - S{season_order:2}E{episode:2} ({absolute_episode}) - {quality}.{ext}",
            _ => "{title} - S{season:2}E{episode:2} - {quality}.{ext}",
        }
    }

    fn default_library_path(&self) -> &str {
        match self.media_facet {
            MediaFacet::Anime => "/media/anime",
            _ => "/media/series",
        }
    }

    fn has_episodes(&self) -> bool {
        true
    }

    fn title_added_activity_kind(&self) -> Option<ActivityKind> {
        None
    }

    fn search_category(&self) -> &str {
        match self.media_facet {
            MediaFacet::Anime => "anime",
            _ => "series",
        }
    }

    fn rename_scope_id(&self) -> &str {
        "series"
    }

    async fn hydrate_metadata(
        &self,
        gateway: &dyn MetadataGateway,
        tvdb_id: i64,
        language: &str,
    ) -> AppResult<HydrationResult> {
        let series = gateway.get_series(tvdb_id, language).await?;
        let update = TitleMetadataUpdate {
            year: series.year.filter(|&y| y > 0),
            overview: non_empty(series.overview),
            poster_url: non_empty(series.poster_url),
            sort_title: non_empty(series.sort_name),
            slug: non_empty(series.slug),
            imdb_id: None,
            runtime_minutes: if series.runtime_minutes > 0 { Some(series.runtime_minutes) } else { None },
            genres: series.genres,
            content_status: non_empty(series.content_status),
            language: None,
            first_aired: non_empty(series.first_aired),
            network: non_empty(series.network),
            studio: None,
            country: non_empty(series.country),
            aliases: series.aliases,
            metadata_language: Some(language.to_string()),
            metadata_fetched_at: Some(Utc::now().to_rfc3339()),
            ..Default::default()
        };
        Ok(HydrationResult {
            metadata_update: update,
            seasons: series.seasons,
            episodes: series.episodes,
            anime_mappings: series.anime_mappings,
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
        crate::app_usecase_library::build_series_rename_plan_item(
            title,
            collection,
            template,
            collision_policy,
            missing_metadata_policy,
            planned_targets,
        )
    }
}

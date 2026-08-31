use async_trait::async_trait;
use scryer_domain::MediaFacet;

use crate::facet_handler::{
    FacetHandler, HydrationResult, hydrate_referenced_movie_metadata, series_to_hydration_result,
};
use crate::{AppResult, MetadataGateway};

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
        self.media_facet.as_str()
    }

    fn download_category(&self) -> &str {
        self.media_facet.as_str()
    }

    fn library_path_key(&self) -> &str {
        match self.media_facet {
            MediaFacet::Anime => "anime.path",
            _ => "series.path",
        }
    }

    fn root_folders_key(&self) -> &str {
        match self.media_facet {
            MediaFacet::Anime => "anime.root_folders",
            _ => "series.root_folders",
        }
    }

    fn default_library_path(&self) -> &str {
        match self.media_facet {
            MediaFacet::Anime => "/data/anime",
            _ => "/data/series",
        }
    }

    fn has_episodes(&self) -> bool {
        true
    }

    fn search_category(&self) -> &str {
        match self.media_facet {
            MediaFacet::Anime => "anime",
            _ => "series",
        }
    }

    async fn hydrate_metadata(
        &self,
        gateway: &dyn MetadataGateway,
        tvdb_id: i64,
        language: &str,
    ) -> AppResult<HydrationResult> {
        let series = gateway.get_series(tvdb_id, language).await?;
        let movie_metadata = hydrate_referenced_movie_metadata(gateway, &[&series], language)
            .await
            .inspect_err(|error| {
                tracing::warn!(tvdb_id, error = %error, "linked movie metadata hydration failed");
            })
            .unwrap_or_default();
        let mut result = series_to_hydration_result(series, language);
        result.movie_metadata = movie_metadata;
        Ok(result)
    }
}

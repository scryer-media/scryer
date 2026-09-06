use async_trait::async_trait;
use chrono::Utc;
use scryer_domain::{ExternalId, MediaFacet};

use crate::{
    AnimeMapping, AnimeMovie, AppResult, DiscoveryTitle, EpisodeMetadata, MetadataFieldUpdate,
    MetadataGateway, MovieMetadata, SeasonMetadata, SeriesMetadata, TitleMetadataUpdate,
};

/// Result of hydrating a title's metadata from a metadata gateway.
/// Movies return empty seasons/episodes. Series include full season/episode data.
pub struct HydrationResult {
    pub metadata_update: TitleMetadataUpdate,
    pub seasons: Vec<SeasonMetadata>,
    pub episodes: Vec<EpisodeMetadata>,
    pub anime_mappings: Vec<AnimeMapping>,
    pub anime_movies: Vec<AnimeMovie>,
    /// Community season layout for an anime series; `None` for every other
    /// facet and for anime SMG has no bridge for.
    pub anime_numbering_bridge: Option<scryer_domain::AnimeNumberingBridge>,
    pub movie_metadata: std::collections::HashMap<i64, MovieMetadata>,
    pub more_like_this: Vec<DiscoveryTitle>,
}

pub(crate) fn external_ids_from_hydration_metadata(
    mut external_ids: Vec<ExternalId>,
    metadata_update: &TitleMetadataUpdate,
) -> Vec<ExternalId> {
    if let Some(imdb_id) = metadata_update
        .imdb_id
        .as_deref()
        .and_then(crate::normalize::normalize_imdb_id)
    {
        external_ids.push(ExternalId {
            source: "imdb".to_string(),
            value: imdb_id,
        });
    }
    external_ids.extend(metadata_update.extra_external_ids.iter().cloned());
    external_ids
}

#[derive(Clone, Copy)]
pub struct RenameFacetSettings {
    pub scope_id: &'static str,
    pub collision_policy_key: &'static str,
    pub missing_metadata_policy_key: &'static str,
}

pub fn rename_facet_settings(facet: &MediaFacet) -> RenameFacetSettings {
    match facet {
        MediaFacet::Movie => RenameFacetSettings {
            scope_id: "movie",
            collision_policy_key: "rename.collision_policy.movie.global",
            missing_metadata_policy_key: "rename.missing_metadata_policy.movie.global",
        },
        MediaFacet::Series => RenameFacetSettings {
            scope_id: "series",
            collision_policy_key: "rename.collision_policy.series.global",
            missing_metadata_policy_key: "rename.missing_metadata_policy.series.global",
        },
        MediaFacet::Anime => RenameFacetSettings {
            scope_id: "anime",
            collision_policy_key: "rename.collision_policy.anime.global",
            missing_metadata_policy_key: "rename.missing_metadata_policy.anime.global",
        },
    }
}

fn non_empty(s: String) -> Option<String> {
    if s.trim().is_empty() { None } else { Some(s) }
}

fn original_language_update(language: Option<&str>) -> MetadataFieldUpdate<String> {
    language
        .and_then(crate::normalize_detected_audio_language_code)
        .map_or(MetadataFieldUpdate::Unchanged, MetadataFieldUpdate::Set)
}

pub(crate) fn primary_anime_mapping(anime_mappings: &[AnimeMapping]) -> Option<&AnimeMapping> {
    anime_mappings
        .iter()
        .find(|mapping| mapping.mapping_type != "S")
        .or(anime_mappings.first())
}

fn primary_anime_mapping_extra_external_ids(anime_mappings: &[AnimeMapping]) -> Vec<ExternalId> {
    let Some(mapping) = primary_anime_mapping(anime_mappings) else {
        return Vec::new();
    };

    let mut external_ids = Vec::new();
    push_positive_external_id(&mut external_ids, "mal", mapping.mal_id);
    push_positive_external_id(&mut external_ids, "anilist", mapping.anilist_id);
    push_positive_external_id(&mut external_ids, "anidb", mapping.anidb_id);
    push_positive_external_id(&mut external_ids, "kitsu", mapping.kitsu_id);
    push_positive_external_id(&mut external_ids, "simkl", mapping.simkl_id);
    push_positive_external_id(&mut external_ids, "tvdb", mapping.thetvdb_id);
    push_positive_external_id(&mut external_ids, "tmdb", mapping.themoviedb_id);
    push_positive_imdb_external_id(&mut external_ids, mapping.imdb_id);
    push_positive_external_id(&mut external_ids, "trakt", mapping.trakt_id);
    external_ids
}

fn push_positive_external_id(external_ids: &mut Vec<ExternalId>, source: &str, value: Option<i64>) {
    if let Some(value) = value.filter(|value| *value > 0) {
        external_ids.push(ExternalId {
            source: source.to_string(),
            value: value.to_string(),
        });
    }
}

fn push_positive_imdb_external_id(external_ids: &mut Vec<ExternalId>, value: Option<i64>) {
    let Some(imdb_id) = value
        .filter(|value| *value > 0)
        .and_then(|value| crate::normalize::normalize_imdb_id(&value.to_string()))
    else {
        return;
    };
    external_ids.push(ExternalId {
        source: "imdb".to_string(),
        value: imdb_id,
    });
}

/// Build a [`HydrationResult`] from an already-fetched [`MovieMetadata`].
///
/// Shared by the single-title facet handler path and the bulk hydration loop.
pub fn movie_to_hydration_result(movie: MovieMetadata, language: &str) -> HydrationResult {
    let mut extra_external_ids = Vec::new();
    if let Some(smg_id) = movie.smg_id {
        extra_external_ids.push(scryer_domain::ExternalId {
            source: "smg".into(),
            value: smg_id.to_string(),
        });
    }
    push_positive_external_id(&mut extra_external_ids, "tvdb", movie.tvdb_id);
    if let Some(imdb_id) = crate::normalize::normalize_imdb_id(movie.imdb_id.as_str()) {
        extra_external_ids.push(scryer_domain::ExternalId {
            source: "imdb".into(),
            value: imdb_id,
        });
    }
    if let Some(anidb_id) = movie.anidb_id {
        extra_external_ids.push(scryer_domain::ExternalId {
            source: "anidb".into(),
            value: anidb_id.to_string(),
        });
    }
    if let Some(tmdb_id) = movie.tmdb_id {
        extra_external_ids.push(scryer_domain::ExternalId {
            source: "tmdb".into(),
            value: tmdb_id.to_string(),
        });
    }

    let update = TitleMetadataUpdate {
        name: non_empty(movie.name),
        year: movie.year.filter(|&y| y > 0),
        overview: non_empty(movie.overview),
        poster_url: non_empty(movie.poster_url),
        background_url: movie.background_url.and_then(non_empty),
        sort_title: non_empty(movie.sort_title),
        slug: non_empty(movie.slug),
        imdb_id: non_empty(movie.imdb_id),
        runtime_minutes: if movie.runtime_minutes > 0 {
            Some(movie.runtime_minutes)
        } else {
            None
        },
        popularity: movie.popularity.filter(|value| value.is_finite()),
        canonical_tags: movie.canonical_tags,
        content_status: non_empty(movie.content_status),
        language: original_language_update(movie.original_language.as_deref()),
        first_aired: None,
        network: None,
        studio: non_empty(movie.studio),
        country: None,
        aliases: vec![],
        metadata_language: Some(language.to_string()),
        metadata_fetched_at: Some(Utc::now().to_rfc3339()),
        digital_release_date: movie.tmdb_release_date,
        ratings: Some(movie.ratings),
        credits: Some(movie.credits),
        extra_external_ids,
        ..Default::default()
    };
    HydrationResult {
        metadata_update: update,
        seasons: vec![],
        episodes: vec![],
        anime_mappings: vec![],
        anime_movies: vec![],
        anime_numbering_bridge: None,
        movie_metadata: std::collections::HashMap::new(),
        more_like_this: vec![],
    }
}

pub(crate) async fn hydrate_referenced_movie_metadata(
    gateway: &dyn MetadataGateway,
    series_items: &[&SeriesMetadata],
    language: &str,
) -> AppResult<std::collections::HashMap<i64, MovieMetadata>> {
    let movie_tvdb_ids = series_items
        .iter()
        .flat_map(|series| series.anime_movies.iter())
        .filter_map(|movie| movie.movie_tvdb_id)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if movie_tvdb_ids.is_empty() {
        return Ok(std::collections::HashMap::new());
    }

    Ok(gateway
        .get_metadata_bulk(&movie_tvdb_ids, &[], language)
        .await?
        .movies)
}

/// Build a [`HydrationResult`] from an already-fetched [`SeriesMetadata`].
pub fn series_to_hydration_result(series: SeriesMetadata, language: &str) -> HydrationResult {
    let extra_external_ids = primary_anime_mapping_extra_external_ids(&series.anime_mappings);
    let update = TitleMetadataUpdate {
        name: non_empty(series.name),
        year: series.year.filter(|&y| y > 0),
        overview: non_empty(series.overview),
        poster_url: non_empty(series.poster_url),
        background_url: series.background_url.and_then(non_empty),
        sort_title: non_empty(series.sort_name),
        slug: non_empty(series.slug),
        imdb_id: None,
        runtime_minutes: if series.runtime_minutes > 0 {
            Some(series.runtime_minutes)
        } else {
            None
        },
        canonical_tags: series.canonical_tags,
        content_status: non_empty(series.content_status),
        language: original_language_update(series.original_language.as_deref()),
        first_aired: non_empty(series.first_aired),
        network: non_empty(series.network),
        studio: None,
        country: non_empty(series.country),
        aliases: series.aliases,
        tagged_aliases: series.tagged_aliases,
        metadata_language: Some(language.to_string()),
        metadata_fetched_at: Some(Utc::now().to_rfc3339()),
        ratings: Some(series.ratings),
        credits: Some(series.credits),
        extra_external_ids,
        ..Default::default()
    };
    HydrationResult {
        metadata_update: update,
        seasons: series.seasons,
        episodes: series.episodes,
        anime_mappings: series.anime_mappings,
        anime_movies: series.anime_movies,
        anime_numbering_bridge: series.anime_numbering_bridge,
        movie_metadata: std::collections::HashMap::new(),
        more_like_this: vec![],
    }
}

/// Configuration and strategies for a specific media facet.
/// Each facet (movie, series, anime) implements this trait to define
/// its metadata hydration, rename strategy, import routing, and
/// acquisition behavior.
#[async_trait]
pub trait FacetHandler: Send + Sync {
    /// The domain enum variant this handler covers.
    fn facet(&self) -> MediaFacet;

    /// String ID used in settings keys, database columns, audit logs.
    /// e.g. "movie", "series", "anime"
    fn facet_id(&self) -> &str;

    /// Download client category string.
    fn download_category(&self) -> &str;

    /// Settings key for the library root path (e.g. "movies.path").
    fn library_path_key(&self) -> &str;

    /// Settings key for the root folders JSON array (e.g. "movies.root_folders").
    fn root_folders_key(&self) -> &str;

    /// Default library root path.
    fn default_library_path(&self) -> &str;

    /// Whether this facet has episode-level structure.
    fn has_episodes(&self) -> bool;

    /// Indexer search category (e.g. "movie", "series", "anime").
    fn search_category(&self) -> &str;

    /// Hydrate a title's metadata by calling the metadata gateway.
    async fn hydrate_metadata(
        &self,
        gateway: &dyn MetadataGateway,
        tvdb_id: i64,
        language: &str,
    ) -> AppResult<HydrationResult>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{TitleCredit, TitleRatingSummary};

    fn anime_mapping(mapping_type: &str, anidb_id: Option<i64>) -> AnimeMapping {
        AnimeMapping {
            mal_id: None,
            mal_dub_id: None,
            anilist_id: None,
            anidb_id,
            kitsu_id: None,
            simkl_id: None,
            thetvdb_id: None,
            themoviedb_id: None,
            imdb_id: None,
            trakt_id: None,
            alt_tvdb_id: None,
            thetvdb_season: Some(1),
            thetvdb_part: None,
            score: None,
            anime_media_type: String::new(),
            global_media_type: String::new(),
            status: String::new(),
            mapping_type: mapping_type.to_string(),
            episode_mappings: vec![],
        }
    }

    fn test_series(anime_mappings: Vec<AnimeMapping>) -> SeriesMetadata {
        SeriesMetadata {
            target_key: None,
            tvdb_id: 12345,
            name: "Sword Art Online".to_string(),
            sort_name: "Sword Art Online".to_string(),
            slug: "sword-art-online".to_string(),
            year: Some(2012),
            content_status: String::new(),
            first_aired: String::new(),
            overview: String::new(),
            network: String::new(),
            runtime_minutes: 24,
            poster_url: String::new(),
            background_url: None,
            original_language: Some("jpn".to_string()),
            country: String::new(),
            canonical_tags: vec![],
            aliases: vec![],
            tagged_aliases: vec![],
            seasons: vec![],
            episodes: vec![],
            anime_mappings,
            anime_movies: vec![],
            anime_numbering_bridge: None,
            ratings: Default::default(),
            credits: Vec::new(),
        }
    }

    #[test]
    fn series_hydration_uses_primary_anime_mapping_for_title_level_external_ids() {
        let mut secondary_mapping = anime_mapping("S", Some(9999));
        secondary_mapping.mal_id = Some(999);
        let mut primary_mapping = anime_mapping("R", Some(15146));
        primary_mapping.mal_id = Some(111_001);
        primary_mapping.anilist_id = Some(222_002);
        primary_mapping.kitsu_id = Some(444_004);
        primary_mapping.simkl_id = Some(555_005);
        primary_mapping.thetvdb_id = Some(12345);
        primary_mapping.themoviedb_id = Some(666_006);
        primary_mapping.imdb_id = Some(777_007);
        primary_mapping.trakt_id = Some(888_008);

        let result = series_to_hydration_result(
            test_series(vec![secondary_mapping, primary_mapping]),
            "eng",
        );

        assert_eq!(
            result.metadata_update.extra_external_ids,
            vec![
                ExternalId {
                    source: "mal".to_string(),
                    value: "111001".to_string(),
                },
                ExternalId {
                    source: "anilist".to_string(),
                    value: "222002".to_string(),
                },
                ExternalId {
                    source: "anidb".to_string(),
                    value: "15146".to_string(),
                },
                ExternalId {
                    source: "kitsu".to_string(),
                    value: "444004".to_string(),
                },
                ExternalId {
                    source: "simkl".to_string(),
                    value: "555005".to_string(),
                },
                ExternalId {
                    source: "tvdb".to_string(),
                    value: "12345".to_string(),
                },
                ExternalId {
                    source: "tmdb".to_string(),
                    value: "666006".to_string(),
                },
                ExternalId {
                    source: "imdb".to_string(),
                    value: "tt777007".to_string(),
                },
                ExternalId {
                    source: "trakt".to_string(),
                    value: "888008".to_string(),
                },
            ]
        );
    }

    #[test]
    fn series_hydration_sets_original_language_independently_of_display_locale() {
        let mut series = test_series(vec![]);
        series.original_language = Some("JA".to_string());

        let result = series_to_hydration_result(series, "eng");

        assert_eq!(
            result.metadata_update.language,
            MetadataFieldUpdate::Set("jpn".to_string())
        );
        assert_eq!(
            result.metadata_update.metadata_language.as_deref(),
            Some("eng")
        );
    }

    #[test]
    fn series_hydration_preserves_language_when_original_language_is_missing() {
        let mut series = test_series(vec![]);
        series.original_language = None;

        let result = series_to_hydration_result(series, "eng");

        assert_eq!(
            result.metadata_update.language,
            MetadataFieldUpdate::Unchanged
        );
    }

    #[test]
    fn series_hydration_preserves_language_when_original_language_is_invalid() {
        let mut series = test_series(vec![]);
        series.original_language = Some("und".to_string());

        let result = series_to_hydration_result(series, "eng");

        assert_eq!(
            result.metadata_update.language,
            MetadataFieldUpdate::Unchanged
        );
    }

    #[test]
    fn rename_facet_settings_for_anime_use_anime_scope_and_keys() {
        let settings = rename_facet_settings(&MediaFacet::Anime);
        assert_eq!(settings.scope_id, "anime");
        assert_eq!(
            settings.collision_policy_key,
            "rename.collision_policy.anime.global"
        );
        assert_eq!(
            settings.missing_metadata_policy_key,
            "rename.missing_metadata_policy.anime.global"
        );
    }

    fn test_movie(credits: Vec<TitleCredit>) -> MovieMetadata {
        MovieMetadata {
            target_key: None,
            smg_id: None,
            primary_source: "tvdb".to_string(),
            tvdb_id: Some(909),
            name: "Fixture Movie".to_string(),
            slug: "fixture-movie".to_string(),
            year: Some(2026),
            content_status: String::new(),
            overview: String::new(),
            poster_url: String::new(),
            background_url: None,
            language: "eng".to_string(),
            original_language: Some("eng".to_string()),
            runtime_minutes: 90,
            sort_title: "fixture movie".to_string(),
            imdb_id: String::new(),
            tmdb_id: None,
            popularity: None,
            anidb_id: None,
            canonical_tags: vec![],
            studio: String::new(),
            tmdb_release_date: None,
            ratings: Default::default(),
            credits,
        }
    }

    fn test_credits() -> Vec<TitleCredit> {
        vec![
            TitleCredit {
                kind: "actor".to_string(),
                person_id: "p1".to_string(),
                person_name: "Lead Actor".to_string(),
                person_original_name: "主演".to_string(),
                person_image_url: "https://example.test/p1.jpg".to_string(),
                person_source: "tmdb".to_string(),
                person_external_id: "tmdb-1".to_string(),
                character_name: "Hero".to_string(),
                language: "eng".to_string(),
                billing_order: 0,
                episode_count: Some(12),
            },
            TitleCredit {
                kind: "director".to_string(),
                person_id: "p2".to_string(),
                person_name: "The Director".to_string(),
                billing_order: 1,
                ..Default::default()
            },
        ]
    }

    struct CreditsMetadataGateway;

    #[async_trait]
    impl MetadataGateway for CreditsMetadataGateway {
        async fn search_tvdb(
            &self,
            _query: &str,
            _type_hint: &str,
            _year: Option<i32>,
        ) -> AppResult<Vec<crate::MetadataSearchItem>> {
            unimplemented!("credits fixture gateway only serves title metadata")
        }

        async fn search_tvdb_batch(
            &self,
            _queries: &[crate::MetadataSearchQuery],
            _language: &str,
        ) -> AppResult<
            std::collections::HashMap<crate::MetadataSearchQuery, Vec<crate::MetadataSearchItem>>,
        > {
            unimplemented!("credits fixture gateway only serves title metadata")
        }

        async fn search_tvdb_rich(
            &self,
            _query: &str,
            _type_hint: &str,
            _limit: i32,
            _language: &str,
            _year: Option<i32>,
        ) -> AppResult<Vec<crate::RichMetadataSearchItem>> {
            unimplemented!("credits fixture gateway only serves title metadata")
        }

        async fn search_tvdb_multi(
            &self,
            _query: &str,
            _limit: i32,
            _language: &str,
        ) -> AppResult<crate::MultiMetadataSearchResult> {
            unimplemented!("credits fixture gateway only serves title metadata")
        }

        async fn get_movie(&self, _tvdb_id: i64, _language: &str) -> AppResult<MovieMetadata> {
            Ok(test_movie(test_credits()))
        }

        async fn get_series(&self, _tvdb_id: i64, _language: &str) -> AppResult<SeriesMetadata> {
            let mut series = test_series(vec![]);
            series.credits = test_credits();
            Ok(series)
        }

        async fn get_metadata_bulk(
            &self,
            movie_tvdb_ids: &[i64],
            series_tvdb_ids: &[i64],
            language: &str,
        ) -> AppResult<crate::BulkMetadataResult> {
            assert_eq!(movie_tvdb_ids, &[808, 909]);
            assert!(series_tvdb_ids.is_empty());
            assert_eq!(language, "eng");
            let mut movie = test_movie(test_credits());
            movie.ratings = TitleRatingSummary {
                rating: Some(8.4),
                rating_sources: vec!["tmdb".to_string()],
                external_ratings: vec![],
            };
            Ok(crate::BulkMetadataResult {
                movies: std::collections::HashMap::from([(909, movie)]),
                series: std::collections::HashMap::new(),
            })
        }
    }

    fn test_anime_movie(tvdb_id: Option<i64>) -> AnimeMovie {
        AnimeMovie {
            movie_tvdb_id: tvdb_id,
            movie_tmdb_id: None,
            movie_imdb_id: None,
            movie_mal_id: None,
            movie_anidb_id: None,
            name: "Fixture Movie".to_string(),
            slug: "fixture-movie".to_string(),
            year: Some(2026),
            content_status: "released".to_string(),
            overview: String::new(),
            poster_url: String::new(),
            language: "eng".to_string(),
            runtime_minutes: 90,
            sort_title: "fixture movie".to_string(),
            imdb_id: String::new(),
            studio: String::new(),
            digital_release_date: None,
            association_confidence: "high".to_string(),
            continuity_status: "canon".to_string(),
            movie_form: "movie".to_string(),
            placement: "ordered".to_string(),
            confidence: "high".to_string(),
            signal_summary: String::new(),
        }
    }

    #[test]
    fn movie_hydration_carries_the_complete_credit_list() {
        let result = movie_to_hydration_result(test_movie(test_credits()), "eng");

        assert_eq!(result.metadata_update.credits, Some(test_credits()));
    }

    #[test]
    fn movie_hydration_carries_the_smg_external_id() {
        let mut movie = test_movie(vec![]);
        movie.smg_id = Some(42_001);

        let result = movie_to_hydration_result(movie, "eng");

        assert!(
            result
                .metadata_update
                .extra_external_ids
                .contains(&ExternalId {
                    source: "smg".to_string(),
                    value: "42001".to_string(),
                })
        );
    }

    #[test]
    fn series_hydration_carries_the_complete_credit_list() {
        let mut series = test_series(vec![]);
        series.credits = test_credits();

        let result = series_to_hydration_result(series, "eng");

        assert_eq!(result.metadata_update.credits, Some(test_credits()));
    }

    #[test]
    fn hydration_reports_an_empty_credit_list_as_a_clearing_replacement() {
        let movie = movie_to_hydration_result(test_movie(vec![]), "eng");
        let series = series_to_hydration_result(test_series(vec![]), "eng");

        assert_eq!(movie.metadata_update.credits, Some(vec![]));
        assert_eq!(series.metadata_update.credits, Some(vec![]));
    }

    #[tokio::test]
    async fn every_facet_handler_hydrates_credits() {
        let gateway = CreditsMetadataGateway;
        let handlers: Vec<Box<dyn FacetHandler>> = vec![
            Box::new(crate::catalog::facets::movie::MovieFacetHandler),
            Box::new(crate::catalog::facets::series::SeriesFacetHandler::new(
                MediaFacet::Series,
            )),
            Box::new(crate::catalog::facets::series::SeriesFacetHandler::new(
                MediaFacet::Anime,
            )),
        ];

        for handler in handlers {
            let result = handler
                .hydrate_metadata(&gateway, 12345, "eng")
                .await
                .expect("facet hydration should succeed");
            assert_eq!(
                result.metadata_update.credits,
                Some(test_credits()),
                "{} hydration must persist credits",
                handler.facet_id()
            );
        }
    }

    #[tokio::test]
    async fn referenced_movie_hydration_deduplicates_ids() {
        let mut first = test_series(vec![]);
        first.anime_movies = vec![test_anime_movie(Some(909)), test_anime_movie(Some(909))];
        let mut second = test_series(vec![]);
        second.anime_movies = vec![test_anime_movie(Some(808)), test_anime_movie(None)];

        let hydrated =
            hydrate_referenced_movie_metadata(&CreditsMetadataGateway, &[&first, &second], "eng")
                .await
                .expect("linked movie metadata hydration");

        assert_eq!(hydrated.len(), 1);
        assert_eq!(hydrated[&909].ratings.rating, Some(8.4));
        assert_eq!(hydrated[&909].credits, test_credits());
        assert!(!hydrated.contains_key(&808));
    }

    #[test]
    fn rename_facet_settings_for_series_remain_series_owned() {
        let settings = rename_facet_settings(&MediaFacet::Series);
        assert_eq!(settings.scope_id, "series");
        assert_eq!(
            settings.collision_policy_key,
            "rename.collision_policy.series.global"
        );
        assert_eq!(
            settings.missing_metadata_policy_key,
            "rename.missing_metadata_policy.series.global"
        );
    }
}

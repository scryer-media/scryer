use std::collections::HashMap;

use async_graphql::{Context, ID, Object, Result as GqlResult};
use scryer_application::{AppError, AppUseCase, ImageProxyKind, MovieTitleRef};
use scryer_domain::MediaServerPlaybackEntityKind;
use scryer_interface_core::{actor_from_ctx, app_from_ctx, to_gql_error};
use scryer_interface_media::mappers::{from_calendar_episode, parse_iso_date};
use scryer_interface_media::types::*;

#[derive(Default)]
pub struct MetadataQueries;

fn from_metadata_search_item(
    app: &AppUseCase,
    item: scryer_application::RichMetadataSearchItem,
) -> MetadataSearchItemPayload {
    let owner_id = item
        .smg_id
        .map(|id| id.to_string())
        .unwrap_or_else(|| item.tvdb_id.to_string());
    let poster_url = app.media_image_url(
        item.poster_url.as_deref(),
        Some("metadata_search"),
        Some(&owner_id),
        ImageProxyKind::Poster,
        "w250",
    );
    MetadataSearchItemPayload {
        tvdb_id: item.tvdb_id,
        smg_id: item.smg_id,
        tmdb_id: item
            .external_ids
            .iter()
            .find(|external_id| external_id.source.eq_ignore_ascii_case("tmdb"))
            .and_then(|external_id| external_id.value.parse().ok()),
        primary_source: item.primary_source,
        external_ids: item
            .external_ids
            .into_iter()
            .map(|external_id| ExternalIdPayload {
                source: external_id.source,
                value: external_id.value,
            })
            .collect(),
        name: item.name,
        imdb_id: item.imdb_id,
        slug: item.slug,
        type_hint: item.type_hint,
        year: item.year,
        status: item.status,
        overview: item.overview,
        popularity: item.popularity,
        poster_url,
        language: item.language,
        runtime_minutes: item.runtime_minutes,
        sort_title: item.sort_title,
    }
}

fn parse_metadata_date(value: String, field: &str) -> GqlResult<Date> {
    parse_iso_date(Some(value))
        .ok_or_else(|| to_gql_error(AppError::Validation(format!("invalid {field} date"))))
}

#[allow(clippy::too_many_arguments)]
#[Object]
impl MetadataQueries {
    /// Search provider metadata for one facet, returning at most 100 results.
    async fn search_metadata(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Provider search text.")] query: String,
        #[graphql(
            name = "type",
            desc = "Facet to search, exposed as the GraphQL `type` argument."
        )]
        type_hint: MediaFacetValue,
        #[graphql(
            default = 25,
            desc = "Result count, defaulting to 25 and clamped to the range 1 through 100."
        )]
        limit: i32,
        #[graphql(
            default_with = "\"eng\".to_string()",
            desc = "Metadata language, defaulting to `eng`."
        )]
        language: String,
        #[graphql(desc = "Optional release-year hint passed to the provider.")] year: Option<i32>,
    ) -> GqlResult<Vec<MetadataSearchItemPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let limit = limit.clamp(1, 100);
        let results = app
            .search_metadata(
                &actor,
                &query,
                type_hint.as_scope_id(),
                limit,
                &language,
                year,
            )
            .await
            .map_err(to_gql_error)?;
        Ok(results
            .into_iter()
            .map(|item| from_metadata_search_item(&app, item))
            .collect())
    }

    /// Search provider metadata across movie, series, and anime facets.
    async fn search_metadata_multi(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Provider search text.")] query: String,
        #[graphql(
            default = 25,
            desc = "Result count, defaulting to 25 and clamped to the range 1 through 100."
        )]
        limit: i32,
        #[graphql(
            default_with = "\"eng\".to_string()",
            desc = "Metadata language, defaulting to `eng`."
        )]
        language: String,
    ) -> GqlResult<MetadataSearchMultiPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let limit = limit.clamp(1, 100);
        let result = app
            .search_metadata_multi(&actor, &query, limit, &language)
            .await
            .map_err(to_gql_error)?;
        Ok(MetadataSearchMultiPayload {
            movies: result
                .movies
                .into_iter()
                .map(|item| from_metadata_search_item(&app, item))
                .collect(),
            series: result
                .series
                .into_iter()
                .map(|item| from_metadata_search_item(&app, item))
                .collect(),
            anime: result
                .anime
                .into_iter()
                .map(|item| from_metadata_search_item(&app, item))
                .collect(),
        })
    }

    /// Fetch one movie metadata record by a supplied identity and language.
    async fn metadata_movie(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Movie identity and optional metadata language, which defaults to `eng`."
        )]
        input: MetadataMovieInput,
    ) -> GqlResult<MetadataMoviePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let parse_optional_id = |value: Option<String>, field: &str| {
            value
                .filter(|value| !value.trim().is_empty())
                .map(|value| {
                    value.parse().map_err(|_| {
                        to_gql_error(AppError::Validation(format!("invalid {field} id")))
                    })
                })
                .transpose()
        };
        let movie_ref = MovieTitleRef {
            smg_id: input.smg_id,
            tvdb_id: parse_optional_id(input.tvdb_id, "tvdb")?,
            tmdb_id: input.tmdb_id,
            imdb_id: input.imdb_id.filter(|value| !value.trim().is_empty()),
        };
        let language = input.language.unwrap_or_else(|| "eng".to_string());
        let movie = app
            .get_metadata_movie_by_ref(&actor, &movie_ref, &language)
            .await
            .map_err(to_gql_error)?;
        let tvdb_id = movie.tvdb_id.map(|id| id.to_string()).unwrap_or_default();
        let owner_id = movie
            .smg_id
            .map(|id| id.to_string())
            .or_else(|| movie.tvdb_id.map(|id| id.to_string()))
            .or_else(|| movie.tmdb_id.map(|id| id.to_string()))
            .or_else(|| (!movie.imdb_id.trim().is_empty()).then(|| movie.imdb_id.clone()));
        let poster_url = app
            .media_image_url(
                Some(movie.poster_url.as_str()),
                Some("metadata_movie"),
                owner_id.as_deref(),
                ImageProxyKind::Poster,
                "w250",
            )
            .unwrap_or_default();
        Ok(MetadataMoviePayload {
            tvdb_id,
            smg_id: movie.smg_id,
            tmdb_id: movie.tmdb_id,
            name: movie.name,
            slug: movie.slug,
            year: movie.year,
            status: movie.content_status,
            overview: movie.overview,
            poster_url,
            language: movie.language,
            runtime_minutes: movie.runtime_minutes,
            sort_title: movie.sort_title,
            imdb_id: movie.imdb_id,
            studio: movie.studio,
            tmdb_release_date: parse_iso_date(movie.tmdb_release_date),
        })
    }

    /// Fetch one series metadata record, optionally including its episode list.
    async fn metadata_series(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "TVDB identity, language, and whether episode metadata should be included, defaulting to true."
        )]
        input: MetadataSeriesInput,
    ) -> GqlResult<MetadataSeriesPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let tvdb_id: i64 = input
            .tvdb_id
            .parse()
            .map_err(|_| to_gql_error(AppError::Validation("invalid tvdb id".to_string())))?;
        let include_episodes = input.include_episodes.unwrap_or(true);
        let language = input.language.unwrap_or_else(|| "eng".to_string());
        let series = app
            .get_metadata_series(&actor, tvdb_id, &language)
            .await
            .map_err(to_gql_error)?;
        let series_owner_id = series.tvdb_id.to_string();
        let poster_url = app
            .media_image_url(
                Some(series.poster_url.as_str()),
                Some("metadata_series"),
                Some(&series_owner_id),
                ImageProxyKind::Poster,
                "w250",
            )
            .expect("metadata series image registration with an owner always returns a URL");
        let episodes = if include_episodes {
            series
                .episodes
                .into_iter()
                .map(|e| {
                    let owner_id = e.tvdb_id.to_string();
                    let image_url = app.media_image_url(
                        Some(e.image_url.as_str()),
                        Some("metadata_episode"),
                        Some(&owner_id),
                        ImageProxyKind::EpisodeStill,
                        "w300",
                    )
                    .expect("metadata episode image registration with an owner always returns a URL");
                    Ok(MetadataEpisodePayload {
                        tvdb_id: e.tvdb_id.to_string(),
                        episode_number: e.episode_number,
                        season_number: e.season_number,
                        name: e.name,
                        aired: parse_metadata_date(e.aired, "metadata episode aired")?,
                        runtime_minutes: e.runtime_minutes,
                        is_filler: e.is_filler,
                        image_url,
                    })
                })
                .collect::<GqlResult<Vec<_>>>()?
        } else {
            vec![]
        };

        Ok(MetadataSeriesPayload {
            tvdb_id: series.tvdb_id.to_string(),
            name: series.name,
            sort_name: series.sort_name,
            slug: series.slug,
            year: series.year,
            status: series.content_status,
            first_aired: parse_metadata_date(series.first_aired, "metadata series first_aired")?,
            overview: series.overview,
            network: series.network,
            runtime_minutes: series.runtime_minutes,
            poster_url,
            country: series.country,
            aliases: series.aliases,
            seasons: series
                .seasons
                .into_iter()
                .map(|s| MetadataSeasonPayload {
                    tvdb_id: s.tvdb_id.to_string(),
                    number: s.number,
                    label: s.label,
                    episode_type: s.episode_type,
                })
                .collect(),
            episodes,
        })
    }

    /// List calendar episodes between two dates, optionally restricted to libraries.
    async fn calendar_episodes(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Start date of the calendar range.")] start_date: Date,
        #[graphql(desc = "End date of the calendar range.")] end_date: Date,
        #[graphql(
            desc = "Optional library IDs; omitted, null, or an empty list includes all accessible libraries."
        )]
        library_ids: Option<Vec<ID>>,
    ) -> GqlResult<Vec<CalendarEpisodePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let start_date = start_date.to_iso_string();
        let end_date = end_date.to_iso_string();
        let library_ids =
            library_ids.map(|ids| ids.into_iter().map(String::from).collect::<Vec<String>>());
        let episodes = app
            .list_calendar_episodes(&actor, &start_date, &end_date, library_ids)
            .await
            .map_err(to_gql_error)?;
        if episodes.is_empty() {
            return Ok(Vec::new());
        }

        let mut title_ids = episodes
            .iter()
            .map(|episode| episode.title_id.clone())
            .collect::<Vec<_>>();
        title_ids.sort_unstable();
        title_ids.dedup();
        let mut availability_by_episode = app
            .list_episode_media_availability(&actor, &title_ids)
            .await
            .map_err(to_gql_error)?
            .into_iter()
            .map(|availability| (availability.episode_id.clone(), availability))
            .collect::<HashMap<_, _>>();

        let playback_entities = episodes
            .iter()
            .map(|episode| {
                if episode.title_facet == "movie" {
                    (
                        MediaServerPlaybackEntityKind::Title,
                        episode.title_id.clone(),
                    )
                } else {
                    (MediaServerPlaybackEntityKind::Episode, episode.id.clone())
                }
            })
            .collect::<Vec<_>>();
        let mut playback_links_by_entity = app
            .media_server_playback_links_for_authorized_entities(&actor, &playback_entities)
            .await
            .map_err(to_gql_error)?;

        let mut payloads = Vec::with_capacity(episodes.len());
        for episode in episodes {
            let availability = availability_by_episode.remove(&episode.id);
            let entity_key = if episode.title_facet == "movie" {
                (
                    MediaServerPlaybackEntityKind::Title,
                    episode.title_id.clone(),
                )
            } else {
                (MediaServerPlaybackEntityKind::Episode, episode.id.clone())
            };
            let playback_links = playback_links_by_entity
                .remove(&entity_key)
                .unwrap_or_default()
                .into_iter()
                .map(|link| MediaServerPlaybackLinkPayload {
                    connection_id: link.connection_id.into(),
                    display_name: link.display_name,
                    provider: MediaServerProviderValue::from_domain(link.provider),
                    href: link.href,
                })
                .collect();
            payloads.push(from_calendar_episode(
                &app,
                episode,
                availability,
                playback_links,
            ));
        }
        Ok(payloads)
    }
}

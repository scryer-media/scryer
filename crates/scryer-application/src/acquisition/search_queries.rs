use crate::{AcquisitionScopeState, FacetRegistry};
use scryer_domain::{Episode, EpisodeType, ExternalId, Title};

pub(crate) struct SearchQueryResult {
    pub(crate) queries: Vec<String>,
    pub(crate) imdb_id: Option<String>,
    pub(crate) tmdb_id: Option<String>,
    pub(crate) tvdb_id: Option<String>,
    pub(crate) anidb_id: Option<String>,
    pub(crate) mal_id: Option<String>,
    pub(crate) category: String,
    pub(crate) season: Option<u32>,
    pub(crate) episode: Option<u32>,
}

pub(crate) fn build_search_queries(
    title: &Title,
    item: &AcquisitionScopeState,
    episode: Option<&Episode>,
    facet_registry: &FacetRegistry,
) -> SearchQueryResult {
    let imdb_id = imdb_id_from_title(title);
    let tmdb_id = tmdb_id_from_external_ids(&title.external_ids);
    let tvdb_id = tvdb_id_from_external_ids(&title.external_ids);
    let anidb_id = anidb_id_from_external_ids(&title.external_ids);
    let mal_id = mal_id_from_external_ids(&title.external_ids);

    let category = facet_registry
        .get(&title.facet)
        .map(|handler| handler.search_category().to_string())
        .unwrap_or_else(|| "series".to_string());

    match item.media_type.as_str() {
        "movie" | "series_movie" => build_movie_search_queries(title, &item.media_type, category),
        "episode" => {
            let mut queries = Vec::new();
            let mut season_param: Option<u32> = None;
            let mut episode_param: Option<u32> = None;

            if let Some(episode) = episode {
                // Season 0 is a real season — the specials season — so it is
                // reported as `Some(0)` rather than folded into "no season".
                // The acceptance layer needs that distinction: with no expected
                // season, a release parsed as S01E02 satisfies the wanted
                // S00E02 special on the episode number alone. Season 0 is still
                // not a searchable Newznab parameter, and the request builder
                // and the season-pack lane drop it before it reaches an
                // indexer.
                let parsed_season = episode
                    .season_number
                    .as_deref()
                    .and_then(|value| value.trim().parse::<u32>().ok());
                let season_num: usize = parsed_season.unwrap_or(0) as usize;
                let episode_num: usize = episode
                    .episode_number
                    .as_deref()
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(0);

                season_param = parsed_season;
                if episode_num > 0 {
                    episode_param = Some(episode_num as u32);
                }

                if season_num > 0 && episode_num > 0 {
                    queries.push(format!(
                        "{} S{:0>2}E{:0>2}",
                        title.name, season_num, episode_num
                    ));
                    queries.push(format!("{} S{:0>2}", title.name, season_num));
                }

                if season_num == 0 && title.facet == scryer_domain::MediaFacet::Anime {
                    if let Some(label) = episode
                        .episode_label
                        .as_deref()
                        .filter(|label| !label.is_empty())
                    {
                        queries.push(format!("{} {}", title.name, label));
                    }
                    if episode_num > 0 {
                        if episode.episode_type == EpisodeType::Ova {
                            queries.push(format!("{} OVA {:0>2}", title.name, episode_num));
                        } else {
                            queries.push(format!("{} Special {:0>2}", title.name, episode_num));
                        }
                    }
                }

                if title.facet == scryer_domain::MediaFacet::Anime
                    && let Some(absolute) = episode
                        .absolute_number
                        .as_deref()
                        .and_then(|value| value.parse::<usize>().ok())
                        .filter(|&value| value > 0 && value != episode_num)
                {
                    queries.insert(0, format!("{} {:0>3}", title.name, absolute));
                }

                if title.facet == scryer_domain::MediaFacet::Anime && !title.name.is_empty() {
                    queries.push(title.name.clone());
                }

                if !queries.is_empty() {
                    let mut seen = std::collections::HashSet::new();
                    queries.retain(|query| seen.insert(query.to_ascii_lowercase()));
                }
            }

            if queries.is_empty() {
                queries.push(title.name.clone());
            }

            SearchQueryResult {
                queries,
                imdb_id,
                tmdb_id,
                tvdb_id,
                anidb_id,
                mal_id,
                category,
                season: season_param,
                episode: episode_param,
            }
        }
        _ => SearchQueryResult {
            queries: vec![],
            imdb_id: None,
            tmdb_id: None,
            tvdb_id: None,
            anidb_id: None,
            mal_id: None,
            category,
            season: None,
            episode: None,
        },
    }
}

pub(crate) fn build_movie_search_queries(
    title: &Title,
    _media_type: &str,
    category: String,
) -> SearchQueryResult {
    let imdb_id = imdb_id_from_title(title);
    let tmdb_id = tmdb_id_from_external_ids(&title.external_ids);
    let tvdb_id = tvdb_id_from_external_ids(&title.external_ids);
    let anidb_id = anidb_id_from_external_ids(&title.external_ids);
    let mal_id = mal_id_from_external_ids(&title.external_ids);
    let mut queries = Vec::new();
    let query = movie_text_search_query(&title.name, title.year);
    if !query.is_empty() {
        queries.push(query);
    }
    let mut seen = std::collections::HashSet::new();
    queries.retain(|query| seen.insert(query.to_ascii_lowercase()));
    if queries.is_empty() && (imdb_id.is_some() || tmdb_id.is_some()) {
        queries.push(String::new());
    }
    SearchQueryResult {
        queries,
        imdb_id,
        tmdb_id,
        tvdb_id,
        anidb_id,
        mal_id,
        category,
        season: None,
        episode: None,
    }
}

pub(crate) fn movie_text_search_query(title: &str, year: Option<i32>) -> String {
    let title = title.trim();
    if title.is_empty() {
        return String::new();
    }

    year.map_or_else(|| title.to_string(), |year| format!("{title} {year}"))
}

pub(crate) fn tmdb_id_from_external_ids(external_ids: &[ExternalId]) -> Option<String> {
    external_ids
        .iter()
        .find(|id| id.source.eq_ignore_ascii_case("tmdb"))
        .map(|id| id.value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub(crate) fn tvdb_id_from_external_ids(external_ids: &[ExternalId]) -> Option<String> {
    external_ids
        .iter()
        .find(|id| id.source.eq_ignore_ascii_case("tvdb"))
        .map(|id| id.value.clone())
}

pub(crate) fn anidb_id_from_external_ids(external_ids: &[ExternalId]) -> Option<String> {
    external_ids
        .iter()
        .find(|id| id.source.eq_ignore_ascii_case("anidb"))
        .map(|id| id.value.clone())
}

pub(crate) fn mal_id_from_external_ids(external_ids: &[ExternalId]) -> Option<String> {
    external_ids
        .iter()
        .find(|id| id.source.eq_ignore_ascii_case("mal"))
        .map(|id| id.value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub(crate) fn imdb_id_from_external_ids(external_ids: &[ExternalId]) -> Option<String> {
    external_ids
        .iter()
        .find(|id| id.source.eq_ignore_ascii_case("imdb"))
        .and_then(|id| crate::normalize::normalize_imdb_id(&id.value))
}

pub(crate) fn imdb_id_from_title(title: &Title) -> Option<String> {
    title
        .imdb_id
        .as_deref()
        .and_then(crate::normalize::normalize_imdb_id)
        .or_else(|| imdb_id_from_external_ids(&title.external_ids))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn movie_text_search_query_includes_year_only_for_named_movies() {
        assert_eq!(
            movie_text_search_query(" Amber Circuit ", Some(2026)),
            "Amber Circuit 2026"
        );
        assert_eq!(
            movie_text_search_query("Amber Circuit", None),
            "Amber Circuit"
        );
        assert_eq!(movie_text_search_query("  ", Some(2026)), "");
    }
}

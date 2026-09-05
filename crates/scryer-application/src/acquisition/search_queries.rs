use crate::{AcquisitionScopeState, FacetRegistry};
use scryer_domain::{AnimeNumberingBridge, Episode, EpisodeType, ExternalId, Title};

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

/// Build the text queries and id parameters for one wanted item.
///
/// `anime_numbering_bridge` is the title's community (per-cour) season layout
/// when the catalog stores one. It only ever *adds* queries, after the ones
/// this function already produced, so a title without a bridge searches
/// exactly as it does today.
pub(crate) fn build_search_queries(
    title: &Title,
    item: &AcquisitionScopeState,
    episode: Option<&Episode>,
    facet_registry: &FacetRegistry,
    anime_numbering_bridge: Option<&AnimeNumberingBridge>,
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
                let season_num: usize = episode
                    .season_number
                    .as_deref()
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(0);
                let episode_num: usize = episode
                    .episode_number
                    .as_deref()
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(0);

                if season_num > 0 {
                    season_param = Some(season_num as u32);
                }
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

                // Release groups number many anime per cour, not per TVDB
                // season, so a wanted episode of a long official season is
                // posted under a season and episode number the queries above
                // never mention. These extra forms ask for it the way the
                // groups actually name it.
                queries.extend(community_numbering_queries(
                    title,
                    episode,
                    season_num as i32,
                    episode_num as i32,
                    anime_numbering_bridge,
                ));

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

/// The community-numbered forms of one wanted TVDB episode: the community
/// `SxxEyy` pair, the cour's own title with a bare episode number (how a cour
/// with a distinct name is usually posted), and the absolute number in the
/// dashed form some groups prefer.
///
/// Returns nothing for a non-anime title, a title with no bridge, and an
/// episode no community season covers. Season-level and pack queries are out
/// of scope: the bridge speaks about episodes.
fn community_numbering_queries(
    title: &Title,
    episode: &Episode,
    season_num: i32,
    episode_num: i32,
    bridge: Option<&AnimeNumberingBridge>,
) -> Vec<String> {
    if title.facet != scryer_domain::MediaFacet::Anime || title.name.trim().is_empty() {
        return Vec::new();
    }
    let Some(bridge) = bridge.filter(|bridge| !bridge.is_empty()) else {
        return Vec::new();
    };
    if season_num <= 0 || episode_num <= 0 {
        return Vec::new();
    }
    let Some(coordinates) = crate::anime_numbering::community_coordinates_for_tvdb_episode(
        bridge,
        season_num,
        episode_num,
    ) else {
        return Vec::new();
    };

    let mut queries = Vec::new();
    if coordinates.season > 0 && coordinates.episode > 0 {
        queries.push(format!(
            "{} S{:0>2}E{:0>2}",
            title.name, coordinates.season, coordinates.episode
        ));
    }
    // A cour with its own name is posted under that name with a bare episode
    // number; using the series name there would find nothing.
    if let Some(season_title) =
        coordinates
            .season_title
            .as_deref()
            .map(str::trim)
            .filter(|season_title| {
                !season_title.is_empty() && !season_title.eq_ignore_ascii_case(title.name.trim())
            })
        && coordinates.episode > 0
    {
        queries.push(format!("{season_title} - {:0>2}", coordinates.episode));
    }
    // The absolute number the catalog carries, or the one the bridge's season
    // start implies when the catalog has none.
    if let Some(absolute) = episode
        .absolute_number
        .as_deref()
        .and_then(|value| value.trim().parse::<i32>().ok())
        .filter(|&value| value > 0)
        .or_else(|| {
            bridge
                .season(coordinates.season)
                .and_then(|season| season.absolute_start)
                .map(|start| start + coordinates.episode - 1)
                .filter(|&value| value > 0)
        })
    {
        queries.push(format!("{} - {:0>2}", title.name, absolute));
    }
    queries
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

    // ── community-numbered anime queries ──────────────────────────────────
    //
    // Synthetic four-cour layout: TVDB carries one official season of 60
    // episodes; the community carries four seasons of 14 / 12 / 10 / 24.

    const SERIES_NAME: &str = "Lantern Verge";
    const COUR_TITLES: [&str; 4] = [
        "Lantern Verge",
        "Lantern Verge: Ember Circuit",
        "Lantern Verge: Glass Meridian",
        "Lantern Verge: Final Chorus",
    ];
    const COUR_LENGTHS: [i32; 4] = [14, 12, 10, 24];

    fn anime_title() -> Title {
        Title {
            id: "title-1".to_string(),
            name: SERIES_NAME.to_string(),
            facet: scryer_domain::MediaFacet::Anime,
            library_id: scryer_domain::default_library_id_for_facet(
                &scryer_domain::MediaFacet::Anime,
            ),
            root_folder_id: scryer_domain::root_folder_id_for_path("/data/test"),
            monitored: true,
            tags: Vec::new(),
            canonical_tags: Vec::new(),
            external_ids: Vec::new(),
            created_by: None,
            created_at: chrono::Utc::now(),
            year: Some(2025),
            overview: None,
            poster_url: None,
            poster_source_url: None,
            background_url: None,
            background_source_url: None,
            sort_title: None,
            catalog_sort_key: String::new(),
            slug: None,
            imdb_id: None,
            runtime_minutes: None,
            popularity: None,
            content_status: None,
            language: None,
            first_aired: None,
            network: None,
            studio: None,
            country: None,
            aliases: Vec::new(),
            tagged_aliases: Vec::new(),
            metadata_language: None,
            metadata_fetched_at: None,
            min_availability: None,
            digital_release_date: None,
            folder_path: None,
        }
    }

    fn official_episode(number: u32, absolute: Option<u32>) -> Episode {
        Episode {
            id: format!("ep-{number}"),
            title_id: "title-1".to_string(),
            collection_id: Some("season-1".to_string()),
            episode_type: EpisodeType::Standard,
            episode_number: Some(number.to_string()),
            season_number: Some("1".to_string()),
            episode_label: None,
            title: None,
            air_date: None,
            duration_seconds: Some(1_440),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: absolute.map(|value| value.to_string()),
            overview: None,
            tvdb_id: None,
            image_url: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        }
    }

    fn bridge() -> AnimeNumberingBridge {
        let mut seasons = Vec::new();
        let mut tvdb_start = 1;
        for (offset, length) in COUR_LENGTHS.iter().enumerate() {
            let index = i32::try_from(offset).expect("small index") + 1;
            seasons.push(scryer_domain::AnimeCommunitySeason {
                index,
                anidb_id: Some(90_000 + i64::from(index)),
                anilist_id: None,
                mal_id: None,
                titles: vec![COUR_TITLES[offset].to_string()],
                ranges: vec![scryer_domain::AnimeCommunitySeasonRange {
                    community_episode_start: 1,
                    community_episode_end: Some(*length),
                    tvdb_season: 1,
                    tvdb_episode_start: tvdb_start,
                    tvdb_episode_end: Some(tvdb_start + length - 1),
                }],
                absolute_start: Some(tvdb_start),
                episode_count: Some(*length),
            });
            tvdb_start += length;
        }
        AnimeNumberingBridge {
            generated_on: "2026-08-30".to_string(),
            corroborating_order: Some("dvd".to_string()),
            seasons,
        }
    }

    fn wanted_episode_item(episode: &Episode) -> AcquisitionScopeState {
        AcquisitionScopeState {
            id: "wanted-1".to_string(),
            title_id: "title-1".to_string(),
            title_name: Some(SERIES_NAME.to_string()),
            title_slug: None,
            title_facet: Some("anime".to_string()),
            library_id: None,
            library_name: None,
            library_slug: None,
            episode_id: Some(episode.id.clone()),
            collection_id: None,
            series_movie_link_id: None,
            season_number: episode.season_number.clone(),
            episode_number: episode.episode_number.clone(),
            media_type: "episode".to_string(),
            last_search_at: None,
            status: crate::AcquisitionScopeStatus::Wanted,
            grabbed_release: None,
            landed_bar: None,
            latest_release_decision: None,
            mismatch_recovery_eligible: false,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    fn queries_for(episode: &Episode, bridge: Option<&AnimeNumberingBridge>) -> Vec<String> {
        let title = anime_title();
        let item = wanted_episode_item(episode);
        build_search_queries(&title, &item, Some(episode), &FacetRegistry::new(), bridge).queries
    }

    #[test]
    fn a_wanted_episode_inside_a_later_cour_is_also_searched_in_community_numbering() {
        // Official S01E56 is community S04E20 (cour 4 starts at official 37).
        let episode = official_episode(56, Some(56));
        let queries = queries_for(&episode, Some(&bridge()));

        assert!(queries.contains(&"Lantern Verge S01E56".to_string()));
        assert!(queries.contains(&"Lantern Verge S04E20".to_string()));
        assert!(queries.contains(&"Lantern Verge: Final Chorus - 20".to_string()));
        assert!(queries.contains(&"Lantern Verge - 56".to_string()));
        // The community forms come after the ones the search already used.
        let official = queries
            .iter()
            .position(|query| query == "Lantern Verge S01E56")
            .expect("official query");
        let community = queries
            .iter()
            .position(|query| query == "Lantern Verge S04E20")
            .expect("community query");
        assert!(official < community);
    }

    #[test]
    fn the_first_cour_adds_no_cour_title_query_when_it_shares_the_series_name() {
        // Official S01E03 is community S01E03 of a cour named for the series.
        let episode = official_episode(3, Some(3));
        let queries = queries_for(&episode, Some(&bridge()));

        assert!(queries.contains(&"Lantern Verge S01E03".to_string()));
        assert!(
            !queries.iter().any(
                |query| query.starts_with("Lantern Verge - 0") && query != "Lantern Verge - 03"
            )
        );
        // The cour's name is the series name, so no bare-numbered form for it.
        assert!(!queries.iter().any(|query| query == "Lantern Verge - 3"));
        // The dashed absolute form is still offered.
        assert!(queries.contains(&"Lantern Verge - 03".to_string()));
    }

    #[test]
    fn an_absolute_number_is_derived_from_the_bridge_when_the_catalog_has_none() {
        let episode = official_episode(30, None);
        let queries = queries_for(&episode, Some(&bridge()));

        // Official 30 is community S03E04; cour 3 starts at absolute 27.
        assert!(queries.contains(&"Lantern Verge S03E04".to_string()));
        assert!(queries.contains(&"Lantern Verge - 30".to_string()));
    }

    #[test]
    fn a_title_without_a_bridge_searches_exactly_as_before() {
        let episode = official_episode(56, Some(56));
        let without = queries_for(&episode, None);
        let empty = AnimeNumberingBridge::default();
        let with_empty = queries_for(&episode, Some(&empty));

        assert_eq!(without, with_empty);
        assert!(!without.iter().any(|query| query == "Lantern Verge S04E20"));
    }

    #[test]
    fn an_episode_no_community_season_covers_adds_nothing() {
        // Official season 2 is outside every bridge range.
        let mut episode = official_episode(1, None);
        episode.season_number = Some("2".to_string());
        let queries = queries_for(&episode, Some(&bridge()));

        assert!(queries.contains(&"Lantern Verge S02E01".to_string()));
        assert_eq!(
            queries.iter().filter(|query| query.contains('-')).count(),
            0
        );
    }
}

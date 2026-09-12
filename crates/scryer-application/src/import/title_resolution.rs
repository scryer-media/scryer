use crate::ParsedReleaseMetadata;
use scryer_domain::{MediaFacet, Title, TitleMatchType};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

const CONTEXT_CANDIDATE_LIMIT: usize = 8;

pub(crate) struct ResolvedMonitoredTitle<'a> {
    pub title: &'a Title,
    pub match_type: TitleMatchType,
}

#[derive(Clone, Default)]
pub(crate) struct MonitoredTitleMatcherCache {
    pub matcher: Option<Arc<MonitoredTitleMatcher>>,
    pub dirty: bool,
    /// Bumped on every invalidation; a rebuild only clears `dirty` when no
    /// invalidation raced the build, so a mid-build title change is never
    /// silently absorbed into a "clean" stale matcher.
    pub generation: u64,
    /// When the cached matcher was built. Title renames, alias updates, and
    /// monitor toggles do not reliably reach the event-driven invalidation
    /// (`TitleUpdated` has no production emitter), so age alone also dirties
    /// the cache and bounds staleness.
    pub built_at: Option<std::time::Instant>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct MonitoredTitleMatcher {
    spelling_index: Arc<crate::title_matching::relaxed::SpellingIndex>,
    titles: Vec<Title>,
    normalized_title_index: HashMap<String, Vec<usize>>,
    imdb_index: HashMap<String, Vec<usize>>,
    tmdb_index: HashMap<String, Vec<usize>>,
    /// Pillar A tier 0 collision index over ALL library titles (monitored or
    /// not — an unmonitored collider is still a collider), keyed by the
    /// year-stripped lookup key so `tide chart` and `tide chart 2023` are the
    /// same identity shape. Values are title ids, not `titles` indexes,
    /// because unmonitored titles are not resolvable and live only here.
    ambiguity_index: HashMap<String, Vec<String>>,
}

/// `Title (Year)` and bare `Title` are the same canonical identity for
/// collision purposes: the matching loop bridges the two shapes, so the
/// collision detector must too or a year-suffixed alias reads as "unique".
pub(crate) fn strip_trailing_year_key(key: &str) -> &str {
    if let Some((head, tail)) = key.rsplit_once(' ')
        && tail.len() == 4
        && tail.chars().all(|c| c.is_ascii_digit())
        && (tail.starts_with("19") || tail.starts_with("20"))
        && !head.is_empty()
    {
        return head;
    }
    key
}

impl MonitoredTitleMatcher {
    pub(crate) fn new(titles: Vec<Title>) -> Self {
        let mut matcher = Self {
            spelling_index: Arc::new(crate::title_matching::relaxed::SpellingIndex::new(&titles)),
            ..Self::default()
        };

        for title in &titles {
            for normalized in crate::acquisition_release_search::canonical_title_lookup_keys(title)
            {
                let stripped = strip_trailing_year_key(&normalized);
                let entry = matcher
                    .ambiguity_index
                    .entry(stripped.to_string())
                    .or_default();
                if !entry.contains(&title.id) {
                    entry.push(title.id.clone());
                }
            }
        }

        for title in titles.into_iter().filter(|title| title.monitored) {
            let index = matcher.titles.len();

            for normalized in crate::acquisition_release_search::canonical_title_lookup_keys(&title)
            {
                matcher
                    .normalized_title_index
                    .entry(normalized)
                    .or_default()
                    .push(index);
            }

            for external_id in &title.external_ids {
                if external_id.source.eq_ignore_ascii_case("imdb") {
                    if let Some(imdb_id) = normalize_imdb_id(&external_id.value) {
                        matcher.imdb_index.entry(imdb_id).or_default().push(index);
                    }
                } else if external_id.source.eq_ignore_ascii_case("tmdb") {
                    let tmdb_id = external_id.value.trim();
                    if !tmdb_id.is_empty() {
                        matcher
                            .tmdb_index
                            .entry(tmdb_id.to_string())
                            .or_default()
                            .push(index);
                    }
                }
            }

            matcher.titles.push(title);
        }

        matcher
    }

    /// Pillar A tier 0: the subset of `keys` that at least one *other* library
    /// title (monitored or not — an unmonitored collider is still a collider)
    /// also claims, compared on the year-stripped shape so `X` collides with
    /// `X <year>`. Index lookups only — no extra repository work.
    pub(crate) fn shared_lookup_keys(&self, title_id: &str, keys: &[String]) -> Vec<String> {
        keys.iter()
            .filter(|key| {
                self.ambiguity_index
                    .get(strip_trailing_year_key(key))
                    .is_some_and(|ids| ids.iter().any(|id| id != title_id))
            })
            .cloned()
            .collect()
    }

    /// True when any of `keys` is a lookup key some *other* library title owns
    /// — the raw name those keys came from positively asserts a different
    /// subject's identity. Year-suffix bridged like
    /// [`Self::shared_lookup_keys`].
    pub(crate) fn keys_name_another_title(&self, title_id: &str, keys: &[String]) -> bool {
        keys.iter().any(|key| {
            self.ambiguity_index
                .get(strip_trailing_year_key(key))
                .is_some_and(|ids| ids.iter().any(|id| id != title_id))
        })
    }

    pub(crate) fn identity_ambiguity(
        &self,
        title: &Title,
    ) -> crate::acquisition_release_search::TitleIdentityAmbiguity {
        crate::acquisition_release_search::TitleIdentityAmbiguity {
            shared_lookup_keys: self.shared_lookup_keys(
                &title.id,
                &crate::acquisition_release_search::canonical_title_lookup_keys(title),
            ),
            spelling_index: Some(self.spelling_index.clone()),
        }
    }

    pub(crate) fn resolve_movie<'a>(
        &'a self,
        parsed: &ParsedReleaseMetadata,
    ) -> Option<ResolvedMonitoredTitle<'a>> {
        parsed
            .imdb_id
            .as_deref()
            .and_then(normalize_imdb_id)
            .and_then(|imdb_id| {
                lookup_unique_title(
                    self.imdb_index.get(&imdb_id).map(Vec::as_slice),
                    &self.titles,
                    |title| title.facet == MediaFacet::Movie,
                )
            })
            .map(|title| ResolvedMonitoredTitle {
                title,
                match_type: TitleMatchType::IdOnly,
            })
            .or_else(|| {
                parsed
                    .tmdb_id
                    .as_deref()
                    .and_then(|tmdb_id| {
                        lookup_unique_title(
                            self.tmdb_index.get(tmdb_id).map(Vec::as_slice),
                            &self.titles,
                            |title| title.facet == MediaFacet::Movie,
                        )
                    })
                    .map(|title| ResolvedMonitoredTitle {
                        title,
                        match_type: TitleMatchType::IdOnly,
                    })
            })
            .or_else(|| {
                let (year_matches, any_matches) =
                    self.collect_name_matches(parsed, |title| title.facet == MediaFacet::Movie);

                if year_matches.len() == 1 {
                    return Some(ResolvedMonitoredTitle {
                        title: year_matches[0],
                        match_type: TitleMatchType::TitleParse,
                    });
                }

                if any_matches.len() == 1 {
                    return Some(ResolvedMonitoredTitle {
                        title: any_matches[0],
                        match_type: TitleMatchType::TitleParse,
                    });
                }

                contextual_candidate_bank_match(
                    if !year_matches.is_empty() {
                        &year_matches
                    } else {
                        &any_matches
                    },
                    parsed,
                    Some("movie"),
                )
                .map(|title| ResolvedMonitoredTitle {
                    title,
                    match_type: TitleMatchType::TitleParse,
                })
            })
    }

    pub(crate) fn resolve_episode<'a>(
        &'a self,
        parsed: &ParsedReleaseMetadata,
        facet_hint: Option<&str>,
    ) -> Option<ResolvedMonitoredTitle<'a>> {
        let mut external_matches = HashSet::new();

        if let Some(imdb_id) = parsed.imdb_id.as_deref().and_then(normalize_imdb_id) {
            for index in self.imdb_index.get(&imdb_id).into_iter().flatten().copied() {
                if self.titles.get(index).is_some_and(|title| {
                    episodic_facet_matches_hint(title.facet.clone(), facet_hint)
                }) {
                    external_matches.insert(index);
                }
            }
        }

        if let Some(tmdb_id) = parsed.tmdb_id.as_deref() {
            for index in self.tmdb_index.get(tmdb_id).into_iter().flatten().copied() {
                if self.titles.get(index).is_some_and(|title| {
                    episodic_facet_matches_hint(title.facet.clone(), facet_hint)
                }) {
                    external_matches.insert(index);
                }
            }
        }

        if external_matches.len() == 1
            && let Some(index) = external_matches.into_iter().next()
            && let Some(title) = self.titles.get(index)
        {
            return Some(ResolvedMonitoredTitle {
                title,
                match_type: TitleMatchType::IdOnly,
            });
        }

        let (year_matches, any_matches) = self.collect_name_matches(parsed, |title| {
            episodic_facet_matches_hint(title.facet.clone(), facet_hint)
        });

        if year_matches.len() == 1 {
            return Some(ResolvedMonitoredTitle {
                title: year_matches[0],
                match_type: TitleMatchType::TitleParse,
            });
        }

        (any_matches.len() == 1).then(|| ResolvedMonitoredTitle {
            title: any_matches[0],
            match_type: TitleMatchType::TitleParse,
        })
    }

    fn collect_name_matches<'a, F>(
        &'a self,
        parsed: &ParsedReleaseMetadata,
        filter: F,
    ) -> (Vec<&'a Title>, Vec<&'a Title>)
    where
        F: Fn(&Title) -> bool,
    {
        let candidates = normalized_release_title_candidates(parsed);
        if candidates.is_empty() {
            return (Vec::new(), Vec::new());
        }

        let mut year_matches = Vec::<&Title>::new();
        let mut any_matches = Vec::<&Title>::new();
        let mut seen = HashSet::<usize>::new();
        let mut seen_year = HashSet::<usize>::new();

        for candidate in candidates {
            for index in self
                .normalized_title_index
                .get(&candidate)
                .into_iter()
                .flatten()
                .copied()
            {
                let Some(title) = self.titles.get(index) else {
                    continue;
                };
                if !filter(title) {
                    continue;
                }

                if seen.insert(index) {
                    any_matches.push(title);
                }

                if let Some(year) = parsed.year
                    && title.year == Some(year)
                    && seen_year.insert(index)
                {
                    year_matches.push(title);
                }
            }
        }

        if any_matches.is_empty() {
            let (anchors, _) =
                crate::title_matching::relaxed::neutral_spelling_anchors(&parsed.raw_title);
            let ids = self.spelling_index.candidates(&anchors, None);
            for title in self
                .titles
                .iter()
                .filter(|title| ids.contains(&title.id) && filter(title))
            {
                let evidence = crate::acquisition_release_search::canonical_title_evidence(title)
                    .with_ambiguity(self.identity_ambiguity(title));
                if crate::acquisition_release_search::match_parsed_release_to_title_evidence(
                    parsed, &evidence,
                )
                .is_some()
                {
                    any_matches.push(title);
                    if parsed.year.is_some() && parsed.year == title.year {
                        year_matches.push(title);
                    }
                }
            }
        }
        (year_matches, any_matches)
    }
}

fn lookup_unique_title<'a, F>(
    indexes: Option<&[usize]>,
    titles: &'a [Title],
    filter: F,
) -> Option<&'a Title>
where
    F: Fn(&Title) -> bool,
{
    let mut matches = indexes
        .into_iter()
        .flatten()
        .copied()
        .filter_map(|index| titles.get(index))
        .filter(|title| filter(title))
        .collect::<Vec<_>>();
    (matches.len() == 1).then(|| matches.pop()).flatten()
}

#[cfg(test)]
pub(crate) fn find_monitored_movie_title_from_release(
    titles: &[Title],
    parsed: &ParsedReleaseMetadata,
) -> Option<Title> {
    resolve_monitored_movie_title_from_release(titles, parsed)
        .map(|resolved| resolved.title.clone())
}

#[cfg(test)]
pub(crate) fn find_monitored_episode_title_from_release(
    titles: &[Title],
    parsed: &ParsedReleaseMetadata,
    facet_hint: Option<&str>,
) -> Option<Title> {
    resolve_monitored_episode_title_from_release(titles, parsed, facet_hint)
        .map(|resolved| resolved.title.clone())
}

pub(crate) fn resolve_monitored_movie_title_from_release<'a>(
    titles: &'a [Title],
    parsed: &ParsedReleaseMetadata,
) -> Option<ResolvedMonitoredTitle<'a>> {
    let monitored_movies = titles
        .iter()
        .filter(|title| title.monitored && title.facet == MediaFacet::Movie)
        .collect::<Vec<_>>();

    find_title_by_external_ids(&monitored_movies, parsed)
        .map(|title| ResolvedMonitoredTitle {
            title,
            match_type: TitleMatchType::IdOnly,
        })
        .or_else(|| {
            find_movie_title_by_name(&monitored_movies, parsed).map(|title| {
                ResolvedMonitoredTitle {
                    title,
                    match_type: TitleMatchType::TitleParse,
                }
            })
        })
        .or_else(|| {
            let matcher = MonitoredTitleMatcher::new(titles.to_vec());
            let resolved = matcher.resolve_movie(parsed)?;
            titles
                .iter()
                .find(|title| title.id == resolved.title.id)
                .map(|title| ResolvedMonitoredTitle {
                    title,
                    match_type: resolved.match_type,
                })
        })
}

pub(crate) fn resolve_monitored_episode_title_from_release<'a>(
    titles: &'a [Title],
    parsed: &ParsedReleaseMetadata,
    facet_hint: Option<&str>,
) -> Option<ResolvedMonitoredTitle<'a>> {
    let monitored_episodes = titles
        .iter()
        .filter(|title| {
            title.monitored && episodic_facet_matches_hint(title.facet.clone(), facet_hint)
        })
        .collect::<Vec<_>>();

    find_unique_title_by_external_ids(&monitored_episodes, parsed)
        .map(|title| ResolvedMonitoredTitle {
            title,
            match_type: TitleMatchType::IdOnly,
        })
        .or_else(|| {
            find_unique_title_by_name(&monitored_episodes, parsed).map(|title| {
                ResolvedMonitoredTitle {
                    title,
                    match_type: TitleMatchType::TitleParse,
                }
            })
        })
        .or_else(|| {
            let matcher = MonitoredTitleMatcher::new(titles.to_vec());
            let resolved = matcher.resolve_episode(parsed, facet_hint)?;
            titles
                .iter()
                .find(|title| title.id == resolved.title.id)
                .map(|title| ResolvedMonitoredTitle {
                    title,
                    match_type: resolved.match_type,
                })
        })
}

pub(crate) fn normalize_imdb_id(raw_imdb_id: &str) -> Option<String> {
    crate::normalize::normalize_imdb_id(raw_imdb_id)
}

fn episodic_facet_matches_hint(facet: MediaFacet, facet_hint: Option<&str>) -> bool {
    match facet_hint.map(|value| value.trim().to_ascii_lowercase()) {
        Some(hint) if hint == "anime" => facet == MediaFacet::Anime,
        Some(hint) if matches!(hint.as_str(), "series" | "tv") => facet == MediaFacet::Series,
        _ => matches!(facet, MediaFacet::Series | MediaFacet::Anime),
    }
}

fn normalized_release_title_candidates(parsed: &ParsedReleaseMetadata) -> Vec<String> {
    if !parsed.raw_title.is_ascii() {
        return crate::title_matching::relaxed::neutral_spelling_anchors(&parsed.raw_title).0;
    }
    let raw_candidates = if parsed.normalized_title_variants.is_empty() {
        vec![parsed.normalized_title.clone()]
    } else {
        parsed.normalized_title_variants.clone()
    };

    raw_candidates
        .into_iter()
        .map(|title| crate::app_usecase_rss::normalize_for_matching(&title))
        .filter(|title| !title.is_empty())
        .fold(Vec::<String>::new(), |mut acc, value| {
            if !acc.iter().any(|existing| existing == &value) {
                acc.push(value);
            }
            acc
        })
}

fn title_matches_normalized_candidate(title: &Title, candidate: &str) -> bool {
    if crate::app_usecase_rss::normalize_for_matching(&title.name) == candidate {
        return true;
    }

    title
        .aliases
        .iter()
        .any(|alias| crate::app_usecase_rss::normalize_for_matching(alias) == candidate)
        || title
            .tagged_aliases
            .iter()
            .any(|alias| crate::app_usecase_rss::normalize_for_matching(&alias.name) == candidate)
}

fn find_title_by_external_ids<'a>(
    titles: &[&'a Title],
    parsed: &ParsedReleaseMetadata,
) -> Option<&'a Title> {
    if let Some(parsed_imdb_id) = parsed.imdb_id.as_deref().and_then(normalize_imdb_id) {
        let mut matches = titles
            .iter()
            .copied()
            .filter(|title| {
                title.external_ids.iter().any(|external_id| {
                    external_id.source.eq_ignore_ascii_case("imdb")
                        && normalize_imdb_id(&external_id.value).as_deref()
                            == Some(parsed_imdb_id.as_str())
                })
            })
            .collect::<Vec<_>>();
        if matches.len() == 1 {
            return matches.pop();
        }
    }

    if let Some(parsed_tmdb_id) = parsed.tmdb_id.as_deref() {
        let mut matches = titles
            .iter()
            .copied()
            .filter(|title| {
                title.external_ids.iter().any(|external_id| {
                    external_id.source.eq_ignore_ascii_case("tmdb")
                        && external_id.value.trim() == parsed_tmdb_id
                })
            })
            .collect::<Vec<_>>();
        if matches.len() == 1 {
            return matches.pop();
        }
    }

    None
}

fn find_unique_title_by_external_ids<'a>(
    titles: &[&'a Title],
    parsed: &ParsedReleaseMetadata,
) -> Option<&'a Title> {
    let matches = titles
        .iter()
        .copied()
        .filter(|title| {
            parsed
                .imdb_id
                .as_deref()
                .and_then(normalize_imdb_id)
                .is_some_and(|parsed_imdb_id| {
                    title.external_ids.iter().any(|external_id| {
                        external_id.source.eq_ignore_ascii_case("imdb")
                            && normalize_imdb_id(&external_id.value).as_deref()
                                == Some(parsed_imdb_id.as_str())
                    })
                })
                || parsed.tmdb_id.as_deref().is_some_and(|parsed_tmdb_id| {
                    title.external_ids.iter().any(|external_id| {
                        external_id.source.eq_ignore_ascii_case("tmdb")
                            && external_id.value.trim() == parsed_tmdb_id
                    })
                })
        })
        .collect::<Vec<_>>();

    (matches.len() == 1).then(|| matches[0])
}

fn find_movie_title_by_name<'a>(
    titles: &[&'a Title],
    parsed: &ParsedReleaseMetadata,
) -> Option<&'a Title> {
    let candidates = normalized_release_title_candidates(parsed);
    if candidates.is_empty() {
        return None;
    }

    let mut year_matches = Vec::<&Title>::new();
    let mut any_matches = Vec::<&Title>::new();

    for candidate in candidates {
        for title in titles {
            if !title_matches_normalized_candidate(title, &candidate) {
                continue;
            }

            if !any_matches.iter().any(|existing| existing.id == title.id) {
                any_matches.push(*title);
            }

            if let Some(year) = parsed.year
                && title.year == Some(year)
                && !year_matches.iter().any(|existing| existing.id == title.id)
            {
                year_matches.push(*title);
            }
        }
    }

    if year_matches.len() == 1 {
        return year_matches.into_iter().next();
    }

    if any_matches.len() == 1 {
        return any_matches.into_iter().next();
    }

    contextual_candidate_bank_match(
        if !year_matches.is_empty() {
            &year_matches
        } else {
            &any_matches
        },
        parsed,
        Some("movie"),
    )
}

fn find_unique_title_by_name<'a>(
    titles: &[&'a Title],
    parsed: &ParsedReleaseMetadata,
) -> Option<&'a Title> {
    let candidates = normalized_release_title_candidates(parsed);
    if candidates.is_empty() {
        return None;
    }

    let mut year_matches = Vec::<&Title>::new();
    let mut any_matches = Vec::<&Title>::new();

    for candidate in candidates {
        for title in titles {
            if !title_matches_normalized_candidate(title, &candidate) {
                continue;
            }

            if !any_matches.iter().any(|existing| existing.id == title.id) {
                any_matches.push(*title);
            }

            if let Some(year) = parsed.year
                && title.year == Some(year)
                && !year_matches.iter().any(|existing| existing.id == title.id)
            {
                year_matches.push(*title);
            }
        }
    }

    if year_matches.len() == 1 {
        return year_matches.into_iter().next();
    }

    if any_matches.len() == 1 {
        return Some(any_matches[0]);
    }
    if year_matches.is_empty() && any_matches.len() > 1 {
        return None;
    }

    contextual_candidate_bank_match(
        if !year_matches.is_empty() {
            &year_matches
        } else {
            &any_matches
        },
        parsed,
        None,
    )
}

fn contextual_candidate_bank_match<'a>(
    titles: &[&'a Title],
    parsed: &ParsedReleaseMetadata,
    _facet_hint: Option<&str>,
) -> Option<&'a Title> {
    if titles.len() < 2 || titles.len() > CONTEXT_CANDIDATE_LIMIT {
        return None;
    }

    let mut proven = titles.iter().copied().filter(|title| {
        let evidence = crate::acquisition_release_search::canonical_title_evidence(title);
        crate::acquisition_release_search::match_parsed_release_to_title_evidence(parsed, &evidence)
            .is_some_and(|evidence_match| !evidence_match.requires_external_id)
    });
    let matched = proven.next()?;
    proven.next().is_none().then_some(matched)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use scryer_domain::{ExternalId, Id, TitleMatchType};

    fn test_title(name: &str, facet: MediaFacet, year: Option<i32>, aliases: &[&str]) -> Title {
        Title {
            id: Id::new().0,
            name: name.to_string(),
            library_id: scryer_domain::default_library_id_for_facet(&facet),
            root_folder_id: scryer_domain::root_folder_id_for_path("/data/test"),
            facet,
            monitored: true,
            tags: vec![],
            canonical_tags: vec![],
            external_ids: vec![],
            created_by: None,
            created_at: Utc::now(),
            year,
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
            aliases: aliases.iter().map(|value| value.to_string()).collect(),
            tagged_aliases: vec![],
            metadata_language: None,
            metadata_fetched_at: None,
            min_availability: None,
            digital_release_date: None,
            folder_path: None,
        }
    }

    #[test]
    fn finds_unique_episodic_title_from_release_name() {
        let titles = vec![test_title(
            "RAVENCOURT The Last Regent",
            MediaFacet::Anime,
            None,
            &[],
        )];
        let parsed =
            crate::parse_release_metadata("RAVENCOURT.The.Last.Regent.S01E18.1080p.WEB-DL");

        let matched = find_monitored_episode_title_from_release(&titles, &parsed, Some("anime"))
            .expect("matched title");

        assert_eq!(matched.name, titles[0].name);
    }

    #[test]
    fn finds_episodic_title_by_alias() {
        let titles = vec![test_title(
            "House of Ravens",
            MediaFacet::Anime,
            None,
            &["RAVENCOURT The Last Regent"],
        )];
        let parsed =
            crate::parse_release_metadata("RAVENCOURT.The.Last.Regent.S01E18.1080p.WEB-DL");

        let matched = find_monitored_episode_title_from_release(&titles, &parsed, Some("anime"))
            .expect("matched title by alias");

        assert_eq!(matched.id, titles[0].id);
    }

    #[test]
    fn does_not_match_ambiguous_episodic_titles() {
        let titles = vec![
            test_title("Farwander", MediaFacet::Series, Some(2014), &[]),
            test_title("Farwander", MediaFacet::Anime, Some(2000), &[]),
        ];
        let parsed = crate::parse_release_metadata("Farwander.S08E05.1080p.WEB-DL");

        let matched = find_monitored_episode_title_from_release(&titles, &parsed, None);

        assert!(matched.is_none());
    }

    #[test]
    fn does_not_match_ambiguous_movie_titles_without_year() {
        let titles = vec![
            test_title("Cold Relic", MediaFacet::Movie, Some(1982), &[]),
            test_title("Cold Relic", MediaFacet::Movie, Some(2011), &[]),
        ];
        let mut parsed = crate::parse_release_metadata("Cold.Relic.1080p.WEB-DL");
        parsed.year = None;

        let matched = find_monitored_movie_title_from_release(&titles, &parsed);

        assert!(matched.is_none());
    }

    #[test]
    fn matches_unique_episodic_title_by_imdb_id() {
        let mut title = test_title("Completely Different Name", MediaFacet::Series, None, &[]);
        title.external_ids.push(ExternalId {
            source: "imdb".to_string(),
            value: "tt0944947".to_string(),
        });
        let titles = vec![title.clone()];
        let parsed = crate::parse_release_metadata("Farwander.S08E05.[tt0944947].1080p.WEB-DL");

        let matched = find_monitored_episode_title_from_release(&titles, &parsed, Some("series"))
            .expect("matched title by imdb id");

        assert_eq!(matched.id, title.id);
    }

    #[test]
    fn resolve_monitored_episode_title_marks_external_id_matches_as_id_only() {
        let mut title = test_title("Completely Different Name", MediaFacet::Series, None, &[]);
        title.external_ids.push(ExternalId {
            source: "imdb".to_string(),
            value: "tt0944947".to_string(),
        });
        let titles = vec![title.clone()];
        let parsed = crate::parse_release_metadata("Farwander.S08E05.[tt0944947].1080p.WEB-DL");

        let matched =
            resolve_monitored_episode_title_from_release(&titles, &parsed, Some("series"))
                .expect("matched title by imdb id");

        assert_eq!(matched.title.id, title.id);
        assert_eq!(matched.match_type, TitleMatchType::IdOnly);
    }

    #[test]
    fn resolve_monitored_episode_title_marks_name_matches_as_title_parse() {
        let titles = vec![test_title(
            "RAVENCOURT The Last Regent",
            MediaFacet::Anime,
            None,
            &[],
        )];
        let parsed =
            crate::parse_release_metadata("RAVENCOURT.The.Last.Regent.S01E18.1080p.WEB-DL");

        let matched = resolve_monitored_episode_title_from_release(&titles, &parsed, Some("anime"))
            .expect("matched title by name");

        assert_eq!(matched.title.id, titles[0].id);
        assert_eq!(matched.match_type, TitleMatchType::TitleParse);
    }

    #[test]
    fn contextual_candidate_bank_prefers_stacked_anime_alias_match() {
        let titles = [
            test_title("Random Other Show", MediaFacet::Anime, Some(2023), &[]),
            test_title(
                "Silver Horizon Beyond the Vale",
                MediaFacet::Anime,
                Some(2023),
                &["Sora no Vale", "Silver Horizon Beyond the Vale"],
            ),
        ];
        let parsed = crate::parse_release_metadata(
            "[SubsPlease] Sora.no.Vale.Silver.Horizon.Beyond.the.Vale.-.01.[1080p].[HEVC]",
        );

        let matched =
            contextual_candidate_bank_match(&[&titles[0], &titles[1]], &parsed, Some("anime"))
                .expect("contextual match");

        assert_eq!(matched.id, titles[1].id);
    }

    #[test]
    fn strip_trailing_year_key_strips_only_plausible_year_suffixes() {
        assert_eq!(strip_trailing_year_key("tide chart 2023"), "tide chart");
        assert_eq!(strip_trailing_year_key("tide chart"), "tide chart");
        assert_eq!(
            strip_trailing_year_key("signal runner 2049 2017"),
            "signal runner 2049"
        );
        assert_eq!(strip_trailing_year_key("2023"), "2023");
        assert_eq!(strip_trailing_year_key("area 5150"), "area 5150");
    }

    #[test]
    fn shared_lookup_keys_bridges_year_suffixed_titles_and_sees_unmonitored_colliders() {
        let mut anime = test_title("Tide Chart", MediaFacet::Anime, Some(1999), &[]);
        anime.monitored = false;
        let live_action = test_title(
            "Tide Chart",
            MediaFacet::Series,
            Some(2023),
            &["Tide Chart (2023)"],
        );
        let matcher = MonitoredTitleMatcher::new(vec![anime, live_action.clone()]);

        let keys = crate::acquisition_release_search::canonical_title_lookup_keys(&live_action);
        let shared = matcher.shared_lookup_keys(&live_action.id, &keys);
        assert!(
            !shared.is_empty(),
            "live-action keys must collide with the bare (unmonitored) anime: {keys:?}"
        );
        // The year-suffixed alias itself is a shared shape — this is what kills
        // the synthesized-unique-alias laundering in the matching loop.
        assert!(
            shared.iter().any(|key| key == "tide chart 2023"),
            "year-suffixed key must be recognized as shared: {shared:?}"
        );
    }
}

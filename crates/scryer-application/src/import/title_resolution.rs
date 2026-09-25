use crate::ParsedReleaseMetadata;
use scryer_domain::{MediaFacet, Title, TitleMatchType};
use std::collections::HashSet;
use std::sync::Arc;

const CONTEXT_CANDIDATE_LIMIT: usize = 8;

pub(crate) struct ResolvedMonitoredTitle {
    pub title: Title,
    pub match_type: TitleMatchType,
}

/// Release and import title resolution, answered from the persisted title
/// index.
///
/// Nothing here is sized by the catalog. The matcher used to hold every
/// monitored title, four maps over them and a spelling index over every name
/// in the library, rebuilt whenever anything in the catalog changed; it now
/// holds a repository handle and asks it for the few names a release can
/// possibly mean.
///
/// # Who resolves a title, and through which lane
///
/// Every consumer below reaches the same persisted projection: the exact
/// lanes are SQL equality over `title_search_terms` (match form,
/// romanization key, collation key, lookup key, external id) and the
/// bounded-distance lane is the tantivy index behind
/// [`crate::ports::TitleNameBucketQuery::typo_distance`]. There is no other
/// fuzzy lane and no in-memory copy of the catalog on any of these paths.
///
/// * **Acquisition search** — `acquisition/release_search.rs`:
///   `evaluate_search_results_for_subject` (exact + bounded, per-release
///   anchors loaded through `subject_with_release_anchors`), reached from
///   `acquisition/workflow/task_runner.rs:1511,3168,3388,4288`,
///   `acquisition/convergence.rs:148`, `acquisition/wanted_views.rs:639`,
///   `catalog/release_search.rs:2002,2062,2114` and
///   `catalog/workflow/queueing.rs:1050,1069,1099`. The pack and season-pack
///   arbitration at the end of the walk is deliberate and stays where it is
///   (commit b51668f76): the walk commits once, after every candidate has
///   been seen.
/// * **Interactive search** — `catalog/interactive_release_search.rs:1775,
///   1781,1786`: the same subject resolution and the same evaluation, with
///   the operator's own query as the anchor.
/// * **RSS** — `acquisition/rss.rs:1047` builds the poll's bank over this
///   matcher; `match_release_to_title_context` (`rss.rs:620`) runs the exact
///   anchor keys and their token prefixes, then the full anchored proof.
/// * **The identity gate** — `import/completed_download/check.rs:499` and
///   `import/completed_download/lookup.rs:879`: exact + bounded, with the
///   completion sources loaded as anchors before the proof.
/// * **The tracked-download sweep** — `integration/tracked_downloads.rs:1140`.
/// * **Import** — `import/workflow/series.rs:2753`
///   (`resolve_title_from_release_candidate_via_port`), called from
///   `import/workflow/series_movie.rs` for the titleless archive probe, the
///   no-release-name fallback and the srrdb filename recovery. A manual
///   import names its title by id and never reaches a matching lane.
/// * **Relaxed spelling candidates** — `library/title_matching/relaxed.rs:372`
///   is the one place `find_title_name_candidates` is called; every bounded
///   lookup above goes through it.
///
/// Not a matching consumer, and deliberately still a full read:
/// `subtitles/orchestration.rs:2158` enumerates every monitored title for the
/// subtitle sweep, which is an iteration over the catalog rather than a
/// question about one release's name.
#[derive(Clone)]
pub(crate) struct MonitoredTitleMatcher {
    titles: TitleSource,
}

/// Where a matcher's candidates come from.
///
/// `Repository` is the real path: every question below is an indexed query.
/// `Fixed` is a set the caller supplied — one title under test, a fixture —
/// and never the catalog; it answers with the same derivation the projection
/// is written from, so the two agree name for name.
#[derive(Clone)]
enum TitleSource {
    Repository(Arc<dyn crate::ports::TitleRepository>),
    /// A caller-supplied set. Only tests construct one now — production
    /// resolves through the repository — but the arms that read it are
    /// production code, so the variant is not compiled away.
    #[cfg_attr(not(test), allow(dead_code))]
    Fixed(Arc<Vec<Title>>),
}

/// `Title (Year)` and bare `Title` are the same canonical identity for
/// collision purposes: the matching loop bridges the two shapes, so the
/// collision detector must too or a year-suffixed alias reads as "unique".
pub(crate) fn strip_trailing_year_key(key: &str) -> &str {
    scryer_domain::title_spelling::strip_trailing_year(key)
}

/// Fold the names the index holds for `title` into a copy of it, as tagged
/// aliases, so `CanonicalTitleEvidence` — lookup keys and spelling identity
/// alike — sees them. Names the row already answers to are left alone.
pub(crate) fn title_with_index_names(
    title: &Title,
    names: Vec<crate::ports::TitleNameCandidate>,
) -> Title {
    let mut seen = std::iter::once(title.name.as_str())
        .chain(title.aliases.iter().map(String::as_str))
        .chain(title.tagged_aliases.iter().map(|alias| alias.name.as_str()))
        .map(crate::title_matching::canonical_lookup_key)
        .collect::<HashSet<_>>();
    let mut evidence_title = title.clone();
    for name in names {
        if name.title_id != title.id
            || !seen.insert(crate::title_matching::canonical_lookup_key(&name.raw_term))
        {
            continue;
        }
        evidence_title
            .tagged_aliases
            .push(scryer_domain::TaggedAlias {
                name: name.raw_term,
                language: name.language_tag.unwrap_or_default(),
            });
    }
    evidence_title
}

impl MonitoredTitleMatcher {
    pub(crate) fn new(titles: Arc<dyn crate::ports::TitleRepository>) -> Self {
        Self {
            titles: TitleSource::Repository(titles),
        }
    }

    /// A matcher over an explicitly supplied set of titles.
    ///
    /// Test-only since the import path stopped reading the whole catalog to
    /// match one release name: every production consumer resolves through the
    /// repository, which is the point of a persisted index.
    ///
    /// The set is the caller's — one title under test, a fixture, a request's
    /// own candidates — and never the catalog. It answers from the same
    /// derivation the projection is written with, so it agrees with the
    /// repository-backed matcher name for name.
    #[cfg(test)]
    pub(crate) fn over_titles(titles: Vec<Title>) -> Self {
        Self {
            titles: TitleSource::Fixed(Arc::new(titles)),
        }
    }

    /// `title` as the index names it: the catalog row plus every name the
    /// persisted index holds for it that the row does not carry.
    ///
    /// The index is the canonical record of what a title answers to. An anime
    /// numbering bridge's cour names are written there and nowhere on the
    /// title row, so evidence built from the row alone lets a cour name
    /// discover its title and then fail to prove it. Every proof — lookup
    /// keys, spelling identity, collision guard — is built from this.
    ///
    /// The result is evidence only. It is never the title handed back to a
    /// caller, so an index-only name can not be written onto the catalog row.
    pub(crate) async fn evidence_title(&self, title: &Title) -> crate::AppResult<Title> {
        let TitleSource::Repository(titles) = &self.titles else {
            // A fixed set is the caller's own and already whole.
            return Ok(title.clone());
        };
        let names = titles.list_title_index_names(&title.id).await?;
        Ok(title_with_index_names(title, names))
    }

    /// Pillar A tier 0: the subset of `keys` that at least one *other* library
    /// title (monitored or not — an unmonitored collider is still a collider)
    /// also claims, compared on the year-stripped shape so `X` collides with
    /// `X <year>`.
    pub(crate) async fn shared_lookup_keys(
        &self,
        title_id: &str,
        keys: &[String],
    ) -> crate::AppResult<Vec<String>> {
        match &self.titles {
            TitleSource::Repository(titles) => {
                titles
                    .lookup_keys_claimed_by_other_titles(title_id, keys)
                    .await
            }
            TitleSource::Fixed(titles) => Ok(crate::ports::lookup_keys_claimed_by_others(
                titles, title_id, keys,
            )),
        }
    }

    /// True when any of `keys` is a lookup key some *other* library title owns
    /// — the raw name those keys came from positively asserts a different
    /// subject's identity. Year-suffix bridged like
    /// [`Self::shared_lookup_keys`].
    pub(crate) async fn keys_name_another_title(
        &self,
        title_id: &str,
        keys: &[String],
    ) -> crate::AppResult<bool> {
        Ok(!self.shared_lookup_keys(title_id, keys).await?.is_empty())
    }

    pub(crate) async fn identity_ambiguity(
        &self,
        title: &Title,
    ) -> crate::AppResult<crate::acquisition_release_search::TitleIdentityAmbiguity> {
        Ok(
            crate::acquisition_release_search::TitleIdentityAmbiguity::from_shared_keys(
                self.shared_lookup_keys(
                    &title.id,
                    &crate::acquisition_release_search::canonical_title_lookup_keys(title),
                )
                .await?,
            ),
        )
    }

    /// Identity ambiguity plus the spelling candidates the subject's own names
    /// touch — the evidence shape the acquisition lane builds once per search
    /// subject, before any release is in hand.
    ///
    /// The candidates are what the relaxed lane proves a recovered spelling
    /// against: without them every non-exact spelling is refused outright, so
    /// this and [`Self::identity_ambiguity`] must not be confused.
    pub(crate) async fn evidence_ambiguity(
        &self,
        title: &Title,
    ) -> crate::AppResult<crate::acquisition_release_search::TitleIdentityAmbiguity> {
        let ambiguity = self.identity_ambiguity(title).await?;
        Ok(ambiguity.with_spelling_candidates(self.title_spelling_candidates(title).await?))
    }

    /// The spelling candidates a single subject's own names touch.
    pub(crate) async fn title_spelling_candidates(
        &self,
        title: &Title,
    ) -> crate::AppResult<Arc<crate::title_matching::relaxed::SpellingCandidates>> {
        Ok(Arc::new(match &self.titles {
            TitleSource::Repository(titles) => {
                crate::title_matching::relaxed::SpellingCandidates::load_for_title(
                    titles.as_ref(),
                    title,
                )
                .await?
            }
            TitleSource::Fixed(titles) => {
                crate::title_matching::relaxed::SpellingCandidates::from_titles(titles)
            }
        }))
    }

    /// The spelling candidates a release's anchors touch. Bounded by the
    /// anchors, not by the library.
    pub(crate) async fn spelling_candidates(
        &self,
        anchors: &[(String, String)],
        facet: Option<&str>,
    ) -> crate::AppResult<Arc<crate::title_matching::relaxed::SpellingCandidates>> {
        Ok(Arc::new(match &self.titles {
            TitleSource::Repository(titles) => {
                crate::title_matching::relaxed::SpellingCandidates::load(
                    titles.as_ref(),
                    anchors,
                    facet,
                )
                .await?
            }
            TitleSource::Fixed(titles) => {
                crate::title_matching::relaxed::SpellingCandidates::from_titles(titles)
            }
        }))
    }

    /// Fold the buckets `anchors` touch into an index already in hand.
    ///
    /// This is how evidence built from a subject picks up the release that
    /// will be compared against it: the release's name is an anchor like any
    /// other, fetched at the same distance, so the collision guard sees the
    /// competitors it has to see. `facet` is the evidence identity's: the
    /// collision guard reads only that facet's bucket.
    pub(crate) async fn extend_spelling_candidates(
        &self,
        index: &mut crate::title_matching::relaxed::SpellingCandidates,
        anchors: &[(String, String)],
        facet: &str,
    ) -> crate::AppResult<()> {
        match &self.titles {
            TitleSource::Repository(titles) => {
                index
                    .extend_for_anchors(titles.as_ref(), anchors, Some(facet))
                    .await
            }
            // A fixed set is the caller's own and is already whole: there is
            // no wider bucket to fetch.
            TitleSource::Fixed(_) => Ok(()),
        }
    }

    /// [`Self::extend_spelling_candidates`] for a batch of releases compared
    /// against one identity: only the anchors that can reach its collision
    /// check are fetched, and the index then refuses a check for any other.
    pub(crate) async fn extend_spelling_candidates_for_collisions(
        &self,
        index: &mut crate::title_matching::relaxed::SpellingCandidates,
        anchors: &[(String, String)],
        identity: &crate::title_matching::relaxed::SpellingIdentity,
    ) -> crate::AppResult<()> {
        match &self.titles {
            TitleSource::Repository(titles) => {
                index
                    .extend_for_collision_checks(titles.as_ref(), anchors, identity)
                    .await
            }
            TitleSource::Fixed(_) => Ok(()),
        }
    }

    /// Titles answering to any of `keys` on their lookup form or its
    /// year-stripped shape.
    pub(crate) async fn titles_naming_key_shapes(
        &self,
        keys: &[String],
    ) -> crate::AppResult<Vec<Title>> {
        match &self.titles {
            TitleSource::Repository(titles) => titles.find_titles_by_lookup_key_shapes(keys).await,
            TitleSource::Fixed(titles) => Ok(crate::ports::titles_matching_lookup_key_shapes(
                titles.as_ref().clone(),
                keys,
            )),
        }
    }

    pub(crate) async fn titles_by_ids(&self, ids: &[String]) -> crate::AppResult<Vec<Title>> {
        match &self.titles {
            TitleSource::Repository(titles) => titles.get_by_ids(ids).await,
            TitleSource::Fixed(titles) => Ok(titles
                .iter()
                .filter(|title| ids.contains(&title.id))
                .cloned()
                .collect()),
        }
    }

    /// Monitored titles carrying `value` for `source`, exposed for the RSS
    /// cycle's indexer-asserted id lane.
    pub(crate) async fn monitored_titles_by_external_id(
        &self,
        source: &str,
        value: &str,
    ) -> crate::AppResult<Vec<Title>> {
        self.monitored_by_external_id(source, value).await
    }

    /// Monitored titles carrying `value` for `source`, compared as the
    /// in-memory index compared them: normalized for IMDb, trimmed for TMDB.
    async fn monitored_by_external_id(
        &self,
        source: &str,
        value: &str,
    ) -> crate::AppResult<Vec<Title>> {
        let (queries, expected) = if source.eq_ignore_ascii_case("imdb") {
            let Some(normalized) = normalize_imdb_id(value) else {
                return Ok(Vec::new());
            };
            let bare = normalized.trim_start_matches("tt").to_string();
            (vec![normalized.clone(), bare], normalized)
        } else {
            let trimmed = value.trim().to_string();
            if trimmed.is_empty() {
                return Ok(Vec::new());
            }
            (vec![trimmed.clone()], trimmed)
        };

        let mut titles = Vec::new();
        let mut seen = HashSet::new();
        for query in queries {
            let found = match &self.titles {
                TitleSource::Repository(titles) => {
                    titles.find_titles_by_external_id(source, &query).await?
                }
                TitleSource::Fixed(titles) => crate::ports::titles_matching_external_id(
                    titles.as_ref().clone(),
                    source,
                    &query,
                ),
            };
            for title in found {
                if !title.monitored || !seen.insert(title.id.clone()) {
                    continue;
                }
                let claims = title.external_ids.iter().any(|external_id| {
                    if !external_id.source.eq_ignore_ascii_case(source) {
                        return false;
                    }
                    if source.eq_ignore_ascii_case("imdb") {
                        normalize_imdb_id(&external_id.value).as_deref() == Some(&expected)
                    } else {
                        external_id.value.trim() == expected
                    }
                });
                if claims {
                    titles.push(title);
                }
            }
        }
        Ok(titles)
    }

    pub(crate) async fn resolve_movie(
        &self,
        parsed: &ParsedReleaseMetadata,
    ) -> crate::AppResult<Option<ResolvedMonitoredTitle>> {
        for (source, value) in [
            ("imdb", parsed.imdb_id.as_deref()),
            ("tmdb", parsed.tmdb_id.as_deref()),
        ] {
            let Some(value) = value else {
                continue;
            };
            let mut matches = self
                .monitored_by_external_id(source, value)
                .await?
                .into_iter()
                .filter(|title| title.facet == MediaFacet::Movie)
                .collect::<Vec<_>>();
            if matches.len() == 1
                && let Some(title) = matches.pop()
            {
                return Ok(Some(ResolvedMonitoredTitle {
                    title,
                    match_type: TitleMatchType::IdOnly,
                }));
            }
        }

        let (year_matches, any_matches) = self
            .collect_name_matches(parsed, None, |title| title.facet == MediaFacet::Movie)
            .await?;

        if year_matches.len() == 1 {
            return Ok(Some(ResolvedMonitoredTitle {
                title: year_matches.into_iter().next().expect("one match"),
                match_type: TitleMatchType::TitleParse,
            }));
        }

        if any_matches.len() == 1 {
            return Ok(Some(ResolvedMonitoredTitle {
                title: any_matches.into_iter().next().expect("one match"),
                match_type: TitleMatchType::TitleParse,
            }));
        }

        let pool = if year_matches.is_empty() {
            &any_matches
        } else {
            &year_matches
        };
        Ok(
            contextual_candidate_bank_match(
                &pool.iter().collect::<Vec<_>>(),
                parsed,
                Some("movie"),
            )
            .cloned()
            .map(|title| ResolvedMonitoredTitle {
                title,
                match_type: TitleMatchType::TitleParse,
            }),
        )
    }

    pub(crate) async fn resolve_episode(
        &self,
        parsed: &ParsedReleaseMetadata,
        facet_hint: Option<&str>,
    ) -> crate::AppResult<Option<ResolvedMonitoredTitle>> {
        let mut external_matches = Vec::new();
        let mut seen = HashSet::new();
        for (source, value) in [
            ("imdb", parsed.imdb_id.as_deref()),
            ("tmdb", parsed.tmdb_id.as_deref()),
        ] {
            let Some(value) = value else {
                continue;
            };
            for title in self.monitored_by_external_id(source, value).await? {
                if episodic_facet_matches_hint(title.facet.clone(), facet_hint)
                    && seen.insert(title.id.clone())
                {
                    external_matches.push(title);
                }
            }
        }

        if external_matches.len() == 1
            && let Some(title) = external_matches.pop()
        {
            return Ok(Some(ResolvedMonitoredTitle {
                title,
                match_type: TitleMatchType::IdOnly,
            }));
        }

        let (year_matches, any_matches) = self
            .collect_name_matches(parsed, facet_hint, |title| {
                episodic_facet_matches_hint(title.facet.clone(), facet_hint)
            })
            .await?;

        if year_matches.len() == 1 {
            return Ok(Some(ResolvedMonitoredTitle {
                title: year_matches.into_iter().next().expect("one match"),
                match_type: TitleMatchType::TitleParse,
            }));
        }

        Ok((any_matches.len() == 1).then(|| ResolvedMonitoredTitle {
            title: any_matches.into_iter().next().expect("one match"),
            match_type: TitleMatchType::TitleParse,
        }))
    }

    /// The exact lane, then the guarded spelling lane when it finds nothing.
    /// Both read the persisted index; neither materializes the catalog.
    async fn collect_name_matches<F>(
        &self,
        parsed: &ParsedReleaseMetadata,
        facet_hint: Option<&str>,
        filter: F,
    ) -> crate::AppResult<(Vec<Title>, Vec<Title>)>
    where
        F: Fn(&Title) -> bool,
    {
        let candidates = normalized_release_title_candidates(parsed);
        if candidates.is_empty() {
            return Ok((Vec::new(), Vec::new()));
        }

        let mut year_matches = Vec::<Title>::new();
        let mut any_matches = Vec::<Title>::new();
        let mut seen = HashSet::<String>::new();
        let mut seen_year = HashSet::<String>::new();

        let by_key = match &self.titles {
            TitleSource::Repository(titles) => {
                titles.find_titles_by_lookup_keys(&candidates).await?
            }
            TitleSource::Fixed(titles) => {
                crate::ports::titles_matching_lookup_keys(titles.as_ref().clone(), &candidates)
            }
        };
        for title in by_key {
            if !title.monitored || !filter(&title) {
                continue;
            }
            let year_match = parsed.year.is_some() && title.year == parsed.year;
            if seen.insert(title.id.clone()) {
                if year_match && seen_year.insert(title.id.clone()) {
                    year_matches.push(title.clone());
                }
                any_matches.push(title);
            }
        }

        if !any_matches.is_empty() {
            return Ok((year_matches, any_matches));
        }

        let (anchors, _) =
            crate::title_matching::relaxed::neutral_spelling_forms(&parsed.raw_title);
        let facet = facet_hint.and_then(canonical_facet_hint);
        let spelling = self.spelling_candidates(&anchors, facet).await?;
        let ids = spelling
            .candidates(&anchors, facet)
            .into_iter()
            .collect::<Vec<_>>();
        let discovered = match &self.titles {
            TitleSource::Repository(titles) => titles.get_by_ids(&ids).await?,
            TitleSource::Fixed(titles) => titles
                .iter()
                .filter(|title| ids.contains(&title.id))
                .cloned()
                .collect(),
        };
        for title in discovered {
            if !title.monitored || !filter(&title) {
                continue;
            }
            // The spelling lane found this title through the index, so the
            // proof has to read the same names the index does.
            let evidence_title = self.evidence_title(&title).await?;
            let evidence =
                crate::acquisition_release_search::canonical_title_evidence(&evidence_title)
                    .with_ambiguity(
                        self.identity_ambiguity(&evidence_title)
                            .await?
                            .with_spelling_candidates(spelling.clone()),
                    );
            if crate::acquisition_release_search::match_parsed_release_to_title_evidence(
                parsed, &evidence,
            )
            .is_some()
            {
                if parsed.year.is_some() && parsed.year == title.year {
                    year_matches.push(title.clone());
                }
                any_matches.push(title);
            }
        }
        Ok((year_matches, any_matches))
    }
}

/// The facet a hint names, when it names exactly one. `None` searches all
/// three, which is what an unhinted release gets.
fn canonical_facet_hint(hint: &str) -> Option<&'static str> {
    match hint.trim().to_ascii_lowercase().as_str() {
        "anime" => Some("anime"),
        "series" | "tv" => Some("series"),
        "movie" => Some("movie"),
        _ => None,
    }
}

#[cfg(test)]
pub(crate) async fn find_monitored_movie_title_from_release(
    titles: &[Title],
    parsed: &ParsedReleaseMetadata,
) -> Option<Title> {
    resolve_monitored_movie_title_from_release(titles, parsed)
        .await
        .map(|resolved| resolved.title)
}

#[cfg(test)]
pub(crate) async fn find_monitored_episode_title_from_release(
    titles: &[Title],
    parsed: &ParsedReleaseMetadata,
    facet_hint: Option<&str>,
) -> Option<Title> {
    resolve_monitored_episode_title_from_release(titles, parsed, facet_hint)
        .await
        .map(|resolved| resolved.title)
}

/// Resolution over an explicitly supplied set of titles.
///
/// The set is the caller's; the matcher fallback stays inside it rather than
/// reaching for the catalog.
#[cfg(test)]
pub(crate) async fn resolve_monitored_movie_title_from_release(
    titles: &[Title],
    parsed: &ParsedReleaseMetadata,
) -> Option<ResolvedMonitoredTitle> {
    let monitored_movies = titles
        .iter()
        .filter(|title| title.monitored && title.facet == MediaFacet::Movie)
        .collect::<Vec<_>>();

    if let Some(title) = find_title_by_external_ids(&monitored_movies, parsed) {
        return Some(ResolvedMonitoredTitle {
            title: title.clone(),
            match_type: TitleMatchType::IdOnly,
        });
    }
    if let Some(title) = find_movie_title_by_name(&monitored_movies, parsed) {
        return Some(ResolvedMonitoredTitle {
            title: title.clone(),
            match_type: TitleMatchType::TitleParse,
        });
    }
    MonitoredTitleMatcher::over_titles(titles.to_vec())
        .resolve_movie(parsed)
        .await
        .ok()
        .flatten()
}

#[cfg(test)]
pub(crate) async fn resolve_monitored_episode_title_from_release(
    titles: &[Title],
    parsed: &ParsedReleaseMetadata,
    facet_hint: Option<&str>,
) -> Option<ResolvedMonitoredTitle> {
    let monitored_episodes = titles
        .iter()
        .filter(|title| {
            title.monitored && episodic_facet_matches_hint(title.facet.clone(), facet_hint)
        })
        .collect::<Vec<_>>();

    if let Some(title) = find_unique_title_by_external_ids(&monitored_episodes, parsed) {
        return Some(ResolvedMonitoredTitle {
            title: title.clone(),
            match_type: TitleMatchType::IdOnly,
        });
    }
    if let Some(title) = find_unique_title_by_name(&monitored_episodes, parsed) {
        return Some(ResolvedMonitoredTitle {
            title: title.clone(),
            match_type: TitleMatchType::TitleParse,
        });
    }
    MonitoredTitleMatcher::over_titles(titles.to_vec())
        .resolve_episode(parsed, facet_hint)
        .await
        .ok()
        .flatten()
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

#[cfg(test)]
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

#[cfg(test)]
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

#[cfg(test)]
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

#[cfg(test)]
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

#[cfg(test)]
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

    #[tokio::test]
    async fn finds_unique_episodic_title_from_release_name() {
        let titles = vec![test_title(
            "RAVENCOURT The Last Regent",
            MediaFacet::Anime,
            None,
            &[],
        )];
        let parsed =
            crate::parse_release_metadata("RAVENCOURT.The.Last.Regent.S01E18.1080p.WEB-DL");

        let matched = find_monitored_episode_title_from_release(&titles, &parsed, Some("anime"))
            .await
            .expect("matched title");

        assert_eq!(matched.name, titles[0].name);
    }

    #[tokio::test]
    async fn finds_episodic_title_by_alias() {
        let titles = vec![test_title(
            "House of Ravens",
            MediaFacet::Anime,
            None,
            &["RAVENCOURT The Last Regent"],
        )];
        let parsed =
            crate::parse_release_metadata("RAVENCOURT.The.Last.Regent.S01E18.1080p.WEB-DL");

        let matched = find_monitored_episode_title_from_release(&titles, &parsed, Some("anime"))
            .await
            .expect("matched title by alias");

        assert_eq!(matched.id, titles[0].id);
    }

    #[tokio::test]
    async fn does_not_match_ambiguous_episodic_titles() {
        let titles = vec![
            test_title("Farwander", MediaFacet::Series, Some(2014), &[]),
            test_title("Farwander", MediaFacet::Anime, Some(2000), &[]),
        ];
        let parsed = crate::parse_release_metadata("Farwander.S08E05.1080p.WEB-DL");

        let matched = find_monitored_episode_title_from_release(&titles, &parsed, None).await;

        assert!(matched.is_none());
    }

    #[tokio::test]
    async fn does_not_match_ambiguous_movie_titles_without_year() {
        let titles = vec![
            test_title("Cold Relic", MediaFacet::Movie, Some(1982), &[]),
            test_title("Cold Relic", MediaFacet::Movie, Some(2011), &[]),
        ];
        let mut parsed = crate::parse_release_metadata("Cold.Relic.1080p.WEB-DL");
        parsed.year = None;

        let matched = find_monitored_movie_title_from_release(&titles, &parsed).await;

        assert!(matched.is_none());
    }

    #[tokio::test]
    async fn matches_unique_episodic_title_by_imdb_id() {
        let mut title = test_title("Completely Different Name", MediaFacet::Series, None, &[]);
        title
            .external_ids
            .push(ExternalId::new("imdb".to_string(), "tt0944947".to_string()));
        let titles = vec![title.clone()];
        let parsed = crate::parse_release_metadata("Farwander.S08E05.[tt0944947].1080p.WEB-DL");

        let matched = find_monitored_episode_title_from_release(&titles, &parsed, Some("series"))
            .await
            .expect("matched title by imdb id");

        assert_eq!(matched.id, title.id);
    }

    #[tokio::test]
    async fn resolve_monitored_episode_title_marks_external_id_matches_as_id_only() {
        let mut title = test_title("Completely Different Name", MediaFacet::Series, None, &[]);
        title
            .external_ids
            .push(ExternalId::new("imdb".to_string(), "tt0944947".to_string()));
        let titles = vec![title.clone()];
        let parsed = crate::parse_release_metadata("Farwander.S08E05.[tt0944947].1080p.WEB-DL");

        let matched =
            resolve_monitored_episode_title_from_release(&titles, &parsed, Some("series"))
                .await
                .expect("matched title by imdb id");

        assert_eq!(matched.title.id, title.id);
        assert_eq!(matched.match_type, TitleMatchType::IdOnly);
    }

    #[tokio::test]
    async fn resolve_monitored_episode_title_marks_name_matches_as_title_parse() {
        let titles = vec![test_title(
            "RAVENCOURT The Last Regent",
            MediaFacet::Anime,
            None,
            &[],
        )];
        let parsed =
            crate::parse_release_metadata("RAVENCOURT.The.Last.Regent.S01E18.1080p.WEB-DL");

        let matched = resolve_monitored_episode_title_from_release(&titles, &parsed, Some("anime"))
            .await
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

    /// Import and manual-import candidacy both run through
    /// `collect_name_matches`, which falls back to the same canonical relaxed
    /// matcher acquisition uses. A downloaded file named in romaji must
    /// resolve to the title whose romanized alias spells the name another way.
    #[tokio::test]
    async fn resolves_a_romanized_file_name_to_the_tagged_romaji_alias() {
        let mut title = test_title(
            "Fullmetal Alchemist Brotherhood",
            MediaFacet::Anime,
            None,
            &[],
        );
        title.metadata_language = Some("eng".into());
        title.tagged_aliases = vec![scryer_domain::TaggedAlias {
            name: "Hagane no Renkinjutsushi Saigo no Gassho o Utau Toki no Hikari to Kage no Uta"
                .into(),
            language: "x-jat".into(),
        }];
        let title_id = title.id.clone();
        let matcher = MonitoredTitleMatcher::over_titles(vec![title]);
        let parsed = crate::parse_release_metadata(
            "Hagane no Renkinjutsushi Saigo no Gasshou wo Utau Toki no Hikari to Kage no Uta - 23.720p.WEB-DL.AV1.AAC2.0-NTb",
        );

        let matched = matcher
            .resolve_episode(&parsed, Some("anime"))
            .await
            .expect("resolve episode")
            .expect("a romanized file name must resolve to the romaji alias");
        assert_eq!(matched.title.id, title_id);
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

    #[tokio::test]
    async fn shared_lookup_keys_bridges_year_suffixed_titles_and_sees_unmonitored_colliders() {
        let mut anime = test_title("Tide Chart", MediaFacet::Anime, Some(1999), &[]);
        anime.monitored = false;
        let live_action = test_title(
            "Tide Chart",
            MediaFacet::Series,
            Some(2023),
            &["Tide Chart (2023)"],
        );
        let matcher = MonitoredTitleMatcher::over_titles(vec![anime, live_action.clone()]);

        let keys = crate::acquisition_release_search::canonical_title_lookup_keys(&live_action);
        let shared = matcher
            .shared_lookup_keys(&live_action.id, &keys)
            .await
            .expect("shared lookup keys");
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

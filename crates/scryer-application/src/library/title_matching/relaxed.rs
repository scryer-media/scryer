//! Guarded spelling recovery shared by acquisition and import. The index
//! contains every library identity, including titles that are not monitored.

use super::{bounded_levenshtein_distance, canonical_lookup_key};
use scryer_domain::{
    Title,
    title_spelling::{
        JAPANESE_ROMANIZATION_TAG, SpellingEquivalence, TitleScript, compare_title_spelling,
        japanese_romanization_key, title_script, title_spelling_key, title_spelling_profiles,
    },
};
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Clone, Debug)]
pub(crate) struct SpellingName {
    pub key: String,
    pub text: String,
    /// The name as the catalog wrote it. Only the numbers guard reads this:
    /// `title_numbers` decides `Rocky II` from letter case, which `key` and
    /// `text` have already lowercased away.
    pub raw: String,
    pub language: Option<String>,
    pub year: Option<i32>,
}

#[derive(Clone, Debug)]
pub(crate) struct SpellingIdentity {
    pub id: String,
    pub facet: String,
    pub names: Vec<SpellingName>,
    pub tvdb_id: Option<String>,
    pub tmdb_id: Option<String>,
    pub imdb_id: Option<String>,
}

impl SpellingIdentity {
    pub fn new(title: &Title) -> Self {
        let mut seen = HashSet::new();
        let names = title
            .tagged_aliases
            .iter()
            .map(|alias| (alias.name.as_str(), Some(alias.language.as_str())))
            .chain(std::iter::once((
                title.name.as_str(),
                title.metadata_language.as_deref(),
            )))
            .chain(
                title
                    .aliases
                    .iter()
                    .map(|name| (name.as_str(), title.metadata_language.as_deref())),
            )
            .map(|(name, language)| {
                let (text, year) =
                    scryer_domain::title_spelling::title_match_form(name, &title.name, title.year);
                SpellingName {
                    text,
                    raw: name.to_string(),
                    key: canonical_lookup_key(name),
                    language: language.map(str::to_string),
                    year,
                }
            })
            .filter(|name| !name.text.is_empty() && seen.insert(name.key.clone()))
            .collect();
        Self {
            id: title.id.clone(),
            facet: title.facet.as_str().to_string(),
            names,
            tvdb_id: crate::acquisition_search_queries::tvdb_id_from_external_ids(
                &title.external_ids,
            ),
            tmdb_id: crate::acquisition_search_queries::tmdb_id_from_external_ids(
                &title.external_ids,
            ),
            imdb_id: crate::acquisition_search_queries::imdb_id_from_title(title),
        }
    }

    pub fn parsed_ids(&self, parsed: &crate::ParsedReleaseMetadata) -> Option<bool> {
        let mut agreement = None;
        for (observed, expected) in [
            (parsed.tvdb_id.as_deref(), self.tvdb_id.as_deref()),
            (parsed.tmdb_id.as_deref(), self.tmdb_id.as_deref()),
            (parsed.imdb_id.as_deref(), self.imdb_id.as_deref()),
        ] {
            if let (Some(observed), Some(expected)) = (observed, expected) {
                if !observed.eq_ignore_ascii_case(expected) {
                    return Some(false);
                }
                agreement = Some(true);
            }
        }
        agreement
    }
}

#[derive(Clone, Debug)]
struct IndexedName {
    identity: String,
    name: SpellingName,
}

// (trigram, title length) -> (name index, occurrence count).
type TrigramIndex = HashMap<([char; 3], usize), Vec<(usize, usize)>>;

#[derive(Clone, Debug, Default)]
struct SpellingBucket {
    names: Vec<IndexedName>,
    exact: HashMap<String, Vec<usize>>,
    /// Romanization variance is not a bounded edit distance, so the trigram
    /// filter below can miss it. Keying the folded form keeps both discovery
    /// and the collision check exhaustive for romanized names.
    romanized: HashMap<String, Vec<usize>>,
    locales: HashMap<&'static str, HashMap<Vec<u8>, Vec<usize>>>,
    by_length: BTreeMap<usize, Vec<usize>>,
    grams: TrigramIndex,
}

fn trigrams(value: &str) -> HashMap<[char; 3], usize> {
    let chars = value.chars().collect::<Vec<_>>();
    let mut counts = HashMap::new();
    for window in chars.windows(3) {
        *counts.entry([window[0], window[1], window[2]]).or_default() += 1;
    }
    counts
}

impl SpellingBucket {
    fn insert(&mut self, entry: IndexedName) {
        let index = self.names.len();
        let length = entry.name.text.chars().count();
        self.exact
            .entry(entry.name.text.clone())
            .or_default()
            .push(index);
        self.by_length.entry(length).or_default().push(index);
        if let Some(key) =
            japanese_romanization_key(&entry.name.text, entry.name.language.as_deref())
        {
            self.romanized.entry(key).or_default().push(index);
        }
        for profile in title_spelling_profiles(&entry.name.text, entry.name.language.as_deref()) {
            if let Some(key) = title_spelling_key(&entry.name.text, profile) {
                self.locales
                    .entry(profile)
                    .or_default()
                    .entry(key)
                    .or_default()
                    .push(index);
            }
        }
        for (gram, count) in trigrams(&entry.name.text) {
            self.grams
                .entry((gram, length))
                .or_default()
                .push((index, count));
        }
        self.names.push(entry);
    }

    /// Exact and locale equality use direct keys. A Levenshtein edit destroys
    /// at most three trigrams, so length and multiset overlap give a lossless
    /// candidate filter before ICU/DP proof. Short collision checks with no
    /// positive overlap bound inspect only the relevant short-length buckets.
    fn possible_matches(&self, observed: &str, rival_bound: Option<usize>) -> HashSet<usize> {
        let mut possible = self
            .exact
            .get(observed)
            .into_iter()
            .flatten()
            .copied()
            .collect::<HashSet<_>>();
        // Only Japanese-romanized catalog names are keyed here, so folding the
        // observed spelling can only reach names the catalog itself marked as
        // romanizations.
        if let Some(key) = japanese_romanization_key(observed, Some("ja"))
            && let Some(indexes) = self.romanized.get(&key)
        {
            possible.extend(indexes);
        }
        for (&profile, keys) in &self.locales {
            if let Some(key) = title_spelling_key(observed, profile)
                && let Some(indexes) = keys.get(&key)
            {
                possible.extend(indexes);
            }
        }
        let nonspace_length = observed.chars().filter(|ch| !ch.is_whitespace()).count();
        if nonspace_length < 10 && rival_bound.is_none() {
            return possible;
        }
        let bound = rival_bound.unwrap_or_else(|| {
            if title_script(observed) == TitleScript::Cjk {
                (nonspace_length / 20).clamp(1, 2)
            } else {
                (nonspace_length / 10).min(3)
            }
        });
        let length = observed.chars().count();
        let grams = trigrams(observed);
        for (&other_length, indexes) in self
            .by_length
            .range(length.saturating_sub(bound)..=length.saturating_add(bound))
        {
            let required = length
                .max(other_length)
                .saturating_sub(2)
                .saturating_sub(3 * bound);
            if required == 0 {
                possible.extend(indexes);
                continue;
            }
            let mut overlaps = HashMap::<usize, usize>::new();
            for (&gram, &count) in &grams {
                if let Some(postings) = self.grams.get(&(gram, other_length)) {
                    for &(index, other_count) in postings {
                        *overlaps.entry(index).or_default() += count.min(other_count);
                    }
                }
            }
            possible.extend(
                overlaps
                    .into_iter()
                    .filter_map(|(index, overlap)| (overlap >= required).then_some(index)),
            );
        }
        possible
    }
}

type Bucket = (String, TitleScript, String);

/// The slice of the persisted name index one release's anchors touch.
///
/// This used to be `SpellingIndex`: every name in the library, bucketed in
/// memory and rebuilt whenever the catalog changed. It now holds only the
/// buckets a single matching operation asked for, fetched from
/// `title_search_terms`. The comparison below is unchanged — what changed is
/// where the names come from and how many of them are ever resident.
#[derive(Clone, Debug, Default)]
pub(crate) struct SpellingCandidates {
    buckets: HashMap<Bucket, SpellingBucket>,
    /// `(facet, anchor, numbers key)` of every anchor the projection has
    /// answered for, empty answers included. A bucket is keyed coarser than a
    /// fetch — each fetch is the names near one anchor — so a present bucket
    /// says nothing about whether *this* anchor's neighbours are in it.
    fetched: HashSet<(String, String, String)>,
    /// Set once a caller fetched only the anchors that can reach the
    /// collision check ([`Self::extend_for_collision_checks`]). From then on
    /// a collision check for an anchor nobody fetched is refused instead of
    /// answered from whatever the bucket happens to hold.
    selective: bool,
}

/// How many names one bucket's typo lane may fetch. A common bucket in a large
/// library is unbounded, and the typo lane is a length band, not a key: the cap
/// is what keeps a single release from reading the catalog. The equality lanes
/// are never capped.
const BUCKET_FETCH_LIMIT: i64 = 2_000;

/// The widest edit distance any comparison below admits, plus the one extra
/// edit a competitor check allows itself.
///
/// [`spelling_distance`] never admits more than three edits, and
/// [`SpellingCandidates::has_competitor`] asks for one more than the winning
/// distance, so nothing this module compares can be further than this from
/// the anchor. It is the ceiling on [`anchor_fetch_distance`], not the
/// distance any single anchor is fetched at: a short name cannot consume
/// anywhere near four edits, and asking the index for edits the comparison
/// will throw away is what turns a bucket read into a catalog read.
const MAX_SPELLING_DISTANCE: u8 = 4;

/// The numbers guard, owned by the domain so the persisted projection can key
/// a column on exactly what this compares. Volume II must not become Volume I
/// through a typo allowance.
///
/// Takes the spelling as written on both sides — the catalog name's `raw` and
/// the release's observed segment — because the Roman-numeral rule reads
/// letter case.
pub(crate) fn numbers(value: &str) -> Vec<String> {
    scryer_domain::title_spelling::title_numbers(value)
}

/// The same guard as [`numbers`], in the single-string form the projection
/// stores and buckets on.
fn numbers_key(value: &str) -> String {
    scryer_domain::title_spelling::title_numbers_key(value)
}

impl SpellingCandidates {
    /// Every name of an explicitly supplied set of titles.
    ///
    /// The set is the caller's, never the catalog: one title under test, or a
    /// fixture. Repository-backed callers use [`Self::load`], which reads the
    /// same names out of the projection.
    pub fn from_titles(titles: &[Title]) -> Self {
        let mut index = Self::default();
        for title in titles {
            let identity = SpellingIdentity::new(title);
            for name in identity.names {
                index.insert(&identity.facet, &identity.id, name);
            }
        }
        index
    }

    /// Fetch the buckets `anchors` touch from the persisted projection.
    ///
    /// One bucket per (anchor, facet): the same key the in-memory index
    /// bucketed on. `facet` narrows to a single one when the release names it.
    pub async fn load(
        titles: &dyn crate::ports::TitleRepository,
        anchors: &[(String, String)],
        facet: Option<&str>,
    ) -> crate::AppResult<Self> {
        Self::load_anchored(titles, anchors, facet).await
    }

    /// The buckets a *subject's own names* touch.
    ///
    /// Evidence built from a subject — the acquisition lane's search subject,
    /// the identity gate's linked title — starts here and is then widened per
    /// release by [`Self::extend_for_anchors`], so the observed spelling gets
    /// a fetch of its own at the same distance every other anchor gets. There
    /// is no wider band and no approximation: the subject's names and the
    /// release's name are both anchors.
    pub async fn load_for_title(
        titles: &dyn crate::ports::TitleRepository,
        title: &Title,
    ) -> crate::AppResult<Self> {
        let identity = SpellingIdentity::new(title);
        let anchors = identity
            .names
            .iter()
            .map(|name| (name.text.clone(), name.raw.clone()))
            .collect::<Vec<_>>();
        Self::load_anchored(titles, &anchors, Some(identity.facet.as_str())).await
    }

    /// Fold the buckets `anchors` touch into an index that already holds
    /// some. Cheap and idempotent: a name already present is inserted into
    /// the same bucket and deduplicated there.
    pub async fn extend_for_anchors(
        &mut self,
        titles: &dyn crate::ports::TitleRepository,
        anchors: &[(String, String)],
        facet: Option<&str>,
    ) -> crate::AppResult<()> {
        let fetched = Self::load_anchored(titles, anchors, facet).await?;
        for (key, bucket) in fetched.buckets {
            let target = self.buckets.entry(key).or_default();
            for indexed in bucket.names {
                target.insert(indexed);
            }
        }
        self.fetched.extend(fetched.fetched);
        Ok(())
    }

    /// Fold in only the anchors whose comparison with `identity` can reach
    /// [`Self::has_competitor`], and only in `identity`'s facet — the one
    /// bucket that check reads.
    ///
    /// A release that names the subject exactly, or names nothing near it,
    /// never consults the collision index, and fetching its neighbours was
    /// most of what a search paid for. Skipping them is only sound while the
    /// filter and the matcher agree, so the index turns selective: a
    /// collision check for an anchor this skipped is refused, never waved
    /// through on an absent bucket.
    pub async fn extend_for_collision_checks(
        &mut self,
        titles: &dyn crate::ports::TitleRepository,
        anchors: &[(String, String)],
        identity: &SpellingIdentity,
    ) -> crate::AppResult<()> {
        let reachable = anchors
            .iter()
            .filter(|(observed, observed_raw)| {
                anchor_reaches_collision_check(observed, observed_raw, identity)
            })
            .cloned()
            .collect::<Vec<_>>();
        self.selective = true;
        self.extend_for_anchors(titles, &reachable, Some(identity.facet.as_str()))
            .await
    }

    async fn load_anchored(
        titles: &dyn crate::ports::TitleRepository,
        anchors: &[(String, String)],
        facet: Option<&str>,
    ) -> crate::AppResult<Self> {
        let mut index = Self::default();
        for (anchor, observed_raw) in anchors {
            if anchor.is_empty() {
                continue;
            }
            let numbers_key = scryer_domain::title_spelling::title_numbers_key(observed_raw);
            let collation_keys = scryer_domain::title_spelling::COLLATION_PROFILES
                .iter()
                .filter_map(|profile| {
                    title_spelling_key(anchor, profile).map(|key| (*profile, key))
                })
                .collect::<Vec<_>>();
            for candidate_facet in ["movie", "series", "anime"] {
                if facet.is_some_and(|facet| facet != candidate_facet) {
                    continue;
                }
                let rows = titles
                    .find_title_name_candidates(crate::ports::TitleNameBucketQuery {
                        facet: Some(candidate_facet),
                        script: title_script(anchor).as_str(),
                        numbers_key: &numbers_key,
                        typo_distance: Some(anchor_fetch_distance(anchor)),
                        match_term: anchor,
                        romanization_key: japanese_romanization_key(anchor, Some("ja")).as_deref(),
                        collation_keys: &collation_keys,
                        limit: BUCKET_FETCH_LIMIT,
                    })
                    .await?;
                for row in rows {
                    index.insert(
                        &row.facet,
                        &row.title_id,
                        SpellingName {
                            key: row.literal_term,
                            text: row.match_term,
                            raw: row.raw_term,
                            language: row.language_tag,
                            year: row.match_year,
                        },
                    );
                }
                index.fetched.insert((
                    candidate_facet.to_string(),
                    anchor.clone(),
                    numbers_key.clone(),
                ));
            }
        }
        Ok(index)
    }

    fn insert(&mut self, facet: &str, title_id: &str, name: SpellingName) {
        self.buckets
            .entry((
                facet.to_string(),
                title_script(&name.text),
                numbers_key(&name.raw),
            ))
            .or_default()
            .insert(IndexedName {
                identity: title_id.to_string(),
                name,
            });
    }

    /// Discovery only. Full-title matching, corroboration and collisions are
    /// still checked before any returned identity can be selected.
    ///
    /// Anchors are `(lookup key, observed spelling)`: the key drives every
    /// equality and distance test, the observed spelling only the numbers
    /// guard, which reads letter case.
    pub fn candidates(&self, anchors: &[(String, String)], facet: Option<&str>) -> HashSet<String> {
        let mut ids = HashSet::new();
        for (anchor, observed_raw) in anchors {
            for kind in ["movie", "series", "anime"] {
                if facet.is_some_and(|facet| facet != kind) {
                    continue;
                }
                if let Some(bucket) = self.buckets.get(&(
                    kind.to_string(),
                    title_script(anchor),
                    numbers_key(observed_raw),
                )) {
                    for index in bucket.possible_matches(anchor, None) {
                        let entry = &bucket.names[index];
                        if spelling_distance(anchor, observed_raw, &entry.name, None).is_some() {
                            ids.insert(entry.identity.clone());
                        }
                    }
                }
            }
        }
        ids
    }

    /// How many `(facet, anchor, numbers key)` fetches this index holds.
    #[cfg(test)]
    pub fn fetched_anchor_count(&self) -> usize {
        self.fetched.len()
    }

    pub fn has_competitor(
        &self,
        identity: &SpellingIdentity,
        observed: &str,
        observed_raw: &str,
        year: Option<i32>,
        distance: usize,
    ) -> bool {
        if self.selective
            && !self.fetched.contains(&(
                identity.facet.clone(),
                observed.to_string(),
                numbers_key(observed_raw),
            ))
        {
            tracing::debug!(
                title_id = identity.id,
                observed,
                "title spelling collision check refused: anchor was never fetched"
            );
            return true;
        }
        let Some(bucket) = self.buckets.get(&(
            identity.facet.clone(),
            title_script(observed),
            numbers_key(observed_raw),
        )) else {
            return false;
        };
        let bound = distance.saturating_add(1);
        bucket
            .possible_matches(observed, Some(bound))
            .into_iter()
            .any(|index| {
                let entry = &bucket.names[index];
                entry.identity != identity.id
                    && !year
                        .zip(entry.name.year)
                        .is_some_and(|(left, right)| left != right)
                    && spelling_distance(observed, observed_raw, &entry.name, Some(bound)).is_some()
            })
    }
}

#[derive(Clone, Debug)]
pub(crate) struct SpellingMatch {
    pub key: String,
    pub observed: String,
    pub distance: usize,
    pub locale: Option<&'static str>,
    pub exact: bool,
}

/// The number of edits a comparison over names of `length` non-whitespace
/// characters admits. The one source of truth for that arithmetic:
/// [`spelling_distance`] applies it to a pair of names, and
/// [`anchor_fetch_distance`] applies it to the anchor alone to decide what to
/// ask the index for. Nothing else may restate it.
fn spelling_bound(length: usize, cjk: bool) -> usize {
    if cjk {
        (length / 20).clamp(1, 2)
    } else {
        (length / 10).min(3)
    }
}

/// The widest distance the name index has to answer for one anchor.
///
/// [`spelling_distance`] bounds a comparison by the *shorter* of the two
/// names, so the anchor's own length is an upper bound on every comparison it
/// can take part in, and [`SpellingCandidates::has_competitor`] adds exactly
/// one edit on top of a winning distance. Fetching at that number rather than
/// at [`MAX_SPELLING_DISTANCE`] keeps the bucket read proportional to what the
/// comparison can actually consume; the ceiling still applies so no caller can
/// ask the index for a distance this module would never honour.
pub(crate) fn anchor_fetch_distance(anchor: &str) -> u8 {
    let length = anchor.chars().filter(|ch| !ch.is_whitespace()).count();
    let cjk = title_script(anchor) == TitleScript::Cjk;
    let bound = spelling_bound(length, cjk).saturating_add(1);
    u8::try_from(bound)
        .unwrap_or(MAX_SPELLING_DISTANCE)
        .min(MAX_SPELLING_DISTANCE)
}

/// The one gate between a name comparison and the collision index: the
/// distance and locale when `observed` is within reach of `name`, plus
/// whether it *is* `name`. [`find_spelling_match`] consults
/// [`SpellingCandidates::has_competitor`] only for a comparison that is not
/// literally exact, and [`anchor_reaches_collision_check`] asks the same
/// question to decide what to prefetch, so the two cannot drift apart.
fn spelling_comparison(
    observed: &str,
    observed_raw: &str,
    name: &SpellingName,
) -> Option<(usize, Option<&'static str>, bool)> {
    let (distance, locale) = spelling_distance(observed, observed_raw, name, None)?;
    Some((distance, locale, observed == name.text))
}

/// Whether any comparison of `observed` with `identity`'s names can reach the
/// collision check. Year and corroboration filters come later in
/// [`find_spelling_match`] and only remove comparisons, so this is a superset
/// of the anchors that check is ever asked about.
pub(crate) fn anchor_reaches_collision_check(
    observed: &str,
    observed_raw: &str,
    identity: &SpellingIdentity,
) -> bool {
    identity.names.iter().any(|name| {
        spelling_comparison(observed, observed_raw, name)
            .is_some_and(|(_, _, literally_exact)| !literally_exact)
    })
}

fn spelling_distance(
    observed: &str,
    observed_raw: &str,
    name: &SpellingName,
    rival_bound: Option<usize>,
) -> Option<(usize, Option<&'static str>)> {
    if numbers(observed_raw) != numbers(&name.raw) {
        return None;
    }
    match compare_title_spelling(observed, &name.text, name.language.as_deref())? {
        SpellingEquivalence::Exact => return Some((0, None)),
        SpellingEquivalence::Locale(locale) => return Some((0, Some(locale))),
        SpellingEquivalence::Different => {}
    }
    let cjk = title_script(observed) == TitleScript::Cjk;
    if !cjk && observed.split_whitespace().count() != name.text.split_whitespace().count() {
        return None;
    }
    let length = observed
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .count()
        .min(name.text.chars().filter(|ch| !ch.is_whitespace()).count());
    if length < 10 && rival_bound.is_none() {
        return None;
    }
    let bound = rival_bound.unwrap_or_else(|| spelling_bound(length, cjk));
    bounded_levenshtein_distance(observed, &name.text, bound).map(|distance| (distance, None))
}

/// `anchors` are `(lookup key, observed spelling)` pairs, as
/// [`neutral_spelling_forms`] returns them. The key drives every equality and
/// distance test; the observed spelling feeds the numbers guard, which reads
/// letter case and so cannot work from the lowercased key.
pub(crate) fn find_spelling_match(
    anchors: &[(String, String)],
    identity: &SpellingIdentity,
    index: Option<&SpellingCandidates>,
    year: Option<i32>,
    ids: Option<bool>,
    episode_years: &HashSet<i32>,
) -> Option<SpellingMatch> {
    if ids == Some(false) {
        return None;
    }
    let mut best = None;
    let episode_year_matches = year.is_some_and(|year| episode_years.contains(&year));
    for (observed, observed_raw) in anchors {
        for name in &identity.names {
            if year
                .zip(name.year)
                .is_some_and(|(left, right)| left != right)
                && !episode_year_matches
            {
                continue;
            }
            let Some((distance, locale, literally_exact)) =
                spelling_comparison(observed, observed_raw, name)
            else {
                continue;
            };
            // A romanization is a transliteration convention, not a spelling a
            // release group can get wrong: the catalog carries the romanized
            // alias precisely so a release named in romaji is recognizable,
            // and an anime episode name carries neither a year nor, usually,
            // an indexer id to corroborate one. Every other locale equivalence
            // keeps its corroboration requirement. A romanization still has to
            // be the only library identity that spelling can name, which the
            // competitor check below proves.
            let romanization = distance == 0 && locale == Some(JAPANESE_ROMANIZATION_TAG);
            let exact = literally_exact || romanization;
            if !literally_exact {
                let native_typo = distance > 0 && title_script(observed) == TitleScript::Cjk;
                let year_matches = (year.is_some() && year == name.year) || episode_year_matches;
                if ids != Some(true) && !romanization && (native_typo || !year_matches) {
                    tracing::debug!(
                        title_id = identity.id,
                        observed,
                        alias = name.key,
                        native_typo,
                        "title spelling rejected: missing corroboration"
                    );
                    continue;
                }
                let Some(index) = index else {
                    tracing::debug!(
                        title_id = identity.id,
                        "title spelling rejected: collision index unavailable"
                    );
                    continue;
                };
                // An episode air year cannot eliminate a competing series
                // merely because that series started in another year.
                let collision_year = if episode_year_matches { None } else { year };
                if index.has_competitor(identity, observed, observed_raw, collision_year, distance)
                {
                    tracing::debug!(
                        title_id = identity.id,
                        observed,
                        alias = name.key,
                        distance,
                        "title spelling rejected: competing library identity"
                    );
                    continue;
                }
            }
            if best.as_ref().is_none_or(|previous: &SpellingMatch| {
                (usize::from(!exact), distance) < (usize::from(!previous.exact), previous.distance)
            }) {
                best = Some(SpellingMatch {
                    key: name.key.clone(),
                    observed: observed.clone(),
                    distance,
                    locale,
                    exact,
                });
            }
        }
    }
    best
}

/// Recover the complete observed title before any library target is supplied.
/// The widest observed segment prevents a synthesized short context from
/// projecting a small prefix out of a longer release name.
pub(crate) fn neutral_spelling_anchors(raw: &str) -> (Vec<String>, crate::ParsedReleaseMetadata) {
    let (forms, parsed) = neutral_spelling_forms(raw);
    (forms.into_iter().map(|(key, _)| key).collect(), parsed)
}

/// The distinct anchors a batch of release names asks the index for: one per
/// `(key, numbers key)`, which is what a fetch is recorded under. The numbers
/// guard reads letter case, so `MIX` and `Mix` share a key but not a fetch,
/// and deduplicating on the key alone would leave the second one unfetched.
pub(crate) fn release_batch_anchors<'a>(
    raw_titles: impl IntoIterator<Item = &'a str>,
) -> Vec<(String, String)> {
    let mut seen = HashSet::new();
    raw_titles
        .into_iter()
        .flat_map(|raw| neutral_spelling_forms(raw).0)
        .filter(|(key, raw)| seen.insert((key.clone(), numbers_key(raw))))
        .collect()
}

/// Keep the observed punctuation for the later contextual parser check.
pub(crate) fn neutral_spelling_forms(
    raw: &str,
) -> (Vec<(String, String)>, crate::ParsedReleaseMetadata) {
    let analysis = crate::quality::release_parser::neutral_release_analysis(raw);
    let Some(candidate) = analysis.best_candidate() else {
        return (Vec::new(), crate::ParsedReleaseMetadata::default());
    };
    let mut anchors = Vec::new();
    if let Some(segment) = candidate
        .title_segments
        .iter()
        .max_by_key(|segment| segment.token_end - segment.token_start)
    {
        let key = canonical_lookup_key(&segment.raw);
        if !key.is_empty() {
            if key.contains(" aka ") {
                anchors.extend(
                    key.split(" aka ")
                        .map(|part| (part.to_string(), part.to_string())),
                );
            }
            anchors.push((key, segment.raw.clone()));
        }
    }
    (anchors, candidate.projected.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(text: &str, language: &str) -> IndexedName {
        let key = canonical_lookup_key(text);
        IndexedName {
            identity: text.to_string(),
            name: SpellingName {
                text: key.clone(),
                key,
                raw: text.to_string(),
                language: Some(language.into()),
                year: Some(2019),
            },
        }
    }

    /// What the index is asked for must cover every bound the comparison can
    /// apply to that anchor, and nothing wider. The two are one decision, so
    /// they are pinned against each other rather than restated.
    #[test]
    fn the_fetch_distance_covers_every_bound_the_comparison_can_apply() {
        for anchor in [
            "Vex",
            "Zolar",
            "Quorrel Vane",
            "Quorrel Vane of the Pale Meridian",
            "Quorrel Vane of the Pale Meridian and the Long Quiet Harbour",
            "蒼雲",
            "蒼雲の記録",
            "蒼雲の記録と遠い灯りの物語について語られたこと",
        ] {
            let fetch = anchor_fetch_distance(anchor) as usize;
            assert!(
                (1..=MAX_SPELLING_DISTANCE as usize).contains(&fetch),
                "{anchor}: fetch distance {fetch} is outside the module's own ceiling"
            );
            let cjk = title_script(anchor) == TitleScript::Cjk;
            let anchor_length = anchor.chars().filter(|ch| !ch.is_whitespace()).count();
            for rival_length in 1..=120usize {
                // `spelling_distance` bounds on the shorter of the two names,
                // and `has_competitor` adds one edit to the winning distance.
                let needed = (spelling_bound(anchor_length.min(rival_length), cjk) + 1)
                    .min(MAX_SPELLING_DISTANCE as usize);
                assert!(
                    needed <= fetch,
                    "{anchor} against a {rival_length}-character name needs {needed} \
                     edits but the index is only asked for {fetch}"
                );
            }
        }
        // A short name cannot consume the blanket distance this used to send.
        assert_eq!(anchor_fetch_distance("Zolar"), 1);
        assert_eq!(anchor_fetch_distance("Quorrel Vane"), 2);
        assert_eq!(anchor_fetch_distance("蒼雲"), 2);
    }

    #[test]
    fn indexed_spelling_filter_preserves_exhaustive_matches() {
        let mut bucket = SpellingBucket::default();
        for (language, text) in [
            ("eng", "The Silver Harbour"),
            ("eng", "aaaaaaaaaaaa"),
            ("eng", "Up"),
            ("deu", "Götter über der Straße"),
            ("deu", "ÄÖÜ ÄÖÜ ÄÖÜ"),
            ("fra", "Le cœur de Chloé"),
            ("spa", "El último día"),
            ("ita", "L’amore nella città"),
            ("por", "Coração de açúcar"),
            ("rus", "Майский тихий вечер"),
            ("zho", "我们一起走过漫长而安静的夜晚"),
            ("jpn", "静かな夜に遠い空を見上げる物語"),
            ("kor", "우리가함께걸었던아름다운밤의이야기"),
        ] {
            bucket.insert(entry(text, language));
        }
        let mut queries = vec![
            canonical_lookup_key("Goetter ueber der Strasse"),
            canonical_lookup_key("AEOEUE AEOEUE AEOEUE"),
        ];
        for entry in &bucket.names {
            queries.push(entry.name.text.clone());
            let chars = entry.name.text.chars().collect::<Vec<_>>();
            for position in (0..chars.len()).step_by(3) {
                let mut substitution = chars.clone();
                substitution[position] = 'x';
                let mut insertion = chars.clone();
                insertion.insert(position, 'x');
                let mut deletion = chars.clone();
                deletion.remove(position);
                queries.extend(
                    [substitution, insertion, deletion]
                        .map(|chars| chars.into_iter().collect::<String>()),
                );
            }
            for edits in 2..=4 {
                let mut changed = chars.clone();
                for position in (0..changed.len()).step_by(3).take(edits) {
                    changed[position] = 'x';
                }
                queries.push(changed.into_iter().collect());
            }
        }
        for observed in queries {
            for bound in [None, Some(1), Some(2), Some(3), Some(4)] {
                let possible = bucket.possible_matches(&observed, bound);
                for (index, entry) in bucket.names.iter().enumerate() {
                    if spelling_distance(&observed, &observed, &entry.name, bound).is_some() {
                        assert!(
                            possible.contains(&index),
                            "lost {observed:?} / {:?}, bound {bound:?}",
                            entry.name.text
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn indexed_spelling_discovery_prunes_a_large_common_bucket() {
        let mut bucket = SpellingBucket::default();
        for mut n in 0..10_000 {
            let suffix = (0..6)
                .map(|_| {
                    let ch = char::from(b'a' + (n % 26) as u8);
                    n /= 26;
                    ch
                })
                .collect::<String>();
            bucket.insert(entry(&format!("amber meadow autumn {suffix}"), "eng"));
        }
        let target = bucket.names.len();
        bucket.insert(entry("the distant violet harbour", "eng"));
        let candidates = bucket.possible_matches("the distant violet harbor", None);
        assert_eq!(candidates, HashSet::from([target]));
        assert!(
            bucket
                .possible_matches("the distant violet planet", None)
                .is_empty()
        );
    }
}

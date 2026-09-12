//! Guarded spelling recovery shared by acquisition and import. The index
//! contains every library identity, including titles that are not monitored.

use super::{bounded_levenshtein_distance, canonical_lookup_key};
use scryer_domain::{
    Title,
    title_spelling::{
        SpellingEquivalence, TitleScript, compare_title_spelling, title_script, title_spelling_key,
        title_spelling_profiles,
    },
};
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Clone, Debug)]
pub(crate) struct SpellingName {
    pub key: String,
    pub text: String,
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
        let canonical = canonical_lookup_key(&title.name);
        let canonical_shape = crate::import_title_resolution::strip_trailing_year_key(&canonical);
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
                let key = canonical_lookup_key(name);
                let stripped = crate::import_title_resolution::strip_trailing_year_key(&key);
                let explicit_year = (stripped != key)
                    .then(|| key.rsplit_once(' ').and_then(|(_, year)| year.parse().ok()))
                    .flatten()
                    .filter(|year| {
                        Some(*year) == title.year
                            || (key != canonical && stripped != canonical_shape)
                    });
                SpellingName {
                    text: if explicit_year.is_some() {
                        stripped.to_string()
                    } else {
                        key.clone()
                    },
                    key,
                    language: language.map(str::to_string),
                    year: explicit_year.or(title.year),
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

type Bucket = (String, TitleScript, Vec<String>);

#[derive(Clone, Debug, Default)]
pub(crate) struct SpellingIndex {
    buckets: HashMap<Bucket, SpellingBucket>,
}

pub(crate) fn numbers(value: &str) -> Vec<String> {
    static ROMAN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"^m{0,3}(cm|cd|d?c{0,3})(xc|xl|l?x{0,3})(ix|iv|v?i{0,3})$")
            .expect("valid Roman numeral pattern")
    });
    value
        .split(|ch: char| !ch.is_numeric())
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        // Volume II must not become Volume I through a typo allowance. NFKC
        // also puts Unicode Roman numerals into this same spelling.
        .chain(
            value
                .split_whitespace()
                .filter(|part| ROMAN.is_match(part))
                .map(|part| format!("roman:{part}")),
        )
        .collect()
}

impl SpellingIndex {
    pub fn new(titles: &[Title]) -> Self {
        let mut index = Self::default();
        for title in titles {
            let identity = SpellingIdentity::new(title);
            for name in identity.names {
                index
                    .buckets
                    .entry((
                        identity.facet.clone(),
                        title_script(&name.text),
                        numbers(&name.text),
                    ))
                    .or_default()
                    .insert(IndexedName {
                        identity: identity.id.clone(),
                        name,
                    });
            }
        }
        index
    }

    /// Discovery only. Full-title matching, corroboration and collisions are
    /// still checked before any returned identity can be selected.
    pub fn candidates(&self, anchors: &[String], facet: Option<&str>) -> HashSet<String> {
        let mut ids = HashSet::new();
        for anchor in anchors {
            for kind in ["movie", "series", "anime"] {
                if facet.is_some_and(|facet| facet != kind) {
                    continue;
                }
                if let Some(bucket) =
                    self.buckets
                        .get(&(kind.to_string(), title_script(anchor), numbers(anchor)))
                {
                    for index in bucket.possible_matches(anchor, None) {
                        let entry = &bucket.names[index];
                        if spelling_distance(anchor, &entry.name, None).is_some() {
                            ids.insert(entry.identity.clone());
                        }
                    }
                }
            }
        }
        ids
    }

    pub fn has_competitor(
        &self,
        identity: &SpellingIdentity,
        observed: &str,
        year: Option<i32>,
        distance: usize,
    ) -> bool {
        let Some(bucket) = self.buckets.get(&(
            identity.facet.clone(),
            title_script(observed),
            numbers(observed),
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
                    && spelling_distance(observed, &entry.name, Some(bound)).is_some()
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

fn spelling_distance(
    observed: &str,
    name: &SpellingName,
    rival_bound: Option<usize>,
) -> Option<(usize, Option<&'static str>)> {
    if numbers(observed) != numbers(&name.text) {
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
    let bound = rival_bound.unwrap_or_else(|| {
        if cjk {
            (length / 20).clamp(1, 2)
        } else {
            (length / 10).min(3)
        }
    });
    bounded_levenshtein_distance(observed, &name.text, bound).map(|distance| (distance, None))
}

pub(crate) fn find_spelling_match(
    anchors: &[String],
    identity: &SpellingIdentity,
    index: Option<&SpellingIndex>,
    year: Option<i32>,
    ids: Option<bool>,
    episode_years: &HashSet<i32>,
) -> Option<SpellingMatch> {
    if ids == Some(false) {
        return None;
    }
    let mut best = None;
    let episode_year_matches = year.is_some_and(|year| episode_years.contains(&year));
    for observed in anchors {
        for name in &identity.names {
            if year
                .zip(name.year)
                .is_some_and(|(left, right)| left != right)
                && !episode_year_matches
            {
                continue;
            }
            let Some((distance, locale)) = spelling_distance(observed, name, None) else {
                continue;
            };
            let exact = observed == &name.text;
            if !exact {
                let native_typo = distance > 0 && title_script(observed) == TitleScript::Cjk;
                let year_matches = (year.is_some() && year == name.year) || episode_year_matches;
                if ids != Some(true) && (native_typo || !year_matches) {
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
                if index.has_competitor(identity, observed, collision_year, distance) {
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
                language: Some(language.into()),
                year: Some(2019),
            },
        }
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
                    if spelling_distance(&observed, &entry.name, bound).is_some() {
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

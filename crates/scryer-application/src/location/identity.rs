//! Destination-title detection by stable metadata identity (FR-055, US6/US7).
//!
//! When a title crosses into another library, the question "is this title
//! already there?" has exactly four answers, and FR-055 fixes all four:
//!
//! | Answer | Meaning |
//! |---|---|
//! | [`DestinationIdentityMatch::Unique`] | One destination title shares a canonical identity — a merge candidate. |
//! | [`DestinationIdentityMatch::None`] | Nothing shares an identity — a plain transfer into a new destination title. |
//! | [`DestinationIdentityMatch::Ambiguous`] | More than one destination title shares an identity — the user resolves it before the job starts (FR-016). |
//! | [`DestinationIdentityMatch::SameNameNoIdentity`] | A destination title has the same name and no shared identity — a transfer, **never** an auto-merge. |
//!
//! # Identities, never title text
//!
//! The match key is the `title_external_ids` pair `(source, external_id)`. Title
//! text is only ever used to *warn*: it can raise
//! [`DestinationIdentityMatch::SameNameNoIdentity`], and it can never on its own
//! produce a merge. That is the whole point of FR-055 — two unrelated films
//! called "The Gift" must not fold into each other.
//!
//! `title_external_ids` is unique on `(library_id, source, external_id)`
//! (migration 0104 replaced the earlier facet-scoped index), so *within one
//! destination library* a single identity pair can only ever point at one title.
//! Ambiguity therefore never comes from one identity — it comes from a source
//! title whose several identities disagree about which destination title they
//! mean. The stored `facet` column is retained as a sanity gate rather than part
//! of the key: a candidate whose facet could never accept the source facet
//! (movie vs episodic, per [`facets_are_compatible`]) is not a match at all,
//! while the series↔anime crossover FR-057 converts stays matchable.
//!
//! # Redirects
//!
//! Metadata sources retire ids: SMG answers a movie fetch with `from_id → to_id`
//! redirect pairs, and hydration rewrites the stored `smg` external id to the
//! surviving one (see `TitleRepository::persist_smg_id`). Scryer keeps no
//! redirect ledger of its own, so nothing guarantees both sides of a transfer
//! were hydrated since the redirect was published: the source title may still
//! hold `A` while the destination title already holds `B`. Detection therefore
//! takes the redirect edges as an input ([`IdentityRedirects`]) and canonicalises
//! *both* sides before comparing, so a stale id on either side still matches.
//!
//! # Facts in, outcome out
//!
//! Like [`crate::location::classify`], this module performs no IO. The caller
//! assembles the source title's identity set and the destination-library
//! candidates — `TitleRepository::find_by_external_id_in_library_and_facet` is
//! the existing per-identity read surface for that — and the detection here is a
//! pure function over those facts, so every FR-055 rule is testable from
//! literals.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use scryer_domain::{ExternalId, MediaFacet};

use crate::location::classify::{facets_are_compatible, reason_codes};
use crate::location::merge::DestinationIdentityMatch;

/// How far a redirect chain is followed before it is treated as unusable.
/// Metadata redirects are short in practice; the cap exists so a malformed or
/// cyclic edge set can never spin.
pub const MAX_REDIRECT_HOPS: usize = 8;

/// The metadata source SMG title-id redirects are published under.
pub const SMG_IDENTITY_SOURCE: &str = "smg";

/// One stable metadata identity: a `title_external_ids` `(source, external_id)`
/// pair, normalised.
///
/// Sources compare case-insensitively (the stores match on `LOWER(source)`);
/// external ids compare exactly after trimming, because `tt0111161` and
/// `TT0111161` are the same id but `12` and `120` are not.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MetadataIdentity {
    pub source: String,
    pub external_id: String,
}

impl MetadataIdentity {
    /// Build a normalised identity, or `None` when either half is blank — a
    /// half-empty pair identifies nothing and must never match another blank.
    pub fn new(source: impl AsRef<str>, external_id: impl AsRef<str>) -> Option<Self> {
        let source = source.as_ref().trim().to_ascii_lowercase();
        let external_id = external_id.as_ref().trim().to_string();
        if source.is_empty() || external_id.is_empty() {
            return None;
        }
        Some(Self {
            source,
            external_id,
        })
    }

    /// The catalog's own external-id shape (`source` / `value`).
    pub fn from_external_id(external_id: &ExternalId) -> Option<Self> {
        Self::new(&external_id.source, &external_id.value)
    }

    /// Normalise a whole external-id set, dropping the blanks.
    pub fn from_external_ids(external_ids: &[ExternalId]) -> Vec<Self> {
        external_ids
            .iter()
            .filter_map(Self::from_external_id)
            .collect()
    }

    /// `source:external_id`, for explanations and log lines.
    pub fn display(&self) -> String {
        format!("{}:{}", self.source, self.external_id)
    }
}

/// A `from → to` identity redirect published by a metadata source.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IdentityRedirect {
    pub from: MetadataIdentity,
    pub to: MetadataIdentity,
}

/// The redirect edges detection canonicalises identities through.
///
/// Empty is the normal case: with no edges, [`IdentityRedirects::resolve`] is the
/// identity function and detection compares raw pairs.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct IdentityRedirects {
    edges: BTreeMap<MetadataIdentity, MetadataIdentity>,
}

impl IdentityRedirects {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_edges(edges: impl IntoIterator<Item = IdentityRedirect>) -> Self {
        let mut redirects = Self::new();
        for edge in edges {
            redirects.insert(edge.from, edge.to);
        }
        redirects
    }

    /// The SMG movie-fetch shape: `(from_id, to_id)` pairs, as
    /// `MetadataGateway` hands them to hydration.
    pub fn from_smg_pairs(pairs: &[(i64, i64)]) -> Self {
        let mut redirects = Self::new();
        for (from, to) in pairs {
            if from == to {
                continue;
            }
            if let (Some(from), Some(to)) = (
                MetadataIdentity::new(SMG_IDENTITY_SOURCE, from.to_string()),
                MetadataIdentity::new(SMG_IDENTITY_SOURCE, to.to_string()),
            ) {
                redirects.insert(from, to);
            }
        }
        redirects
    }

    /// Record `from → to`. A self-edge is dropped rather than stored as a
    /// one-hop cycle.
    pub fn insert(&mut self, from: MetadataIdentity, to: MetadataIdentity) {
        if from == to {
            return;
        }
        self.edges.insert(from, to);
    }

    pub fn is_empty(&self) -> bool {
        self.edges.is_empty()
    }

    pub fn len(&self) -> usize {
        self.edges.len()
    }

    /// Follow `identity` to the end of its redirect chain.
    ///
    /// Cycles and over-long chains stop at the last identity reached rather than
    /// erroring: a broken redirect set degrades to "no redirect", which can only
    /// ever cost a merge that would have been offered — never cause a wrong one.
    pub fn resolve(&self, identity: &MetadataIdentity) -> MetadataIdentity {
        let mut current = identity.clone();
        let mut seen: BTreeSet<MetadataIdentity> = BTreeSet::new();
        seen.insert(current.clone());
        for _ in 0..MAX_REDIRECT_HOPS {
            let Some(next) = self.edges.get(&current) else {
                break;
            };
            if !seen.insert(next.clone()) {
                break;
            }
            current = next.clone();
        }
        current
    }

    fn resolve_all(&self, identities: &[MetadataIdentity]) -> BTreeSet<MetadataIdentity> {
        identities
            .iter()
            .map(|identity| self.resolve(identity))
            .collect()
    }
}

/// The title being placed into the destination library.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceTitleIdentity {
    pub title_id: String,
    /// Only ever used for the same-name warning, never to match.
    pub title_name: String,
    pub facet: MediaFacet,
    pub identities: Vec<MetadataIdentity>,
}

impl SourceTitleIdentity {
    pub fn new(title_id: impl Into<String>, facet: MediaFacet) -> Self {
        let title_id = title_id.into();
        Self {
            title_name: title_id.clone(),
            title_id,
            facet,
            identities: Vec::new(),
        }
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.title_name = name.into();
        self
    }

    pub fn with_identity(mut self, source: impl AsRef<str>, external_id: impl AsRef<str>) -> Self {
        if let Some(identity) = MetadataIdentity::new(source, external_id) {
            self.identities.push(identity);
        }
        self
    }

    pub fn with_external_ids(mut self, external_ids: &[ExternalId]) -> Self {
        self.identities = MetadataIdentity::from_external_ids(external_ids);
        self
    }
}

/// One title already living in the destination library, as the caller read it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DestinationTitleCandidate {
    pub title_id: String,
    pub title_name: String,
    pub facet: MediaFacet,
    pub identities: Vec<MetadataIdentity>,
}

impl DestinationTitleCandidate {
    pub fn new(title_id: impl Into<String>, facet: MediaFacet) -> Self {
        let title_id = title_id.into();
        Self {
            title_name: title_id.clone(),
            title_id,
            facet,
            identities: Vec::new(),
        }
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.title_name = name.into();
        self
    }

    pub fn with_identity(mut self, source: impl AsRef<str>, external_id: impl AsRef<str>) -> Self {
        if let Some(identity) = MetadataIdentity::new(source, external_id) {
            self.identities.push(identity);
        }
        self
    }

    pub fn with_external_ids(mut self, external_ids: &[ExternalId]) -> Self {
        self.identities = MetadataIdentity::from_external_ids(external_ids);
        self
    }
}

/// A destination title that shares at least one canonical identity with the
/// source title, and the identities that agreed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IdentityCandidate {
    pub title_id: String,
    pub title_name: String,
    /// Canonical (redirect-resolved) identities present on both sides, sorted so
    /// the outcome is stable across candidate ordering.
    pub shared_identities: Vec<MetadataIdentity>,
}

impl IdentityCandidate {
    /// `"Name (tvdb:1234, imdb:tt1)"` — the phrasing the resolution prompt uses.
    pub fn describe(&self) -> String {
        let identities = self
            .shared_identities
            .iter()
            .map(MetadataIdentity::display)
            .collect::<Vec<_>>()
            .join(", ");
        if identities.is_empty() {
            self.title_name.clone()
        } else {
            format!("{} ({identities})", self.title_name)
        }
    }
}

/// What detection concluded about one source title's destination.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DestinationIdentityOutcome {
    pub match_kind: DestinationIdentityMatch,
    /// The destination title to merge into. Only ever set for
    /// [`DestinationIdentityMatch::Unique`].
    pub matched_title_id: Option<String>,
    /// Every destination title an identity pointed at, in candidate order. One
    /// entry for `Unique`, two or more for `Ambiguous`, empty otherwise.
    pub candidates: Vec<IdentityCandidate>,
    /// A destination title carrying the same name but no shared identity. Set
    /// whenever one exists — including alongside a `Unique` match on a
    /// *different* title — so the preview can always say so (FR-055).
    pub same_name_title_id: Option<String>,
    pub same_name_title_name: Option<String>,
}

impl DestinationIdentityOutcome {
    /// The "nothing in the destination relates to this title" outcome: a plain
    /// transfer into a new destination title.
    pub fn transfer() -> Self {
        Self {
            match_kind: DestinationIdentityMatch::None,
            matched_title_id: None,
            candidates: Vec::new(),
            same_name_title_id: None,
            same_name_title_name: None,
        }
    }

    /// The destination title this source title merges into, if any. `None` for
    /// every non-`Unique` outcome — in particular for `SameNameNoIdentity`,
    /// which is a transfer no matter how convincing the name is.
    pub fn merge_target(&self) -> Option<&str> {
        match self.match_kind {
            DestinationIdentityMatch::Unique => self.matched_title_id.as_deref(),
            _ => None,
        }
    }

    /// Whether this title enters the merge path (US7) rather than the
    /// transfer-without-match path (FR-056).
    pub fn is_merge_candidate(&self) -> bool {
        self.merge_target().is_some()
    }

    /// Whether the user must decide before the job can start (FR-016, FR-055).
    pub fn needs_resolution(&self) -> bool {
        self.match_kind.needs_resolution()
    }

    /// Ids of the destination titles the user is choosing between. Empty unless
    /// the outcome is ambiguous.
    pub fn ambiguous_title_ids(&self) -> Vec<String> {
        if !self.needs_resolution() {
            return Vec::new();
        }
        self.candidates
            .iter()
            .map(|candidate| candidate.title_id.clone())
            .collect()
    }

    /// The machine-readable blocked reason for an outcome that blocks the start,
    /// reusing [`reason_codes`] rather than a parallel vocabulary (FR-016).
    pub fn reason_code(&self) -> Option<&'static str> {
        self.needs_resolution()
            .then_some(reason_codes::AMBIGUOUS_DESTINATION_IDENTITY)
    }

    /// The prose shown beside the resolution control, naming every candidate.
    pub fn blocked_reason(&self, source_title_name: &str) -> Option<String> {
        if !self.needs_resolution() {
            return None;
        }
        let candidates = self
            .candidates
            .iter()
            .map(IdentityCandidate::describe)
            .collect::<Vec<_>>()
            .join("; ");
        Some(format!(
            "\"{source_title_name}\" matches more than one title in the destination library ({candidates}); choose which one it is before starting"
        ))
    }
}

/// Detect the destination title for one source title (FR-055).
///
/// The rules, in order:
///
/// 1. A candidate whose facet could never accept the source facet is not a
///    candidate at all — the movie/episodic boundary is not crossed by an
///    identity coincidence. Series↔anime stays matchable (FR-057).
/// 2. The source title itself is never its own destination.
/// 3. Both sides' identities are canonicalised through `redirects`, then
///    intersected. Any non-empty intersection makes the candidate a match,
///    regardless of how the titles are spelled.
/// 4. One match → `Unique`; two or more distinct destination titles → `Ambiguous`;
///    none → `SameNameNoIdentity` if a same-named candidate exists, else `None`.
///
/// Several identity sources agreeing on one destination is still exactly one
/// destination title, so it is `Unique`, not `Ambiguous`.
pub fn detect_destination_title(
    source: &SourceTitleIdentity,
    candidates: &[DestinationTitleCandidate],
    redirects: &IdentityRedirects,
) -> DestinationIdentityOutcome {
    let source_identities = redirects.resolve_all(&source.identities);
    let source_name = normalize_title_name(&source.title_name);

    let mut matches: Vec<IdentityCandidate> = Vec::new();
    let mut same_name: Option<(String, String)> = None;
    let mut seen_title_ids: BTreeSet<&str> = BTreeSet::new();

    for candidate in candidates {
        if candidate.title_id == source.title_id {
            continue;
        }
        if !facets_are_compatible(&source.facet, &candidate.facet) {
            continue;
        }
        if !seen_title_ids.insert(candidate.title_id.as_str()) {
            continue;
        }

        let shared: Vec<MetadataIdentity> = redirects
            .resolve_all(&candidate.identities)
            .into_iter()
            .filter(|identity| source_identities.contains(identity))
            .collect();

        if shared.is_empty() {
            if same_name.is_none()
                && !source_name.is_empty()
                && normalize_title_name(&candidate.title_name) == source_name
            {
                same_name = Some((candidate.title_id.clone(), candidate.title_name.clone()));
            }
            continue;
        }

        matches.push(IdentityCandidate {
            title_id: candidate.title_id.clone(),
            title_name: candidate.title_name.clone(),
            // `resolve_all` returns a BTreeSet, so this is already sorted.
            shared_identities: shared,
        });
    }

    let (match_kind, matched_title_id) = match matches.len() {
        0 if same_name.is_some() => (DestinationIdentityMatch::SameNameNoIdentity, None),
        0 => (DestinationIdentityMatch::None, None),
        1 => (
            DestinationIdentityMatch::Unique,
            Some(matches[0].title_id.clone()),
        ),
        _ => (DestinationIdentityMatch::Ambiguous, None),
    };

    let (same_name_title_id, same_name_title_name) = match same_name {
        Some((id, name)) => (Some(id), Some(name)),
        None => (None, None),
    };

    DestinationIdentityOutcome {
        match_kind,
        matched_title_id,
        candidates: matches,
        same_name_title_id,
        same_name_title_name,
    }
}

/// Detect destination titles for a whole selection, keyed by source title id.
///
/// **This is the entry point the cross-library preview wiring calls.** It takes
/// the destination library's candidate titles once and answers for every
/// selected title, so a bulk selection spanning several source libraries costs
/// one read of the destination library rather than one per title. The result
/// feeds [`crate::location::classify::TitleClassificationFacts::with_destination_identity`],
/// which is what turns an ambiguous outcome into a blocked title (FR-016).
pub fn detect_destination_titles(
    sources: &[SourceTitleIdentity],
    candidates: &[DestinationTitleCandidate],
    redirects: &IdentityRedirects,
) -> BTreeMap<String, DestinationIdentityOutcome> {
    sources
        .iter()
        .map(|source| {
            (
                source.title_id.clone(),
                detect_destination_title(source, candidates, redirects),
            )
        })
        .collect()
}

/// Case- and whitespace-insensitive title text, for the same-name warning only.
fn normalize_title_name(name: &str) -> String {
    name.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(id: &str) -> SourceTitleIdentity {
        SourceTitleIdentity::new(id, MediaFacet::Movie).with_name(id)
    }

    fn candidate(id: &str) -> DestinationTitleCandidate {
        DestinationTitleCandidate::new(id, MediaFacet::Movie).with_name(id)
    }

    fn no_redirects() -> IdentityRedirects {
        IdentityRedirects::new()
    }

    /// FR-055: exactly one destination title shares the identity, so the title
    /// is a merge candidate pointing at it.
    #[test]
    fn one_shared_identity_is_a_unique_merge_candidate() {
        let source = source("src").with_identity("tmdb", "603");
        let candidates = vec![
            candidate("other").with_identity("tmdb", "604"),
            candidate("dest").with_identity("tmdb", "603"),
        ];

        let outcome = detect_destination_title(&source, &candidates, &no_redirects());

        assert_eq!(outcome.match_kind, DestinationIdentityMatch::Unique);
        assert_eq!(outcome.merge_target(), Some("dest"));
        assert!(outcome.is_merge_candidate());
        assert!(!outcome.needs_resolution());
        assert_eq!(outcome.candidates.len(), 1);
        assert_eq!(
            outcome.candidates[0].shared_identities,
            vec![MetadataIdentity::new("tmdb", "603").expect("identity")]
        );
    }

    /// FR-055's "and redirects": the source still holds the retired id while the
    /// destination already holds the surviving one, and they must still match.
    #[test]
    fn a_redirected_identity_still_matches_its_successor() {
        let source = source("src").with_identity(SMG_IDENTITY_SOURCE, "100");
        let candidates = vec![candidate("dest").with_identity(SMG_IDENTITY_SOURCE, "202")];
        let redirects = IdentityRedirects::from_smg_pairs(&[(100, 202)]);

        let outcome = detect_destination_title(&source, &candidates, &redirects);

        assert_eq!(outcome.match_kind, DestinationIdentityMatch::Unique);
        assert_eq!(outcome.merge_target(), Some("dest"));

        // Without the redirect edges the same facts are a plain transfer, which
        // is what makes the redirect load-bearing rather than incidental.
        let without = detect_destination_title(&source, &candidates, &no_redirects());
        assert_eq!(without.match_kind, DestinationIdentityMatch::None);
    }

    /// The stale id can sit on either side: only one of the two titles has been
    /// re-hydrated since the redirect was published.
    #[test]
    fn a_redirect_matches_in_both_directions() {
        let redirects = IdentityRedirects::from_smg_pairs(&[(100, 202)]);
        let stale_destination = vec![candidate("dest").with_identity(SMG_IDENTITY_SOURCE, "100")];
        let current_source = source("src").with_identity(SMG_IDENTITY_SOURCE, "202");

        let outcome = detect_destination_title(&current_source, &stale_destination, &redirects);

        assert_eq!(outcome.match_kind, DestinationIdentityMatch::Unique);
        assert_eq!(outcome.merge_target(), Some("dest"));
    }

    /// A redirect chain resolves to its terminal id, and a cycle degrades to
    /// "no redirect" instead of spinning.
    #[test]
    fn redirect_chains_terminate_and_cycles_do_not_spin() {
        let redirects = IdentityRedirects::from_smg_pairs(&[(1, 2), (2, 3)]);
        let source = source("src").with_identity(SMG_IDENTITY_SOURCE, "1");
        let candidates = vec![candidate("dest").with_identity(SMG_IDENTITY_SOURCE, "3")];

        assert_eq!(
            detect_destination_title(&source, &candidates, &redirects).merge_target(),
            Some("dest")
        );

        let cyclic = IdentityRedirects::from_smg_pairs(&[(1, 2), (2, 1)]);
        let resolved =
            cyclic.resolve(&MetadataIdentity::new(SMG_IDENTITY_SOURCE, "1").expect("id"));
        assert!(
            resolved.external_id == "1" || resolved.external_id == "2",
            "a cycle stops on the chain, got {resolved:?}"
        );
    }

    /// FR-055: nothing shares an identity and nothing shares a name, so the
    /// title transfers into a new destination title.
    #[test]
    fn no_shared_identity_is_a_plain_transfer() {
        let source = source("src")
            .with_name("Arrival")
            .with_identity("tmdb", "329865");
        let candidates = vec![
            candidate("a")
                .with_name("Sicario")
                .with_identity("tmdb", "273481"),
            candidate("b")
                .with_name("Dune")
                .with_identity("tmdb", "438631"),
        ];

        let outcome = detect_destination_title(&source, &candidates, &no_redirects());

        assert_eq!(outcome.match_kind, DestinationIdentityMatch::None);
        assert_eq!(outcome, DestinationIdentityOutcome::transfer());
        assert!(!outcome.is_merge_candidate());
        assert!(!outcome.needs_resolution());
        assert!(outcome.candidates.is_empty());
    }

    /// A destination library with nothing in it at all is still a transfer.
    #[test]
    fn an_empty_destination_library_is_a_plain_transfer() {
        let outcome = detect_destination_title(
            &source("src").with_identity("tmdb", "603"),
            &[],
            &no_redirects(),
        );
        assert_eq!(outcome.match_kind, DestinationIdentityMatch::None);
    }

    /// FR-055: two destination titles claim identities this title holds, so the
    /// user resolves it before the job starts — it is never auto-merged into
    /// either one.
    #[test]
    fn two_identity_candidates_are_ambiguous_and_block_the_start() {
        let source = source("src")
            .with_name("The Gift")
            .with_identity("tmdb", "603")
            .with_identity("imdb", "tt0133093");
        let candidates = vec![
            candidate("dest-a")
                .with_name("The Matrix")
                .with_identity("tmdb", "603"),
            candidate("dest-b")
                .with_name("The Matrix Reloaded")
                .with_identity("imdb", "tt0133093"),
        ];

        let outcome = detect_destination_title(&source, &candidates, &no_redirects());

        assert_eq!(outcome.match_kind, DestinationIdentityMatch::Ambiguous);
        assert!(outcome.needs_resolution());
        assert!(
            outcome.merge_target().is_none(),
            "an ambiguous outcome must never name a merge target"
        );
        assert!(!outcome.is_merge_candidate());
        assert_eq!(
            outcome.ambiguous_title_ids(),
            vec!["dest-a".to_string(), "dest-b".to_string()]
        );
        assert_eq!(
            outcome.reason_code(),
            Some(reason_codes::AMBIGUOUS_DESTINATION_IDENTITY)
        );
        let reason = outcome.blocked_reason("The Gift").expect("blocked reason");
        assert!(reason.contains("The Gift"), "names the source: {reason}");
        assert!(reason.contains("The Matrix"), "names a candidate: {reason}");
        assert!(
            reason.contains("tmdb:603"),
            "names the identity that pointed there: {reason}"
        );
    }

    /// FR-055: same name, no shared identity — never an auto-merge. The outcome
    /// still records the same-named title so the preview can say it exists.
    #[test]
    fn same_name_without_identity_is_never_merged() {
        let source = source("src")
            .with_name("The Gift")
            .with_identity("tmdb", "603");
        let candidates = vec![
            candidate("dest")
                .with_name("the  GIFT")
                .with_identity("tmdb", "9999"),
        ];

        let outcome = detect_destination_title(&source, &candidates, &no_redirects());

        assert_eq!(
            outcome.match_kind,
            DestinationIdentityMatch::SameNameNoIdentity
        );
        assert!(
            outcome.merge_target().is_none(),
            "FR-055: same-name-without-identity is a transfer"
        );
        assert!(!outcome.is_merge_candidate());
        assert!(
            !outcome.needs_resolution(),
            "it is a transfer, not a decision the user owes"
        );
        assert_eq!(outcome.same_name_title_id.as_deref(), Some("dest"));
        assert_eq!(outcome.same_name_title_name.as_deref(), Some("the  GIFT"));
        assert!(outcome.candidates.is_empty());
    }

    /// A blank name never matches another blank name.
    #[test]
    fn blank_names_do_not_produce_a_same_name_warning() {
        let source = SourceTitleIdentity::new("src", MediaFacet::Movie)
            .with_name("   ")
            .with_identity("tmdb", "603");
        let candidates = vec![
            DestinationTitleCandidate::new("dest", MediaFacet::Movie)
                .with_name("")
                .with_identity("tmdb", "9999"),
        ];

        let outcome = detect_destination_title(&source, &candidates, &no_redirects());

        assert_eq!(outcome.match_kind, DestinationIdentityMatch::None);
        assert!(outcome.same_name_title_id.is_none());
    }

    /// FR-055's core claim: identity decides, title text does not. The names
    /// disagree completely and the merge still happens; a same-named stranger is
    /// recorded beside it rather than winning.
    #[test]
    fn identity_match_beats_a_name_mismatch() {
        let source = source("src")
            .with_name("Le Fabuleux Destin d'Amélie Poulain")
            .with_identity("tmdb", "194");
        let candidates = vec![
            candidate("same-name")
                .with_name("Le Fabuleux Destin d'Amélie Poulain")
                .with_identity("tmdb", "999999"),
            candidate("real")
                .with_name("Amélie")
                .with_identity("tmdb", "194"),
        ];

        let outcome = detect_destination_title(&source, &candidates, &no_redirects());

        assert_eq!(outcome.match_kind, DestinationIdentityMatch::Unique);
        assert_eq!(outcome.merge_target(), Some("real"));
        // The name twin is still surfaced, so the preview can warn about it.
        assert_eq!(outcome.same_name_title_id.as_deref(), Some("same-name"));
    }

    /// Several identity sources agreeing on one destination title is one
    /// destination title: `Unique`, not `Ambiguous`.
    #[test]
    fn multiple_sources_agreeing_on_one_destination_is_unique() {
        let source = source("src")
            .with_identity("tmdb", "603")
            .with_identity("imdb", "tt0133093")
            .with_identity("smg", "42");
        let candidates = vec![
            candidate("dest")
                .with_identity("tmdb", "603")
                .with_identity("imdb", "tt0133093")
                .with_identity("smg", "42"),
        ];

        let outcome = detect_destination_title(&source, &candidates, &no_redirects());

        assert_eq!(outcome.match_kind, DestinationIdentityMatch::Unique);
        assert_eq!(outcome.merge_target(), Some("dest"));
        assert_eq!(outcome.candidates.len(), 1);
        assert_eq!(
            outcome.candidates[0].shared_identities.len(),
            3,
            "every agreeing identity is evidence on the one candidate"
        );
    }

    /// Conflicting sources pointing at different destination titles is the
    /// ambiguous case, even though each individual identity is unambiguous.
    #[test]
    fn conflicting_sources_pointing_at_different_destinations_is_ambiguous() {
        let source = source("src")
            .with_identity("tmdb", "603")
            .with_identity("imdb", "tt0133093");
        let candidates = vec![
            candidate("by-tmdb").with_identity("tmdb", "603"),
            candidate("by-imdb").with_identity("imdb", "tt0133093"),
        ];

        let outcome = detect_destination_title(&source, &candidates, &no_redirects());

        assert_eq!(outcome.match_kind, DestinationIdentityMatch::Ambiguous);
        assert_eq!(outcome.candidates.len(), 2);
        assert!(outcome.needs_resolution());
    }

    /// `title_external_ids` is unique on `(library_id, source, external_id)`, so
    /// the facet column is a sanity gate rather than part of the key: a movie
    /// never merges into an episodic title on an id collision, while the
    /// series↔anime crossover FR-057 converts still matches.
    #[test]
    fn facet_gates_the_movie_episodic_boundary_but_not_series_anime() {
        let series = SourceTitleIdentity::new("src", MediaFacet::Series)
            .with_name("Cowboy Bebop")
            .with_identity("tvdb", "76885");

        let anime_candidate = vec![
            DestinationTitleCandidate::new("anime", MediaFacet::Anime)
                .with_name("Cowboy Bebop")
                .with_identity("tvdb", "76885"),
        ];
        let outcome = detect_destination_title(&series, &anime_candidate, &no_redirects());
        assert_eq!(outcome.match_kind, DestinationIdentityMatch::Unique);
        assert_eq!(outcome.merge_target(), Some("anime"));

        let movie_candidate = vec![
            DestinationTitleCandidate::new("movie", MediaFacet::Movie)
                .with_name("Cowboy Bebop")
                .with_identity("tvdb", "76885"),
        ];
        let outcome = detect_destination_title(&series, &movie_candidate, &no_redirects());
        assert_eq!(
            outcome.match_kind,
            DestinationIdentityMatch::None,
            "an id collision across the movie/episodic boundary is not a match"
        );
        assert!(
            outcome.same_name_title_id.is_none(),
            "an incompatible candidate is not even a same-name warning"
        );
    }

    /// Sources are case-insensitive (the stores match on `LOWER(source)`);
    /// external ids are exact after trimming.
    #[test]
    fn source_case_is_ignored_and_external_ids_are_exact() {
        let source = source("src").with_identity("TMDB", " 603 ");
        let candidates = vec![candidate("dest").with_identity("tmdb", "603")];
        assert_eq!(
            detect_destination_title(&source, &candidates, &no_redirects()).merge_target(),
            Some("dest")
        );

        let near_miss = vec![candidate("dest").with_identity("tmdb", "6030")];
        assert_eq!(
            detect_destination_title(&source, &near_miss, &no_redirects()).match_kind,
            DestinationIdentityMatch::None
        );
    }

    /// A blank source or blank id identifies nothing, so it can never match
    /// another blank.
    #[test]
    fn blank_identity_halves_are_dropped() {
        assert!(MetadataIdentity::new("", "603").is_none());
        assert!(MetadataIdentity::new("tmdb", "  ").is_none());

        let source = source("src").with_identity("tmdb", "");
        let candidates = vec![candidate("dest").with_identity("", "")];
        assert_eq!(
            detect_destination_title(&source, &candidates, &no_redirects()).match_kind,
            DestinationIdentityMatch::None
        );
    }

    /// A title never detects itself as its own merge destination.
    #[test]
    fn a_title_is_never_its_own_destination() {
        let source = source("same").with_identity("tmdb", "603");
        let candidates = vec![candidate("same").with_identity("tmdb", "603")];

        let outcome = detect_destination_title(&source, &candidates, &no_redirects());

        assert_eq!(outcome.match_kind, DestinationIdentityMatch::None);
    }

    /// The same candidate listed twice is one candidate, not an ambiguity.
    #[test]
    fn a_duplicated_candidate_row_is_not_an_ambiguity() {
        let source = source("src").with_identity("tmdb", "603");
        let candidates = vec![
            candidate("dest").with_identity("tmdb", "603"),
            candidate("dest").with_identity("tmdb", "603"),
        ];

        let outcome = detect_destination_title(&source, &candidates, &no_redirects());

        assert_eq!(outcome.match_kind, DestinationIdentityMatch::Unique);
        assert_eq!(outcome.candidates.len(), 1);
    }

    /// The catalog's own `ExternalId` shape feeds detection directly.
    #[test]
    fn catalog_external_ids_convert_into_identities() {
        let external_ids = vec![
            ExternalId {
                source: "TMDB".to_string(),
                value: "603".to_string(),
            },
            ExternalId {
                source: "  ".to_string(),
                value: "ignored".to_string(),
            },
        ];

        let source = SourceTitleIdentity::new("src", MediaFacet::Movie)
            .with_name("The Matrix")
            .with_external_ids(&external_ids);

        assert_eq!(source.identities.len(), 1);
        assert_eq!(source.identities[0].display(), "tmdb:603");
    }

    /// The selection entry point answers for every source title exactly once,
    /// over one read of the destination library.
    #[test]
    fn selection_detection_answers_every_title_once() {
        let sources = vec![
            source("merges").with_identity("tmdb", "603"),
            source("transfers")
                .with_name("Brand New")
                .with_identity("tmdb", "1"),
            source("ambiguous")
                .with_identity("tmdb", "2")
                .with_identity("imdb", "tt2"),
        ];
        let candidates = vec![
            candidate("dest").with_identity("tmdb", "603"),
            candidate("a").with_identity("tmdb", "2"),
            candidate("b").with_identity("imdb", "tt2"),
        ];

        let detected = detect_destination_titles(&sources, &candidates, &no_redirects());

        assert_eq!(detected.len(), 3);
        assert_eq!(
            detected["merges"].merge_target(),
            Some("dest"),
            "unique matches merge"
        );
        assert_eq!(
            detected["transfers"].match_kind,
            DestinationIdentityMatch::None
        );
        assert!(detected["ambiguous"].needs_resolution());
    }
}

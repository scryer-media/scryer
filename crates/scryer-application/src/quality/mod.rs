pub(crate) mod canonical;
pub(crate) mod canonical_context;

/// Canonicality invariants for [`canonical`]: one term set, two evidence
/// levels, no incumbent state. They live beside the module they pin.
#[cfg(test)]
#[path = "canonical_tests.rs"]
mod canonical_tests;

#[cfg(test)]
mod trash_pack_parity_tests;

#[cfg(test)]
mod trash_size_parity_tests;

#[cfg(test)]
pub(crate) mod pack_test_support;
pub(crate) mod profile;
pub mod release_dedup;
pub(crate) mod release_group_db;
pub(crate) mod release_parser;
pub(crate) mod scoring_weights;

/// The hand-authored TRaSH ranking corpus. It spans the parser, the profile
/// scoring path, the release-group tiers and the managed locale packs, so it
/// hangs off the quality module rather than any one of them.
#[cfg(test)]
#[path = "trash_ranking_corpus_tests.rs"]
mod trash_ranking_corpus_tests;

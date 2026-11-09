//! The decision pipeline: the order in which a quality profile compares two
//! releases.
//!
//! Every comparison between a candidate and something it would replace (a
//! file on disk, a download already in the queue) and every ordering of search
//! results walks the same list of gates. Each gate compares one property of
//! the two releases; the first gate that tells them apart decides, and the
//! score is the implicit final step after the last gate. A profile chooses
//! which gates run and in what order. A gate it leaves out has no effect on
//! any comparison.
//!
//! The gates themselves never change meaning with their position:
//!
//! - **quality tier** compares positions in the profile's own tier ordering.
//! - **revision** compares PROPER/REPACK ranks, only between releases of the
//!   same tier from the same release group.
//! - **preferred language** compares the position of the first preferred audio
//!   language each carries.
//! - **optional languages** counts the optional audio languages each carries.
//!
//! Adding a gate takes a variant on [`DecisionGate`], its entry in
//! [`DecisionGate::ALL`], and its arms in the comparator and the ordinal below.
//! Every consumer goes through [`walk`], [`rank_key`] or [`held_key`], so none
//! of them needs to learn about it.

use std::cmp::Ordering;
use std::fmt;

use serde::{Deserialize, Serialize};

/// One step of a profile's [`DecisionPipeline`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DecisionGate {
    QualityTier,
    Revision,
    PreferredLanguage,
    OptionalLanguages,
}

impl DecisionGate {
    /// Every gate, in the default order.
    pub const ALL: [Self; 4] = [
        Self::QualityTier,
        Self::Revision,
        Self::PreferredLanguage,
        Self::OptionalLanguages,
    ];

    /// How many gates exist; the most a pipeline can hold.
    pub const COUNT: usize = Self::ALL.len();

    pub fn as_str(self) -> &'static str {
        match self {
            Self::QualityTier => "quality_tier",
            Self::Revision => "revision",
            Self::PreferredLanguage => "preferred_language",
            Self::OptionalLanguages => "optional_languages",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim();
        Self::ALL
            .into_iter()
            .find(|gate| gate.as_str().eq_ignore_ascii_case(value))
    }

    /// Whether a win at this gate admits a candidate even when the profile
    /// forbids upgrades. Only a revision does: a PROPER or REPACK repairs the
    /// release already held rather than chasing a better one.
    pub(crate) fn admits_despite_upgrade_guard(self) -> bool {
        matches!(self, Self::Revision)
    }

    /// Which exception to a reached cutoff a win at this gate counts towards.
    pub(crate) fn cutoff_escape(self) -> CutoffEscape {
        match self {
            Self::QualityTier => CutoffEscape::None,
            Self::Revision => CutoffEscape::Revision,
            Self::PreferredLanguage | Self::OptionalLanguages => CutoffEscape::Language,
        }
    }

    /// The comparator: `Greater` means the candidate is better than what it is
    /// measured against.
    fn compare(self, candidate: &Facts<'_>, held: &Facts<'_>) -> Ordering {
        match self {
            Self::QualityTier => tier_cmp(candidate.tier_index, held.tier_index),
            Self::Revision => candidate.revision_cmp(held),
            Self::PreferredLanguage => held
                .language
                .matched
                .preferred_rank
                .cmp(&candidate.language.matched.preferred_rank),
            Self::OptionalLanguages => candidate
                .language
                .matched
                .optional_hits
                .cmp(&held.language.matched.optional_hits),
        }
    }

    /// The gate as an ascending sort key: lower sorts first. Consistent with
    /// [`DecisionGate::compare`] for every gate but the revision, whose sort
    /// key is the context-derived revision preference (see
    /// [`crate::admission::revision_preference`]).
    fn ordinal(self, facts: &RankFacts) -> i64 {
        match self {
            Self::QualityTier => i64::try_from(facts.tier_sort_key).unwrap_or(i64::MAX),
            Self::Revision => i64::from(facts.negated_revision),
            Self::PreferredLanguage => i64::from(facts.language.matched.preferred_rank),
            Self::OptionalLanguages => -i64::from(facts.language.matched.optional_hits),
        }
    }
}

impl fmt::Display for DecisionGate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for DecisionGate {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

/// The exception to a reached cutoff a gate's win belongs to. Gates of one
/// family are compared together, in pipeline order; the families are
/// independent, so a candidate escapes through either.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CutoffEscape {
    None,
    Revision,
    Language,
}

/// A profile's ordered gates. The score always follows the last one.
///
/// `Copy` so that it travels on [`crate::admission::CandidateFacts`] and the
/// search rank head without lifetimes; it never holds more than
/// [`DecisionGate::COUNT`] gates, each at most once.
#[derive(Clone, Copy)]
pub struct DecisionPipeline {
    gates: [DecisionGate; DecisionGate::COUNT],
    len: usize,
}

/// The order a profile uses when it does not choose one: tier, revision,
/// preferred language, optional languages.
const DEFAULT_GATES: [DecisionGate; DecisionGate::COUNT] = [
    DecisionGate::QualityTier,
    DecisionGate::Revision,
    DecisionGate::PreferredLanguage,
    DecisionGate::OptionalLanguages,
];

/// The same gate was listed twice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DuplicateDecisionGate(pub DecisionGate);

impl DecisionPipeline {
    /// An empty pipeline: the score decides alone.
    pub fn score_only() -> Self {
        Self {
            gates: [DecisionGate::ALL[0]; DecisionGate::COUNT],
            len: 0,
        }
    }

    /// Quality tier, revision, preferred language, optional languages: the
    /// order every profile ran before orders were configurable.
    pub const DEFAULT: Self = Self {
        gates: DEFAULT_GATES,
        len: DEFAULT_GATES.len(),
    };

    pub fn default_order() -> Self {
        Self::DEFAULT
    }

    /// Build a pipeline, refusing a gate listed twice.
    pub fn try_from_gates(
        gates: impl IntoIterator<Item = DecisionGate>,
    ) -> Result<Self, DuplicateDecisionGate> {
        let mut pipeline = Self::score_only();
        for gate in gates {
            if pipeline.contains(gate) {
                return Err(DuplicateDecisionGate(gate));
            }
            pipeline.push(gate);
        }
        Ok(pipeline)
    }

    /// Build a pipeline, keeping the first of any gate listed twice.
    pub fn lenient(gates: impl IntoIterator<Item = DecisionGate>) -> Self {
        let mut pipeline = Self::score_only();
        for gate in gates {
            if !pipeline.contains(gate) {
                pipeline.push(gate);
            }
        }
        pipeline
    }

    fn push(&mut self, gate: DecisionGate) {
        self.gates[self.len] = gate;
        self.len += 1;
    }

    pub fn gates(&self) -> &[DecisionGate] {
        &self.gates[..self.len]
    }

    pub fn contains(&self, gate: DecisionGate) -> bool {
        self.gates().contains(&gate)
    }

    pub fn is_default(&self) -> bool {
        self.gates() == DEFAULT_GATES
    }

    /// The same pipeline without `gate`.
    pub fn without(&self, gate: DecisionGate) -> Self {
        Self::lenient(self.gates().iter().copied().filter(|g| *g != gate))
    }

    /// The same pipeline with the revision gate present: unchanged when it
    /// already is, otherwise placed right after the quality tier, or first
    /// when there is no quality tier.
    pub fn with_revision(&self) -> Self {
        if self.contains(DecisionGate::Revision) {
            return *self;
        }
        let at = self
            .gates()
            .iter()
            .position(|gate| *gate == DecisionGate::QualityTier)
            .map_or(0, |tier| tier + 1);
        let mut gates = self.gates().to_vec();
        gates.insert(at, DecisionGate::Revision);
        Self::lenient(gates)
    }

    /// The pipeline a profile runs given its legacy "ignore PROPER/REPACK"
    /// switch: ignoring revisions means running without the revision gate.
    pub fn reconciled_with_ignore_proper_repack(&self, ignore_proper_repack: bool) -> Self {
        if ignore_proper_repack {
            self.without(DecisionGate::Revision)
        } else {
            *self
        }
    }

    /// Whether this pipeline is what a profile with the given
    /// `ignore_proper_repack` switch runs when it stores no pipeline at all.
    /// Storage and fingerprints omit the pipeline exactly then, so profiles
    /// that never chose an order keep the bytes they had before it existed.
    pub fn is_implied_by(&self, ignore_proper_repack: bool) -> bool {
        *self == Self::default_order().reconciled_with_ignore_proper_repack(ignore_proper_repack)
    }

    /// The gates a season pack refused by every member is summarised under,
    /// in order: the pipeline up to and including the quality tier, or the
    /// whole pipeline when it has no tier gate. A gate after the tier only
    /// decides between members of one tier, so "every member beats it there"
    /// would not be the reason it was refused.
    pub(crate) fn pack_summary_gates(&self) -> impl Iterator<Item = DecisionGate> + '_ {
        let end = self
            .gates()
            .iter()
            .position(|gate| *gate == DecisionGate::QualityTier)
            .map_or(self.len, |tier| tier + 1);
        self.gates()[..end].iter().copied()
    }
}

impl Default for DecisionPipeline {
    fn default() -> Self {
        Self::default_order()
    }
}

impl PartialEq for DecisionPipeline {
    fn eq(&self, other: &Self) -> bool {
        self.gates() == other.gates()
    }
}

impl Eq for DecisionPipeline {}

impl std::hash::Hash for DecisionPipeline {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.gates().hash(state);
    }
}

impl fmt::Debug for DecisionPipeline {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_list().entries(self.gates()).finish()
    }
}

impl Serialize for DecisionPipeline {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.gates())
    }
}

/// Lenient on purpose: an unknown gate name (written by a newer build) is
/// dropped, a repeated gate keeps its first position, and anything that is
/// not a list reads as the default order. A stored profile must always load.
impl<'de> Deserialize<'de> for DecisionPipeline {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        Ok(match value {
            serde_json::Value::Array(items) => Self::lenient(
                items
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .filter_map(DecisionGate::parse),
            ),
            _ => Self::default_order(),
        })
    }
}

/// Which step of the pipeline decided an automatic comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReleaseDecidingStep {
    Gate(DecisionGate),
    /// Every gate tied; the score decided.
    Score,
}

impl ReleaseDecidingStep {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Gate(gate) => gate.as_str(),
            Self::Score => "score",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        if value.trim().eq_ignore_ascii_case("score") {
            return Some(Self::Score);
        }
        DecisionGate::parse(value).map(Self::Gate)
    }

    /// See [`DecisionGate::admits_despite_upgrade_guard`]. The score never does.
    pub(crate) fn admits_despite_upgrade_guard(self) -> bool {
        matches!(self, Self::Gate(gate) if gate.admits_despite_upgrade_guard())
    }
}

/// What one side of a comparison is, as far as the gates are concerned.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Facts<'a> {
    pub tier_index: Option<usize>,
    pub release_group: Option<&'a str>,
    pub revision: i32,
    pub language: crate::AudioLanguageRank,
    /// Only the candidate's matters: whether revisions are compared at all.
    pub ignores_revision: bool,
}

impl Facts<'_> {
    /// Same tier, same release group, and revisions honoured.
    pub(crate) fn is_related_to(&self, held: &Facts<'_>) -> bool {
        !self.ignores_revision
            && self.tier_index == held.tier_index
            && self
                .release_group
                .map(str::trim)
                .filter(|g| !g.is_empty())
                .zip(held.release_group.map(str::trim).filter(|g| !g.is_empty()))
                .is_some_and(|(left, right)| left.eq_ignore_ascii_case(right))
    }

    fn revision_cmp(&self, held: &Facts<'_>) -> Ordering {
        if self.is_related_to(held) {
            self.revision.cmp(&held.revision)
        } else {
            Ordering::Equal
        }
    }
}

/// Where the ladder stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LadderOutcome {
    /// The candidate is better at this gate.
    Won(DecisionGate),
    /// What the candidate is measured against is better at this gate.
    Lost(DecisionGate),
    /// Every gate tied; the caller compares scores.
    ReachedScore,
}

/// Compare at a single gate, without walking the pipeline.
pub(crate) fn compare_at(gate: DecisionGate, candidate: &Facts<'_>, held: &Facts<'_>) -> Ordering {
    gate.compare(candidate, held)
}

/// The one comparison entry point: walk `pipeline` and stop at the first gate
/// that tells the two apart.
pub(crate) fn walk(
    pipeline: &DecisionPipeline,
    candidate: &Facts<'_>,
    held: &Facts<'_>,
) -> LadderOutcome {
    for gate in pipeline.gates() {
        match gate.compare(candidate, held) {
            Ordering::Greater => return LadderOutcome::Won(*gate),
            Ordering::Less => return LadderOutcome::Lost(*gate),
            Ordering::Equal => {}
        }
    }
    LadderOutcome::ReachedScore
}

/// Whether the candidate passes a reached cutoff on the family of gates
/// `escape` names: those gates, compared in pipeline order, find it better.
pub(crate) fn wins_cutoff_escape(
    pipeline: &DecisionPipeline,
    escape: CutoffEscape,
    candidate: &Facts<'_>,
    held: &Facts<'_>,
) -> bool {
    pipeline
        .gates()
        .iter()
        .filter(|gate| gate.cutoff_escape() == escape)
        .map(|gate| gate.compare(candidate, held))
        .find(|ordering| ordering.is_ne())
        == Some(Ordering::Greater)
}

/// The values a search rank reads its gates from.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RankFacts {
    /// [`crate::admission::tier_sort_key`] of the release's tier.
    pub tier_sort_key: usize,
    /// The context-derived revision preference, negated.
    pub negated_revision: i32,
    pub language: crate::AudioLanguageRank,
}

/// One ordinal per pipeline position, lower first; positions past the end of
/// the pipeline are zero.
pub(crate) type GateKey = [i64; DecisionGate::COUNT];

/// The gates of `pipeline` as an ascending sort key, in pipeline order.
pub(crate) fn rank_key(pipeline: &DecisionPipeline, facts: &RankFacts) -> GateKey {
    let mut key = [0; DecisionGate::COUNT];
    for (slot, gate) in key.iter_mut().zip(pipeline.gates()) {
        *slot = gate.ordinal(facts);
    }
    key
}

/// The sort key of a held file, best first: the gates of `pipeline` that do
/// not depend on a counterpart. A revision only orders a candidate against a
/// related release, so it is neutral between held files.
pub(crate) fn held_key(
    pipeline: &DecisionPipeline,
    tier_index: Option<usize>,
    language: crate::AudioLanguageRank,
) -> GateKey {
    rank_key(
        pipeline,
        &RankFacts {
            tier_sort_key: crate::admission::tier_sort_key(tier_index),
            negated_revision: 0,
            language,
        },
    )
}

/// Compare two positions in the profile's quality ordering.
///
/// Lower index is better. A quality the profile does not list (`None`) ranks
/// below every listed one — it is only here at all because some other gate let
/// it through.
fn tier_cmp(left: Option<usize>, right: Option<usize>) -> Ordering {
    match (left, right) {
        (Some(l), Some(r)) => r.cmp(&l),
        (Some(_), None) => Ordering::Greater,
        (None, Some(_)) => Ordering::Less,
        (None, None) => Ordering::Equal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AudioLanguageRank;

    /// Every pipeline a profile can hold: each ordered selection of distinct
    /// gates, the empty one included. With four gates there are 65.
    fn every_pipeline() -> Vec<DecisionPipeline> {
        fn extend(prefix: Vec<DecisionGate>, out: &mut Vec<DecisionPipeline>) {
            out.push(DecisionPipeline::try_from_gates(prefix.iter().copied()).unwrap());
            for gate in DecisionGate::ALL {
                if !prefix.contains(&gate) {
                    let mut next = prefix.clone();
                    next.push(gate);
                    extend(next, out);
                }
            }
        }
        let mut out = Vec::new();
        extend(Vec::new(), &mut out);
        out
    }

    /// Every combination of the values the gates read, all from one group.
    fn every_side() -> Vec<Facts<'static>> {
        let mut sides = Vec::new();
        for tier_index in [Some(0), Some(1), None] {
            for revision in [0, 1] {
                for preferred_rank in [0, 1, 2] {
                    for optional_hits in [0, 1] {
                        sides.push(Facts {
                            tier_index,
                            release_group: Some("GRP"),
                            revision,
                            language: AudioLanguageRank::for_test(preferred_rank, optional_hits),
                            ignores_revision: false,
                        });
                    }
                }
            }
        }
        sides
    }

    /// The search rank of `candidate` measured against `held`, the way the
    /// search lanes build it: the revision ordinal is the candidate's
    /// preference over the held release, and the held release itself is
    /// neutral on it.
    fn rank_pair(
        pipeline: &DecisionPipeline,
        candidate: &Facts,
        held: &Facts,
    ) -> (GateKey, GateKey) {
        let preference = match candidate.revision_cmp(held) {
            Ordering::Greater => 1,
            Ordering::Less => -1,
            Ordering::Equal => 0,
        };
        let facts = |side: &Facts, negated_revision| RankFacts {
            tier_sort_key: crate::admission::tier_sort_key(side.tier_index),
            negated_revision,
            language: side.language,
        };
        (
            rank_key(pipeline, &facts(candidate, -preference)),
            rank_key(pipeline, &facts(held, 0)),
        )
    }

    #[test]
    fn there_are_sixty_five_pipelines_and_each_is_distinct() {
        let pipelines = every_pipeline();
        assert_eq!(pipelines.len(), 65);
        let distinct: std::collections::HashSet<_> = pipelines.iter().collect();
        assert_eq!(distinct.len(), 65);
    }

    /// For every pipeline, the sort key and the ladder agree: a candidate the
    /// ladder finds better at some gate sorts ahead of what it is measured
    /// against, a worse one sorts behind, and one that reaches the score
    /// ties on every gate slot.
    #[test]
    fn the_sort_key_agrees_with_the_ladder_for_every_pipeline() {
        let sides = every_side();
        for pipeline in every_pipeline() {
            for candidate in &sides {
                for held in &sides {
                    let (candidate_key, held_key) = rank_pair(&pipeline, candidate, held);
                    let expected = match walk(&pipeline, candidate, held) {
                        LadderOutcome::Won(_) => Ordering::Less,
                        LadderOutcome::Lost(_) => Ordering::Greater,
                        LadderOutcome::ReachedScore => Ordering::Equal,
                    };
                    assert_eq!(
                        candidate_key.cmp(&held_key),
                        expected,
                        "{pipeline:?}: {candidate:?} against {held:?}"
                    );
                }
            }
        }
    }

    /// The value a gate reads, changed on the candidate alone.
    fn vary(gate: DecisionGate, side: &Facts<'static>) -> Facts<'static> {
        let mut varied = *side;
        match gate {
            DecisionGate::QualityTier => {
                varied.tier_index = match side.tier_index {
                    Some(0) => Some(1),
                    _ => Some(0),
                }
            }
            DecisionGate::Revision => varied.revision = 1 - side.revision,
            DecisionGate::PreferredLanguage => {
                varied.language.matched.preferred_rank =
                    (side.language.matched.preferred_rank + 1) % 3
            }
            DecisionGate::OptionalLanguages => {
                varied.language.matched.optional_hits = 1 - side.language.matched.optional_hits
            }
        }
        varied
    }

    /// A gate a pipeline leaves out has no effect: changing only what that
    /// gate reads changes neither the ladder's outcome nor the sort key.
    ///
    /// The revision gate compares only releases of one tier, so a tier change
    /// is checked with the two revisions equal, where it cannot reach the
    /// revision gate either.
    #[test]
    fn a_gate_left_out_of_the_pipeline_has_no_effect() {
        let sides = every_side();
        for pipeline in every_pipeline() {
            for gate in DecisionGate::ALL {
                if pipeline.contains(gate) {
                    continue;
                }
                for candidate in &sides {
                    for held in &sides {
                        if gate == DecisionGate::QualityTier && candidate.revision != held.revision
                        {
                            continue;
                        }
                        let varied = vary(gate, candidate);
                        assert_eq!(
                            walk(&pipeline, &varied, held),
                            walk(&pipeline, candidate, held),
                            "{pipeline:?} without {gate}: {candidate:?} -> {varied:?} against {held:?}"
                        );
                        assert_eq!(
                            rank_pair(&pipeline, &varied, held),
                            rank_pair(&pipeline, candidate, held),
                            "{pipeline:?} without {gate}: {candidate:?} -> {varied:?} against {held:?}"
                        );
                    }
                }
            }
        }
    }

    /// Removing a gate gives the same outcome as the full pipeline would on
    /// facts that tie at that gate: absence and a tie are the same thing.
    #[test]
    fn removing_a_gate_is_the_same_as_tying_on_it() {
        let sides = every_side();
        for pipeline in every_pipeline() {
            for gate in pipeline.gates().iter().copied() {
                let without = pipeline.without(gate);
                for candidate in &sides {
                    for held in &sides {
                        if compare_at(gate, candidate, held).is_ne() {
                            continue;
                        }
                        let outcome = walk(&pipeline, candidate, held);
                        assert_eq!(
                            walk(&without, candidate, held),
                            outcome,
                            "{pipeline:?} without {gate}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_repeated_gate_is_refused_and_a_lenient_read_keeps_its_first_place() {
        assert_eq!(
            DecisionPipeline::try_from_gates([DecisionGate::Revision, DecisionGate::Revision]),
            Err(DuplicateDecisionGate(DecisionGate::Revision))
        );
        let lenient = DecisionPipeline::lenient([
            DecisionGate::PreferredLanguage,
            DecisionGate::QualityTier,
            DecisionGate::PreferredLanguage,
        ]);
        assert_eq!(
            lenient.gates(),
            [DecisionGate::PreferredLanguage, DecisionGate::QualityTier]
        );
    }

    #[test]
    fn the_pipeline_serializes_as_gate_names_and_reads_them_back() {
        for pipeline in every_pipeline() {
            let json = serde_json::to_value(pipeline).unwrap();
            assert_eq!(
                json.as_array().unwrap().len(),
                pipeline.gates().len(),
                "{pipeline:?}"
            );
            assert_eq!(
                serde_json::from_value::<DecisionPipeline>(json).unwrap(),
                pipeline
            );
        }
    }

    /// The revision gate comes back after the tier, or first without one,
    /// and the revision switch implies exactly the default order without it.
    #[test]
    fn the_revision_gate_follows_the_revision_switch() {
        for pipeline in every_pipeline() {
            let restored = pipeline.with_revision();
            assert!(restored.contains(DecisionGate::Revision));
            if pipeline.contains(DecisionGate::Revision) {
                assert_eq!(restored, pipeline);
            } else {
                assert_eq!(restored.without(DecisionGate::Revision), pipeline);
                let at = restored
                    .gates()
                    .iter()
                    .position(|gate| *gate == DecisionGate::Revision)
                    .unwrap();
                let tier = pipeline
                    .gates()
                    .iter()
                    .position(|gate| *gate == DecisionGate::QualityTier);
                assert_eq!(at, tier.map_or(0, |tier| tier + 1), "{pipeline:?}");
            }
            assert!(
                !pipeline
                    .reconciled_with_ignore_proper_repack(true)
                    .contains(DecisionGate::Revision)
            );
            assert_eq!(
                pipeline.reconciled_with_ignore_proper_repack(false),
                pipeline
            );
        }
        let default = DecisionPipeline::default_order();
        assert!(default.is_implied_by(false));
        assert!(default.without(DecisionGate::Revision).is_implied_by(true));
        assert!(!default.without(DecisionGate::Revision).is_implied_by(false));
        assert!(!DecisionPipeline::score_only().is_implied_by(true));
    }

    #[test]
    fn a_profile_stores_its_order_only_when_the_switch_does_not_imply_it() {
        for pipeline in every_pipeline() {
            for ignore in [false, true] {
                let mut criteria = crate::QualityProfileCriteria::default();
                criteria.ignore_proper_repack = ignore;
                criteria.decision_pipeline = pipeline;
                criteria.reconcile_revision_gate();
                assert_eq!(
                    criteria.ignore_proper_repack,
                    !criteria.decision_pipeline.contains(DecisionGate::Revision)
                );
                // What storage keeps reads back to the same profile.
                let mut reread = crate::QualityProfileCriteria::default();
                reread.ignore_proper_repack = criteria.ignore_proper_repack;
                reread.decision_pipeline = criteria.stored_decision_pipeline().unwrap_or_default();
                reread.reconcile_revision_gate();
                assert_eq!(
                    reread.decision_pipeline, criteria.decision_pipeline,
                    "{pipeline:?} ignore={ignore}"
                );
            }
        }
    }

    #[test]
    fn deciding_steps_round_trip_through_their_names() {
        let steps = DecisionGate::ALL
            .into_iter()
            .map(ReleaseDecidingStep::Gate)
            .chain([ReleaseDecidingStep::Score]);
        for step in steps {
            assert_eq!(ReleaseDecidingStep::parse(step.as_str()), Some(step));
        }
        assert_eq!(ReleaseDecidingStep::parse("something_new"), None);
    }
}

//! The decision pipeline: the order in which a quality profile compares two
//! releases.
//!
//! Every comparison between a candidate and something it would replace (a
//! file on disk, a download already in the queue) and every ordering of search
//! results walks the same list of steps. Each step compares one property of
//! the two releases; the first step that tells them apart decides, and the
//! score is the implicit final step after the last one. A profile chooses
//! which steps run and in what order. A step it leaves out has no effect on
//! any comparison.
//!
//! This module is the engine: the step ids ([`DecisionGate`]), a profile's
//! ordered list of them ([`DecisionPipeline`]), the facts a step reads
//! ([`Facts`], [`RankFacts`], [`SideFacts`]) and the walks over a pipeline
//! ([`walk`], [`rank_key`], [`held_key`]). What each
//! step does lives on its [`PreferenceStep`] in
//! [`crate::acquisition::decision_steps`], which is also where the list of
//! what it takes to add a step is kept. Every consumer goes through the walks
//! here or through the pipeline's own questions
//! ([`DecisionPipeline::still_wants`],
//! [`DecisionPipeline::passes_reached_cutoff`],
//! [`DecisionPipeline::compares_revisions`]), so none of them needs to learn
//! about a new step.

use std::cmp::Ordering;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::acquisition::decision_steps::{PreferenceStep, STEPS};

/// Declares [`DecisionGate`] and its [`DecisionGate::ALL`] from one list, so
/// the two cannot drift apart.
macro_rules! decision_gates {
    ($($(#[$meta:meta])* $variant:ident),+ $(,)?) => {
        /// One step of a profile's [`DecisionPipeline`]: the id a preference
        /// step is stored, hashed and exposed under. What the step does is the
        /// [`PreferenceStep`] it resolves to through [`DecisionGate::step`].
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum DecisionGate {
            $($(#[$meta])* $variant),+
        }

        impl DecisionGate {
            /// Every step, in declaration order, which is also the order of
            /// the step registry. Not the default order of a profile: that is
            /// [`DecisionPipeline::DEFAULT`].
            pub const ALL: [Self; [$(DecisionGate::$variant),+].len()] =
                [$(Self::$variant),+];
        }
    };
}

decision_gates! {
    /// Positions in the profile's own tier ordering.
    QualityTier,
    /// PROPER/REPACK ranks, only between releases of the same tier from the
    /// same release group.
    Revision,
    /// The position of the first preferred audio language each carries.
    PreferredLanguage,
    /// How many of the optional audio languages each carries.
    OptionalLanguages,
}

impl DecisionGate {
    /// How many steps exist; the most a pipeline can hold.
    pub const COUNT: usize = Self::ALL.len();

    /// The step this id names, from the registry.
    pub(crate) fn step(self) -> &'static dyn PreferenceStep {
        STEPS[self as usize]
    }

    /// The name the step is stored and recorded under.
    pub fn as_str(self) -> &'static str {
        self.step().name()
    }

    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim();
        Self::ALL
            .into_iter()
            .find(|gate| gate.as_str().eq_ignore_ascii_case(value))
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

/// The exception to a reached cutoff a step's win belongs to. Steps of one
/// family are compared together; the families are independent, so a
/// candidate escapes through either.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CutoffEscape {
    None,
    Revision,
    Language,
}

impl CutoffEscape {
    /// Every family that is an exception to a reached cutoff.
    pub(crate) const FAMILIES: [Self; 2] = [Self::Revision, Self::Language];
}

/// A profile's ordered steps. The score always follows the last one.
///
/// `Copy` so that it travels on [`crate::admission::CandidateFacts`] and the
/// search rank head without lifetimes; it never holds more than
/// [`DecisionGate::COUNT`] steps, each at most once.
#[derive(Clone, Copy)]
pub struct DecisionPipeline {
    gates: [DecisionGate; DecisionGate::COUNT],
    len: usize,
}

/// The order a profile uses when it does not choose one: tier, then revision,
/// which is how every profile compared releases before orders were
/// configurable. The language steps are opt-in.
///
/// Deliberately an explicit list and not [`DecisionGate::ALL`]: a step added
/// later is not in it, so profiles that never chose an order keep comparing
/// exactly as they did before that step existed.
const DEFAULT_GATES: &[DecisionGate] = &[DecisionGate::QualityTier, DecisionGate::Revision];

/// The same gate was listed twice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DuplicateDecisionGate(pub DecisionGate);

impl DecisionPipeline {
    /// An empty pipeline: the score decides alone.
    pub fn score_only() -> Self {
        Self::from_const_gates(&[])
    }

    /// Quality tier, then revision: the order every profile ran before orders
    /// were configurable.
    pub const DEFAULT: Self = Self::from_const_gates(DEFAULT_GATES);

    /// A pipeline from a list known at compile time. The list must not repeat
    /// a step; a list longer than [`DecisionGate::COUNT`] fails to compile.
    const fn from_const_gates(list: &[DecisionGate]) -> Self {
        let mut gates = [DecisionGate::ALL[0]; DecisionGate::COUNT];
        let mut index = 0;
        while index < list.len() {
            gates[index] = list[index];
            index += 1;
        }
        Self {
            gates,
            len: list.len(),
        }
    }

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

    /// The steps this pipeline runs, in its order.
    pub(crate) fn steps(&self) -> impl Iterator<Item = &'static dyn PreferenceStep> + '_ {
        self.gates().iter().map(|gate| gate.step())
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

    /// Whether some step this pipeline runs still wants something the held
    /// file lacks; see [`PreferenceStep::still_wanted`]. A step the profile
    /// has removed cannot be improved on, so it keeps nothing wanted.
    pub(crate) fn still_wants(&self, held: &SideFacts) -> bool {
        self.steps().any(|step| step.still_wanted(held))
    }

    /// Whether `candidate` passes a reached cutoff over the file `held`
    /// through the `escape` family: the first of the family's steps this
    /// pipeline runs that settles it, see [`PreferenceStep::cutoff_verdict`].
    ///
    /// The family's steps are asked in registry order, not in the pipeline's:
    /// a cutoff exception is a property of the step, not of where the profile
    /// ranks it.
    pub(crate) fn passes_cutoff(
        &self,
        escape: CutoffEscape,
        candidate: &Facts<'_>,
        held: &Facts<'_>,
    ) -> bool {
        STEPS
            .iter()
            .filter(|step| step.cutoff_escape() == escape && self.contains(step.id()))
            .find_map(|step| step.cutoff_verdict(candidate, held))
            .unwrap_or(false)
    }

    /// The one cutoff exception rule: whether `candidate` passes a cutoff
    /// `held` has reached through any family, see [`Self::passes_cutoff`].
    /// A file on disk and a recent grab at its cutoff both ask this.
    pub(crate) fn passes_reached_cutoff(&self, candidate: &Facts<'_>, held: &Facts<'_>) -> bool {
        CutoffEscape::FAMILIES
            .into_iter()
            .any(|escape| self.passes_cutoff(escape, candidate, held))
    }

    // ── The legacy revision switch ─────────────────────────────────────────
    //
    // Profiles stored before orders were configurable carry an "ignore
    // PROPER/REPACK" switch instead of a revision step. These functions are the
    // only places that join the two; nothing else names the revision step.

    /// Whether this pipeline compares PROPER/REPACK ranks at all.
    pub fn compares_revisions(&self) -> bool {
        self.contains(DecisionGate::Revision)
    }

    /// Whether PROPER/REPACK ranks are ignored under the legacy switch
    /// `ignore_proper_repack`: the switch is on, or this pipeline has no
    /// revision step.
    pub fn ignores_revisions(&self, ignore_proper_repack: bool) -> bool {
        ignore_proper_repack || !self.compares_revisions()
    }

    /// The same pipeline with the revision gate present: unchanged when it
    /// already is, otherwise placed right after the quality tier, or first
    /// when there is no quality tier.
    pub fn with_revision(&self) -> Self {
        if self.compares_revisions() {
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

    /// The pipeline after the legacy switch is set to `ignore_proper_repack`:
    /// it loses or regains its revision gate.
    pub fn with_ignore_proper_repack(&self, ignore_proper_repack: bool) -> Self {
        if ignore_proper_repack {
            self.without(DecisionGate::Revision)
        } else {
            self.with_revision()
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
    /// in order: the pipeline up to and including the first step that closes
    /// the summary (the quality tier), or the whole pipeline when none does.
    /// A step after the tier only decides between members of one tier, so
    /// "every member beats it there" would not be the reason it was refused.
    pub(crate) fn pack_summary_gates(&self) -> impl Iterator<Item = DecisionGate> + '_ {
        let end = self
            .steps()
            .position(|step| step.closes_pack_summary())
            .map_or(self.len, |at| at + 1);
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

    /// See [`PreferenceStep::admits_despite_upgrade_guard`]. The score never
    /// does.
    pub(crate) fn admits_despite_upgrade_guard(self) -> bool {
        matches!(self, Self::Gate(gate) if gate.step().admits_despite_upgrade_guard())
    }
}

/// What one release or file is, as the steps read it on its own.
///
/// The one place a property a step reads is declared. Each source has exactly
/// one builder: [`crate::admission::CandidateFacts::facts`] (a candidate),
/// [`crate::admission::Incumbent::facts`],
/// [`crate::quality::landed_bar_memo::LandedBar::side`] and
/// [`crate::quality::canonical_context::IncumbentFacts::side`] (a file on disk),
/// [`crate::admission::QueuedRelease::facts`] (a queued release) and
/// [`SideFacts::of_decision`] (a scored search result). A step that reads a
/// new property adds a field here and fills it in those builders.
///
/// `Default` is the best tier and a neutral language standing, which is what
/// an unranked search head has always sorted as.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct SideFacts {
    /// [`crate::admission::tier_sort_key`] of the release's tier: lower is
    /// better, an unlisted quality is `usize::MAX`.
    pub tier_sort_key: usize,
    pub language: crate::AudioLanguageRank,
}

/// What one side of a comparison is, as far as the steps are concerned.
///
/// Kept apart from [`RankFacts`] because the revision is not a property of one
/// side: a comparison reads both releases' raw revisions and release groups
/// and decides whether they are related at all, while a search rank can only
/// carry a revision preference derived from everything the scope already
/// holds. One struct for both would carry fields that mean nothing in one of
/// the two modes. Everything else a step reads is shared, in [`SideFacts`].
#[derive(Debug, Clone, Copy)]
pub(crate) struct Facts<'a> {
    pub side: SideFacts,
    pub release_group: Option<&'a str>,
    pub revision: i32,
    /// Only the candidate's matters: whether revisions are compared at all.
    pub ignores_revision: bool,
}

impl Facts<'_> {
    /// Same tier, same release group, and revisions honoured.
    pub(crate) fn is_related_to(&self, held: &Facts<'_>) -> bool {
        !self.ignores_revision
            && self.side.tier_sort_key == held.side.tier_sort_key
            && self
                .release_group
                .map(str::trim)
                .filter(|g| !g.is_empty())
                .zip(held.release_group.map(str::trim).filter(|g| !g.is_empty()))
                .is_some_and(|(left, right)| left.eq_ignore_ascii_case(right))
    }

    /// The revisions of two related releases; `Equal` for unrelated ones.
    pub(crate) fn revision_cmp(&self, held: &Facts<'_>) -> Ordering {
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
    gate.step().compare(candidate, held)
}

/// The one comparison entry point: walk `pipeline` and stop at the first gate
/// that tells the two apart.
pub(crate) fn walk(
    pipeline: &DecisionPipeline,
    candidate: &Facts<'_>,
    held: &Facts<'_>,
) -> LadderOutcome {
    for gate in pipeline.gates() {
        match compare_at(*gate, candidate, held) {
            Ordering::Greater => return LadderOutcome::Won(*gate),
            Ordering::Less => return LadderOutcome::Lost(*gate),
            Ordering::Equal => {}
        }
    }
    LadderOutcome::ReachedScore
}

/// The values a search rank reads its steps from. See [`Facts`] for why this
/// is a separate struct.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RankFacts {
    pub side: SideFacts,
    /// The context-derived revision preference, negated; see
    /// [`crate::admission::revision_preference`].
    pub negated_revision: i32,
}

/// One ordinal per pipeline position, lower first; positions past the end of
/// the pipeline are zero.
pub(crate) type GateKey = [i64; DecisionGate::COUNT];

/// The gates of `pipeline` as an ascending sort key, in pipeline order.
pub(crate) fn rank_key(pipeline: &DecisionPipeline, facts: &RankFacts) -> GateKey {
    let mut key = [0; DecisionGate::COUNT];
    for (slot, step) in key.iter_mut().zip(pipeline.steps()) {
        *slot = step.ordinal(facts);
    }
    key
}

/// The sort key of a held file, best first: the gates of `pipeline` that do
/// not depend on a counterpart. A revision only orders a candidate against a
/// related release, so it is neutral between held files.
pub(crate) fn held_key(pipeline: &DecisionPipeline, side: SideFacts) -> GateKey {
    rank_key(
        pipeline,
        &RankFacts {
            side,
            negated_revision: 0,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AudioLanguageRank;

    /// The steps whose every order the exhaustive tests below enumerate.
    ///
    /// Fixed on purpose rather than [`DecisionGate::ALL`]: ordered selections
    /// grow factorially (65 with four steps, 326 with five, 1,957 with six),
    /// so a new step is covered by the registry-wide contract tests instead,
    /// and enumerating it as well is a choice made here.
    const ENUMERATED: [DecisionGate; 4] = [
        DecisionGate::QualityTier,
        DecisionGate::Revision,
        DecisionGate::PreferredLanguage,
        DecisionGate::OptionalLanguages,
    ];

    /// Every pipeline a profile can hold out of [`ENUMERATED`]: each ordered
    /// selection of distinct gates, the empty one included. With four gates
    /// there are 65.
    fn every_pipeline() -> Vec<DecisionPipeline> {
        fn extend(prefix: Vec<DecisionGate>, out: &mut Vec<DecisionPipeline>) {
            out.push(DecisionPipeline::try_from_gates(prefix.iter().copied()).unwrap());
            for gate in ENUMERATED {
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

    fn side(
        tier_index: Option<usize>,
        revision: i32,
        language: AudioLanguageRank,
    ) -> Facts<'static> {
        Facts {
            side: SideFacts {
                tier_sort_key: crate::admission::tier_sort_key(tier_index),
                language,
            },
            release_group: Some("GRP"),
            revision,
            ignores_revision: false,
        }
    }

    /// Every combination of the values the gates read, all from one group.
    fn every_side() -> Vec<Facts<'static>> {
        let mut sides = Vec::new();
        for tier_index in [Some(0), Some(1), None] {
            for revision in [0, 1] {
                for preferred_rank in [0, 1, 2] {
                    for optional_hits in [0, 1] {
                        sides.push(side(
                            tier_index,
                            revision,
                            AudioLanguageRank::for_test(preferred_rank, optional_hits),
                        ));
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
            side: side.side,
            negated_revision,
        };
        (
            rank_key(pipeline, &facts(candidate, -preference)),
            rank_key(pipeline, &facts(held, 0)),
        )
    }

    // ── The registry ──────────────────────────────────────────────────────

    /// Each id resolves to its own step, every id is registered exactly once
    /// and in declaration order, and every name is distinct and reads back
    /// to its id.
    #[test]
    fn the_registry_holds_every_step_exactly_once() {
        assert_eq!(STEPS.len(), DecisionGate::ALL.len());
        for (index, gate) in DecisionGate::ALL.into_iter().enumerate() {
            assert_eq!(gate as usize, index, "{gate:?} is out of declaration order");
            assert_eq!(STEPS[index].id(), gate, "registry slot {index}");
            assert_eq!(gate.step().id(), gate);
            assert_eq!(DecisionGate::parse(gate.as_str()), Some(gate));
            assert_eq!(
                DecisionGate::parse(&gate.as_str().to_ascii_uppercase()),
                Some(gate)
            );
        }
        let names: std::collections::HashSet<_> =
            DecisionGate::ALL.iter().map(|gate| gate.as_str()).collect();
        assert_eq!(names.len(), DecisionGate::COUNT);
        assert!(!names.contains("score"), "the score's name is reserved");
        assert_eq!(DecisionGate::parse("something_new"), None);
    }

    /// The default order is frozen: a step added later is not in it.
    #[test]
    fn the_default_order_is_tier_then_revision() {
        assert_eq!(
            DecisionPipeline::DEFAULT.gates(),
            [DecisionGate::QualityTier, DecisionGate::Revision]
        );
        assert!(DecisionPipeline::DEFAULT.is_default());
        assert_eq!(
            DecisionPipeline::try_from_gates(DEFAULT_GATES.iter().copied()),
            Ok(DecisionPipeline::DEFAULT)
        );
        assert!(DecisionPipeline::score_only().gates().is_empty());
    }

    // ── Per-step contract, over the whole registry ────────────────────────

    /// Every step compares a side with itself as equal and is antisymmetric:
    /// swapping the two sides reverses the answer.
    #[test]
    fn every_step_compares_reflexively_and_antisymmetrically() {
        let sides = every_side();
        for step in STEPS {
            for left in &sides {
                assert_eq!(step.compare(left, left), Ordering::Equal, "{}", step.name());
                for right in &sides {
                    assert_eq!(
                        step.compare(left, right),
                        step.compare(right, left).reverse(),
                        "{}: {left:?} against {right:?}",
                        step.name()
                    );
                }
            }
        }
    }

    /// A step whose ordinal claims to follow its comparator sorts a side
    /// measured on its own exactly as the comparator orders it.
    #[test]
    fn an_ordinal_sorts_like_its_comparator_where_the_step_says_it_does() {
        let sides = every_side();
        let ordinal = |step: &dyn PreferenceStep, side: &Facts| {
            step.ordinal(&RankFacts {
                side: side.side,
                negated_revision: 0,
            })
        };
        let mut checked = 0;
        for step in STEPS
            .iter()
            .copied()
            .filter(|step| step.ordinal_follows_compare())
        {
            checked += 1;
            for left in &sides {
                for right in &sides {
                    assert_eq!(
                        ordinal(step, left).cmp(&ordinal(step, right)),
                        step.compare(left, right).reverse(),
                        "{}: {left:?} against {right:?}",
                        step.name()
                    );
                }
            }
        }
        assert!(checked > 0);
    }

    /// A step acts only where it sits: appended to any pipeline that lacks
    /// it, it leaves every outcome the pipeline already reached alone, decides
    /// exactly the ties, and adds one key slot after the pipeline's own.
    #[test]
    fn a_step_appended_to_a_pipeline_only_breaks_its_ties() {
        let sides = every_side();
        for step in STEPS {
            let gate = step.id();
            for pipeline in every_pipeline() {
                if pipeline.contains(gate) {
                    continue;
                }
                let extended = DecisionPipeline::try_from_gates(
                    pipeline.gates().iter().copied().chain([gate]),
                )
                .unwrap();
                let slot = pipeline.gates().len();
                for candidate in &sides {
                    for held in &sides {
                        let before = walk(&pipeline, candidate, held);
                        let expected = match (before, step.compare(candidate, held)) {
                            (LadderOutcome::ReachedScore, Ordering::Greater) => {
                                LadderOutcome::Won(gate)
                            }
                            (LadderOutcome::ReachedScore, Ordering::Less) => {
                                LadderOutcome::Lost(gate)
                            }
                            (outcome, _) => outcome,
                        };
                        assert_eq!(
                            walk(&extended, candidate, held),
                            expected,
                            "{extended:?}: {candidate:?} against {held:?}"
                        );
                        let (short, _) = rank_pair(&pipeline, candidate, held);
                        let (long, _) = rank_pair(&extended, candidate, held);
                        assert_eq!(short[..slot], long[..slot], "{extended:?}");
                        assert!(short[slot..].iter().all(|ordinal| *ordinal == 0));
                    }
                }
            }
        }
    }

    /// A step's cutoff verdict carries its own guard: in every pipeline, a
    /// candidate passes a reached cutoff through a family other than the
    /// revision's only over a held file some step still wants something for.
    #[test]
    fn a_cutoff_escape_other_than_a_revision_needs_a_held_file_still_wanted() {
        let sides = every_side();
        for pipeline in every_pipeline() {
            for candidate in &sides {
                for held in &sides {
                    for optional_wanted in [false, true] {
                        let mut held = *held;
                        held.side.language.optional_wanted = optional_wanted;
                        for escape in CutoffEscape::FAMILIES {
                            if escape == CutoffEscape::Revision
                                || !pipeline.passes_cutoff(escape, candidate, &held)
                            {
                                continue;
                            }
                            assert!(
                                pipeline.still_wants(&held.side),
                                "{pipeline:?} {escape:?}: {candidate:?} against {held:?}"
                            );
                        }
                    }
                }
            }
        }
    }

    // ── Every order of the enumerated steps ───────────────────────────────

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

    /// The value a gate reads, changed on the candidate alone. Only the
    /// [`ENUMERATED`] steps have a variation here.
    fn vary(gate: DecisionGate, side: &Facts<'static>) -> Facts<'static> {
        let mut varied = *side;
        let tier = |index| crate::admission::tier_sort_key(Some(index));
        match gate {
            DecisionGate::QualityTier => {
                varied.side.tier_sort_key = if side.side.tier_sort_key == tier(0) {
                    tier(1)
                } else {
                    tier(0)
                }
            }
            DecisionGate::Revision => varied.revision = 1 - side.revision,
            DecisionGate::PreferredLanguage => {
                varied.side.language.matched.preferred_rank =
                    (side.side.language.matched.preferred_rank + 1) % 3
            }
            DecisionGate::OptionalLanguages => {
                varied.side.language.matched.optional_hits =
                    1 - side.side.language.matched.optional_hits
            }
            #[allow(unreachable_patterns, reason = "a later step is not enumerated")]
            other => panic!("{other} is not one of the enumerated steps"),
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
            for gate in ENUMERATED {
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
            assert_eq!(pipeline.with_ignore_proper_repack(false), restored);
            assert_eq!(
                pipeline.with_ignore_proper_repack(true),
                pipeline.without(DecisionGate::Revision)
            );
            assert_eq!(
                pipeline.compares_revisions(),
                pipeline.contains(DecisionGate::Revision)
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
                let mut criteria = crate::QualityProfileCriteria {
                    ignore_proper_repack: ignore,
                    decision_pipeline: pipeline,
                    ..Default::default()
                };
                criteria.reconcile_revision_gate();
                assert_eq!(
                    criteria.ignore_proper_repack,
                    !criteria.decision_pipeline.contains(DecisionGate::Revision)
                );
                // What storage keeps reads back to the same profile.
                let mut reread = crate::QualityProfileCriteria {
                    ignore_proper_repack: criteria.ignore_proper_repack,
                    decision_pipeline: criteria.stored_decision_pipeline().unwrap_or_default(),
                    ..Default::default()
                };
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

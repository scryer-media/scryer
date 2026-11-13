//! The preference steps a quality profile's decision pipeline is built from.
//!
//! Each step is one unit struct implementing [`PreferenceStep`], holding all
//! of that step's behaviour: how it compares two releases, how it sorts search
//! results, which cutoff exception it belongs to, what it still wants from a
//! held file, and how it explains a refusal. The engine in
//! [`crate::decision_pipeline`] walks a profile's steps through [`STEPS`] and
//! never names one.

use std::cmp::Ordering;

use crate::admission::{
    AdmissionPolicy, AdmissionRejection, AdmissionRejectionReason, CandidateFacts, Incumbent,
};
use crate::decision_pipeline::{CutoffEscape, DecisionGate, Facts, RankFacts, SideFacts};

/// One preference step of a quality profile's decision pipeline.
///
/// # Adding a preference step
///
/// 1. **Id.** Add a variant to the `decision_gates!` list in
///    `acquisition/decision_pipeline.rs`. `DecisionGate::ALL` and
///    `DecisionGate::COUNT` follow from that list.
/// 2. **Behaviour.** Add a unit struct below implementing this trait: its
///    [`id`](Self::id), its stored [`name`](Self::name) (snake_case, never
///    renamed once released: stored profiles and recorded decisions carry
///    it), [`compare`](Self::compare) and [`ordinal`](Self::ordinal). Override
///    a hook only when the step takes part in what it describes; every
///    default leaves the step out of it.
/// 3. **Registry.** Add one line to [`STEPS`], at the position of the
///    variant. A missing line fails to compile; a misplaced one fails
///    `the_registry_holds_every_step_exactly_once`.
/// 4. **API.** Add one entry to the `decision_step_values!` list in
///    `scryer-interface-media-types/src/types/schema_values.rs` (the GraphQL
///    enum values and their conversions all come from it), then regenerate
///    `api/graphql/schema.graphql` with the `export-graphql-schema` bin.
/// 5. **Inputs**, only when the step reads a property no step reads yet: one
///    field on [`SideFacts`], filled in by its builders
///    ([`CandidateFacts::facts`], [`Incumbent::facts`],
///    [`crate::admission::QueuedRelease::facts`],
///    [`crate::quality::landed_bar_memo::LandedBar::side`],
///    [`crate::quality::canonical_context::IncumbentFacts::side`] and
///    [`SideFacts::of_decision`]). A source that does not carry the property
///    yet has to learn it first.
///
/// Nothing else changes. A new step is not in the default order
/// ([`crate::DecisionPipeline::DEFAULT`]), so no existing profile compares
/// differently until it adds the step. A lost comparison is refused with
/// [`AdmissionRejectionReason::LostAtStep`] unless the step overrides
/// [`refusal`](Self::refusal), and import, the decision log and the deciding
/// step all understand that refusal already.
pub(crate) trait PreferenceStep: Sync {
    /// The id this step is stored, hashed and exposed under.
    fn id(&self) -> DecisionGate;

    /// The name the step is stored and recorded under. Released names are
    /// API: never rename or reuse one.
    fn name(&self) -> &'static str;

    /// The comparator: `Greater` means the candidate is better than what it
    /// is measured against. Must give `Equal` for a side against itself and
    /// reverse when the sides swap.
    fn compare(&self, candidate: &Facts<'_>, held: &Facts<'_>) -> Ordering;

    /// The step as an ascending sort key for search results: lower sorts
    /// first.
    fn ordinal(&self, facts: &RankFacts) -> i64;

    /// Whether [`ordinal`](Self::ordinal) sorts two sides measured on their
    /// own exactly as [`compare`](Self::compare) orders them. Only a step
    /// whose sort key is derived from what the scope already holds, rather
    /// than from the release alone, says no. The step contract tests read it.
    #[cfg(test)]
    fn ordinal_follows_compare(&self) -> bool {
        true
    }

    /// The exception to a reached cutoff a win at this step counts towards.
    fn cutoff_escape(&self) -> CutoffEscape {
        CutoffEscape::None
    }

    /// Whether `candidate` passes a reached cutoff over the file `held` on
    /// this step: `Some(true)` passes, `Some(false)` does not, and `None`
    /// leaves it to the next step of the family. Only steps of a
    /// [`cutoff_escape`](Self::cutoff_escape) family are asked.
    ///
    /// The verdict is the whole rule: a step that only passes while the held
    /// file still wants what it would bring checks that here, so no caller
    /// adds a guard of its own.
    fn cutoff_verdict(&self, _candidate: &Facts<'_>, _held: &Facts<'_>) -> Option<bool> {
        None
    }

    /// Whether the held file still lacks something this step would prefer,
    /// so a scope at its cutoff is not finished while the step is in the
    /// profile's pipeline.
    fn still_wanted(&self, _held: &SideFacts) -> bool {
        false
    }

    /// Whether a win at this step admits a candidate even when the profile
    /// forbids upgrades.
    fn admits_despite_upgrade_guard(&self) -> bool {
        false
    }

    /// Whether a season pack refused by every member is summarised no later
    /// than this step: the steps after it only decide between members it
    /// already ties.
    fn closes_pack_summary(&self) -> bool {
        false
    }

    /// The refusal for losing to `incumbent` at this step. No churn
    /// threshold or format cutoff weighs it: a step is a step, not a delta.
    fn refusal(
        &self,
        incumbent: &Incumbent,
        _candidate: CandidateFacts<'_>,
        _policy: &AdmissionPolicy,
    ) -> AdmissionRejection {
        AdmissionRejection {
            reason: AdmissionRejectionReason::LostAtStep { step: self.id() },
            message: format!(
                "existing file {} is preferred at the profile's {} step",
                incumbent.file_path,
                self.name()
            ),
            incumbent_file_id: incumbent.file_id.clone(),
            incumbent_file_path: incumbent.file_path.clone(),
        }
    }

    /// The summary for a season pack every member of which beats the
    /// candidate at this step, when the step has one; `blocker` is the file
    /// the refusal names.
    fn pack_refusal(
        &self,
        _blocker: &Incumbent,
        _candidate: CandidateFacts<'_>,
    ) -> Option<(AdmissionRejectionReason, String)> {
        None
    }

    /// Whether `reason` is a refusal of this step's own kind. Only a step that
    /// overrides [`refusal`](Self::refusal) with a reason of its own needs
    /// this; [`AdmissionRejectionReason::LostAtStep`] names its step already.
    fn claims(&self, _reason: &AdmissionRejectionReason) -> bool {
        false
    }
}

/// The step registry, one entry per [`DecisionGate`], in declaration order.
pub(crate) static STEPS: [&'static dyn PreferenceStep; DecisionGate::COUNT] = [
    &QualityTierStep,
    &RevisionStep,
    &PreferredLanguageStep,
    &OptionalLanguagesStep,
];

/// Positions in the profile's own tier ordering. A quality the profile does
/// not list ranks below every listed one; it is only here at all because some
/// other gate let it through.
struct QualityTierStep;

impl PreferenceStep for QualityTierStep {
    fn id(&self) -> DecisionGate {
        DecisionGate::QualityTier
    }

    fn name(&self) -> &'static str {
        "quality_tier"
    }

    fn compare(&self, candidate: &Facts<'_>, held: &Facts<'_>) -> Ordering {
        held.side.tier_sort_key.cmp(&candidate.side.tier_sort_key)
    }

    fn ordinal(&self, facts: &RankFacts) -> i64 {
        i64::try_from(facts.side.tier_sort_key).unwrap_or(i64::MAX)
    }

    fn closes_pack_summary(&self) -> bool {
        true
    }

    /// No score rescues a lower tier: this is Sonarr's
    /// `qualityCompare < 0 => BetterQuality`, and it is what stops a
    /// custom-format bonus or a size term buying its way across a resolution
    /// step.
    fn refusal(
        &self,
        incumbent: &Incumbent,
        _candidate: CandidateFacts<'_>,
        _policy: &AdmissionPolicy,
    ) -> AdmissionRejection {
        AdmissionRejection {
            reason: AdmissionRejectionReason::LowerQualityTier,
            message: format!(
                "existing file {} is a better quality than this candidate",
                incumbent.file_path
            ),
            incumbent_file_id: incumbent.file_id.clone(),
            incumbent_file_path: incumbent.file_path.clone(),
        }
    }

    fn pack_refusal(
        &self,
        _blocker: &Incumbent,
        _candidate: CandidateFacts<'_>,
    ) -> Option<(AdmissionRejectionReason, String)> {
        Some((
            AdmissionRejectionReason::LowerQualityTier,
            "every episode in this pack is already held by a better quality file".to_string(),
        ))
    }

    fn claims(&self, reason: &AdmissionRejectionReason) -> bool {
        matches!(reason, AdmissionRejectionReason::LowerQualityTier)
    }
}

/// PROPER/REPACK ranks, only between releases of the same tier from the same
/// release group. Its sort key is the context-derived revision preference
/// (see [`crate::admission::revision_preference`]), not a property of the
/// release alone.
struct RevisionStep;

impl PreferenceStep for RevisionStep {
    fn id(&self) -> DecisionGate {
        DecisionGate::Revision
    }

    fn name(&self) -> &'static str {
        "revision"
    }

    fn compare(&self, candidate: &Facts<'_>, held: &Facts<'_>) -> Ordering {
        candidate.revision_cmp(held)
    }

    fn ordinal(&self, facts: &RankFacts) -> i64 {
        i64::from(facts.negated_revision)
    }

    #[cfg(test)]
    fn ordinal_follows_compare(&self) -> bool {
        false
    }

    fn cutoff_escape(&self) -> CutoffEscape {
        CutoffEscape::Revision
    }

    /// A later revision of the release held passes the cutoff; anything else
    /// does not.
    fn cutoff_verdict(&self, candidate: &Facts<'_>, held: &Facts<'_>) -> Option<bool> {
        Some(self.compare(candidate, held).is_gt())
    }

    /// A PROPER or REPACK repairs the release already held rather than
    /// chasing a better one.
    fn admits_despite_upgrade_guard(&self) -> bool {
        true
    }

    fn refusal(
        &self,
        incumbent: &Incumbent,
        candidate: CandidateFacts<'_>,
        policy: &AdmissionPolicy,
    ) -> AdmissionRejection {
        if !policy.allow_upgrades {
            return crate::admission::upgrades_disabled(incumbent);
        }
        AdmissionRejection {
            reason: AdmissionRejectionReason::LowerRevision {
                incumbent_revision: incumbent.revision,
                candidate_revision: candidate.revision,
            },
            message: format!(
                "existing file {} is a later revision (PROPER/REPACK {} > {})",
                incumbent.file_path, incumbent.revision, candidate.revision
            ),
            incumbent_file_id: incumbent.file_id.clone(),
            incumbent_file_path: incumbent.file_path.clone(),
        }
    }

    fn claims(&self, reason: &AdmissionRejectionReason) -> bool {
        matches!(reason, AdmissionRejectionReason::LowerRevision { .. })
    }
}

/// The position of the first preferred audio language each carries.
struct PreferredLanguageStep;

impl PreferenceStep for PreferredLanguageStep {
    fn id(&self) -> DecisionGate {
        DecisionGate::PreferredLanguage
    }

    fn name(&self) -> &'static str {
        "preferred_language"
    }

    fn compare(&self, candidate: &Facts<'_>, held: &Facts<'_>) -> Ordering {
        held.side
            .language
            .matched
            .preferred_rank
            .cmp(&candidate.side.language.matched.preferred_rank)
    }

    fn ordinal(&self, facts: &RankFacts) -> i64 {
        i64::from(facts.side.language.matched.preferred_rank)
    }

    fn cutoff_escape(&self) -> CutoffEscape {
        CutoffEscape::Language
    }

    /// A better preferred rank always passes the cutoff, a worse one never
    /// does. A held file something ranks better than is not at the top rank,
    /// so it still wants a preferred language.
    fn cutoff_verdict(&self, candidate: &Facts<'_>, held: &Facts<'_>) -> Option<bool> {
        match self.compare(candidate, held) {
            Ordering::Greater => Some(true),
            Ordering::Less => Some(false),
            Ordering::Equal => None,
        }
    }

    fn still_wanted(&self, held: &SideFacts) -> bool {
        held.language.preferred_wanted
    }

    fn refusal(
        &self,
        incumbent: &Incumbent,
        candidate: CandidateFacts<'_>,
        _policy: &AdmissionPolicy,
    ) -> AdmissionRejection {
        AdmissionRejection {
            reason: AdmissionRejectionReason::LowerLanguagePreference {
                incumbent_preferred_rank: incumbent.language.matched.preferred_rank,
                candidate_preferred_rank: candidate.language.matched.preferred_rank,
            },
            message: format!(
                "existing file {} carries a more preferred audio language than this candidate",
                incumbent.file_path
            ),
            incumbent_file_id: incumbent.file_id.clone(),
            incumbent_file_path: incumbent.file_path.clone(),
        }
    }

    fn pack_refusal(
        &self,
        blocker: &Incumbent,
        candidate: CandidateFacts<'_>,
    ) -> Option<(AdmissionRejectionReason, String)> {
        Some((
            AdmissionRejectionReason::LowerLanguagePreference {
                incumbent_preferred_rank: blocker.language.matched.preferred_rank,
                candidate_preferred_rank: candidate.language.matched.preferred_rank,
            },
            "every episode in this pack is already held by a file in a more preferred audio language"
                .to_string(),
        ))
    }

    fn claims(&self, reason: &AdmissionRejectionReason) -> bool {
        matches!(
            reason,
            AdmissionRejectionReason::LowerLanguagePreference { .. }
        )
    }
}

/// How many of the optional audio languages each carries.
struct OptionalLanguagesStep;

impl PreferenceStep for OptionalLanguagesStep {
    fn id(&self) -> DecisionGate {
        DecisionGate::OptionalLanguages
    }

    fn name(&self) -> &'static str {
        "optional_languages"
    }

    fn compare(&self, candidate: &Facts<'_>, held: &Facts<'_>) -> Ordering {
        candidate
            .side
            .language
            .matched
            .optional_hits
            .cmp(&held.side.language.matched.optional_hits)
    }

    fn ordinal(&self, facts: &RankFacts) -> i64 {
        -i64::from(facts.side.language.matched.optional_hits)
    }

    fn cutoff_escape(&self) -> CutoffEscape {
        CutoffEscape::Language
    }

    /// More optional languages pass the cutoff only while the held file is
    /// still wanted for them, which is when the profile's libraries keep
    /// searching for optional languages; otherwise an optional language
    /// improves a file when one turns up but is not worth going past the
    /// cutoff for.
    fn cutoff_verdict(&self, candidate: &Facts<'_>, held: &Facts<'_>) -> Option<bool> {
        Some(held.side.language.optional_wanted && self.compare(candidate, held).is_gt())
    }

    fn still_wanted(&self, held: &SideFacts) -> bool {
        held.language.optional_wanted
    }

    fn refusal(
        &self,
        incumbent: &Incumbent,
        candidate: CandidateFacts<'_>,
        _policy: &AdmissionPolicy,
    ) -> AdmissionRejection {
        AdmissionRejection {
            reason: AdmissionRejectionReason::FewerOptionalLanguages {
                incumbent_optional_hits: incumbent.language.matched.optional_hits,
                candidate_optional_hits: candidate.language.matched.optional_hits,
            },
            message: format!(
                "existing file {} carries more of the optional audio languages ({} > {})",
                incumbent.file_path,
                incumbent.language.matched.optional_hits,
                candidate.language.matched.optional_hits
            ),
            incumbent_file_id: incumbent.file_id.clone(),
            incumbent_file_path: incumbent.file_path.clone(),
        }
    }

    fn pack_refusal(
        &self,
        blocker: &Incumbent,
        candidate: CandidateFacts<'_>,
    ) -> Option<(AdmissionRejectionReason, String)> {
        Some((
            AdmissionRejectionReason::FewerOptionalLanguages {
                incumbent_optional_hits: blocker.language.matched.optional_hits,
                candidate_optional_hits: candidate.language.matched.optional_hits,
            },
            "every episode in this pack is already held by a file with more of the optional audio languages"
                .to_string(),
        ))
    }

    fn claims(&self, reason: &AdmissionRejectionReason) -> bool {
        matches!(
            reason,
            AdmissionRejectionReason::FewerOptionalLanguages { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ReleaseDecidingStep;

    /// A step that overrides nothing it does not have to, standing in for
    /// one added later: it borrows an existing id because ids are a closed
    /// enum.
    struct BareStep;

    impl PreferenceStep for BareStep {
        fn id(&self) -> DecisionGate {
            DecisionGate::OptionalLanguages
        }

        fn name(&self) -> &'static str {
            "bare_step"
        }

        fn compare(&self, _candidate: &Facts<'_>, _held: &Facts<'_>) -> Ordering {
            Ordering::Equal
        }

        fn ordinal(&self, _facts: &RankFacts) -> i64 {
            0
        }
    }

    fn incumbent() -> Incumbent {
        Incumbent {
            tier_index: Some(0),
            revision: 0,
            file_id: "file-1".to_string(),
            file_path: "/library/Sample Title/Sample Title.mkv".to_string(),
            release_group: None,
            score: 100,
            language: crate::AudioLanguageRank::default(),
            covers: Vec::new(),
            created_at: String::new(),
        }
    }

    /// The defaults leave a step out of every hook, and its refusal is the
    /// generic one, which names the step it came from.
    #[test]
    fn a_step_that_overrides_no_hook_takes_part_in_none() {
        let step = BareStep;
        let held = incumbent();
        let candidate = CandidateFacts::new(Some(0), 0, 100);
        assert!(step.ordinal_follows_compare());
        assert_eq!(step.cutoff_escape(), CutoffEscape::None);
        assert_eq!(step.cutoff_verdict(&candidate.facts(), &held.facts()), None);
        assert!(!step.still_wanted(&held.facts().side));
        assert!(!step.admits_despite_upgrade_guard());
        assert!(!step.closes_pack_summary());
        assert!(step.pack_refusal(&held, candidate).is_none());

        let refusal = step.refusal(
            &held,
            candidate,
            &crate::admission::AdmissionPolicy::not_a_downgrade(),
        );
        assert_eq!(
            refusal.reason,
            AdmissionRejectionReason::LostAtStep {
                step: DecisionGate::OptionalLanguages
            }
        );
        assert_eq!(
            refusal.message,
            "existing file /library/Sample Title/Sample Title.mkv is preferred at the profile's bare_step step"
        );
        assert_eq!(refusal.incumbent_file_id, "file-1");
        assert!(!step.claims(&refusal.reason));
        assert_eq!(
            refusal.reason.deciding_step(),
            Some(ReleaseDecidingStep::Gate(DecisionGate::OptionalLanguages))
        );
    }

    /// Each registered step claims exactly the refusals it builds, so the
    /// deciding step of a refusal is the step that raised it.
    #[test]
    fn every_step_claims_the_refusal_it_builds() {
        let held = incumbent();
        let candidate = CandidateFacts::new(Some(0), 0, 100);
        for policy in [
            crate::admission::AdmissionPolicy::not_a_downgrade(),
            crate::admission::AdmissionPolicy::manual(),
        ] {
            for step in STEPS {
                let reason = step.refusal(&held, candidate, &policy).reason;
                let claimants: Vec<_> = STEPS
                    .iter()
                    .filter(|other| other.claims(&reason))
                    .map(|other| other.id())
                    .collect();
                match reason {
                    AdmissionRejectionReason::LostAtStep { step: id } => {
                        assert!(claimants.is_empty());
                        assert_eq!(id, step.id());
                    }
                    _ => assert_eq!(claimants, [step.id()], "{reason:?}"),
                }
                assert_eq!(
                    reason.deciding_step(),
                    Some(ReleaseDecidingStep::Gate(step.id()))
                );
                if let Some((pack_reason, _)) = step.pack_refusal(&held, candidate) {
                    assert!(step.claims(&pack_reason), "{pack_reason:?}");
                }
            }
        }
    }
}

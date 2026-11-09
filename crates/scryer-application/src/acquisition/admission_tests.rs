//! Admission gate behaviour.
//!
//! These pin the predicate that both the grab path and the import path share.
//! A divergence here is the bug this work exists to remove, so the cases are
//! written from the gate's contract rather than from either caller.

use super::admission::*;
use crate::AudioLanguageRank;
use crate::decision_pipeline::{DecisionGate, DecisionPipeline};

/// The order that compares the preferred language ahead of the quality tier.
fn language_ahead_of_tier() -> DecisionPipeline {
    DecisionPipeline::try_from_gates([
        DecisionGate::PreferredLanguage,
        DecisionGate::QualityTier,
        DecisionGate::Revision,
        DecisionGate::OptionalLanguages,
    ])
    .unwrap()
}

fn incumbent(id: &str, score: i32, covers: &[&str]) -> (Incumbent, bool) {
    (
        Incumbent {
            // Existing cases all sit in one tier, so score remains the decider.
            tier_index: Some(0),
            // …and at revision 0, so the score still decides. Revision cases
            // build on this helper and raise it explicitly.
            revision: 0,
            file_id: id.to_string(),
            file_path: format!("/data/TV/Show/Season 01/{id}.mkv"),
            release_group: Some("group".into()),
            score,
            language: Default::default(),
            covers: covers.iter().map(|value| (*value).to_string()).collect(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
        },
        true,
    )
}

fn episodes(ids: &[&str]) -> AdmissionScope {
    AdmissionScope::Episodes(ids.iter().map(|value| (*value).to_string()).collect())
}

fn auto(min_delta: i32) -> AdmissionPolicy {
    AdmissionPolicy {
        allow_upgrades: true,
        min_delta,
        cutoff_score: None,
        manual_override: false,
        applies_to_queue: false,
    }
}

#[test]
fn unoccupied_scope_admits_without_consulting_a_score() {
    let subject = AdmissionSubject::new(episodes(&["ep-1"]), []);

    assert!(subject.is_unoccupied());
    assert_eq!(subject.best_score(), None);

    // Even a deeply negative candidate: with nothing on disk there is no bar.
    let verdict = evaluate_admission(&subject, CandidateFacts::new(Some(0), 0, -500), &auto(200));
    assert!(verdict.is_admitted());
}

#[test]
fn additional_role_files_are_not_upgrade_targets() {
    let (extra, _) = incumbent("file-extra", 9_000, &["ep-1"]);
    let subject = AdmissionSubject::new(episodes(&["ep-1"]), [(extra, false)]);

    assert!(
        subject.is_unoccupied(),
        "only primary files may block an upgrade"
    );
    assert!(
        evaluate_admission(&subject, CandidateFacts::new(Some(0), 0, 100), &auto(200))
            .is_admitted()
    );
}

#[test]
fn equal_score_is_not_an_upgrade() {
    let subject =
        AdmissionSubject::new(episodes(&["ep-1"]), [incumbent("file-1", 3_270, &["ep-1"])]);
    let verdict = evaluate_admission(&subject, CandidateFacts::new(Some(0), 0, 3_270), &auto(1));

    let rejection = verdict.rejection().expect("equal score must not land");
    assert!(matches!(
        rejection.reason,
        AdmissionRejectionReason::NotAnUpgrade { .. }
    ));
}

#[test]
fn improvement_below_the_required_delta_is_rejected() {
    let subject =
        AdmissionSubject::new(episodes(&["ep-1"]), [incumbent("file-1", 3_000, &["ep-1"])]);

    // +90 against a 200-point requirement — the shape of the AMZN WEB-DL row.
    let verdict = evaluate_admission(&subject, CandidateFacts::new(Some(0), 0, 3_090), &auto(200));
    assert!(matches!(
        verdict.rejection().map(|r| &r.reason),
        Some(AdmissionRejectionReason::NotAnUpgrade { .. })
    ));

    // Clearing the bar lands.
    assert!(
        evaluate_admission(&subject, CandidateFacts::new(Some(0), 0, 3_250), &auto(200))
            .is_admitted()
    );
}

#[test]
fn profile_that_disallows_upgrades_blocks_an_occupied_scope() {
    let subject = AdmissionSubject::new(episodes(&["ep-1"]), [incumbent("file-1", 100, &["ep-1"])]);
    let policy = AdmissionPolicy {
        allow_upgrades: false,
        min_delta: 0,
        cutoff_score: None,
        manual_override: false,
        applies_to_queue: false,
    };

    let verdict = evaluate_admission(&subject, CandidateFacts::new(Some(0), 0, 10_000), &policy);
    assert!(matches!(
        verdict.rejection().map(|r| &r.reason),
        Some(AdmissionRejectionReason::UpgradesDisabled)
    ));
}

#[test]
fn manual_replacement_bypasses_every_score_comparison() {
    let subject =
        AdmissionSubject::new(episodes(&["ep-1"]), [incumbent("file-1", 7_570, &["ep-1"])]);

    // Far below the incumbent, and upgrades would otherwise be refused.
    let verdict = evaluate_admission(
        &subject,
        CandidateFacts::new(Some(0), 0, 10),
        &AdmissionPolicy::manual(),
    );
    assert!(
        verdict.is_admitted(),
        "an operator asking for this by hand must not be second-guessed by a score"
    );
}

#[test]
fn manual_replacement_still_cannot_drop_episode_coverage() {
    // A multi-episode incumbent against a single-episode target: landing it
    // would silently lose ep-2. That is data loss, not a preference.
    let subject = AdmissionSubject::new(
        episodes(&["ep-1"]),
        [incumbent("file-1", 10, &["ep-1", "ep-2"])],
    );

    let verdict = evaluate_admission(
        &subject,
        CandidateFacts::new(Some(0), 0, 10_000),
        &AdmissionPolicy::manual(),
    );
    assert!(matches!(
        verdict.rejection().map(|r| &r.reason),
        Some(AdmissionRejectionReason::BroaderIncumbentSpan)
    ));
}

#[test]
fn pack_replaces_singles_only_when_it_beats_all_of_them() {
    // The reason the incumbent set is plural: one candidate, many primaries.
    let subject = AdmissionSubject::new(
        episodes(&["ep-1", "ep-2"]),
        [
            incumbent("file-1", 300, &["ep-1"]),
            incumbent("file-2", 450, &["ep-2"]),
        ],
    );

    let verdict = evaluate_admission(&subject, CandidateFacts::new(Some(0), 0, 900), &auto(1));
    match verdict {
        AdmissionVerdict::Admit {
            ranked_superseded,
            previous_best_score,
            ..
        } => {
            assert_eq!(ranked_superseded, vec!["file-2", "file-1"], "best-first");
            assert_eq!(previous_best_score, 450);
        }
        AdmissionVerdict::Reject(rejection) => {
            panic!("pack beating both singles should land: {rejection:?}")
        }
    }

    // Losing to just one member is enough to refuse the whole pack.
    assert!(
        !evaluate_admission(&subject, CandidateFacts::new(Some(0), 0, 400), &auto(1)).is_admitted()
    );
}

#[test]
fn incumbent_order_is_total_and_independent_of_input_order() {
    let forward = AdmissionSubject::new(
        episodes(&["ep-1", "ep-2"]),
        [
            incumbent("file-a", 500, &["ep-1"]),
            incumbent("file-b", 500, &["ep-2"]),
        ],
    );
    let reversed = AdmissionSubject::new(
        episodes(&["ep-1", "ep-2"]),
        [
            incumbent("file-b", 500, &["ep-2"]),
            incumbent("file-a", 500, &["ep-1"]),
        ],
    );

    let ids = |subject: &AdmissionSubject| {
        subject
            .incumbents()
            .iter()
            .map(|incumbent| incumbent.file_id.clone())
            .collect::<Vec<_>>()
    };

    assert_eq!(
        ids(&forward),
        ids(&reversed),
        "tied scores must not let row order decide the verdict"
    );
}

#[test]
fn title_scope_ignores_the_episode_span_guard() {
    let mut movie = incumbent("file-1", 100, &[]);
    movie.0.covers.clear();
    let subject = AdmissionSubject::new(AdmissionScope::Title, [movie]);

    assert!(
        evaluate_admission(&subject, CandidateFacts::new(Some(0), 0, 900), &auto(1)).is_admitted()
    );
}

/// The gate is a pure function: the same subject and policy must always give the
/// same answer, which is what lets grab and import rely on agreeing.
#[test]
fn verdicts_are_deterministic() {
    let subject =
        AdmissionSubject::new(episodes(&["ep-1"]), [incumbent("file-1", 3_270, &["ep-1"])]);
    let policy = auto(200);

    let first = evaluate_admission(&subject, CandidateFacts::new(Some(0), 0, 3_500), &policy);
    let second = evaluate_admission(&subject, CandidateFacts::new(Some(0), 0, 3_500), &policy);

    assert_eq!(first.is_admitted(), second.is_admitted());
}

// ---------------------------------------------------------------------------
// Regression: the rows that started this work
// ---------------------------------------------------------------------------
//
// Three Erai-raws anime releases were auto-grabbed, downloaded to 100%, and then
// refused at import with "existing episode file … is equal or better". The grab
// side had compared them against a ledger score instead of the library, so it
// never saw the incumbent that was going to refuse them.
//
// The scores below are the ones the blocked imports reported. What these pin is
// that the same verdict is reached *before* the bytes move.

/// Grab must reach the same conclusion the import gate did, from the same facts.
fn grab_policy() -> AdmissionPolicy {
    AdmissionPolicy {
        allow_upgrades: true,
        min_delta: 200, // Balanced persona
        cutoff_score: None,
        manual_override: false,
        applies_to_queue: false,
    }
}

#[test]
fn erai_raws_webrip_is_declined_at_grab_not_after_downloading() {
    // Heroine Saint No: incumbent WEB-DL H.264 at 7570, candidate WEBRip HEVC at 3310.
    let subject = AdmissionSubject::new(
        episodes(&["ep-09"]),
        [incumbent("file-1", 7_570, &["ep-09"])],
    );

    let verdict = evaluate_admission(
        &subject,
        CandidateFacts::new(Some(0), 0, 3_310),
        &grab_policy(),
    );
    assert!(
        !verdict.is_admitted(),
        "a candidate 4260 points below the incumbent must never be queued"
    );

    // The Villager of Level 999: 3970 incumbent, 2710 candidate.
    let villager = AdmissionSubject::new(
        episodes(&["ep-09"]),
        [incumbent("file-2", 3_970, &["ep-09"])],
    );
    assert!(
        !evaluate_admission(
            &villager,
            CandidateFacts::new(Some(0), 0, 2_710),
            &grab_policy()
        )
        .is_admitted()
    );
}

/// The narrow one: +90 over the incumbent. It cleared nothing, and under the
/// churn threshold it is not worth a download either.
#[test]
fn marginal_improvement_does_not_justify_a_download() {
    // From Old Country Bumpkin: incumbent WEBRip H.265 at 3270, candidate
    // AMZN WEB-DL AVC at 3180 — the candidate is actually *behind*.
    let subject = AdmissionSubject::new(
        episodes(&["ep-07"]),
        [incumbent("file-3", 3_270, &["ep-07"])],
    );
    assert!(
        !evaluate_admission(
            &subject,
            CandidateFacts::new(Some(0), 0, 3_180),
            &grab_policy()
        )
        .is_admitted()
    );

    // Even had it edged ahead, +90 is below the persona's churn threshold.
    assert!(
        !evaluate_admission(
            &subject,
            CandidateFacts::new(Some(0), 0, 3_360),
            &grab_policy()
        )
        .is_admitted()
    );
}

/// Grab must be at least as strict as import, or it can authorise a download the
/// import gate will refuse. This is the property that makes "never queue what it
/// will not import" structural rather than a matter of tuning.
#[test]
fn grab_is_never_more_permissive_than_import() {
    let subject = AdmissionSubject::new(
        episodes(&["ep-01"]),
        [incumbent("file-1", 3_000, &["ep-01"])],
    );

    for candidate_score in [2_000, 2_999, 3_000, 3_001, 3_100, 3_199, 3_200, 4_500] {
        let grab = evaluate_admission(
            &subject,
            CandidateFacts::new(Some(0), 0, candidate_score),
            &grab_policy(),
        )
        .is_admitted();
        let import = evaluate_admission(
            &subject,
            CandidateFacts::new(Some(0), 0, candidate_score),
            &AdmissionPolicy::not_a_downgrade(),
        )
        .is_admitted();

        assert!(
            !grab || import,
            "score {candidate_score} would be grabbed but refused at import"
        );
    }
}

/// A whole-tier improvement clears the churn guard — which is there to stop
/// trimmings, not genuine upgrades.
///
/// This used to be spelled as a *score* relaxation (`CROSS_TIER_DELTA`: any
/// delta above 1000 dropped the requirement from 200 to 30), which only worked
/// while the tier was worth 3200/900/300 inside the score. It is now the tier
/// comparison itself, above the delta, so the candidate can score *less* than
/// the file it replaces and still be an upgrade.
#[test]
fn a_better_tier_clears_the_churn_threshold_at_any_delta() {
    let subject = AdmissionSubject::new(
        episodes(&["ep-01"]),
        [tiered("file-1", Some(2), 3_000, &["ep-01"])],
    );

    for candidate_score in [1_000, 3_000, 3_050, 4_000] {
        assert!(
            evaluate_admission(
                &subject,
                CandidateFacts::new(Some(0), 0, candidate_score),
                &grab_policy()
            )
            .is_admitted(),
            "a whole tier better must not be held by a same-tier churn threshold \
             (candidate score {candidate_score})"
        );
    }
}

/// Once an incumbent has reached the profile's format cutoff, ordinary
/// improvements stop earning bandwidth; only a tier jump does.
///
/// The reason is its own (D19/n1): reporting `NotAnUpgrade` with a
/// `required_delta` reads as a threshold an operator can tune, and no delta
/// clears a cutoff.
#[test]
fn an_incumbent_past_the_format_cutoff_resists_same_tier_nudges() {
    let subject = AdmissionSubject::new(
        episodes(&["ep-01"]),
        [incumbent("file-1", 3_000, &["ep-01"])],
    );
    let policy = AdmissionPolicy {
        cutoff_score: Some(2_500),
        ..grab_policy()
    };

    // Same tier, however far ahead: the cutoff is a full stop, not a threshold.
    for candidate_score in [3_400, 4_100] {
        let verdict = evaluate_admission(
            &subject,
            CandidateFacts::new(Some(0), 0, candidate_score),
            &policy,
        );
        assert!(
            matches!(
                verdict.rejection().map(|rejection| &rejection.reason),
                Some(AdmissionRejectionReason::FormatCutoffReached {
                    incumbent_score: 3_000,
                    cutoff_score: 2_500,
                })
            ),
            "score {candidate_score} against a satisfied incumbent: {verdict:?}"
        );
    }

    // A better tier still admits: it never reaches the floor comparison.
    let tiered_subject = AdmissionSubject::new(
        episodes(&["ep-01"]),
        [tiered("file-1", Some(1), 3_000, &["ep-01"])],
    );
    assert!(
        evaluate_admission(
            &tiered_subject,
            CandidateFacts::new(Some(0), 0, 100),
            &policy
        )
        .is_admitted()
    );
}

// ── Quality tier gates above score ────────────────────────────────────────

fn tiered(id: &str, tier_index: Option<usize>, score: i32, covers: &[&str]) -> (Incumbent, bool) {
    let (mut incumbent, primary) = incumbent(id, score, covers);
    incumbent.tier_index = tier_index;
    (incumbent, primary)
}

/// The defect in one test: a lower-tier release must never displace a
/// higher-tier one, however good its score.
///
/// Tier used to be worth 3200/900/300 *inside* the score, so a size penalty or a
/// custom-format bonus could argue across a resolution step — a WEBRip beating a
/// WEB-DL over a −700 size cliff. Now the tier is compared first and no score
/// rescues a downgrade, which is Sonarr's `qualityCompare < 0 => BetterQuality`.
#[test]
fn a_lower_tier_candidate_is_refused_at_any_score() {
    let subject = AdmissionSubject::new(
        episodes(&["ep-01"]),
        [tiered("file-1", Some(0), 3_000, &["ep-01"])],
    );

    for candidate_score in [3_001, 5_000, 50_000] {
        let verdict = evaluate_admission(
            &subject,
            CandidateFacts::new(Some(1), 0, candidate_score),
            &auto(1),
        );
        assert!(
            matches!(
                verdict.rejection().map(|r| &r.reason),
                Some(AdmissionRejectionReason::LowerQualityTier)
            ),
            "score {candidate_score} bought its way down a tier: {verdict:?}"
        );
    }
}

/// The converse: a whole tier better is an upgrade even when the score does not
/// clear the churn threshold, because the threshold exists to stop trimmings.
#[test]
fn a_higher_tier_candidate_clears_the_churn_threshold() {
    let subject = AdmissionSubject::new(
        episodes(&["ep-01"]),
        [tiered("file-1", Some(2), 3_000, &["ep-01"])],
    );

    assert!(
        evaluate_admission(&subject, CandidateFacts::new(Some(0), 0, 2_000), &auto(500))
            .is_admitted()
    );
}

/// A quality the profile does not list ranks below every quality it does.
#[test]
fn an_unlisted_quality_ranks_below_a_listed_one() {
    let listed = AdmissionSubject::new(
        episodes(&["ep-01"]),
        [tiered("file-1", Some(1), 100, &["ep-01"])],
    );
    assert!(
        !evaluate_admission(&listed, CandidateFacts::new(None, 0, 10_000), &auto(1)).is_admitted()
    );

    let unlisted = AdmissionSubject::new(
        episodes(&["ep-01"]),
        [tiered("file-1", None, 10_000, &["ep-01"])],
    );
    assert!(
        evaluate_admission(&unlisted, CandidateFacts::new(Some(1), 0, 100), &auto(1)).is_admitted()
    );
}

// ── Import refuses only downgrades ────────────────────────────────────────

/// The bytes are already on disk. Refusing a file that merely ties the incumbent
/// is what produced "existing file is equal or better" on a release Scryer
/// itself queued, so import accepts a tie and rejects only a downgrade.
#[test]
fn import_accepts_a_tie_and_refuses_a_downgrade() {
    let subject = AdmissionSubject::new(
        episodes(&["ep-01"]),
        [incumbent("file-1", 3_000, &["ep-01"])],
    );

    assert!(
        evaluate_admission(
            &subject,
            CandidateFacts::new(Some(0), 0, 3_000),
            &AdmissionPolicy::not_a_downgrade()
        )
        .is_admitted(),
        "an equally scored import is not a downgrade"
    );
    assert!(
        !evaluate_admission(
            &subject,
            CandidateFacts::new(Some(0), 0, 2_999),
            &AdmissionPolicy::not_a_downgrade()
        )
        .is_admitted()
    );
}

// ── Season packs are judged per member ────────────────────────────────────

/// A pack arrives as one file per episode, each gated on its own at import, so
/// one improvable member is reason enough to fetch it — Sonarr's
/// `SeasonPackUpgrade::Any`. Returning an empty subject for a pack, as this used
/// to, made every pack look like a first grab.
#[test]
fn a_pack_is_grabbed_when_any_member_would_improve() {
    let subject = AdmissionSubject::new(
        episodes(&["ep-01", "ep-02"]),
        [
            incumbent("file-1", 3_000, &["ep-01"]),
            incumbent("file-2", 100, &["ep-02"]),
        ],
    )
    .per_member();

    let verdict = evaluate_admission(&subject, CandidateFacts::new(Some(0), 0, 2_000), &auto(1));
    assert!(
        verdict.is_admitted(),
        "ep-02 is beatable, so the pack is worth fetching"
    );
}

/// …and refused when every member is already held by an equal or better file,
/// which is the season-pack shape of "never queue what import will refuse".
#[test]
fn a_pack_is_refused_when_every_member_is_already_better() {
    let subject = AdmissionSubject::new(
        episodes(&["ep-01", "ep-02"]),
        [
            incumbent("file-1", 3_000, &["ep-01"]),
            incumbent("file-2", 3_000, &["ep-02"]),
        ],
    )
    .per_member();

    assert!(
        !evaluate_admission(&subject, CandidateFacts::new(Some(0), 0, 2_000), &auto(1))
            .is_admitted()
    );
}

/// A member nobody has filled yet is reason enough on its own: it is missing,
/// and fetching it is the point.
#[test]
fn a_pack_with_a_missing_member_is_always_worth_fetching() {
    let subject = AdmissionSubject::new(
        episodes(&["ep-01", "ep-02"]),
        [incumbent("file-1", 30_000, &["ep-01"])],
    )
    .per_member();

    let verdict = evaluate_admission(&subject, CandidateFacts::new(Some(0), 0, 1), &auto(1));
    assert!(verdict.is_admitted());

    // A single-file subject over the same span admits on its own terms.
    let single_file = AdmissionSubject::new(episodes(&["ep-01"]), []);
    assert!(
        evaluate_admission(&single_file, CandidateFacts::new(Some(0), 0, 1), &auto(1))
            .is_admitted()
    );
}

// ── One pack gate: the season has to exist before it can be packed ────────

/// Sonarr's `FullSeasonSpecification`. A pack cannot contain an episode that has
/// not aired, so fetching one mid-season buys a partial season at pack bandwidth
/// and then blocks the per-episode searches that would actually fill it.
///
/// This overrides the missing-member clause: a mid-season pack is *always*
/// missing members, which is exactly why the old "any missing member is reason
/// enough" logic grabbed it every cycle.
#[test]
fn a_pack_is_refused_while_a_monitored_member_has_not_aired() {
    let subject = AdmissionSubject::new(
        episodes(&["ep-01", "ep-02", "ep-03"]),
        [incumbent("file-1", 100, &["ep-01"])],
    )
    .per_member()
    .with_unaired_members(1);

    let verdict = evaluate_admission(&subject, CandidateFacts::new(Some(0), 0, 5_000), &auto(1));
    assert!(
        matches!(
            verdict.rejection().map(|r| &r.reason),
            Some(AdmissionRejectionReason::SeasonIncomplete)
        ),
        "a season still airing cannot be packed: {verdict:?}"
    );

    // Once everything has aired, the same subject is worth fetching.
    let aired = AdmissionSubject::new(
        episodes(&["ep-01", "ep-02", "ep-03"]),
        [incumbent("file-1", 100, &["ep-01"])],
    )
    .per_member()
    .with_unaired_members(0);
    assert!(
        evaluate_admission(&aired, CandidateFacts::new(Some(0), 0, 5_000), &auto(1)).is_admitted()
    );
}

/// The unaired refusal has to name a season even when nothing is on disk yet —
/// a brand-new season is the common case, and an empty incumbent list must not
/// panic on the way to reporting it.
#[test]
fn an_unaired_season_with_no_files_yet_is_still_a_refusal_not_a_panic() {
    let subject = AdmissionSubject::new(episodes(&["ep-01", "ep-02"]), [])
        .per_member()
        .with_unaired_members(2);

    let verdict = evaluate_admission(&subject, CandidateFacts::new(Some(0), 0, 900), &auto(1));
    assert!(matches!(
        verdict.rejection().map(|r| &r.reason),
        Some(AdmissionRejectionReason::SeasonIncomplete)
    ));
}

/// A collection whose members are all unmonitored resolves to an empty pack
/// subject. Nothing is missing and nothing is improvable, so the "no reason to
/// refuse" fall-through would admit — and fetch an entire season nobody asked
/// for. It is a refusal, and (since D8's monitored filter can produce this
/// shape) not the `.expect` panic it used to be either.
#[test]
fn a_pack_scope_with_no_monitored_members_is_refused_rather_than_admitted() {
    let subject = AdmissionSubject::new(AdmissionScope::Episodes(Vec::new()), []).per_member();

    let verdict = evaluate_admission(&subject, CandidateFacts::new(Some(0), 0, 900), &auto(200));
    assert!(
        !verdict.is_admitted(),
        "an empty pack scope has nothing to fill: {verdict:?}"
    );
    let AdmissionVerdict::Reject(rejection) = &verdict else {
        panic!("expected a rejection, got {verdict:?}");
    };
    assert!(matches!(
        rejection.reason,
        AdmissionRejectionReason::SeasonIncomplete
    ));
    assert!(
        rejection.message.contains("no monitored episodes"),
        "the message must distinguish this from an airing season: {}",
        rejection.message
    );
}

/// A file covering an episode **outside** the scope must not be counted as
/// filling it.
///
/// D8 narrows a pack's members to the monitored episodes, but an incumbent's
/// `covers` is its full span. A two-episode file covering one monitored member
/// and one unmonitored one used to count as two, which made a season with a
/// genuinely missing episode look full — and the pack was then refused as
/// `NotAnUpgrade` against that one file.
#[test]
fn coverage_outside_the_member_set_does_not_mask_a_missing_member() {
    // Members: E01 and E02 (E03 is unmonitored, so not in the scope). One file
    // covers E02+E03. E01 is missing.
    let subject = AdmissionSubject::new(
        episodes(&["ep-01", "ep-02"]),
        [tiered("file-a", Some(0), 900, &["ep-02", "ep-03"])],
    )
    .per_member();

    let verdict = evaluate_admission(&subject, CandidateFacts::new(Some(0), 0, 100), &auto(200));
    assert!(
        verdict.is_admitted(),
        "E01 is missing, so the pack has something to fill: {verdict:?}"
    );
}

/// The bar the cooldown compares against is the best *file*, judged the way the
/// gate judges files: tier first, then score. `best_score` sorts on the number
/// alone, so in this scope it names the 1080p file — and a 1080p candidate would
/// then read as a tier upgrade against a 2160p library.
#[test]
fn the_best_incumbent_is_chosen_by_tier_before_score() {
    let subject = AdmissionSubject::new(
        episodes(&["ep-01"]),
        [
            tiered("file-4k", Some(0), 100, &["ep-01"]),
            tiered("file-1080", Some(1), 900, &["ep-01"]),
        ],
    );

    assert_eq!(subject.best_score(), Some(900));
    assert_eq!(subject.best_incumbent(), Some((Some(0), 100)));
}

// ── Revision: between tier and score (D9) ─────────────────────────────────

/// A revision-stamped incumbent, at the same tier as everything else in this
/// section so the comparison really is about the revision.
fn revised(id: &str, revision: i32, score: i32) -> (Incumbent, bool) {
    let (mut incumbent, primary) = incumbent(id, score, &["ep-01"]);
    incumbent.revision = revision;
    (incumbent, primary)
}

/// Sonarr's `IsRevisionUpgrade`: same quality, higher revision, so the score
/// never gets a say. A re-encode that fixes a sync fault can easily score a
/// little *worse* than the broken original — a churn threshold would refuse
/// exactly the fix the PROPER exists to deliver.
#[test]
fn a_proper_admits_over_the_same_tier_even_when_it_scores_lower() {
    let subject = AdmissionSubject::new(episodes(&["ep-01"]), [revised("file-1", 0, 900)]);

    let verdict = evaluate_admission(
        &subject,
        CandidateFacts::new(Some(0), 1, 400).with_revision_policy(Some("group"), false),
        &auto(200),
    );
    assert!(
        verdict.is_admitted(),
        "a PROPER of what is on disk is not a trimming: {verdict:?}"
    );
}

#[test]
fn cross_group_and_unknown_group_propers_use_score_in_both_directions() {
    for group in [None, Some(""), Some("  "), Some("other")] {
        for (candidate_revision, incumbent_revision) in [(1, 0), (0, 1)] {
            let (mut file, primary) = revised("file-1", incumbent_revision, 900);
            file.release_group = group.map(str::to_string);
            let subject = AdmissionSubject::new(episodes(&["ep-01"]), [(file, primary)]);
            for (score, admitted) in [(400, false), (1_200, true)] {
                let verdict = evaluate_admission(
                    &subject,
                    CandidateFacts::new(Some(0), candidate_revision, score)
                        .with_revision_policy(Some("group"), false),
                    &auto(200),
                );
                assert_eq!(
                    verdict.is_admitted(),
                    admitted,
                    "group={group:?}, candidate revision={candidate_revision}, score={score}: {verdict:?}"
                );
            }
        }
    }
}

/// …and the converse. The original facing the PROPER already on disk is not an
/// upgrade at any score, and it reports the revision rather than a misleading
/// "equal or better score".
#[test]
fn the_original_is_refused_against_a_proper_on_disk() {
    let subject = AdmissionSubject::new(episodes(&["ep-01"]), [revised("file-1", 1, 100)]);

    let verdict = evaluate_admission(
        &subject,
        CandidateFacts::new(Some(0), 0, 9_000).with_revision_policy(Some("group"), false),
        &auto(1),
    );
    assert!(
        matches!(
            verdict.rejection().map(|r| &r.reason),
            Some(AdmissionRejectionReason::LowerRevision {
                incumbent_revision: 1,
                candidate_revision: 0,
            })
        ),
        "score bought its way past a revision: {verdict:?}"
    );
}

/// Tier still wins. A PROPER of a *worse* quality is still a downgrade — the
/// ladder is tier, then revision, then score, and no step reaches past the one
/// above it (I3).
#[test]
fn a_proper_of_a_lower_tier_is_still_a_downgrade() {
    let subject = AdmissionSubject::new(
        episodes(&["ep-01"]),
        [tiered("file-1", Some(0), 100, &["ep-01"])],
    );

    let verdict = evaluate_admission(
        &subject,
        CandidateFacts::new(Some(1), 2, 9_000).with_revision_policy(Some("group"), false),
        &auto(1),
    );
    assert!(matches!(
        verdict.rejection().map(|r| &r.reason),
        Some(AdmissionRejectionReason::LowerQualityTier)
    ));
}

/// Equal revision falls through to the score, unchanged. Two PROPERs of the
/// same quality are judged the way two plain releases would be.
#[test]
fn an_equal_revision_leaves_the_score_in_charge() {
    let subject = AdmissionSubject::new(episodes(&["ep-01"]), [revised("file-1", 1, 900)]);

    assert!(
        evaluate_admission(
            &subject,
            CandidateFacts::new(Some(0), 1, 1_200).with_revision_policy(Some("group"), false),
            &auto(200)
        )
        .is_admitted()
    );
    let verdict = evaluate_admission(
        &subject,
        CandidateFacts::new(Some(0), 1, 1_000).with_revision_policy(Some("group"), false),
        &auto(200),
    );
    assert!(matches!(
        verdict.rejection().map(|r| &r.reason),
        Some(AdmissionRejectionReason::NotAnUpgrade { .. })
    ));
}

/// A revision upgrade admits **regardless of `allow_upgrades`** — Sonarr returns
/// on the revision before it consults `UpgradeAllowed`. "Do not upgrade" means
/// "do not chase better quality"; it does not mean "keep the broken copy".
/// A *lower* revision under the same profile is refused as `UpgradesDisabled`,
/// which is the same order Sonarr uses.
#[test]
fn a_revision_upgrade_is_not_blocked_by_a_no_upgrade_profile() {
    let no_upgrades = AdmissionPolicy {
        allow_upgrades: false,
        ..auto(200)
    };
    let subject = AdmissionSubject::new(episodes(&["ep-01"]), [revised("file-1", 0, 900)]);

    assert!(
        evaluate_admission(
            &subject,
            CandidateFacts::new(Some(0), 1, 400).with_revision_policy(Some("group"), false),
            &no_upgrades
        )
        .is_admitted()
    );

    let on_disk_is_the_proper =
        AdmissionSubject::new(episodes(&["ep-01"]), [revised("file-1", 1, 900)]);
    let verdict = evaluate_admission(
        &on_disk_is_the_proper,
        CandidateFacts::new(Some(0), 0, 9_000).with_revision_policy(Some("group"), false),
        &no_upgrades,
    );
    assert!(matches!(
        verdict.rejection().map(|r| &r.reason),
        Some(AdmissionRejectionReason::UpgradesDisabled)
    ));
}

/// The pack gate runs the same ladder per member: a PROPER improves the member
/// it re-releases even though its score does not clear the churn threshold.
#[test]
fn a_pack_member_is_improvable_by_a_revision_alone() {
    let mut occupied = incumbent("file-1", 900, &["ep-01"]);
    occupied.0.revision = 0;
    let mut second = incumbent("file-2", 900, &["ep-02"]);
    second.0.revision = 1;

    let subject =
        AdmissionSubject::new(episodes(&["ep-01", "ep-02"]), [occupied, second]).per_member();

    // Revision 1 beats file-1 (revision 0) and ties file-2, so exactly one
    // member is improvable — which is reason enough to fetch the pack.
    let verdict = evaluate_admission(
        &subject,
        CandidateFacts::new(Some(0), 1, 500).with_revision_policy(Some("group"), false),
        &auto(200),
    );
    assert_eq!(verdict.superseded(), ["file-1"]);
}

/// A pack whose members are all at a *later* revision than the candidate has
/// nothing to improve, and no score rescues it.
#[test]
fn a_pack_of_originals_does_not_displace_propers() {
    let mut first = incumbent("file-1", 100, &["ep-01"]);
    first.0.revision = 1;
    let mut second = incumbent("file-2", 100, &["ep-02"]);
    second.0.revision = 1;

    let subject =
        AdmissionSubject::new(episodes(&["ep-01", "ep-02"]), [first, second]).per_member();

    let verdict = evaluate_admission(
        &subject,
        CandidateFacts::new(Some(0), 0, 9_000).with_revision_policy(Some("group"), false),
        &auto(1),
    );
    assert!(
        !verdict.is_admitted(),
        "every member already holds the later revision: {verdict:?}"
    );
}

// ── m11: a manual pack grab skips the score comparison, not the structure ──

/// An operator asking for a specific pack by hand gets it, whatever the scores
/// say — the same exemption `evaluate_admission` gives a manual replacement.
#[test]
fn a_manual_pack_grab_bypasses_the_score_comparison() {
    let subject = AdmissionSubject::new(
        episodes(&["ep-01", "ep-02"]),
        [
            incumbent("file-1", 30_000, &["ep-01"]),
            incumbent("file-2", 30_000, &["ep-02"]),
        ],
    )
    .per_member();

    // Automatic: every member is held by a far better file, so there is nothing
    // to fetch.
    assert!(
        !evaluate_admission(
            &subject,
            CandidateFacts::new(Some(0), 0, 10).with_revision_policy(Some("group"), false),
            &auto(1)
        )
        .is_admitted()
    );

    let verdict = evaluate_admission(
        &subject,
        CandidateFacts::new(Some(0), 0, 10).with_revision_policy(Some("group"), false),
        &AdmissionPolicy::manual(),
    );
    assert!(
        verdict.is_admitted(),
        "operator intent is not a score: {verdict:?}"
    );
    // Subject order (best-first, ties broken by id descending), not scope order.
    assert_eq!(verdict.superseded(), ["file-2", "file-1"]);
}

/// …but `SeasonIncomplete` is structural, not a preference. A pack cannot
/// contain an episode that has not aired, however it was asked for, so the
/// manual override does not reach it.
#[test]
fn a_manual_pack_grab_still_cannot_fetch_an_unaired_season() {
    let subject = AdmissionSubject::new(episodes(&["ep-01", "ep-02"]), [])
        .per_member()
        .with_unaired_members(1);

    let verdict = evaluate_admission(
        &subject,
        CandidateFacts::new(Some(0), 0, 900).with_revision_policy(Some("group"), false),
        &AdmissionPolicy::manual(),
    );
    assert!(matches!(
        verdict.rejection().map(|r| &r.reason),
        Some(AdmissionRejectionReason::SeasonIncomplete)
    ));
}

// ── D18: what is already downloading is an incumbent too ──────────────────

fn queued(title: &str, tier_index: Option<usize>, revision: i32, score: i32) -> QueuedRelease {
    QueuedRelease {
        missing_history: false,
        history_cutoff_reached: false,
        release_group: Some("group".into()),
        title: title.to_string(),
        covers: Vec::new(),
        tier_index,
        revision,
        score,
        language: Default::default(),
        release_key: crate::admission::release_key(title),
    }
}

/// The grab policy, with the queue dimension the lanes give it.
fn grab_policy_with_queue(min_delta: i32) -> AdmissionPolicy {
    AdmissionPolicy {
        applies_to_queue: true,
        ..auto(min_delta)
    }
}

#[test]
fn missing_grab_history_honors_cutoff_without_changing_live_queue_policy() {
    for at_cutoff in [false, true] {
        let mut history = queued("Show.S01E01.1080p.WEB-DL-GRP", Some(1), 0, 900);
        history.history_cutoff_reached = at_cutoff;
        let subject = AdmissionSubject::new(AdmissionScope::Title, []).with_queued(vec![history]);
        let policy = grab_policy_with_queue(10);
        for (tier, revision, score, allowed) in [
            (1, 0, 900, false),
            (2, 0, 2000, false),
            (0, 0, 900, !at_cutoff),
            (1, 0, 1000, !at_cutoff),
            (1, 1, 900, true),
        ] {
            let candidate = CandidateFacts::new(Some(tier), revision, score)
                .with_revision_policy(Some("group"), false);
            assert_eq!(
                evaluate_admission(&subject, candidate, &policy)
                    .rejection()
                    .is_none(),
                allowed,
                "cutoff={at_cutoff}, tier={tier}, revision={revision}, score={score}"
            );
        }
    }
}

#[test]
fn missing_grab_history_at_cutoff_still_yields_to_a_language_step() {
    let cases = [
        // (held, candidate, tier, language ahead of the tier, allowed)
        (
            AudioLanguageRank::for_test(1, 0),
            AudioLanguageRank::for_test(0, 0),
            1,
            false,
            true,
        ),
        (
            AudioLanguageRank::for_test(0, 0),
            AudioLanguageRank::for_test(0, 1),
            1,
            false,
            true,
        ),
        (
            AudioLanguageRank::for_test(1, 0),
            AudioLanguageRank::for_test(0, 0),
            2,
            true,
            true,
        ),
        // With the tier ahead, the language step still cannot rescue a lower tier.
        (
            AudioLanguageRank::for_test(1, 0),
            AudioLanguageRank::for_test(0, 0),
            2,
            false,
            false,
        ),
        (
            AudioLanguageRank::for_test(0, 0),
            AudioLanguageRank::for_test(0, 0),
            0,
            false,
            false,
        ),
        (
            AudioLanguageRank::for_test(0, 0),
            AudioLanguageRank::for_test(1, 0),
            0,
            false,
            false,
        ),
    ];
    for (held, wanted, tier, language_ahead, allowed) in cases {
        let mut history = queued("Show.S01E01.1080p.WEB-DL-GRP", Some(1), 0, 900);
        history.history_cutoff_reached = true;
        history.language = held;
        let subject = AdmissionSubject::new(AdmissionScope::Title, []).with_queued(vec![history]);
        let candidate = CandidateFacts::new(Some(tier), 0, 900)
            .with_revision_policy(Some("group"), false)
            .with_language(wanted)
            .with_pipeline(if language_ahead {
                language_ahead_of_tier()
            } else {
                DecisionPipeline::default_order()
            });
        assert_eq!(
            evaluate_admission(&subject, candidate, &grab_policy_with_queue(10))
                .rejection()
                .is_none(),
            allowed,
            "held={held:?}, candidate={wanted:?}, tier={tier}, language_ahead={language_ahead}"
        );
    }
}

/// Sonarr's `QueueSpecification`: a queued release is compared exactly like a
/// file on disk. An equal one refuses, and the reason names it.
#[test]
fn an_equal_queued_release_refuses_the_candidate() {
    let subject = AdmissionSubject::new(episodes(&["ep-01"]), []).with_queued(vec![queued(
        "Show.S01E01.1080p.WEB-DL-GRP",
        Some(1),
        0,
        900,
    )]);

    let verdict = evaluate_admission(
        &subject,
        CandidateFacts::new(Some(1), 0, 900),
        &grab_policy_with_queue(200),
    );
    assert!(
        matches!(
            verdict.rejection().map(|rejection| &rejection.reason),
            Some(AdmissionRejectionReason::QueuedEqualOrBetter {
                queued_score: 900,
                ..
            })
        ),
        "an unoccupied scope with an equal release in flight must not fetch a second: {verdict:?}"
    );
}

/// The Desert Warrior loop: the very release already grabbed for a scope came
/// back from the feed every sync and was fetched again — 79 copies of one
/// BluRay. Whatever the scores say (a re-scored candidate can read *higher*
/// than the copy in flight), the same release is never fetched twice.
#[test]
fn the_release_already_grabbed_is_never_fetched_again() {
    let release = "Desert.Warrior.2025.1080p.BluRay.DD+5.1.x264-SPHD";
    let subject = AdmissionSubject::new(AdmissionScope::Title, []).with_queued(vec![queued(
        release,
        Some(1),
        0,
        900,
    )]);
    let policy = grab_policy_with_queue(0);

    for score in [900, 1_500, 9_000] {
        let verdict = evaluate_admission(
            &subject,
            CandidateFacts::new(Some(1), 0, score).with_release_title(release),
            &policy,
        );
        assert!(
            matches!(
                verdict.rejection().map(|rejection| &rejection.reason),
                Some(AdmissionRejectionReason::QueuedSameRelease { queued_title }) if queued_title == release
            ),
            "score {score}: the grabbed release must be refused by name: {verdict:?}"
        );
    }

    // Name matching is separator- and case-insensitive: the client echoes the
    // job under its own spelling.
    let verdict = evaluate_admission(
        &subject,
        CandidateFacts::new(Some(0), 0, 9_000)
            .with_release_title("desert warrior 2025 1080p bluray dd+5 1 x264 sphd"),
        &policy,
    );
    assert!(
        !verdict.is_admitted(),
        "a re-spelt copy of the grabbed release is still the same release: {verdict:?}"
    );

    // A genuinely different, better release still gets through.
    let verdict = evaluate_admission(
        &subject,
        CandidateFacts::new(Some(0), 0, 9_000)
            .with_release_title("Desert.Warrior.2025.2160p.UHD.BluRay.x265-GRP"),
        &policy,
    );
    assert!(
        verdict.is_admitted(),
        "a better release is not blocked by the same-release guard: {verdict:?}"
    );
}

/// A zero churn threshold used to let an equal score *win* over a queued
/// release (`0 >= 0`), so an identical release under another name was
/// fetched beside the first. A tie is never an upgrade.
#[test]
fn an_equal_score_never_wins_over_a_queued_release_even_with_no_churn_threshold() {
    let subject = AdmissionSubject::new(AdmissionScope::Title, []).with_queued(vec![queued(
        "Movie.2025.1080p.BluRay-A",
        Some(1),
        0,
        900,
    )]);
    let policy = grab_policy_with_queue(0);

    let verdict = evaluate_admission(
        &subject,
        CandidateFacts::new(Some(1), 0, 900).with_release_title("Movie.2025.1080p.BluRay-B"),
        &policy,
    );
    assert!(
        matches!(
            verdict.rejection().map(|rejection| &rejection.reason),
            Some(AdmissionRejectionReason::QueuedEqualOrBetter { .. })
        ),
        "an equal release must not be fetched beside the queued one: {verdict:?}"
    );

    let verdict = evaluate_admission(
        &subject,
        CandidateFacts::new(Some(1), 0, 901).with_release_title("Movie.2025.1080p.BluRay-B"),
        &policy,
    );
    assert!(
        verdict.is_admitted(),
        "with no churn threshold any strict improvement still wins: {verdict:?}"
    );
}

#[test]
fn release_keys_fold_case_and_separators_and_ignore_empty_names() {
    use crate::admission::release_key;
    assert_eq!(
        release_key("Desert.Warrior.2025.1080p.BluRay.DD+5.1.x264-SPHD"),
        release_key("desert warrior 2025 1080p bluray dd+5 1 x264 sphd")
    );
    assert_ne!(
        release_key("Desert.Warrior.2025.1080p.BluRay-SPHD"),
        release_key("Desert.Warrior.2025.2160p.BluRay-SPHD")
    );
    assert_eq!(release_key("   "), None);
    assert_eq!(release_key(""), None);
}

/// …and a worse one, and a same-tier improvement that does not clear the churn
/// threshold (Sonarr applies `MinUpgradeFormatScore` inside the queue spec too).
#[test]
fn a_worse_or_marginal_candidate_is_refused_against_the_queue() {
    let subject = AdmissionSubject::new(episodes(&["ep-01"]), []).with_queued(vec![queued(
        "Show.S01E01.1080p.WEB-DL-GRP",
        Some(1),
        0,
        900,
    )]);
    let policy = grab_policy_with_queue(200);

    for (tier, revision, score) in [(Some(2), 0, 9_000), (Some(1), 0, 500), (Some(1), 0, 1_050)] {
        let verdict = evaluate_admission(
            &subject,
            CandidateFacts::new(tier, revision, score),
            &policy,
        );
        assert!(
            !verdict.is_admitted(),
            "tier {tier:?} revision {revision} score {score} should not join the queue: {verdict:?}"
        );
    }
}

/// Sonarr's `QueueUpgradesNotAllowed`: a candidate that would only be an
/// *upgrade* over the queued release — a better tier or a better score — is not
/// fetched beside it when the profile forbids upgrades. A revision upgrade still
/// is, exactly as it is over a file on disk (D9).
#[test]
fn a_no_upgrade_profile_does_not_fetch_an_upgrade_beside_the_queue() {
    let subject = AdmissionSubject::new(episodes(&["ep-01"]), []).with_queued(vec![queued(
        "Show.S01E01.1080p.WEB-DL-GRP",
        Some(1),
        0,
        900,
    )]);
    let policy = AdmissionPolicy {
        allow_upgrades: false,
        ..grab_policy_with_queue(200)
    };

    for (tier, revision, score) in [(Some(0), 0, 900), (Some(1), 0, 2_000)] {
        let verdict = evaluate_admission(
            &subject,
            CandidateFacts::new(tier, revision, score),
            &policy,
        );
        assert!(
            matches!(
                verdict.rejection().map(|rejection| &rejection.reason),
                Some(AdmissionRejectionReason::UpgradesDisabled)
            ),
            "tier {tier:?} score {score} is an upgrade the profile forbids: {verdict:?}"
        );
    }

    let proper = evaluate_admission(
        &subject,
        CandidateFacts::new(Some(1), 1, 100).with_revision_policy(Some("group"), false),
        &policy,
    );
    assert!(
        proper.is_admitted(),
        "a PROPER is a fix, not an upgrade: {proper:?}"
    );
}

/// The whole point: a genuinely better release still gets grabbed. The old
/// scope-level "something is in flight, skip" could not tell this from a
/// duplicate.
#[test]
fn a_better_candidate_is_admitted_over_a_queued_release() {
    let subject = AdmissionSubject::new(episodes(&["ep-01"]), []).with_queued(vec![queued(
        "Show.S01E01.1080p.WEB-DL-GRP",
        Some(1),
        0,
        900,
    )]);
    let policy = grab_policy_with_queue(200);

    // A better tier.
    assert!(
        evaluate_admission(&subject, CandidateFacts::new(Some(0), 0, 100), &policy).is_admitted()
    );
    // A PROPER of the same tier.
    assert!(
        evaluate_admission(
            &subject,
            CandidateFacts::new(Some(1), 1, 100).with_revision_policy(Some("group"), false),
            &policy
        )
        .is_admitted()
    );
    // A same-tier improvement past the churn threshold.
    assert!(
        evaluate_admission(&subject, CandidateFacts::new(Some(1), 0, 1_100), &policy).is_admitted()
    );
}

/// **I4.** Import never re-litigates the queue: the bytes are already on disk,
/// and refusing them because something else is still downloading would discard
/// a finished download for one that may never finish. Both import policies leave
/// `applies_to_queue` false, so the same subject admits.
#[test]
fn import_never_consults_the_queue() {
    let subject = AdmissionSubject::new(episodes(&["ep-01"]), []).with_queued(vec![queued(
        "Show.S01E01.2160p.WEB-DL-GRP",
        Some(0),
        2,
        9_000,
    )]);

    assert!(
        evaluate_admission(
            &subject,
            CandidateFacts::new(Some(2), 0, 10),
            &AdmissionPolicy::not_a_downgrade()
        )
        .is_admitted()
    );
    assert!(
        evaluate_admission(
            &subject,
            CandidateFacts::new(Some(2), 0, 10),
            &AdmissionPolicy::manual()
        )
        .is_admitted()
    );
}

/// An operator's own grab is not held back by the queue either.
#[test]
fn a_manual_grab_ignores_the_queue() {
    let subject = AdmissionSubject::new(episodes(&["ep-01"]), []).with_queued(vec![queued(
        "Show.S01E01.2160p.WEB-DL-GRP",
        Some(0),
        0,
        9_000,
    )]);
    let policy = AdmissionPolicy {
        applies_to_queue: true,
        ..AdmissionPolicy::manual()
    };

    assert!(
        evaluate_admission(&subject, CandidateFacts::new(Some(2), 0, 10), &policy).is_admitted()
    );
}

/// A pack whose every member is already covered by an equal queued release is
/// not fetched twice.
#[test]
fn a_pack_fully_covered_by_queued_equal_or_better_releases_is_refused() {
    let mut queued_pack = queued("Show.S01.1080p.WEB-DL-GRP", Some(1), 0, 900);
    queued_pack.covers = vec!["ep-01".to_string(), "ep-02".to_string()];
    let subject = AdmissionSubject::new(episodes(&["ep-01", "ep-02"]), [])
        .per_member()
        .with_queued(vec![queued_pack]);

    let verdict = evaluate_admission(
        &subject,
        CandidateFacts::new(Some(1), 0, 900),
        &grab_policy_with_queue(200),
    );
    assert!(matches!(
        verdict.rejection().map(|rejection| &rejection.reason),
        Some(AdmissionRejectionReason::QueuedEqualOrBetter { .. })
    ));
}

/// A pack's recorded claim remains a duplicate guard even if score
/// re-derivation makes the feed candidate look better. The single-scope gate
/// checks release identity before its score ladder; AnyMember must do the same
/// before treating the still-queued members as missing.
#[test]
fn a_per_member_pack_does_not_regrab_its_own_higher_rescored_release() {
    let release = "Show.S01.1080p.WEB-DL-GRP";
    let mut queued_pack = queued(release, Some(1), 0, 900);
    queued_pack.covers = vec!["ep-01".to_string(), "ep-02".to_string()];
    let subject = AdmissionSubject::new(episodes(&["ep-01", "ep-02"]), [])
        .per_member()
        .with_queued(vec![queued_pack]);

    let verdict = evaluate_admission(
        &subject,
        CandidateFacts::new(Some(1), 0, 9_000).with_release_title(release),
        &grab_policy_with_queue(200),
    );

    assert!(
        matches!(
            verdict.rejection().map(|rejection| &rejection.reason),
            Some(AdmissionRejectionReason::QueuedSameRelease { queued_title }) if queued_title == release
        ),
        "a rescored copy of a grabbed pack must still be rejected: {verdict:?}"
    );
}

#[test]
fn a_queued_single_episode_does_not_refuse_a_pack_that_fills_other_missing_members() {
    let mut queued_episode = queued("Show.S01E01.1080p.WEB-DL-GRP", Some(1), 0, 900);
    queued_episode.covers = vec!["ep-01".to_string()];
    let subject = AdmissionSubject::new(episodes(&["ep-01", "ep-02"]), [])
        .per_member()
        .with_queued(vec![queued_episode]);

    let verdict = evaluate_admission(
        &subject,
        CandidateFacts::new(Some(1), 0, 900),
        &grab_policy_with_queue(200),
    );

    assert!(
        verdict.is_admitted(),
        "the pack still fills ep-02, so it must be admitted: {verdict:?}"
    );
}

/// **N3.** A pack every member of which is held by a better *quality* file is
/// refused on the tier, not on the score.
///
/// It used to report `NotAnUpgrade` with a `required_delta` — a threshold an
/// operator might go and tune, when the honest answer is that no score crosses
/// a tier (I3).
#[test]
fn a_pack_below_every_members_tier_is_refused_on_the_tier() {
    let subject = AdmissionSubject::new(
        episodes(&["ep-01", "ep-02"]),
        [
            tiered("file-1", Some(0), 100, &["ep-01"]),
            tiered("file-2", Some(0), 100, &["ep-02"]),
        ],
    )
    .per_member();

    let verdict = evaluate_admission(&subject, CandidateFacts::new(Some(2), 0, 9_000), &auto(200));
    assert!(
        matches!(
            verdict.rejection().map(|rejection| &rejection.reason),
            Some(AdmissionRejectionReason::LowerQualityTier)
        ),
        "score bought its way down a tier in the pack gate: {verdict:?}"
    );

    // A same-tier candidate that merely does not clear the churn threshold is
    // still `NotAnUpgrade`, so the tier arm has not swallowed the score case.
    let same_tier = AdmissionSubject::new(
        episodes(&["ep-01", "ep-02"]),
        [
            tiered("file-1", Some(0), 900, &["ep-01"]),
            tiered("file-2", Some(0), 900, &["ep-02"]),
        ],
    )
    .per_member();
    let verdict = evaluate_admission(&same_tier, CandidateFacts::new(Some(0), 0, 950), &auto(200));
    assert!(matches!(
        verdict.rejection().map(|rejection| &rejection.reason),
        Some(AdmissionRejectionReason::NotAnUpgrade { .. })
    ));

    // …and the format cutoff still reports itself when it is what fired.
    let policy = AdmissionPolicy {
        cutoff_score: Some(500),
        ..auto(200)
    };
    let verdict = evaluate_admission(&same_tier, CandidateFacts::new(Some(0), 0, 9_000), &policy);
    assert!(matches!(
        verdict.rejection().map(|rejection| &rejection.reason),
        Some(AdmissionRejectionReason::FormatCutoffReached { .. })
    ));
}

// ── Audio language preferences: preferred rank and optional hits ───────────

fn spoken(
    id: &str,
    tier_index: Option<usize>,
    score: i32,
    language: crate::AudioLanguageRank,
) -> (Incumbent, bool) {
    let (mut incumbent, primary) = incumbent(id, score, &[]);
    incumbent.tier_index = tier_index;
    incumbent.language = language;
    (incumbent, primary)
}

fn title_scope(file: (Incumbent, bool)) -> AdmissionSubject {
    AdmissionSubject::new(AdmissionScope::Title, [file])
}

#[test]
fn a_language_ahead_of_the_tier_replaces_a_better_tier_without_it() {
    // English 2160p on disk, German 1080p offered, German preferred.
    let subject = title_scope(spoken(
        "english-2160p",
        Some(0),
        900,
        AudioLanguageRank::for_test(1, 0),
    ));
    let candidate = CandidateFacts::new(Some(1), 0, 100)
        .with_language(AudioLanguageRank::for_test(0, 0))
        .with_pipeline(language_ahead_of_tier());
    assert!(evaluate_admission(&subject, candidate, &grab_policy()).is_admitted());
}

#[test]
fn a_language_ahead_of_the_tier_keeps_it_against_a_better_tier() {
    let subject = title_scope(spoken(
        "german-1080p",
        Some(1),
        100,
        AudioLanguageRank::for_test(0, 0),
    ));
    let candidate = CandidateFacts::new(Some(0), 0, 9_000)
        .with_language(AudioLanguageRank::for_test(1, 1))
        .with_pipeline(language_ahead_of_tier());
    let verdict = evaluate_admission(&subject, candidate, &grab_policy());
    assert!(matches!(
        verdict.rejection().map(|r| &r.reason),
        Some(AdmissionRejectionReason::LowerLanguagePreference {
            incumbent_preferred_rank: 0,
            candidate_preferred_rank: 1,
        })
    ));
}

#[test]
fn the_default_order_keeps_the_tier_ahead_of_the_preferred_language() {
    let subject = title_scope(spoken(
        "english-2160p",
        Some(0),
        900,
        AudioLanguageRank::for_test(1, 0),
    ));
    let candidate =
        CandidateFacts::new(Some(1), 0, 100).with_language(AudioLanguageRank::for_test(0, 0));
    let verdict = evaluate_admission(&subject, candidate, &grab_policy());
    assert!(matches!(
        verdict.rejection().map(|r| &r.reason),
        Some(AdmissionRejectionReason::LowerQualityTier)
    ));
}

#[test]
fn the_default_order_upgrades_to_the_preferred_language_within_a_tier() {
    // Same tier and revision: the preferred language wins over any score.
    let subject = title_scope(spoken(
        "english-1080p",
        Some(1),
        9_000,
        AudioLanguageRank::for_test(1, 0),
    ));
    let candidate =
        CandidateFacts::new(Some(1), 0, 100).with_language(AudioLanguageRank::for_test(0, 0));
    assert!(evaluate_admission(&subject, candidate, &grab_policy()).is_admitted());
}

#[test]
fn a_language_upgrade_ignores_the_format_cutoff_and_the_churn_threshold() {
    let subject = title_scope(spoken(
        "english-1080p",
        Some(1),
        9_000,
        AudioLanguageRank::for_test(1, 0),
    ));
    let policy = AdmissionPolicy {
        cutoff_score: Some(500),
        ..auto(5_000)
    };
    let candidate =
        CandidateFacts::new(Some(1), 0, 100).with_language(AudioLanguageRank::for_test(0, 0));
    assert!(evaluate_admission(&subject, candidate, &policy).is_admitted());
}

#[test]
fn more_optional_languages_upgrade_after_tier_and_revision() {
    let subject = title_scope(spoken(
        "german-only",
        Some(0),
        9_000,
        AudioLanguageRank::for_test(0, 0),
    ));
    let dual = CandidateFacts::new(Some(0), 0, 100)
        .with_language(AudioLanguageRank::for_test(0, 1))
        .with_pipeline(language_ahead_of_tier());
    assert!(evaluate_admission(&subject, dual, &grab_policy()).is_admitted());

    let worse_tier_dual = CandidateFacts::new(Some(1), 0, 100)
        .with_language(AudioLanguageRank::for_test(0, 1))
        .with_pipeline(language_ahead_of_tier());
    assert!(matches!(
        evaluate_admission(&subject, worse_tier_dual, &grab_policy())
            .rejection()
            .map(|r| &r.reason),
        Some(AdmissionRejectionReason::LowerQualityTier)
    ));
}

#[test]
fn fewer_optional_languages_are_refused_before_the_score() {
    let subject = title_scope(spoken(
        "dual",
        Some(0),
        100,
        AudioLanguageRank::for_test(0, 1),
    ));
    let single = CandidateFacts::new(Some(0), 0, 9_000)
        .with_language(AudioLanguageRank::for_test(0, 0))
        .with_pipeline(language_ahead_of_tier());
    assert!(matches!(
        evaluate_admission(&subject, single, &grab_policy())
            .rejection()
            .map(|r| &r.reason),
        Some(AdmissionRejectionReason::FewerOptionalLanguages {
            incumbent_optional_hits: 1,
            candidate_optional_hits: 0,
        })
    ));
}

#[test]
fn a_proper_still_beats_more_optional_languages() {
    let (mut held, primary) = spoken("dual", Some(0), 100, AudioLanguageRank::for_test(0, 1));
    held.revision = 0;
    held.release_group = Some("group".into());
    let subject = title_scope((held, primary));
    let proper = CandidateFacts::new(Some(0), 1, 100)
        .with_revision_policy(Some("group"), false)
        .with_language(AudioLanguageRank::for_test(0, 0))
        .with_pipeline(language_ahead_of_tier());
    assert!(evaluate_admission(&subject, proper, &grab_policy()).is_admitted());
}

#[test]
fn an_unrelated_proper_does_not_beat_more_optional_languages() {
    let (mut held, primary) = spoken("dual", Some(0), 100, AudioLanguageRank::for_test(0, 1));
    held.revision = 0;
    held.release_group = Some("group".into());
    let subject = title_scope((held, primary));
    let proper = CandidateFacts::new(Some(0), 1, 100)
        .with_revision_policy(Some("other"), false)
        .with_language(AudioLanguageRank::for_test(0, 0))
        .with_pipeline(language_ahead_of_tier());
    assert!(matches!(
        evaluate_admission(&subject, proper, &grab_policy())
            .rejection()
            .map(|r| &r.reason),
        Some(AdmissionRejectionReason::FewerOptionalLanguages { .. })
    ));
}

#[test]
fn a_language_upgrade_respects_a_profile_that_forbids_upgrades() {
    let subject = title_scope(spoken(
        "english",
        Some(0),
        100,
        AudioLanguageRank::for_test(1, 0),
    ));
    let policy = AdmissionPolicy {
        allow_upgrades: false,
        ..auto(0)
    };
    let candidate = CandidateFacts::new(Some(0), 0, 100)
        .with_language(AudioLanguageRank::for_test(0, 0))
        .with_pipeline(language_ahead_of_tier());
    assert!(matches!(
        evaluate_admission(&subject, candidate, &policy)
            .rejection()
            .map(|r| &r.reason),
        Some(AdmissionRejectionReason::UpgradesDisabled)
    ));
}

#[test]
fn manual_replacement_ignores_the_language_rungs() {
    let subject = title_scope(spoken(
        "german",
        Some(0),
        100,
        AudioLanguageRank::for_test(0, 1),
    ));
    let candidate = CandidateFacts::new(Some(0), 0, 100)
        .with_language(AudioLanguageRank::for_test(1, 0))
        .with_pipeline(language_ahead_of_tier());
    assert!(evaluate_admission(&subject, candidate, &AdmissionPolicy::manual()).is_admitted());
}

#[test]
fn neutral_language_facts_leave_the_ladder_unchanged() {
    let subject = title_scope(spoken(
        "held",
        Some(0),
        1_000,
        crate::AudioLanguageRank::default(),
    ));
    let lower_tier = CandidateFacts::new(Some(1), 0, 9_000);
    assert!(matches!(
        evaluate_admission(&subject, lower_tier, &grab_policy())
            .rejection()
            .map(|r| &r.reason),
        Some(AdmissionRejectionReason::LowerQualityTier)
    ));
}

#[test]
fn a_pack_improves_members_held_without_the_preferred_language() {
    let (mut english, primary) = incumbent("english-e1", 900, &["ep-1"]);
    english.language = AudioLanguageRank::for_test(1, 0);
    let subject = AdmissionSubject::new(episodes(&["ep-1"]), [(english, primary)]).per_member();
    let german_pack = CandidateFacts::new(Some(1), 0, 100)
        .with_language(AudioLanguageRank::for_test(0, 0))
        .with_pipeline(language_ahead_of_tier());
    assert!(evaluate_admission(&subject, german_pack, &grab_policy()).is_admitted());
}

#[test]
fn a_pack_is_refused_when_every_member_has_a_more_preferred_language() {
    let (mut german, primary) = incumbent("german-e1", 100, &["ep-1"]);
    german.language = AudioLanguageRank::for_test(0, 0);
    let subject = AdmissionSubject::new(episodes(&["ep-1"]), [(german, primary)]).per_member();
    let english_pack = CandidateFacts::new(Some(0), 0, 9_000)
        .with_language(AudioLanguageRank::for_test(1, 0))
        .with_pipeline(language_ahead_of_tier());
    assert!(matches!(
        evaluate_admission(&subject, english_pack, &grab_policy())
            .rejection()
            .map(|r| &r.reason),
        Some(AdmissionRejectionReason::LowerLanguagePreference { .. })
    ));
}

#[test]
fn a_queued_release_in_the_preferred_language_blocks_a_better_tier_without_it() {
    let mut german = queued("Movie.2024.German.1080p", Some(1), 0, 100);
    german.language = AudioLanguageRank::for_test(0, 0);
    let subject = AdmissionSubject::new(AdmissionScope::Title, []).with_queued(vec![german]);
    let english = CandidateFacts::new(Some(0), 0, 9_000)
        .with_language(AudioLanguageRank::for_test(1, 0))
        .with_pipeline(language_ahead_of_tier())
        .with_release_title("Movie.2024.2160p");
    assert!(matches!(
        evaluate_admission(&subject, english, &grab_policy_with_queue(0))
            .rejection()
            .map(|r| &r.reason),
        Some(AdmissionRejectionReason::QueuedEqualOrBetter { .. })
    ));

    let mut english_queued = queued("Movie.2024.2160p", Some(0), 0, 9_000);
    english_queued.language = AudioLanguageRank::for_test(1, 0);
    let subject =
        AdmissionSubject::new(AdmissionScope::Title, []).with_queued(vec![english_queued]);
    let german = CandidateFacts::new(Some(1), 0, 100)
        .with_language(AudioLanguageRank::for_test(0, 0))
        .with_pipeline(language_ahead_of_tier())
        .with_release_title("Movie.2024.German.1080p");
    assert!(evaluate_admission(&subject, german, &grab_policy_with_queue(0)).is_admitted());
}

// ── An order without the tier: the score decides across tiers ─────────────

/// The order that keeps only the revision step ahead of the score.
fn revision_then_score() -> DecisionPipeline {
    DecisionPipeline::try_from_gates([DecisionGate::Revision]).unwrap()
}

fn score_first(candidate: CandidateFacts<'_>) -> CandidateFacts<'_> {
    candidate.with_pipeline(revision_then_score())
}

fn rejection_reason(verdict: &AdmissionVerdict) -> Option<&AdmissionRejectionReason> {
    verdict.rejection().map(|rejection| &rejection.reason)
}

#[test]
fn score_first_admits_a_lower_tier_that_scores_higher() {
    let subject = title_scope(tiered("file-2160p", Some(0), 1_000, &[]));
    let candidate = score_first(CandidateFacts::new(Some(2), 0, 1_500));
    assert!(evaluate_admission(&subject, candidate, &grab_policy()).is_admitted());
    // The same facts under the default order are refused on the tier.
    assert!(matches!(
        rejection_reason(&evaluate_admission(
            &subject,
            CandidateFacts::new(Some(2), 0, 1_500),
            &grab_policy()
        )),
        Some(AdmissionRejectionReason::LowerQualityTier)
    ));
}

#[test]
fn score_first_refuses_a_higher_tier_that_scores_lower_as_not_an_upgrade() {
    let subject = title_scope(tiered("file-1080p", Some(2), 1_500, &[]));
    for policy in [grab_policy(), AdmissionPolicy::not_a_downgrade()] {
        let verdict = evaluate_admission(
            &subject,
            score_first(CandidateFacts::new(Some(0), 0, 1_000)),
            &policy,
        );
        assert!(
            matches!(
                rejection_reason(&verdict),
                Some(AdmissionRejectionReason::NotAnUpgrade { .. })
            ),
            "{verdict:?}"
        );
    }
}

#[test]
fn score_first_respects_the_churn_threshold_whatever_the_tier() {
    let subject = title_scope(tiered("file-1080p", Some(1), 1_000, &[]));
    // +199 against 200, even from a better tier: not enough.
    for tier in [Some(0), Some(1), Some(2)] {
        let verdict = evaluate_admission(
            &subject,
            score_first(CandidateFacts::new(tier, 0, 1_199)),
            &grab_policy(),
        );
        assert!(
            matches!(
                rejection_reason(&verdict),
                Some(AdmissionRejectionReason::NotAnUpgrade { .. })
            ),
            "tier {tier:?}: {verdict:?}"
        );
        // +200 clears it from any tier.
        assert!(
            evaluate_admission(
                &subject,
                score_first(CandidateFacts::new(tier, 0, 1_200)),
                &grab_policy()
            )
            .is_admitted(),
            "tier {tier:?}"
        );
    }
}

#[test]
fn score_first_import_ignores_the_tier_at_an_equal_score() {
    // Without its step the tier does not even break a tie: an equal score
    // from any tier is not a downgrade.
    let subject = title_scope(tiered("file-1080p", Some(1), 1_000, &[]));
    let import = AdmissionPolicy::not_a_downgrade();
    for tier in [Some(0), Some(1), Some(2)] {
        assert!(
            evaluate_admission(
                &subject,
                score_first(CandidateFacts::new(tier, 0, 1_000)),
                &import
            )
            .is_admitted(),
            "tier {tier:?}"
        );
    }
    assert!(matches!(
        rejection_reason(&evaluate_admission(
            &subject,
            score_first(CandidateFacts::new(Some(0), 0, 999)),
            &import
        )),
        Some(AdmissionRejectionReason::NotAnUpgrade { .. })
    ));
}

#[test]
fn score_first_keeps_the_revision_rung_ahead_of_the_score() {
    let subject = AdmissionSubject::new(episodes(&["ep-01"]), [revised("file-1", 0, 900)]);
    // A PROPER of what is on disk lands while scoring lower, even with
    // upgrades disabled.
    let proper = score_first(
        CandidateFacts::new(Some(0), 1, 400).with_revision_policy(Some("group"), false),
    );
    assert!(evaluate_admission(&subject, proper, &grab_policy()).is_admitted());
    let no_upgrades = AdmissionPolicy {
        allow_upgrades: false,
        ..grab_policy()
    };
    assert!(evaluate_admission(&subject, proper, &no_upgrades).is_admitted());

    // …and the original never displaces a PROPER, however it scores.
    let subject = AdmissionSubject::new(episodes(&["ep-01"]), [revised("file-1", 1, 400)]);
    let original = score_first(
        CandidateFacts::new(Some(0), 0, 9_000).with_revision_policy(Some("group"), false),
    );
    assert!(matches!(
        rejection_reason(&evaluate_admission(&subject, original, &grab_policy())),
        Some(AdmissionRejectionReason::LowerRevision { .. })
    ));
}

/// The preferred language, then the revision, then the score.
fn language_then_score() -> DecisionPipeline {
    DecisionPipeline::try_from_gates([DecisionGate::PreferredLanguage, DecisionGate::Revision])
        .unwrap()
}

#[test]
fn score_first_keeps_a_language_step_ahead_of_the_score() {
    let english = title_scope(spoken(
        "english",
        Some(0),
        9_000,
        AudioLanguageRank::for_test(1, 0),
    ));
    let german = CandidateFacts::new(Some(2), 0, 100)
        .with_language(AudioLanguageRank::for_test(0, 0))
        .with_pipeline(language_then_score());
    assert!(evaluate_admission(&english, german, &grab_policy()).is_admitted());

    let german_file = title_scope(spoken(
        "german",
        Some(2),
        100,
        AudioLanguageRank::for_test(0, 0),
    ));
    let english = CandidateFacts::new(Some(0), 0, 9_000)
        .with_language(AudioLanguageRank::for_test(1, 0))
        .with_pipeline(language_then_score());
    assert!(matches!(
        rejection_reason(&evaluate_admission(&german_file, english, &grab_policy())),
        Some(AdmissionRejectionReason::LowerLanguagePreference { .. })
    ));
}

#[test]
fn score_first_without_a_language_step_ignores_the_language() {
    let english = title_scope(spoken(
        "english",
        Some(1),
        1_000,
        AudioLanguageRank::for_test(1, 0),
    ));
    let german = |score| {
        score_first(
            CandidateFacts::new(Some(1), 0, score).with_language(AudioLanguageRank::for_test(0, 0)),
        )
    };
    // A lower score in the preferred language is refused on the score.
    assert!(matches!(
        rejection_reason(&evaluate_admission(&english, german(900), &grab_policy())),
        Some(AdmissionRejectionReason::NotAnUpgrade { .. })
    ));
    // Inside the churn band the language does not buy a download.
    assert!(matches!(
        rejection_reason(&evaluate_admission(&english, german(1_100), &grab_policy())),
        Some(AdmissionRejectionReason::NotAnUpgrade { .. })
    ));
    // A real score win lands, and a worse language does not stop it.
    assert!(evaluate_admission(&english, german(1_200), &grab_policy()).is_admitted());
    let german_file = title_scope(spoken(
        "german",
        Some(1),
        1_000,
        AudioLanguageRank::for_test(0, 0),
    ));
    let english_high = score_first(
        CandidateFacts::new(Some(1), 0, 1_200).with_language(AudioLanguageRank::for_test(1, 0)),
    );
    assert!(evaluate_admission(&german_file, english_high, &grab_policy()).is_admitted());
    // Without its step the language does not even break a tie: an equal
    // score imports in either language.
    let import = AdmissionPolicy::not_a_downgrade();
    assert!(evaluate_admission(&english, german(1_000), &import).is_admitted());
    let english_equal = score_first(
        CandidateFacts::new(Some(1), 0, 1_000).with_language(AudioLanguageRank::for_test(1, 0)),
    );
    assert!(evaluate_admission(&german_file, english_equal, &import).is_admitted());
}

#[test]
fn score_first_refuses_a_score_win_when_upgrades_are_disabled() {
    let subject = title_scope(tiered("file", Some(1), 1_000, &[]));
    let policy = AdmissionPolicy {
        allow_upgrades: false,
        ..grab_policy()
    };
    for tier in [Some(0), Some(2)] {
        assert!(matches!(
            rejection_reason(&evaluate_admission(
                &subject,
                score_first(CandidateFacts::new(tier, 0, 9_000)),
                &policy
            )),
            Some(AdmissionRejectionReason::UpgradesDisabled)
        ));
    }
}

#[test]
fn score_first_stops_at_the_format_cutoff_score() {
    let policy = AdmissionPolicy {
        cutoff_score: Some(1_000),
        ..grab_policy()
    };
    let at_cutoff = title_scope(tiered("file", Some(2), 1_000, &[]));
    for tier in [Some(0), Some(2)] {
        assert!(matches!(
            rejection_reason(&evaluate_admission(
                &at_cutoff,
                score_first(CandidateFacts::new(tier, 0, 9_000)),
                &policy
            )),
            Some(AdmissionRejectionReason::FormatCutoffReached { .. })
        ));
    }
    let below_cutoff = title_scope(tiered("file", Some(2), 700, &[]));
    assert!(
        evaluate_admission(
            &below_cutoff,
            score_first(CandidateFacts::new(Some(2), 0, 900)),
            &policy
        )
        .is_admitted()
    );
}

#[test]
fn score_first_queue_compares_score_before_tier() {
    let queued_2160p = queued("Movie.2024.2160p", Some(0), 0, 1_000);
    let subject = AdmissionSubject::new(AdmissionScope::Title, []).with_queued(vec![queued_2160p]);
    let lower_tier_higher =
        score_first(CandidateFacts::new(Some(2), 0, 1_500).with_release_title("Movie.2024.720p"));
    assert!(
        evaluate_admission(&subject, lower_tier_higher, &grab_policy_with_queue(200)).is_admitted()
    );
    let higher_tier_lower = score_first(
        CandidateFacts::new(Some(0), 0, 900).with_release_title("Movie.2024.2160p.Other"),
    );
    assert!(matches!(
        rejection_reason(&evaluate_admission(
            &subject,
            higher_tier_lower,
            &grab_policy_with_queue(200)
        )),
        Some(AdmissionRejectionReason::QueuedEqualOrBetter { .. })
    ));
}

#[test]
fn score_first_pack_members_compare_score_before_tier() {
    let subject = AdmissionSubject::new(
        episodes(&["ep-01", "ep-02"]),
        [
            tiered("file-1", Some(0), 1_000, &["ep-01"]),
            tiered("file-2", Some(0), 1_000, &["ep-02"]),
        ],
    )
    .per_member();
    assert!(
        evaluate_admission(
            &subject,
            score_first(CandidateFacts::new(Some(2), 0, 1_500)),
            &grab_policy()
        )
        .is_admitted(),
        "a lower-tier pack that outscores every member is worth fetching"
    );
    assert!(
        !evaluate_admission(
            &subject,
            score_first(CandidateFacts::new(Some(0), 0, 1_100)),
            &grab_policy()
        )
        .is_admitted(),
        "a lead inside the churn band improves no member"
    );
    let no_upgrades = AdmissionPolicy {
        allow_upgrades: false,
        ..grab_policy()
    };
    assert!(
        !evaluate_admission(
            &subject,
            score_first(CandidateFacts::new(Some(2), 0, 9_000)),
            &no_upgrades
        )
        .is_admitted()
    );
}

/// The search lanes walk candidates in `RankHead` order and stop at the first
/// one admission refuses, so under an order without the tier every
/// candidate ranked below a refused one must be refused too.
#[test]
fn score_first_ranking_order_matches_admission() {
    use crate::acquisition::scoring::RankHead;
    let incumbent = spoken("file", Some(1), 1_000, AudioLanguageRank::for_test(1, 0));
    let subject = title_scope(incumbent);
    let languages = [
        AudioLanguageRank::for_test(0, 0),
        AudioLanguageRank::for_test(1, 0),
        AudioLanguageRank::for_test(1, 1),
        AudioLanguageRank::for_test(2, 0),
    ];
    for policy in [grab_policy(), AdmissionPolicy::not_a_downgrade()] {
        let mut candidates = Vec::new();
        for tier in [Some(0), Some(1), Some(2)] {
            for score in [700, 1_000, 1_100, 1_199, 1_200, 1_600] {
                for language in languages {
                    candidates.push(score_first(
                        CandidateFacts::new(tier, 0, score).with_language(language),
                    ));
                }
            }
        }
        let mut ranked: Vec<_> = candidates
            .into_iter()
            .map(|candidate| {
                let head = RankHead {
                    side: candidate.facts().side,
                    negated_revision: revision_preference(candidate, &subject).saturating_neg(),
                    negated_score: candidate.score.saturating_neg(),
                    pipeline: revision_then_score(),
                    ..Default::default()
                };
                (head.key(), candidate)
            })
            .collect();
        ranked.sort_by_key(|(key, _)| *key);
        let mut refused: Option<CandidateFacts> = None;
        for (_, candidate) in ranked {
            let admitted = evaluate_admission(&subject, candidate, &policy).is_admitted();
            if let Some(earlier) = refused {
                assert!(
                    !admitted,
                    "{candidate:?} admitted after {earlier:?} was refused (min_delta {})",
                    policy.min_delta
                );
            } else if !admitted {
                refused = Some(candidate);
            }
        }
        assert!(refused.is_some(), "the fixture must reach a refusal");
    }
}

// ── Frozen oracle: the ladder as it was before orders were configurable ────

/// A reason, without its payload, so the oracle and the gate can be compared.
fn reason_label(reason: &AdmissionRejectionReason) -> &'static str {
    match reason {
        AdmissionRejectionReason::LowerQualityTier => "lower_quality_tier",
        AdmissionRejectionReason::LowerLanguagePreference { .. } => "lower_language_preference",
        AdmissionRejectionReason::FewerOptionalLanguages { .. } => "fewer_optional_languages",
        AdmissionRejectionReason::LowerRevision { .. } => "lower_revision",
        AdmissionRejectionReason::UpgradesDisabled => "upgrades_disabled",
        AdmissionRejectionReason::NotAnUpgrade { .. } => "not_an_upgrade",
        AdmissionRejectionReason::FormatCutoffReached { .. } => "format_cutoff_reached",
        _ => "other",
    }
}

/// A copy of the single-file ladder exactly as it ran before profiles chose
/// an order, kept here so the two orders it supported can be checked against
/// the pipeline. `language_first` was the library's audio language priority.
/// `None` admits.
fn frozen_ladder(
    language_first: bool,
    candidate: (Option<usize>, &str, i32, i32, AudioLanguageRank),
    incumbent: &Incumbent,
    policy: &AdmissionPolicy,
) -> Option<&'static str> {
    use std::cmp::Ordering::{Equal, Greater, Less};
    let (tier, group, revision, score, language) = candidate;
    let tier_cmp = match (tier, incumbent.tier_index) {
        (Some(l), Some(r)) => r.cmp(&l),
        (Some(_), None) => Greater,
        (None, Some(_)) => Less,
        (None, None) => Equal,
    };
    let preferred_cmp = incumbent
        .language
        .matched
        .preferred_rank
        .cmp(&language.matched.preferred_rank);
    let optional_cmp = language
        .matched
        .optional_hits
        .cmp(&incumbent.language.matched.optional_hits);
    let related = tier == incumbent.tier_index
        && incumbent
            .release_group
            .as_deref()
            .is_some_and(|held| held.eq_ignore_ascii_case(group));
    let revision_cmp = if related {
        revision.cmp(&incumbent.revision)
    } else {
        Equal
    };
    let step = |ordering: std::cmp::Ordering, lost: &'static str| match ordering {
        Less => Some(Some(lost)),
        Greater if !policy.allow_upgrades => Some(Some("upgrades_disabled")),
        Greater => Some(None),
        Equal => None,
    };
    if language_first && let Some(verdict) = step(preferred_cmp, "lower_language_preference") {
        return verdict;
    }
    if let Some(verdict) = step(tier_cmp, "lower_quality_tier") {
        return verdict;
    }
    match revision_cmp {
        Greater => return None,
        Less if !policy.allow_upgrades => return Some("upgrades_disabled"),
        Less => return Some("lower_revision"),
        Equal => {}
    }
    if !language_first && let Some(verdict) = step(preferred_cmp, "lower_language_preference") {
        return verdict;
    }
    if let Some(verdict) = step(optional_cmp, "fewer_optional_languages") {
        return verdict;
    }
    if !policy.allow_upgrades {
        return Some("upgrades_disabled");
    }
    if policy
        .cutoff_score
        .is_some_and(|cutoff| incumbent.score >= cutoff)
    {
        return Some("format_cutoff_reached");
    }
    (score.saturating_sub(incumbent.score) < policy.min_delta).then_some("not_an_upgrade")
}

/// The default order is the old quality-first ladder, and the preferred
/// language moved ahead of the tier is the old language-first one, verdict
/// for verdict, over every combination of tier, group, revision, language and
/// score the two sides can differ in.
#[test]
fn the_default_and_language_ahead_orders_reproduce_the_old_ladder() {
    let tiers = [Some(0), Some(1), None];
    let languages = [
        AudioLanguageRank::for_test(0, 0),
        AudioLanguageRank::for_test(0, 1),
        AudioLanguageRank::for_test(1, 0),
        AudioLanguageRank::for_test(2, 1),
    ];
    let policies = [
        grab_policy(),
        AdmissionPolicy::not_a_downgrade(),
        AdmissionPolicy {
            allow_upgrades: false,
            ..grab_policy()
        },
        AdmissionPolicy {
            cutoff_score: Some(500),
            ..grab_policy()
        },
    ];
    let mut compared = 0;
    for (language_first, pipeline) in [
        (false, DecisionPipeline::default_order()),
        (true, language_ahead_of_tier()),
    ] {
        for held_tier in tiers {
            for held_revision in [0, 1] {
                for held_language in languages {
                    let (mut held, primary) = spoken("held", held_tier, 600, held_language);
                    held.revision = held_revision;
                    let subject = title_scope((held.clone(), primary));
                    for tier in tiers {
                        for group in ["group", "other"] {
                            for revision in [0, 1] {
                                for language in languages {
                                    for score in [300, 600, 700, 900] {
                                        for policy in &policies {
                                            let facts = CandidateFacts::new(tier, revision, score)
                                                .with_revision_policy(Some(group), false)
                                                .with_language(language)
                                                .with_pipeline(pipeline);
                                            let actual =
                                                evaluate_admission(&subject, facts, policy)
                                                    .rejection()
                                                    .map(|rejection| {
                                                        reason_label(&rejection.reason)
                                                    });
                                            let expected = frozen_ladder(
                                                language_first,
                                                (tier, group, revision, score, language),
                                                &held,
                                                policy,
                                            );
                                            assert_eq!(
                                                actual, expected,
                                                "language_first={language_first} held=({held_tier:?}, r{held_revision}, {held_language:?}) candidate=({tier:?}, {group}, r{revision}, {language:?}, {score}) policy={policy:?}"
                                            );
                                            compared += 1;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    assert_eq!(compared, 2 * 3 * 2 * 4 * 3 * 2 * 2 * 4 * 4 * 4);
}

/// The lanes' walk: candidates in `RankHead` order, passing over every one
/// admission refuses. Returns the admitted candidates in the order reached.
/// The tests below pin orders where a refusal ranks above an admissible
/// release, which is why the walk cannot stop at the first refusal.
fn walk_ranked_skipping_refusals<'a>(
    subject: &AdmissionSubject,
    candidates: Vec<CandidateFacts<'a>>,
    policy: &AdmissionPolicy,
) -> Vec<CandidateFacts<'a>> {
    use crate::acquisition::scoring::RankHead;
    let mut ranked: Vec<_> = candidates
        .into_iter()
        .map(|candidate| {
            let head = RankHead {
                side: candidate.facts().side,
                negated_revision: revision_preference(candidate, subject).saturating_neg(),
                negated_score: candidate.score.saturating_neg(),
                pipeline: candidate.pipeline,
                ..Default::default()
            };
            (head.key(), candidate)
        })
        .collect();
    ranked.sort_by_key(|(key, _)| *key);
    let mut admitted = Vec::new();
    for (_, candidate) in ranked {
        if evaluate_admission(subject, candidate, policy).is_admitted() {
            admitted.push(candidate);
        }
    }
    admitted
}

/// A lower revision from the held file's own group is refused on the
/// revision, but ranks as if its revision did not matter, so a high score
/// sorts it ahead of a release from another group that would be admitted —
/// and a walk that stops at the first refusal never reaches that one.
#[test]
fn a_refused_lower_revision_does_not_hide_an_admissible_release_below_it() {
    let (mut held, primary) = incumbent("proper-on-disk", 600, &[]);
    held.tier_index = Some(1);
    held.revision = 1;
    held.release_group = Some("GRP".into());
    let subject = title_scope((held, primary));
    let original = CandidateFacts::new(Some(1), 0, 9_000).with_revision_policy(Some("GRP"), false);
    let other_group =
        CandidateFacts::new(Some(1), 0, 900).with_revision_policy(Some("OTHER"), false);
    assert!(!evaluate_admission(&subject, original, &grab_policy()).is_admitted());
    assert!(evaluate_admission(&subject, other_group, &grab_policy()).is_admitted());
    let reached =
        walk_ranked_skipping_refusals(&subject, vec![original, other_group], &grab_policy());
    assert_eq!(
        reached
            .iter()
            .map(|candidate| candidate.score)
            .collect::<Vec<_>>(),
        [900],
        "the admissible release must be reached"
    );
}

/// With upgrades off, a better tier is refused while a PROPER of the file on
/// disk still lands; the better tier must not hide the PROPER from the walk.
#[test]
fn a_refused_better_tier_does_not_hide_a_proper_when_upgrades_are_off() {
    let (mut held, primary) = incumbent("original-on-disk", 600, &[]);
    held.tier_index = Some(1);
    held.release_group = Some("GRP".into());
    let subject = title_scope((held, primary));
    let policy = AdmissionPolicy {
        allow_upgrades: false,
        ..grab_policy()
    };
    let better_tier =
        CandidateFacts::new(Some(0), 0, 9_000).with_revision_policy(Some("OTHER"), false);
    let proper = CandidateFacts::new(Some(1), 1, 500).with_revision_policy(Some("GRP"), false);
    assert!(!evaluate_admission(&subject, better_tier, &policy).is_admitted());
    assert!(evaluate_admission(&subject, proper, &policy).is_admitted());
    let reached = walk_ranked_skipping_refusals(&subject, vec![better_tier, proper], &policy);
    assert_eq!(
        reached
            .iter()
            .map(|candidate| candidate.score)
            .collect::<Vec<_>>(),
        [500],
        "the PROPER must be reached"
    );
}

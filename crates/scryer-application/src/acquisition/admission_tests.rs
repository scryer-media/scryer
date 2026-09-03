//! Admission gate behaviour.
//!
//! These pin the predicate that both the grab path and the import path share.
//! A divergence here is the bug this work exists to remove, so the cases are
//! written from the gate's contract rather than from either caller.

use super::admission::*;

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
            release_group: None,
            score,
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

    assert!(
        evaluate_admission(&subject, CandidateFacts::new(Some(0), 0, 2_000), &auto(1))
            .is_admitted(),
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

    assert!(
        evaluate_admission(&subject, CandidateFacts::new(Some(0), 0, 1), &auto(1)).is_admitted()
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

    let verdict = evaluate_admission(&subject, CandidateFacts::new(Some(0), 1, 400), &auto(200));
    assert!(
        verdict.is_admitted(),
        "a PROPER of what is on disk is not a trimming: {verdict:?}"
    );
}

/// …and the converse. The original facing the PROPER already on disk is not an
/// upgrade at any score, and it reports the revision rather than a misleading
/// "equal or better score".
#[test]
fn the_original_is_refused_against_a_proper_on_disk() {
    let subject = AdmissionSubject::new(episodes(&["ep-01"]), [revised("file-1", 1, 100)]);

    let verdict = evaluate_admission(&subject, CandidateFacts::new(Some(0), 0, 9_000), &auto(1));
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

    let verdict = evaluate_admission(&subject, CandidateFacts::new(Some(1), 2, 9_000), &auto(1));
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
        evaluate_admission(&subject, CandidateFacts::new(Some(0), 1, 1_200), &auto(200))
            .is_admitted()
    );
    let verdict = evaluate_admission(&subject, CandidateFacts::new(Some(0), 1, 1_000), &auto(200));
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
        evaluate_admission(&subject, CandidateFacts::new(Some(0), 1, 400), &no_upgrades)
            .is_admitted()
    );

    let on_disk_is_the_proper =
        AdmissionSubject::new(episodes(&["ep-01"]), [revised("file-1", 1, 900)]);
    let verdict = evaluate_admission(
        &on_disk_is_the_proper,
        CandidateFacts::new(Some(0), 0, 9_000),
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
    let verdict = evaluate_admission(&subject, CandidateFacts::new(Some(0), 1, 500), &auto(200));
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

    let verdict = evaluate_admission(&subject, CandidateFacts::new(Some(0), 0, 9_000), &auto(1));
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
        !evaluate_admission(&subject, CandidateFacts::new(Some(0), 0, 10), &auto(1)).is_admitted()
    );

    let verdict = evaluate_admission(
        &subject,
        CandidateFacts::new(Some(0), 0, 10),
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
        CandidateFacts::new(Some(0), 0, 900),
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
        title: title.to_string(),
        covers: Vec::new(),
        tier_index,
        revision,
        score,
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

    let proper = evaluate_admission(&subject, CandidateFacts::new(Some(1), 1, 100), &policy);
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
        evaluate_admission(&subject, CandidateFacts::new(Some(1), 1, 100), &policy).is_admitted()
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

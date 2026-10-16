use super::{evaluations::ScoreEvaluationEntry, release_listing::ReleaseListingSnapshot};
use crate::{
    IndexerSearchResult, admission::AdmissionSubject, quality_profile::QualityProfileDecision,
};
use scryer_domain::{GrabScoreComparison, GrabScoringSnapshot};

pub(crate) fn capture(
    decision: &QualityProfileDecision,
    admission: &AdmissionSubject,
    decision_code: &str,
) -> GrabScoringSnapshot {
    capture_values(
        decision.preference_score,
        &decision.scoring_log,
        admission,
        decision_code,
    )
}

pub(crate) fn capture_values(
    candidate_score: i32,
    entries: &[crate::quality_profile::ScoringEntry],
    admission: &AdmissionSubject,
    decision_code: &str,
) -> GrabScoringSnapshot {
    let comparisons: Vec<_> = admission
        .incumbents()
        .iter()
        .map(|file| GrabScoreComparison {
            file_id: file.file_id.clone(),
            episode_ids: file.covers.clone(),
            previous_score: file.score,
            delta: i64::from(candidate_score) - i64::from(file.score),
        })
        .collect();
    // Different incumbents must remain separate comparisons, never a fabricated aggregate.
    let previous_score = comparisons
        .first()
        .map(|entry| entry.previous_score)
        .filter(|score| {
            comparisons
                .iter()
                .all(|entry| entry.previous_score == *score)
        });
    GrabScoringSnapshot {
        candidate_score,
        previous_score,
        delta: previous_score.map(|previous| i64::from(candidate_score) - i64::from(previous)),
        decision_code: if decision_code == "eligible" {
            if comparisons.is_empty() {
                "initial"
            } else {
                "upgrade"
            }
            .into()
        } else {
            decision_code.into()
        },
        comparisons,
        entries: entries.iter().map(ScoreEvaluationEntry::from).collect(),
    }
}

pub(crate) fn attach(
    candidate: &mut IndexerSearchResult,
    admission: &AdmissionSubject,
    code: &str,
    now: chrono::DateTime<chrono::Utc>,
) {
    let Some(decision) = candidate.quality_profile_decision.as_ref() else {
        return;
    };
    let mut listing = candidate
        .release_listing_json
        .as_deref()
        .and_then(ReleaseListingSnapshot::from_json_str)
        .unwrap_or_else(|| ReleaseListingSnapshot::capture_from_search_result(candidate, now));
    listing.grab_scoring = Some(capture(decision, admission, code));
    candidate.release_listing_json = Some(listing.to_json_string());
}

pub(crate) fn from_listing(json: Option<&str>) -> Option<GrabScoringSnapshot> {
    json.and_then(ReleaseListingSnapshot::from_json_str)
        .and_then(|listing| listing.grab_scoring)
}

pub(crate) fn with_snapshot(
    json: Option<String>,
    snapshot: Option<GrabScoringSnapshot>,
) -> Option<String> {
    json.map(|json| match ReleaseListingSnapshot::from_json_str(&json) {
        Some(mut listing) => {
            listing.grab_scoring = snapshot;
            listing.to_json_string()
        }
        None => json,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admission::{AdmissionScope, Incumbent};

    fn subject(scores: &[i32]) -> AdmissionSubject {
        AdmissionSubject::new(
            AdmissionScope::Title,
            scores.iter().enumerate().map(|(i, score)| {
                (
                    Incumbent {
                        tier_index: None,
                        revision: 0,
                        file_id: format!("file-{i}"),
                        file_path: String::new(),
                        release_group: None,
                        score: *score,
                        language: Default::default(),
                        covers: vec![format!("ep-{i}")],
                        created_at: String::new(),
                    },
                    true,
                )
            }),
        )
    }

    #[test]
    fn grab_scoring_preserves_zero_negative_and_differing_pack_comparisons() {
        let first = capture_values(0, &[], &subject(&[]), "eligible");
        assert_eq!(first.candidate_score, 0);
        assert_eq!(first.previous_score, None);
        assert_eq!(first.decision_code, "initial");
        let upgrade = capture_values(-10, &[], &subject(&[-20]), "eligible");
        assert_eq!(upgrade.previous_score, Some(-20));
        assert_eq!(upgrade.delta, Some(10));
        assert_eq!(upgrade.decision_code, "upgrade");
        let pack = capture_values(5, &[], &subject(&[-10, 20]), "eligible");
        assert_eq!(pack.previous_score, None);
        assert_eq!(pack.delta, None);
        assert_eq!(
            pack.comparisons.iter().map(|c| c.delta).collect::<Vec<_>>(),
            vec![-15, 15]
        );
        let overflow = capture_values(i32::MAX, &[], &subject(&[i32::MIN]), "eligible");
        assert_eq!(overflow.delta, Some(4_294_967_295));
    }

    #[test]
    fn grab_scoring_survives_listing_round_trip_and_legacy_is_absent() {
        let json = r#"{"v":1,"captured_at":"2026-01-01T00:00:00Z"}"#.to_owned();
        assert!(from_listing(Some(&json)).is_none());
        let snapshot = capture_values(-1, &[], &subject(&[0]), "manual_override");
        let saved = with_snapshot(Some(json), Some(snapshot.clone()));
        assert_eq!(from_listing(saved.as_deref()), Some(snapshot));
    }
}

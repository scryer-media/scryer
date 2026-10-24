use crate::admission::{
    AdmissionPolicy, AdmissionScope, AdmissionSubject, CandidateFacts, Incumbent,
    evaluate_admission, revision_preference,
};

fn occupied(groups: &[Option<&str>], revision: i32) -> AdmissionSubject {
    AdmissionSubject::new(
        AdmissionScope::Episodes(vec!["ep-1".into(), "ep-2".into()]),
        groups.iter().enumerate().map(|(index, group)| {
            (
                Incumbent {
                    tier_index: Some(0),
                    revision,
                    file_id: format!("file-{index}"),
                    file_path: format!("/library/file-{index}.mkv"),
                    release_group: group.map(str::to_owned),
                    score: 900,
                    language: Default::default(),
                    covers: vec![format!("ep-{}", index + 1)],
                    created_at: "2026-01-01T00:00:00Z".into(),
                },
                true,
            )
        }),
    )
}

fn policy() -> AdmissionPolicy {
    AdmissionPolicy {
        allow_upgrades: true,
        min_delta: 1,
        cutoff_score: None,
        manual_override: false,
        applies_to_queue: true,
    }
}

#[test]
fn revision_related_groups_and_ignore_control_the_exception() {
    let subject = occupied(&[Some(" Group ")], 2);
    for (group, ignore, admitted) in [
        (Some("group"), false, true),
        (Some("OTHER"), false, false),
        (None, false, false),
        (Some(""), false, false),
        (Some("group"), true, false),
    ] {
        let candidate = CandidateFacts::new(Some(0), 3, 100).with_revision_policy(group, ignore);
        assert_eq!(
            evaluate_admission(&subject, candidate, &policy()).is_admitted(),
            admitted
        );
        assert_eq!(
            revision_preference(candidate, &subject),
            if admitted { 3 } else { 0 }
        );
    }
}

#[test]
fn revision_unrelated_or_unknown_incumbent_does_not_protect_a_repack() {
    for group in [Some("OTHER"), None, Some(" ")] {
        let subject = occupied(&[group], 3);
        let candidate =
            CandidateFacts::new(Some(0), 0, 1000).with_revision_policy(Some("group"), false);
        assert!(evaluate_admission(&subject, candidate, &policy()).is_admitted());
    }
}

#[test]
fn revision_ignore_removes_both_priority_and_downgrade_protection() {
    let subject = occupied(&[Some("group")], 3);
    for ignore in [false, true] {
        let candidate =
            CandidateFacts::new(Some(0), 0, 1000).with_revision_policy(Some("group"), ignore);
        assert_eq!(
            evaluate_admission(&subject, candidate, &policy()).is_admitted(),
            ignore
        );
        assert_eq!(
            evaluate_admission(&subject, candidate, &AdmissionPolicy::not_a_downgrade())
                .is_admitted(),
            ignore
        );
    }
    let subject = occupied(&[Some("group")], 2);
    let disabled = AdmissionPolicy {
        allow_upgrades: false,
        cutoff_score: Some(900),
        ..policy()
    };
    for ignore in [false, true] {
        let candidate =
            CandidateFacts::new(Some(0), 3, 100).with_revision_policy(Some("group"), ignore);
        assert_eq!(
            evaluate_admission(&subject, candidate, &disabled).is_admitted(),
            !ignore
        );
    }
}

#[test]
fn revision_pack_rank_requires_every_incumbent_to_be_related() {
    let candidate =
        CandidateFacts::new(Some(0), 3, 1000).with_revision_policy(Some("group"), false);
    for groups in [
        vec![Some("group"), Some("other")],
        vec![Some("group"), None],
    ] {
        let subject = occupied(&groups, 2);
        assert_eq!(revision_preference(candidate, &subject), 0);
        assert!(evaluate_admission(&subject, candidate, &policy()).is_admitted());
    }
    assert_eq!(
        revision_preference(candidate, &occupied(&[Some("group"), Some("GROUP")], 2)),
        3
    );
    assert_eq!(
        revision_preference(candidate, &occupied(&[Some("group")], 4)),
        0
    );
    assert_eq!(revision_preference(candidate, &occupied(&[], 0)), 0);
    assert!(
        !evaluate_admission(
            &occupied(&[Some("group")], 2),
            CandidateFacts::new(Some(1), 9, 10000).with_revision_policy(Some("group"), false),
            &policy()
        )
        .is_admitted()
    );
}

#[test]
fn revision_queue_comparison_uses_the_profile_policy() {
    let subject = occupied(&[], 0).with_queued(vec![crate::admission::QueuedRelease {
        title: "Show.S01E01.1080p.REPACK2-GROUP".into(),
        covers: vec!["ep-1".into(), "ep-2".into()],
        tier_index: Some(0),
        revision: 2,
        release_group: Some("group".into()),
        score: 900,
        language: Default::default(),
        release_key: None,
    }]);
    for (revision, score, group, ignore, admitted) in [
        (3, 100, Some("GROUP"), false, true),
        (1, 1000, Some("group"), false, false),
        (0, 1000, Some("group"), true, true),
        (3, 100, Some("group"), true, false),
        (0, 1000, Some("other"), false, true),
        (0, 1000, None, false, true),
    ] {
        let candidate =
            CandidateFacts::new(Some(0), revision, score).with_revision_policy(group, ignore);
        assert_eq!(
            evaluate_admission(&subject, candidate, &policy()).is_admitted(),
            admitted
        );
    }
}

#[test]
fn revision_mixed_pack_can_fill_missing_members_without_ranking_priority() {
    let subject = occupied(&[Some("other")], 3).per_member();
    let candidate = CandidateFacts::new(Some(0), 1, 100).with_revision_policy(Some("group"), false);
    assert!(evaluate_admission(&subject, candidate, &policy()).is_admitted());
    assert_eq!(revision_preference(candidate, &subject), 0);
    assert!(
        evaluate_admission(&subject, candidate, &policy())
            .superseded()
            .is_empty()
    );
}

use std::collections::HashMap;

use scryer_domain::{
    ListExclusion, ListExclusionScope, ListFilter, ListMembership, ListMembershipState, MediaFacet,
};
use scryer_plugin_sdk::{ListMediaKind, ListProviderRating};

use super::*;
use crate::lists::test_support::{at, membership, resolved_item, subscription, tmdb};

fn exclusion_for(key: &str, scope: ListExclusionScope) -> ListExclusion {
    ListExclusion {
        id: format!("exclusion-{key}"),
        kind: MediaFacet::Movie,
        external_ids: vec![tmdb(&format!("{key}-id"))],
        display_title: format!("Fixture Title {key}"),
        year: None,
        scope,
        created_by_user_id: None,
        created_at: at(0),
    }
}

fn decisions(evaluated: &[EvaluatedItem]) -> Vec<ItemDecision> {
    evaluated.iter().map(|item| item.decision.clone()).collect()
}

fn existing(rows: Vec<ListMembership>) -> HashMap<String, ListMembership> {
    rows.into_iter()
        .map(|row| (row.item_key.clone(), row))
        .collect()
}

#[test]
fn an_exclusion_wins_over_every_other_outcome() {
    let subscription = subscription("list-a");
    let mut item = resolved_item("alpha");
    item.library_title_id = Some("title-alpha".to_string());
    let evaluated = evaluate(
        &subscription,
        vec![item],
        &[exclusion_for("alpha", ListExclusionScope::AllLists)],
        &HashMap::new(),
    );
    assert_eq!(decisions(&evaluated), vec![ItemDecision::Excluded]);
}

#[test]
fn a_list_scoped_exclusion_only_covers_its_own_list() {
    let exclusions = [exclusion_for(
        "alpha",
        ListExclusionScope::List {
            subscription_id: "list-other".to_string(),
        },
    )];
    let evaluated = evaluate(
        &subscription("list-a"),
        vec![resolved_item("alpha")],
        &exclusions,
        &HashMap::new(),
    );
    assert_eq!(decisions(&evaluated), vec![ItemDecision::Candidate]);
}

#[test]
fn filters_are_re_evaluated_so_a_relaxed_filter_lets_the_item_through() {
    let mut strict = subscription("list-a");
    strict.filters = vec![ListFilter::RatingAtLeast {
        scale: "tmdb".to_string(),
        value: 8.0,
    }];
    let mut item = resolved_item("alpha");
    item.item.provider_rating = Some(ListProviderRating {
        scale: "tmdb".to_string(),
        value: 6.5,
    });
    // The row a previous sync wrote for the filtered item.
    let previous = existing(vec![membership(
        "list-a",
        "alpha",
        ListMembershipState::Filtered,
    )]);

    let first = evaluate(&strict, vec![item.clone()], &[], &previous);
    assert!(matches!(first[0].decision, ItemDecision::Filtered { .. }));

    let mut relaxed = strict.clone();
    relaxed.filters.clear();
    let second = evaluate(&relaxed, vec![item], &[], &previous);
    assert_eq!(decisions(&second), vec![ItemDecision::Candidate]);
}

#[test]
fn a_kind_without_a_route_is_filtered_as_no_route() {
    let mut subscription = subscription("list-a");
    subscription.kinds = vec![MediaFacet::Movie, MediaFacet::Series];
    let mut item = resolved_item("alpha");
    item.item.kind_hint = Some(ListMediaKind::Series);
    item.kind = Some(MediaFacet::Series);
    let evaluated = evaluate(&subscription, vec![item], &[], &HashMap::new());
    assert_eq!(
        decisions(&evaluated),
        vec![ItemDecision::Filtered {
            reason: NO_ROUTE_REASON.to_string()
        }]
    );
}

#[test]
fn the_cap_defers_later_candidates_in_list_order_without_counting_them_filtered() {
    let mut subscription = subscription("list-a");
    subscription.max_per_sync = Some(1);
    let mut in_library = resolved_item("beta");
    in_library.library_title_id = Some("title-beta".to_string());
    let evaluated = evaluate(
        &subscription,
        vec![
            resolved_item("alpha"),
            in_library,
            resolved_item("gamma"),
            resolved_item("delta"),
        ],
        &[],
        &HashMap::new(),
    );
    assert_eq!(
        decisions(&evaluated),
        vec![
            ItemDecision::Candidate,
            ItemDecision::InLibrary {
                title_id: "title-beta".to_string()
            },
            ItemDecision::Deferred,
            ItemDecision::Deferred,
        ]
    );

    let counts = count_states([
        ListMembershipState::Added,
        ListMembershipState::InLibrary,
        ListMembershipState::Pending,
        ListMembershipState::Pending,
    ]);
    assert_eq!(counts.total, 4);
    assert_eq!(counts.filtered, 0, "a capped item is not a filtered item");
}

#[test]
fn an_unresolved_item_is_retried_rather_than_acted_on() {
    let mut item = resolved_item("alpha");
    item.resolved = false;
    let evaluated = evaluate(&subscription("list-a"), vec![item], &[], &HashMap::new());
    assert_eq!(decisions(&evaluated), vec![ItemDecision::Unresolved]);
}

#[test]
fn a_title_the_list_added_stays_added_once_it_is_in_the_library() {
    let mut item = resolved_item("alpha");
    item.library_title_id = Some("title-alpha".to_string());
    let previous = existing(vec![membership(
        "list-a",
        "alpha",
        ListMembershipState::Added,
    )]);
    let evaluated = evaluate(&subscription("list-a"), vec![item], &[], &previous);
    assert_eq!(
        decisions(&evaluated),
        vec![ItemDecision::Keep {
            state: ListMembershipState::Added
        }]
    );
}

#[test]
fn a_settled_request_is_kept_but_a_departed_one_is_decided_again() {
    let mut requested = membership("list-a", "alpha", ListMembershipState::Requested);
    let kept = evaluate(
        &subscription("list-a"),
        vec![resolved_item("alpha")],
        &[],
        &existing(vec![requested.clone()]),
    );
    assert_eq!(
        decisions(&kept),
        vec![ItemDecision::Keep {
            state: ListMembershipState::Requested
        }]
    );

    requested.left_at = Some(at(10));
    let returned = evaluate(
        &subscription("list-a"),
        vec![resolved_item("alpha")],
        &[],
        &existing(vec![requested]),
    );
    assert_eq!(decisions(&returned), vec![ItemDecision::Candidate]);
}

fn synced_subscription() -> ListSubscription {
    let mut subscription = subscription("list-a");
    subscription.sync.last_at = Some(at(10));
    subscription
}

fn refused(reason: &str) -> ListMembership {
    let mut row = membership("list-a", "alpha", ListMembershipState::Rejected);
    row.state_reason = Some(reason.to_string());
    row
}

#[test]
fn a_refused_add_is_kept_while_nothing_that_could_lift_it_changed() {
    for reason in ["rejected", "not_found"] {
        let evaluated = evaluate(
            &synced_subscription(),
            vec![resolved_item("alpha")],
            &[],
            &existing(vec![refused(reason)]),
        );
        assert_eq!(
            decisions(&evaluated),
            vec![ItemDecision::Keep {
                state: ListMembershipState::Rejected
            }],
            "{reason}"
        );
    }
}

#[test]
fn a_refused_add_is_tried_again_once_the_item_resolves_differently() {
    let mut item = resolved_item("alpha");
    item.external_ids.push(tmdb("alpha-new-id"));
    let evaluated = evaluate(
        &synced_subscription(),
        vec![item],
        &[],
        &existing(vec![refused("not_found")]),
    );
    assert_eq!(decisions(&evaluated), vec![ItemDecision::Candidate]);

    let mut item = resolved_item("alpha");
    item.smg_title_id = Some(7);
    let evaluated = evaluate(
        &synced_subscription(),
        vec![item],
        &[],
        &existing(vec![refused("rejected")]),
    );
    assert_eq!(decisions(&evaluated), vec![ItemDecision::Candidate]);
}

#[test]
fn a_refused_add_is_tried_again_after_the_list_is_edited() {
    let mut edited = synced_subscription();
    edited.updated_at = at(15);
    let evaluated = evaluate(
        &edited,
        vec![resolved_item("alpha")],
        &[],
        &existing(vec![refused("rejected")]),
    );
    assert_eq!(decisions(&evaluated), vec![ItemDecision::Candidate]);
}

#[test]
fn a_request_a_reviewer_rejected_stays_rejected_after_an_edit() {
    let mut edited = synced_subscription();
    edited.updated_at = at(15);
    let evaluated = evaluate(
        &edited,
        vec![resolved_item("alpha")],
        &[],
        &existing(vec![refused("request_rejected")]),
    );
    assert_eq!(
        decisions(&evaluated),
        vec![ItemDecision::Keep {
            state: ListMembershipState::Rejected
        }]
    );
}

#[test]
fn a_pending_item_is_a_candidate_again() {
    let previous = existing(vec![membership(
        "list-a",
        "alpha",
        ListMembershipState::Pending,
    )]);
    let evaluated = evaluate(
        &subscription("list-a"),
        vec![resolved_item("alpha")],
        &[],
        &previous,
    );
    assert_eq!(decisions(&evaluated), vec![ItemDecision::Candidate]);
}

#[test]
fn a_rating_filter_matches_only_a_rating_from_the_source_it_names() {
    let mut subscription = subscription("list-a");
    subscription.filters = vec![ListFilter::RatingAtLeast {
        scale: "tmdb".to_string(),
        value: 7.0,
    }];
    let rated = |scale: &str, value: f64| {
        let mut item = resolved_item("alpha");
        item.facts = Some(crate::lists::resolve::ListMetadataFacts {
            ratings: vec![scryer_domain::TitleExternalRating {
                source: scale.into(),
                normalized: value,
                ..Default::default()
            }],
            ..Default::default()
        });
        item
    };

    let passes = evaluate(
        &subscription,
        vec![rated(" TMDB ", 7.5)],
        &[],
        &HashMap::new(),
    );
    assert_eq!(decisions(&passes), vec![ItemDecision::Candidate]);

    let low = evaluate(
        &subscription,
        vec![rated("tmdb", 6.9)],
        &[],
        &HashMap::new(),
    );
    assert!(matches!(low[0].decision, ItemDecision::Filtered { .. }));

    let other_source = evaluate(
        &subscription,
        vec![rated("imdb", 9.0)],
        &[],
        &HashMap::new(),
    );
    assert!(matches!(
        other_source[0].decision,
        ItemDecision::Filtered { .. }
    ));
}

#[test]
fn facet_rating_groups_use_explicit_scales_and_truth_tables() {
    use crate::lists::resolve::ListMetadataFacts;
    use scryer_domain::{ListRatingMinimum, TitleExternalRating};
    let mut item = resolved_item("rated");
    item.facts = Some(ListMetadataFacts {
        ratings: vec![TitleExternalRating {
            source: "anilist".into(),
            normalized: 8.0,
            value: Some(80.0),
            ..Default::default()
        }],
        ..Default::default()
    });
    let mut rule = ListFilter::Ratings {
        facet: MediaFacet::Anime,
        match_any: false,
        minimums: vec![
            ListRatingMinimum {
                source: "anilist".into(),
                value: 80.0,
            },
            ListRatingMinimum {
                source: "mal".into(),
                value: 8.0,
            },
        ],
    };
    assert!(passes(&rule, &item), "another facet is unaffected");
    item.kind = Some(MediaFacet::Anime);
    assert!(!passes(&rule, &item), "all requires missing MAL");
    if let ListFilter::Ratings { match_any, .. } = &mut rule {
        *match_any = true;
    }
    assert!(passes(&rule, &item));
    item.facts.as_mut().unwrap().ratings[0].normalized = 7.9;
    assert!(!passes(&rule, &item));
    if let ListFilter::Ratings { minimums, .. } = &mut rule {
        minimums.clear();
    }
    item.facts = None;
    assert!(passes(&rule, &item), "empty any imposes no restriction");
    for (source, scale) in [
        ("imdb", 10.0),
        ("anilist", 100.0),
        ("letterboxd", 5.0),
        ("tomatoes", 100.0),
    ] {
        item.facts = Some(ListMetadataFacts {
            ratings: vec![TitleExternalRating {
                source: source.into(),
                normalized: 8.0,
                ..Default::default()
            }],
            ..Default::default()
        });
        assert!(rating_passes(item.facts.as_ref(), source, scale * 0.8));
        assert!(!rating_passes(item.facts.as_ref(), source, scale * 0.81));
    }
}

#[test]
fn a_missing_score_cannot_pass_even_a_zero_minimum() {
    let mut item = resolved_item("missing-score");
    item.facts = Some(crate::lists::resolve::ListMetadataFacts {
        ratings: vec![scryer_domain::TitleExternalRating {
            source: "imdb".into(),
            ..Default::default()
        }],
        ..Default::default()
    });
    let mut list = subscription("zero-minimum");
    list.filters = vec![ListFilter::RatingAtLeast {
        scale: "imdb".into(),
        value: 0.0,
    }];
    assert_eq!(
        filter_reason(&list, &item).as_deref(),
        Some("missing_rating")
    );
    item.facts.as_mut().unwrap().ratings[0].value = Some(0.0);
    assert_eq!(filter_reason(&list, &item), None);
}

#[test]
fn canonical_exclusions_language_and_missing_facts_are_distinct() {
    use crate::lists::resolve::ListMetadataFacts;
    let mut item = resolved_item("facts");
    let mut list = subscription("list");
    list.filters = vec![ListFilter::Language {
        languages: vec!["eng".into()],
    }];
    assert_eq!(
        filter_reason(&list, &item).as_deref(),
        Some("missing_language")
    );
    item.facts = Some(ListMetadataFacts {
        original_language: Some("en".into()),
        canonical_keys: vec!["canonical:genre:action".into()],
        ..Default::default()
    });
    assert_eq!(filter_reason(&list, &item), None);
    item.facts.as_mut().unwrap().original_language = Some("ja".into());
    assert_eq!(filter_reason(&list, &item).as_deref(), Some("language"));
    list.filters = vec![ListFilter::ExcludeCanonicalTags {
        facet: MediaFacet::Movie,
        keys: vec!["canonical:genre:action".into()],
        unresolved_labels: vec![],
    }];
    assert_eq!(filter_reason(&list, &item).as_deref(), Some("genre"));
    item.kind = Some(MediaFacet::Anime);
    assert!(passes(&list.filters[0], &item));
}

#[test]
fn filter_changes_preserve_existing_linked_membership_protection() {
    let mut list = subscription("list");
    list.filters = vec![ListFilter::RatingAtLeast {
        scale: "imdb".into(),
        value: 10.0,
    }];
    let item = resolved_item("linked");
    for state in [ListMembershipState::Added, ListMembershipState::InLibrary] {
        let mut row = membership("list", "linked", state);
        row.title_id = Some("existing-title".into());
        assert_eq!(
            evaluate(&list, vec![item.clone()], &[], &existing(vec![row]))[0].decision,
            ItemDecision::Keep { state }
        );
    }
    assert!(matches!(
        evaluate(&list, vec![item], &[], &HashMap::new())[0].decision,
        ItemDecision::Filtered { .. }
    ));
}

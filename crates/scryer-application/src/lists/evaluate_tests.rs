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
        item.item.provider_rating = Some(ListProviderRating {
            scale: scale.to_string(),
            value,
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

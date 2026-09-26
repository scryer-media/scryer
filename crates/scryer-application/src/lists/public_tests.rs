use std::collections::HashMap;

use scryer_domain::{ListMembershipState, ListMode, MediaFacet, MediaRequestOrigin};

use super::*;
use crate::lists::evaluate::{EvaluatedItem, ItemDecision};
use crate::lists::test_support::{at, membership, resolved_item, route, subscription};

#[test]
fn public_lists_cannot_request_or_discover() {
    let routes = [route(MediaFacet::Movie, "library-movies")];
    for mode in [ListMode::Request, ListMode::Discover] {
        assert!(validate_public_settings(&[MediaFacet::Movie], None, mode, &routes, None).is_err());
    }
    for mode in [ListMode::Search, ListMode::Add, ListMode::Hold] {
        assert_eq!(
            validate_public_settings(&[MediaFacet::Movie], None, mode, &routes, None).unwrap(),
            vec![MediaFacet::Movie]
        );
    }
}

#[test]
fn kinds_narrow_the_declared_set_and_routes_must_fit_them() {
    let declared = [MediaFacet::Movie, MediaFacet::Series];
    let kinds = validate_public_settings(
        &declared,
        Some(&[MediaFacet::Series]),
        ListMode::Add,
        &[route(MediaFacet::Series, "library-series")],
        Some(5),
    )
    .unwrap();
    assert_eq!(kinds, vec![MediaFacet::Series]);

    // A kind the source never lists.
    assert!(
        validate_public_settings(
            &declared,
            Some(&[MediaFacet::Anime]),
            ListMode::Add,
            &[],
            None
        )
        .is_err()
    );
    // A route for a kind the follow dropped.
    assert!(
        validate_public_settings(
            &declared,
            Some(&[MediaFacet::Series]),
            ListMode::Add,
            &[route(MediaFacet::Movie, "library-movies")],
            None,
        )
        .is_err()
    );
    // Two routes for one kind.
    assert!(
        validate_public_settings(
            &declared,
            None,
            ListMode::Add,
            &[
                route(MediaFacet::Movie, "library-movies"),
                route(MediaFacet::Movie, "library-movies-two"),
            ],
            None,
        )
        .is_err()
    );
    // A zero cap.
    assert!(validate_public_settings(&declared, None, ListMode::Add, &[], Some(0)).is_err());
}

#[test]
fn viewers_without_manage_lists_do_not_see_failure_text() {
    let mut followed = subscription("public-a");
    followed.sync.error_message = Some("The list no longer exists or is private.".into());
    followed.sync.fetch_fingerprint = Some("fingerprint".into());

    let viewer = redact_for_viewer(followed.clone(), false);
    assert_eq!(viewer.sync.error_message, None);
    assert_eq!(viewer.sync.fetch_fingerprint, None);

    let manager = redact_for_viewer(followed, true);
    assert!(manager.sync.error_message.is_some());
    assert_eq!(manager.sync.fetch_fingerprint, None);
}

#[test]
fn membership_pages_follow_list_order() {
    let mut rows = Vec::new();
    for (key, rank) in [("c", None), ("b", Some(2)), ("a", Some(1))] {
        let mut row = membership("public-a", key, ListMembershipState::InLibrary);
        row.rank = rank;
        rows.push(row);
    }
    let page = membership_page(rows.clone(), 2, 0);
    assert_eq!(page.total_count, 3);
    let keys = page
        .items
        .iter()
        .map(|row| row.item_key.as_str())
        .collect::<Vec<_>>();
    assert_eq!(keys, vec!["a", "b"]);

    let rest = membership_page(rows, 2, 2);
    assert_eq!(rest.items.len(), 1);
    assert_eq!(rest.items[0].item_key, "c");
}

#[test]
fn a_preview_adds_exactly_the_candidates() {
    let decisions = vec![
        ("one", ItemDecision::Candidate),
        ("two", ItemDecision::Deferred),
        ("three", ItemDecision::Excluded),
        (
            "four",
            ItemDecision::Filtered {
                reason: "rating".into(),
            },
        ),
        (
            "five",
            ItemDecision::InLibrary {
                title_id: "title-5".into(),
            },
        ),
        ("six", ItemDecision::Unresolved),
        (
            "seven",
            ItemDecision::Keep {
                state: ListMembershipState::Added,
            },
        ),
    ];
    let evaluated = decisions
        .into_iter()
        .map(|(key, decision)| EvaluatedItem {
            item: resolved_item(key),
            decision,
        })
        .collect::<Vec<_>>();
    let posters = HashMap::from([(
        "one".to_string(),
        "https://img.example.test/one.jpg".to_string(),
    )]);

    let preview = summarize_preview(&evaluated, &posters);
    assert_eq!(preview.total, 7);
    assert_eq!(preview.excluded, 1);
    assert_eq!(preview.filtered, 1);
    assert_eq!(preview.in_library, 2);
    assert_eq!(preview.unresolved, 1);
    assert_eq!(preview.would_add.len(), 1);
    assert_eq!(preview.would_add[0].item_key, "one");
    assert_eq!(
        preview.would_add[0].display_title.as_deref(),
        Some("Fixture Title one")
    );
    assert_eq!(
        preview.would_add[0].poster_url.as_deref(),
        Some("https://img.example.test/one.jpg")
    );
}

#[test]
fn list_request_counts_skip_manual_and_old_requests() {
    let request = |id: &str, owner: &str, origin: MediaRequestOrigin, minutes: i64| {
        let mut request = crate::lists::rejection::tests::rejected_request(id, origin);
        request.created_by_user_id = owner.to_string();
        request.created_at = at(minutes);
        request
    };
    let list = MediaRequestOrigin::PublicList {
        subscription_id: "public-a".to_string(),
    };
    let requests = vec![
        request("r1", "member-a", list.clone(), 100),
        request("r2", "member-a", MediaRequestOrigin::Manual, 100),
        request("r3", "member-a", list.clone(), 0),
        request(
            "r4",
            "member-b",
            MediaRequestOrigin::PersonalList {
                subscription_id: "personal-b".to_string(),
            },
            100,
        ),
    ];
    let counts = list_request_counts(&requests, at(50));
    assert_eq!(counts.get("member-a"), Some(&1));
    assert_eq!(counts.get("member-b"), Some(&1));
}

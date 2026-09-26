use scryer_domain::{ListMembershipState, ListScope};

use super::*;
use crate::lists::test_support::{at, membership, subscription};

fn on_title(
    subscription_id: &str,
    key: &str,
    title_id: &str,
    state: ListMembershipState,
) -> ListMembership {
    let mut row = membership(subscription_id, key, state);
    row.title_id = Some(title_id.to_string());
    row
}

#[test]
fn personal_lists_are_never_named_on_a_title() {
    let public = subscription("public-a");
    let mut personal = subscription("personal-a");
    personal.scope = ListScope::Personal;
    personal.name = "Member watchlist".to_string();

    let rows = vec![
        on_title(
            "public-a",
            "item-1",
            "title-one",
            ListMembershipState::InLibrary,
        ),
        on_title(
            "personal-a",
            "item-1",
            "title-one",
            ListMembershipState::InLibrary,
        ),
        on_title(
            "personal-a",
            "item-2",
            "title-two",
            ListMembershipState::InLibrary,
        ),
    ];
    let by_title = public_memberships_by_title(&rows, &[public, personal]);

    assert_eq!(
        by_title.len(),
        1,
        "a title only a personal list holds has no entry"
    );
    let memberships = &by_title["title-one"];
    assert_eq!(memberships.len(), 1);
    assert_eq!(memberships[0].subscription_id, "public-a");
    assert_eq!(memberships[0].name, "Fixture list public-a");
}

#[test]
fn rows_without_a_title_or_a_known_list_are_skipped() {
    let rows = vec![
        membership("public-a", "item-1", ListMembershipState::Unresolved),
        on_title(
            "public-gone",
            "item-2",
            "title-one",
            ListMembershipState::InLibrary,
        ),
    ];
    assert!(public_memberships_by_title(&rows, &[subscription("public-a")]).is_empty());
}

#[test]
fn lists_the_title_is_still_on_come_before_lists_it_left() {
    let mut first = subscription("public-a");
    first.name = "Alpha list".to_string();
    let mut second = subscription("public-b");
    second.name = "Beta list".to_string();
    let mut third = subscription("public-c");
    third.name = "Gamma list".to_string();

    let mut left = on_title(
        "public-a",
        "item-1",
        "title-one",
        ListMembershipState::InLibrary,
    );
    left.left_at = Some(at(90));
    let mut added = on_title(
        "public-c",
        "item-1",
        "title-one",
        ListMembershipState::Added,
    );
    added.added_by_list = true;
    let rows = vec![
        left,
        added,
        on_title(
            "public-b",
            "item-1",
            "title-one",
            ListMembershipState::InLibrary,
        ),
    ];
    let memberships = &public_memberships_by_title(&rows, &[first, second, third])["title-one"];

    let names: Vec<&str> = memberships.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(names, ["Beta list", "Gamma list", "Alpha list"]);
    assert!(memberships[1].added_by_list);
    assert_eq!(memberships[2].left_at, Some(at(90)));
}

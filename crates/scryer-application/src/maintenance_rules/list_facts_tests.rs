use scryer_domain::{ListMembershipState, ListScope};
use scryer_rules::maintenance::Observation;

use super::*;
use crate::lists::test_support::{at, membership, subscription};

fn on_list(subscription_id: &str, key: &str, title_id: &str) -> ListMembership {
    let mut row = membership(subscription_id, key, ListMembershipState::InLibrary);
    row.title_id = Some(title_id.to_string());
    row
}

fn left(mut row: ListMembership, minutes: i64) -> ListMembership {
    row.left_at = Some(at(minutes));
    row
}

fn built_facts() -> MaintenanceFactsDoc {
    let usernames = HashMap::new();
    let watch = super::super::facts::MaintenanceWatchContext::default();
    super::super::facts::build_title_input(
        Utc::now(),
        &super::super::facts::tests::title(None),
        &super::super::facts::MaintenanceLibraryRef {
            id: "library-1".to_string(),
            name: "Movies".to_string(),
        },
        &[],
        super::super::facts::MaintenanceTitlePeople {
            requester_user_ids: None,
            usernames: &usernames,
        },
        super::super::facts::MaintenanceTitleWatch {
            context: &watch,
            signals: None,
        },
        super::super::facts::MaintenanceTitleClaims::none(),
        &[],
    )
    .facts
}

fn envelope<T: serde::Serialize>(observation: &Observation<T>) -> serde_json::Value {
    serde_json::to_value(observation).expect("observation serializes")
}

#[test]
fn a_title_on_a_public_list_is_named() {
    let mut added = on_list("list-a", "alpha", "title-alpha");
    added.added_by_list = true;
    let facts = list_facts_from_rows(&[added], &[subscription("list-a")]);

    assert_eq!(
        facts["title-alpha"],
        TitleListFacts {
            added_by_list: true,
            on_enabled_list: true,
            names: vec!["Fixture list list-a".to_string()],
            left_all: false,
            left_at: None,
            last_list_name: None,
        }
    );
}

#[test]
fn a_personal_list_holds_the_title_without_being_named() {
    let mut personal = subscription("list-p");
    personal.scope = ListScope::Personal;
    let facts = list_facts_from_rows(&[on_list("list-p", "alpha", "title-alpha")], &[personal]);

    let title = &facts["title-alpha"];
    assert!(title.on_enabled_list);
    assert!(title.names.is_empty());
    assert!(!title.left_all);
}

#[test]
fn a_disabled_list_does_not_hold_the_title() {
    let mut disabled = subscription("list-a");
    disabled.enabled = false;
    let facts = list_facts_from_rows(&[on_list("list-a", "alpha", "title-alpha")], &[disabled]);

    let title = &facts["title-alpha"];
    assert!(!title.on_enabled_list);
    assert!(title.names.is_empty());
    assert!(
        !title.left_all,
        "a paused list has not dropped the title; it has only stopped syncing"
    );
}

#[test]
fn leaving_every_list_reports_the_last_departure_and_public_name() {
    let mut personal = subscription("list-p");
    personal.scope = ListScope::Personal;
    let rows = vec![
        left(on_list("list-a", "alpha", "title-alpha"), 30),
        left(on_list("list-b", "alpha", "title-alpha"), 60),
        left(on_list("list-p", "alpha", "title-alpha"), 90),
    ];
    let facts = list_facts_from_rows(
        &rows,
        &[subscription("list-a"), subscription("list-b"), personal],
    );

    let title = &facts["title-alpha"];
    assert!(title.left_all);
    assert_eq!(title.left_at, Some(at(90)));
    assert_eq!(
        title.last_list_name.as_deref(),
        Some("Fixture list list-b"),
        "the personal list left later, but its name is never reported"
    );
}

#[test]
fn still_on_one_list_means_no_departure_time() {
    let rows = vec![
        left(on_list("list-a", "alpha", "title-alpha"), 30),
        on_list("list-b", "alpha", "title-alpha"),
    ];
    let facts = list_facts_from_rows(&rows, &[subscription("list-a"), subscription("list-b")]);

    let title = &facts["title-alpha"];
    assert!(!title.left_all);
    assert_eq!(title.left_at, None);
    assert_eq!(title.last_list_name.as_deref(), Some("Fixture list list-a"));
}

#[test]
fn unread_list_facts_stay_unknown() {
    let mut facts = built_facts();
    assert_eq!(
        envelope(&facts.lists_on_enabled_list)["status"],
        "unknown",
        "a caller that did not load list facts must not report none"
    );

    apply_list_facts(&mut facts, None, "title-alpha");

    assert_eq!(
        envelope(&facts.lists_left_all),
        serde_json::json!({"status": "unknown", "reason": "list_store_unreadable"})
    );
    assert_eq!(envelope(&facts.lists_names)["status"], "unknown");
}

#[test]
fn applied_facts_are_known_or_absent() {
    let mut facts = built_facts();
    let loaded = list_facts_from_rows(
        &[left(on_list("list-a", "alpha", "title-alpha"), 30)],
        &[subscription("list-a")],
    );

    apply_list_facts(&mut facts, Some(&loaded), "title-alpha");
    assert_eq!(
        envelope(&facts.lists_left_all),
        serde_json::json!({"status": "known", "value": true})
    );
    assert_eq!(
        envelope(&facts.lists_left_at),
        serde_json::json!({"status": "known", "value": at(30).to_rfc3339()})
    );

    apply_list_facts(&mut facts, Some(&loaded), "title-never-listed");
    assert_eq!(
        envelope(&facts.lists_added_by_list),
        serde_json::json!({"status": "known", "value": false})
    );
    assert_eq!(
        envelope(&facts.lists_names),
        serde_json::json!({"status": "known", "value": []})
    );
    assert_eq!(
        envelope(&facts.lists_left_at),
        serde_json::json!({"status": "absent", "reason": "not_left_every_list"})
    );
    assert_eq!(
        envelope(&facts.lists_last_list_name),
        serde_json::json!({"status": "absent", "reason": "no_public_list_left"})
    );
}

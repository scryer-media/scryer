use scryer_domain::{ListMembershipState, ListOnLeave, ListScope};

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

fn personal(id: &str) -> ListSubscription {
    let mut personal = subscription(id);
    personal.scope = ListScope::Personal;
    personal
}

/// A subject input built at ten days past the fixture epoch.
fn built_input() -> MaintenanceInput {
    let usernames = HashMap::new();
    let watch = super::super::facts::MaintenanceWatchContext::default();
    super::super::facts::build_title_input(
        at(10 * 24 * 60),
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
            audience: None,
        },
        super::super::facts::MaintenanceTitleClaims::none(),
        &[],
    )
}

fn loaded(lists: ListFactsByTitle) -> MaintenanceListFacts {
    MaintenanceListFacts {
        lists: Some(lists),
        origins: Some(HashMap::new()),
    }
}

fn known(value: serde_json::Value) -> serde_json::Value {
    serde_json::json!({"status": "known", "value": value})
}

fn absent(reason: &str) -> serde_json::Value {
    serde_json::json!({"status": "absent", "reason": reason})
}

fn unknown(reason: &str) -> serde_json::Value {
    serde_json::json!({"status": "unknown", "reason": reason})
}

const LIST_FACTS: [&str; 16] = [
    "lists_added_by_list",
    "lists_on_enabled_list",
    "lists_names",
    "lists_left_all",
    "lists_left_at",
    "lists_last_list_name",
    "lists_enabled_ids",
    "lists_public_count",
    "lists_personal_count",
    "lists_first_seen_at",
    "lists_last_seen_at",
    "lists_days_on_list",
    "lists_best_rank",
    "lists_on_leave",
    "lists_left_handled",
    "lists_membership_states",
];

fn observations(input: &MaintenanceInput) -> serde_json::Value {
    serde_json::to_value(input).expect("input serializes")["observations"].clone()
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
            enabled_ids: vec!["list-a".to_string()],
            public_count: 1,
            personal_count: 0,
            first_seen_at: Some(at(0)),
            last_seen_at: Some(at(0)),
            best_rank: None,
            on_leave: vec!["keep".to_string()],
            left_handled: None,
            membership_states: vec!["in_library".to_string()],
        }
    );
}

#[test]
fn a_personal_list_holds_the_title_without_being_named() {
    let facts = list_facts_from_rows(
        &[on_list("list-p", "alpha", "title-alpha")],
        &[personal("list-p")],
    );

    let title = &facts["title-alpha"];
    assert!(title.on_enabled_list);
    assert!(title.names.is_empty());
    assert!(!title.left_all);
    assert_eq!(title.enabled_ids, vec!["list-p".to_string()]);
    assert_eq!((title.public_count, title.personal_count), (0, 1));
}

#[test]
fn no_fact_names_a_personal_list() {
    let mut subscriptions = vec![personal("list-p"), personal("list-q")];
    subscriptions[1].name = "Private picks of a member".to_string();
    let rows = vec![
        on_list("list-p", "alpha", "title-alpha"),
        left(on_list("list-q", "alpha", "title-alpha"), 30),
    ];
    let origins = request_origins_with_names(
        HashMap::from([(
            "title-alpha".to_string(),
            MediaRequestOrigin::PersonalList {
                subscription_id: "list-q".to_string(),
            },
        )]),
        &subscriptions,
    );
    let mut input = built_input();

    apply_list_facts(
        &mut input,
        &MaintenanceListFacts {
            lists: Some(list_facts_from_rows(&rows, &subscriptions)),
            origins: Some(origins),
        },
        "title-alpha",
    );

    let serialized = serde_json::to_string(&input).expect("input serializes");
    for subscription in &subscriptions {
        assert!(
            !serialized.contains(&subscription.name),
            "personal list name {:?} leaked into the input",
            subscription.name
        );
    }
    let observations = observations(&input);
    assert_eq!(
        observations["request_origin"],
        known("personal_list".into())
    );
    assert_eq!(
        observations["request_list_name"],
        absent("no_public_request_list")
    );
    assert_eq!(observations["lists_personal_count"], known(1.into()));
}

#[test]
fn a_disabled_list_does_not_hold_the_title() {
    let mut disabled = subscription("list-a");
    disabled.enabled = false;
    let facts = list_facts_from_rows(&[on_list("list-a", "alpha", "title-alpha")], &[disabled]);

    let title = &facts["title-alpha"];
    assert!(!title.on_enabled_list);
    assert!(title.names.is_empty());
    assert!(title.enabled_ids.is_empty());
    assert!(title.on_leave.is_empty());
    assert_eq!(
        title.public_count, 1,
        "a paused list still holds a membership, so it still counts by scope"
    );
    assert!(
        !title.left_all,
        "a paused list has not dropped the title; it has only stopped syncing"
    );
}

#[test]
fn leaving_every_list_reports_the_last_departure_and_public_name() {
    let rows = vec![
        left(on_list("list-a", "alpha", "title-alpha"), 30),
        left(on_list("list-b", "alpha", "title-alpha"), 60),
        left(on_list("list-p", "alpha", "title-alpha"), 90),
    ];
    let facts = list_facts_from_rows(
        &rows,
        &[
            subscription("list-a"),
            subscription("list-b"),
            personal("list-p"),
        ],
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
fn a_title_every_list_dropped_has_no_present_membership() {
    let mut handled = left(on_list("list-a", "alpha", "title-alpha"), 30);
    handled.left_handled = true;
    handled.rank = Some(3);
    let facts = list_facts_from_rows(&[handled], &[subscription("list-a")]);
    let mut input = built_input();

    apply_list_facts(&mut input, &loaded(facts), "title-alpha");

    let observations = observations(&input);
    assert_eq!(
        observations["lists_enabled_ids"],
        known(serde_json::json!([]))
    );
    assert_eq!(observations["lists_public_count"], known(0.into()));
    assert_eq!(observations["lists_personal_count"], known(0.into()));
    assert_eq!(observations["lists_first_seen_at"], absent("not_on_a_list"));
    assert_eq!(observations["lists_last_seen_at"], absent("not_on_a_list"));
    assert_eq!(observations["lists_days_on_list"], absent("not_on_a_list"));
    assert_eq!(
        observations["lists_best_rank"],
        absent("no_list_rank"),
        "a departed row's rank is not a rank the title holds"
    );
    assert_eq!(observations["lists_on_leave"], known(serde_json::json!([])));
    assert_eq!(observations["lists_left_handled"], known(true.into()));
    assert_eq!(
        observations["lists_membership_states"],
        known(serde_json::json!([]))
    );
}

#[test]
fn present_memberships_give_sightings_rank_states_and_on_leave() {
    let mut on_a = on_list("list-a", "alpha", "title-alpha");
    on_a.rank = Some(5);
    on_a.first_seen_at = at(60);
    on_a.last_seen_at = at(600);
    let mut on_b = membership("list-b", "alpha", ListMembershipState::Added);
    on_b.title_id = Some("title-alpha".to_string());
    on_b.rank = Some(2);
    on_b.first_seen_at = at(120);
    on_b.last_seen_at = at(900);
    let mut gone = left(on_list("list-c", "alpha", "title-alpha"), 30);
    gone.rank = Some(1);
    gone.first_seen_at = at(0);
    let mut unmonitor = subscription("list-b");
    unmonitor.on_leave = ListOnLeave::Unmonitor;
    let facts = list_facts_from_rows(
        &[on_a, on_b, gone],
        &[subscription("list-a"), unmonitor, subscription("list-c")],
    );
    let mut input = built_input();

    apply_list_facts(&mut input, &loaded(facts), "title-alpha");

    let observations = observations(&input);
    assert_eq!(
        observations["lists_enabled_ids"],
        known(serde_json::json!(["list-a", "list-b"]))
    );
    assert_eq!(observations["lists_public_count"], known(2.into()));
    assert_eq!(
        observations["lists_first_seen_at"],
        known(at(60).to_rfc3339().into())
    );
    assert_eq!(
        observations["lists_last_seen_at"],
        known(at(900).to_rfc3339().into())
    );
    assert_eq!(
        observations["lists_days_on_list"],
        known(9.into()),
        "whole days from the earliest present sighting to the evaluation time"
    );
    assert_eq!(observations["lists_best_rank"], known(2.into()));
    assert_eq!(
        observations["lists_on_leave"],
        known(serde_json::json!(["keep", "unmonitor"]))
    );
    assert_eq!(observations["lists_left_handled"], known(false.into()));
    assert_eq!(
        observations["lists_membership_states"],
        known(serde_json::json!(["added", "in_library"]))
    );
}

#[test]
fn an_unranked_membership_has_no_best_rank() {
    let facts = list_facts_from_rows(
        &[on_list("list-a", "alpha", "title-alpha")],
        &[subscription("list-a")],
    );
    let mut input = built_input();

    apply_list_facts(&mut input, &loaded(facts), "title-alpha");

    assert_eq!(
        observations(&input)["lists_best_rank"],
        absent("no_list_rank")
    );
}

#[test]
fn left_handled_needs_every_departure_handled() {
    let mut handled = left(on_list("list-a", "alpha", "title-alpha"), 30);
    handled.left_handled = true;
    let pending = left(on_list("list-b", "alpha", "title-alpha"), 60);
    let subscriptions = [subscription("list-a"), subscription("list-b")];

    let mixed = list_facts_from_rows(&[handled.clone(), pending], &subscriptions);
    assert_eq!(mixed["title-alpha"].left_handled, Some(false));
    let all = list_facts_from_rows(&[handled], &subscriptions);
    assert_eq!(all["title-alpha"].left_handled, Some(true));
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
    let mut input = built_input();
    let observations_before = observations(&input);
    for fact in LIST_FACTS
        .iter()
        .chain(&["request_origin", "request_list_name"])
    {
        assert_eq!(
            observations_before[fact]["status"], "unknown",
            "a caller that did not load {fact} must not report none"
        );
    }

    apply_list_facts(
        &mut input,
        &MaintenanceListFacts {
            lists: None,
            origins: Some(HashMap::from([(
                "title-alpha".to_string(),
                TitleRequestOrigin {
                    origin: MediaRequestOrigin::PublicList {
                        subscription_id: "list-a".to_string(),
                    },
                    list_name: None,
                },
            )])),
        },
        "title-alpha",
    );

    let observations = observations(&input);
    for fact in LIST_FACTS {
        assert_eq!(
            observations[fact],
            unknown("list_store_unreadable"),
            "{fact} must read unknown when the list store could not be read"
        );
    }
    assert_eq!(observations["request_origin"], known("public_list".into()));
    assert_eq!(
        observations["request_list_name"],
        unknown("list_store_unreadable"),
        "a public list that could not be read is not a list without a name"
    );
}

#[test]
fn an_unread_request_store_leaves_only_origin_facts_unknown() {
    let facts = list_facts_from_rows(
        &[on_list("list-a", "alpha", "title-alpha")],
        &[subscription("list-a")],
    );
    let mut input = built_input();

    apply_list_facts(
        &mut input,
        &MaintenanceListFacts {
            lists: Some(facts),
            origins: None,
        },
        "title-alpha",
    );

    let observations = observations(&input);
    assert_eq!(
        observations["request_origin"],
        unknown("request_store_unreadable")
    );
    assert_eq!(
        observations["request_list_name"],
        unknown("request_store_unreadable")
    );
    assert_eq!(observations["lists_on_enabled_list"], known(true.into()));
}

#[test]
fn request_origin_names_only_a_public_list_the_store_holds() {
    let origins = request_origins_with_names(
        HashMap::from([
            (
                "title-public".to_string(),
                MediaRequestOrigin::PublicList {
                    subscription_id: "list-a".to_string(),
                },
            ),
            (
                "title-removed-list".to_string(),
                MediaRequestOrigin::PublicList {
                    subscription_id: "list-gone".to_string(),
                },
            ),
            ("title-manual".to_string(), MediaRequestOrigin::Manual),
        ]),
        &[subscription("list-a")],
    );
    let lists = MaintenanceListFacts {
        lists: Some(HashMap::new()),
        origins: Some(origins),
    };
    let applied = |title_id: &str| {
        let mut input = built_input();
        apply_list_facts(&mut input, &lists, title_id);
        let observations = observations(&input);
        (
            observations["request_origin"].clone(),
            observations["request_list_name"].clone(),
        )
    };

    assert_eq!(
        applied("title-public"),
        (
            known("public_list".into()),
            known("Fixture list list-a".into())
        )
    );
    assert_eq!(
        applied("title-removed-list"),
        (
            known("public_list".into()),
            absent("no_public_request_list")
        )
    );
    assert_eq!(
        applied("title-manual"),
        (known("manual".into()), absent("no_public_request_list"))
    );
    assert_eq!(
        applied("title-not-requested"),
        (absent("title_not_requested"), absent("title_not_requested"))
    );
}

#[test]
fn applied_facts_are_known_or_absent() {
    let mut input = built_input();
    let facts = list_facts_from_rows(
        &[left(on_list("list-a", "alpha", "title-alpha"), 30)],
        &[subscription("list-a")],
    );
    let lists = loaded(facts);

    apply_list_facts(&mut input, &lists, "title-alpha");
    let observations_alpha = observations(&input);
    assert_eq!(observations_alpha["lists_left_all"], known(true.into()));
    assert_eq!(
        observations_alpha["lists_left_at"],
        known(at(30).to_rfc3339().into())
    );

    apply_list_facts(&mut input, &lists, "title-never-listed");
    let observations = observations(&input);
    assert_eq!(observations["lists_added_by_list"], known(false.into()));
    assert_eq!(observations["lists_names"], known(serde_json::json!([])));
    assert_eq!(observations["lists_left_at"], absent("not_left_every_list"));
    assert_eq!(
        observations["lists_last_list_name"],
        absent("no_public_list_left")
    );
    assert_eq!(observations["lists_first_seen_at"], absent("not_on_a_list"));
    assert_eq!(observations["lists_days_on_list"], absent("not_on_a_list"));
    assert_eq!(observations["lists_left_handled"], absent("no_list_left"));
    assert_eq!(observations["lists_public_count"], known(0.into()));
}

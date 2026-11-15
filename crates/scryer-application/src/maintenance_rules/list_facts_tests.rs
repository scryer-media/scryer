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
    built_input_for(&super::super::facts::tests::title(None))
}

fn built_input_for(title: &Title) -> MaintenanceInput {
    let usernames = HashMap::new();
    let watch = super::super::facts::MaintenanceWatchContext::default();
    super::super::facts::build_title_input(
        at(10 * 24 * 60),
        title,
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
        watchlists: WatchlistLoad::NotLoaded,
        metadata: MetadataLoad::default(),
        episode_shapes: EpisodeShapeLoad::default(),
        favorites: FavoriteLoad::NotLoaded,
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
            watchlists: WatchlistLoad::NotLoaded,
            metadata: MetadataLoad::default(),
            episode_shapes: EpisodeShapeLoad::default(),
            favorites: FavoriteLoad::NotLoaded,
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
            watchlists: WatchlistLoad::NotLoaded,
            metadata: MetadataLoad::default(),
            episode_shapes: EpisodeShapeLoad::default(),
            favorites: FavoriteLoad::NotLoaded,
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
            watchlists: WatchlistLoad::NotLoaded,
            metadata: MetadataLoad::default(),
            episode_shapes: EpisodeShapeLoad::default(),
            favorites: FavoriteLoad::NotLoaded,
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
        watchlists: WatchlistLoad::NotLoaded,
        metadata: MetadataLoad::default(),
        episode_shapes: EpisodeShapeLoad::default(),
        favorites: FavoriteLoad::NotLoaded,
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

// ── Watchlists ─────────────────────────────────────────────────────────────

const WATCHLIST_FACTS: [&str; 6] = [
    "watchlisted_by_user_ids",
    "watchlisted_by_usernames",
    "watchlisted_by_anyone",
    "watchlisted_by_any_requester",
    "watchlist_added_at",
    "watchlist_origins",
];

/// A member's synced watchlist from `provider`, read with their account.
fn watchlist(id: &str, owner: &str, provider: &str, source_type: &str) -> ListSubscription {
    let mut watchlist = personal(id);
    watchlist.owner_user_id = owner.to_string();
    watchlist.source.provider = provider.to_string();
    watchlist.source.source_type = source_type.to_string();
    watchlist.source.params.clear();
    watchlist.credential_id = Some(format!("{id}-credential"));
    watchlist.sync.state = ListSyncState::Ok;
    watchlist.sync.last_at = Some(at(9 * 24 * 60));
    watchlist
}

fn anilist_planning(id: &str, owner: &str) -> ListSubscription {
    let mut planning = watchlist(id, owner, "anilist", "status");
    planning
        .source
        .params
        .insert("status".to_string(), "planning".to_string());
    planning
}

fn seen(mut row: ListMembership, minutes: i64) -> ListMembership {
    row.first_seen_at = at(minutes);
    row
}

fn people_with<'a>(
    requesters: Option<&'a [String]>,
    usernames: &'a HashMap<String, String>,
) -> MaintenanceTitlePeople<'a> {
    MaintenanceTitlePeople {
        requester_user_ids: requesters,
        usernames,
    }
}

fn with_watchlists(watchlists: WatchlistLoad) -> MaintenanceListFacts {
    MaintenanceListFacts {
        watchlists,
        metadata: MetadataLoad::default(),
        episode_shapes: EpisodeShapeLoad::default(),
        favorites: FavoriteLoad::NotLoaded,
        lists: Some(HashMap::new()),
        origins: Some(HashMap::new()),
    }
}

fn usernames() -> HashMap<String, String> {
    HashMap::from([
        ("user-anna".to_string(), "anna".to_string()),
        ("user-boris".to_string(), "boris".to_string()),
    ])
}

/// Every watchlist source Discovery reads counts the same, the anime status
/// lists included once their rows resolved to a title: who wants the title,
/// the earliest sighting, and which providers said so.
#[test]
fn a_title_on_members_watchlists_names_who_wants_it() {
    let now = at(10 * 24 * 60);
    let subscriptions = [
        watchlist("plex-anna", "user-anna", "plex", "watchlist"),
        watchlist("mal-boris", "user-boris", "mal", "status:plan_to_watch"),
        anilist_planning("anilist-boris", "user-boris"),
    ];
    let rows = [
        seen(on_list("plex-anna", "alpha-plex", "title-alpha"), 600),
        seen(on_list("mal-boris", "alpha-mal", "title-alpha"), 300),
        seen(on_list("anilist-boris", "beta-anilist", "title-beta"), 900),
    ];
    let WatchlistLoad::Loaded(by_title) = watchlists_from_rows(&rows, &subscriptions, now) else {
        panic!("synced watchlists load");
    };
    assert_eq!(
        by_title["title-beta"].owner_user_ids,
        BTreeSet::from(["user-boris".to_string()]),
        "an AniList planning row resolved to a title counts like any other"
    );

    let names = usernames();
    let requesters = vec!["user-boris".to_string()];
    let mut input = built_input();
    apply_watchlist_and_metadata_facts(
        &mut input,
        &with_watchlists(WatchlistLoad::Loaded(by_title.clone())),
        "title-alpha",
        people_with(Some(&requesters), &names),
    );
    let observations = observations(&input);
    assert_eq!(
        observations["watchlisted_by_user_ids"],
        known(serde_json::json!(["user-anna", "user-boris"]))
    );
    assert_eq!(
        observations["watchlisted_by_usernames"],
        known(serde_json::json!(["anna", "boris"]))
    );
    assert_eq!(observations["watchlisted_by_anyone"], known(true.into()));
    assert_eq!(
        observations["watchlisted_by_any_requester"],
        known(true.into())
    );
    assert_eq!(
        observations["watchlist_added_at"],
        known(at(300).to_rfc3339().into()),
        "the earliest sighting on any watchlist"
    );
    assert_eq!(
        observations["watchlist_origins"],
        known(serde_json::json!(["mal", "plex"]))
    );

    let mut input = built_input();
    apply_watchlist_and_metadata_facts(
        &mut input,
        &with_watchlists(WatchlistLoad::Loaded(by_title)),
        "title-gamma",
        people_with(None, &names),
    );
    let observations = self::observations(&input);
    assert_eq!(
        observations["watchlisted_by_user_ids"],
        known(serde_json::json!([]))
    );
    assert_eq!(observations["watchlisted_by_anyone"], known(false.into()));
    assert_eq!(
        observations["watchlisted_by_any_requester"],
        absent("title_not_requested")
    );
    assert_eq!(
        observations["watchlist_added_at"],
        absent("not_on_a_watchlist")
    );
}

/// Only a member's own synced watchlist counts. A public list, a disabled
/// one, one read without the member's account, a list that is not a
/// watchlist, and a row that left contribute nothing, and none of them can
/// hold the facts either.
#[test]
fn only_members_own_watchlists_count() {
    let now = at(10 * 24 * 60);
    let mut public = watchlist("public-plex", "user-anna", "plex", "watchlist");
    public.scope = ListScope::Public;
    let mut disabled = watchlist("disabled-plex", "user-anna", "plex", "watchlist");
    disabled.enabled = false;
    disabled.sync.state = ListSyncState::Fail;
    let mut no_account = watchlist("anon-trakt", "user-anna", "trakt", "watchlist");
    no_account.credential_id = None;
    no_account.sync.state = ListSyncState::New;
    let mut rss = watchlist("plex-rss", "user-anna", "plex", "watchlist_rss");
    rss.sync.state = ListSyncState::Fail;
    let mut not_planning = watchlist("anilist-done", "user-anna", "anilist", "status");
    not_planning
        .source
        .params
        .insert("status".to_string(), "completed".to_string());
    let kept = watchlist("plex-boris", "user-boris", "plex", "watchlist");
    let subscriptions = [public, disabled, no_account, rss, not_planning, kept];
    let rows = [
        on_list("public-plex", "a", "title-alpha"),
        on_list("disabled-plex", "b", "title-alpha"),
        on_list("anon-trakt", "c", "title-alpha"),
        on_list("plex-rss", "d", "title-alpha"),
        on_list("anilist-done", "e", "title-alpha"),
        left(on_list("plex-boris", "f", "title-alpha"), 60),
    ];

    assert_eq!(
        watchlists_from_rows(&rows, &subscriptions, now),
        WatchlistLoad::Loaded(HashMap::new())
    );
}

/// A watchlist Scryer has not read lately could hold any title, so every
/// watchlist fact is unknown for every title until it syncs again.
#[test]
fn an_unsynced_or_stale_watchlist_holds_every_watchlist_fact() {
    let now = at(10 * 24 * 60);
    let rows = [on_list("plex-anna", "alpha", "title-alpha")];

    let mut failed = watchlist("plex-anna", "user-anna", "plex", "watchlist");
    failed.sync.state = ListSyncState::Fail;
    let mut never = watchlist("trakt-boris", "user-boris", "trakt", "watchlist");
    never.sync.state = ListSyncState::New;
    never.sync.last_at = None;
    let fresh = watchlist("plex-anna", "user-anna", "plex", "watchlist");
    for subscriptions in [vec![failed], vec![fresh.clone(), never]] {
        assert_eq!(
            watchlists_from_rows(&rows, &subscriptions, now),
            WatchlistLoad::Unknown(unknown_reason::WATCHLIST_NOT_SYNCED)
        );
    }

    // Six-hour polling: a sync two days old is inside the window, one past
    // two days is not.
    let mut stale = fresh.clone();
    stale.sync.last_at = Some(now - chrono::Duration::hours(49));
    let mut recent = fresh;
    recent.sync.last_at = Some(now - chrono::Duration::hours(47));
    assert_eq!(
        watchlists_from_rows(&rows, &[stale], now),
        WatchlistLoad::Unknown(unknown_reason::WATCHLIST_STALE)
    );
    assert!(matches!(
        watchlists_from_rows(&rows, &[recent], now),
        WatchlistLoad::Loaded(_)
    ));

    let names = usernames();
    let mut input = built_input();
    apply_watchlist_and_metadata_facts(
        &mut input,
        &with_watchlists(WatchlistLoad::Unknown(unknown_reason::WATCHLIST_STALE)),
        "title-alpha",
        people_with(None, &names),
    );
    let observations = observations(&input);
    for fact in WATCHLIST_FACTS {
        assert_eq!(observations[fact], unknown("watchlist_stale"), "{fact}");
    }
}

/// A watchlist owner who no longer exists makes the names unknown rather
/// than a shorter list a rule would read as "this person does not want it".
#[test]
fn a_missing_watchlist_owner_makes_the_names_unknown() {
    let mut by_title = HashMap::new();
    by_title.insert(
        "title-alpha".to_string(),
        TitleWatchlist {
            owner_user_ids: BTreeSet::from(["user-anna".to_string(), "user-gone".to_string()]),
            added_at: Some(at(0)),
            origins: BTreeSet::from(["plex".to_string()]),
        },
    );
    let names = usernames();
    let mut input = built_input();
    apply_watchlist_and_metadata_facts(
        &mut input,
        &with_watchlists(WatchlistLoad::Loaded(by_title)),
        "title-alpha",
        people_with(None, &names),
    );
    let observations = observations(&input);
    assert_eq!(
        observations["watchlisted_by_usernames"],
        unknown("user_not_found")
    );
    assert_eq!(
        observations["watchlisted_by_user_ids"],
        known(serde_json::json!(["user-anna", "user-gone"]))
    );
}

/// Facts a batch did not load stay unknown, so a rule can never read a
/// group it was not evaluated with as an answer.
#[test]
fn groups_a_batch_did_not_load_stay_unknown() {
    let names = usernames();
    let mut input = built_input();
    apply_watchlist_and_metadata_facts(
        &mut input,
        &with_watchlists(WatchlistLoad::NotLoaded),
        "title-alpha",
        people_with(None, &names),
    );
    let observations = observations(&input);
    for fact in WATCHLIST_FACTS
        .iter()
        .chain(scryer_rules::maintenance::METADATA_MAINTENANCE_FACTS.iter())
    {
        assert_eq!(observations[*fact], unknown("not_yet_collected"), "{fact}");
    }
}

#[test]
fn a_rule_loads_only_the_groups_it_reads() {
    let reads = |body: &str| {
        MaintenanceFactLoads::for_rule(
            &format!("package whatever\nimport rego.v1\n\nunmonitor if {{\n\t{body}\n}}\n"),
            "rule-fixture",
        )
    };
    assert_eq!(
        reads("input.facts.monitored"),
        MaintenanceFactLoads::default()
    );
    assert_eq!(
        reads("not input.facts.watchlisted_by_anyone"),
        MaintenanceFactLoads {
            watchlists: true,
            metadata: false,
            episode_shapes: false,
            favorites: false,
        }
    );
    assert_eq!(
        reads("\"Horror\" in input.facts.genres"),
        MaintenanceFactLoads {
            watchlists: false,
            metadata: true,
            episode_shapes: false,
            favorites: false,
        }
    );
    assert_eq!(
        reads("input.facts.missing_episode_count > 0"),
        MaintenanceFactLoads {
            watchlists: false,
            metadata: false,
            episode_shapes: true,
            favorites: false,
        }
    );
    assert_eq!(
        MaintenanceFactLoads::for_rule("not rego", "rule-fixture"),
        MaintenanceFactLoads::ALL,
        "a source whose facts cannot be listed loads everything"
    );
}

// ── Genres and ratings ─────────────────────────────────────────────────────

#[test]
fn stored_genres_and_ratings_become_facts() {
    let loaded = MetadataLoad {
        loaded: true,
        genres: Some(HashMap::from([(
            "title-alpha".to_string(),
            vec!["Drama".to_string(), "Mystery".to_string()],
        )])),
        ratings: Some(HashMap::from([(
            "title-alpha".to_string(),
            TitleRatingSummary {
                rating: Some(7.4),
                rating_sources: vec!["fixture-source".to_string()],
                external_ratings: vec![crate::TitleExternalRating {
                    source: "fixture-source".to_string(),
                    value: Some(74.0),
                    score: None,
                    normalized: 7.4,
                    votes: Some(1200),
                    url: String::new(),
                }],
            },
        )])),
    };
    let lists = MaintenanceListFacts {
        metadata: loaded,
        ..with_watchlists(WatchlistLoad::NotLoaded)
    };
    let names = usernames();

    let mut input = built_input();
    apply_watchlist_and_metadata_facts(
        &mut input,
        &lists,
        "title-alpha",
        people_with(None, &names),
    );
    let observations_alpha = observations(&input);
    assert_eq!(
        observations_alpha["genres"],
        known(serde_json::json!(["Drama", "Mystery"]))
    );
    assert_eq!(observations_alpha["rating"], known(7.4.into()));
    assert_eq!(
        observations_alpha["ratings"],
        known(serde_json::json!([
            {"source": "fixture-source", "value": 74.0, "normalized": 7.4, "votes": 1200}
        ]))
    );

    let mut input = built_input();
    apply_watchlist_and_metadata_facts(&mut input, &lists, "title-bare", people_with(None, &names));
    let observations_bare = observations(&input);
    assert_eq!(observations_bare["genres"], known(serde_json::json!([])));
    assert_eq!(
        observations_bare["rating"],
        serde_json::json!({"status": "absent"})
    );
    assert_eq!(observations_bare["ratings"], known(serde_json::json!([])));

    let unreadable = MaintenanceListFacts {
        metadata: MetadataLoad {
            loaded: true,
            genres: None,
            ratings: None,
        },
        ..with_watchlists(WatchlistLoad::NotLoaded)
    };
    let mut input = built_input();
    apply_watchlist_and_metadata_facts(
        &mut input,
        &unreadable,
        "title-alpha",
        people_with(None, &names),
    );
    let observations = observations(&input);
    for fact in ["genres", "rating", "ratings"] {
        assert_eq!(observations[fact], unknown("metadata_unreadable"), "{fact}");
    }
}

// ── Episode shapes ─────────────────────────────────────────────────────────

fn show_input() -> MaintenanceInput {
    let mut show = super::super::facts::tests::faceted_title(None, MediaFacet::Series);
    show.id = "title-show".to_string();
    built_input_for(&show)
}

fn with_episode_shapes(episode_shapes: EpisodeShapeLoad) -> MaintenanceListFacts {
    MaintenanceListFacts {
        episode_shapes,
        ..with_watchlists(WatchlistLoad::NotLoaded)
    }
}

fn shape_load(shapes: Option<HashMap<String, TitleEpisodeShape>>) -> EpisodeShapeLoad {
    EpisodeShapeLoad {
        loaded: true,
        show_title_ids: HashSet::from(["title-show".to_string(), "title-empty-show".to_string()]),
        shapes,
    }
}

#[test]
fn a_show_with_unaired_and_missing_episodes_reports_its_shape() {
    let lists = with_episode_shapes(shape_load(Some(HashMap::from([(
        "title-show".to_string(),
        TitleEpisodeShape {
            season_count: 2,
            aired_episode_count: 8,
            unaired_episode_count: 2,
            aired_episode_file_count: 6,
            missing_episode_count: 1,
            latest_season_number: Some(2),
            latest_aired_episode_date: Some("2024-02-01".to_string()),
            next_episode_air_date: Some("2024-02-08".to_string()),
        },
    )]))));
    let names = usernames();
    let mut input = show_input();
    apply_watchlist_and_metadata_facts(&mut input, &lists, "title-show", people_with(None, &names));
    let observations = observations(&input);
    assert_eq!(observations["season_count"], known(2.into()));
    assert_eq!(observations["aired_episode_count"], known(8.into()));
    assert_eq!(observations["unaired_episode_count"], known(2.into()));
    assert_eq!(observations["missing_episode_count"], known(1.into()));
    assert_eq!(
        observations["aired_episode_file_percent"],
        known(75.0.into())
    );
    assert_eq!(observations["latest_season_number"], known(2.into()));
    assert_eq!(
        observations["latest_aired_episode_date"],
        known("2024-02-01".into())
    );
    assert_eq!(
        observations["next_episode_air_date"],
        known("2024-02-08".into())
    );
}

#[test]
fn a_show_with_no_regular_episode_has_zero_counts_and_no_dates() {
    let lists = with_episode_shapes(shape_load(Some(HashMap::new())));
    let names = usernames();
    let mut input = show_input();
    apply_watchlist_and_metadata_facts(
        &mut input,
        &lists,
        "title-empty-show",
        people_with(None, &names),
    );
    let observations = observations(&input);
    for fact in [
        "season_count",
        "aired_episode_count",
        "unaired_episode_count",
        "missing_episode_count",
    ] {
        assert_eq!(observations[fact], known(0.into()), "{fact}");
    }
    assert_eq!(
        observations["aired_episode_file_percent"],
        absent("no_aired_episode")
    );
    assert_eq!(
        observations["latest_season_number"],
        serde_json::json!({"status": "absent"})
    );
    assert_eq!(
        observations["latest_aired_episode_date"],
        absent("no_aired_episode")
    );
    assert_eq!(
        observations["next_episode_air_date"],
        absent("no_scheduled_episode")
    );
}

#[test]
fn an_unreadable_episode_store_leaves_the_shape_unknown_not_zero() {
    let lists = with_episode_shapes(shape_load(None));
    let names = usernames();
    let mut input = show_input();
    apply_watchlist_and_metadata_facts(&mut input, &lists, "title-show", people_with(None, &names));
    let observations = observations(&input);
    for fact in scryer_rules::maintenance::EPISODE_SHAPE_MAINTENANCE_FACTS {
        assert_eq!(
            observations[fact],
            unknown("episode_store_unreadable"),
            "{fact}"
        );
    }
}

#[test]
fn a_movie_has_no_episode_shape_and_an_unloaded_show_stays_unknown() {
    let names = usernames();
    let mut movie = built_input();
    apply_watchlist_and_metadata_facts(
        &mut movie,
        &with_episode_shapes(shape_load(Some(HashMap::new()))),
        "title-1",
        people_with(None, &names),
    );
    let movie_observations = observations(&movie);
    let mut show = show_input();
    apply_watchlist_and_metadata_facts(
        &mut show,
        &with_watchlists(WatchlistLoad::NotLoaded),
        "title-show",
        people_with(None, &names),
    );
    let show_observations = observations(&show);
    for fact in scryer_rules::maintenance::EPISODE_SHAPE_MAINTENANCE_FACTS {
        assert_eq!(
            movie_observations[fact],
            serde_json::json!({"status": "absent"}),
            "{fact}"
        );
        assert_eq!(
            show_observations[fact],
            unknown("not_yet_collected"),
            "{fact}"
        );
    }
}

// ── Favourites ──────────────────────────────────────────────────────────────

fn favourite(
    item: &str,
    user: Option<&str>,
    kind: MediaServerSignalKind,
    season: Option<i64>,
    episode: Option<&str>,
) -> UserMediaSignal {
    UserMediaSignal {
        id: format!("row-{item}"),
        connection_id: "connection-jellyfin".to_string(),
        provider: scryer_domain::MediaServerProvider::Jellyfin,
        external_user_id: format!("external-{}", user.unwrap_or("nobody")),
        scryer_user_id: user.map(str::to_string),
        provider_item_id: item.to_string(),
        kind,
        scryer_title_id: Some("title-1".to_string()),
        scryer_episode_id: episode.map(str::to_string),
        season_number: season,
        played: false,
        play_count: 0,
        last_played_at: None,
        is_favorite: true,
        observed_at: at(0),
        sync_generation: 1,
        created_at: at(0),
        updated_at: at(0),
    }
}

fn favourite_rows(
    rows: Vec<UserMediaSignal>,
    linked: &[&str],
    observable: &[&str],
) -> FavoriteLoad {
    FavoriteLoad::Loaded(FavoriteRows {
        by_title: if rows.is_empty() {
            HashMap::new()
        } else {
            HashMap::from([("title-1".to_string(), rows)])
        },
        linked_user_ids: linked.iter().map(|id| (*id).to_string()).collect(),
        favorite_user_ids: observable.iter().map(|id| (*id).to_string()).collect(),
    })
}

fn with_favourites(favorites: FavoriteLoad) -> MaintenanceListFacts {
    MaintenanceListFacts {
        favorites,
        ..with_watchlists(WatchlistLoad::NotLoaded)
    }
}

fn favourite_observations(
    mut input: MaintenanceInput,
    favorites: FavoriteLoad,
    requesters: Option<&[String]>,
) -> serde_json::Value {
    let names = usernames();
    apply_watchlist_and_metadata_facts(
        &mut input,
        &with_favourites(favorites),
        "title-1",
        people_with(requesters, &names),
    );
    observations(&input)
}

const FAVOURITE_FACTS: [&str; 4] = [
    "favorited_by_user_ids",
    "favorited_by_usernames",
    "favorited_by_anyone",
    "favorited_by_any_requester",
];

/// A favourite nobody played is a fact: the stored row is unplayed, and it
/// still names who favourited the title.
#[test]
fn an_unplayed_favourite_becomes_a_fact() {
    let requesters = vec!["user-boris".to_string()];
    let observations = favourite_observations(
        built_input(),
        favourite_rows(
            vec![favourite(
                "movie",
                Some("user-anna"),
                MediaServerSignalKind::Movie,
                None,
                None,
            )],
            &["user-anna", "user-boris"],
            &["user-anna", "user-boris"],
        ),
        Some(&requesters),
    );
    assert_eq!(
        observations["favorited_by_user_ids"],
        known(serde_json::json!(["user-anna"]))
    );
    assert_eq!(
        observations["favorited_by_usernames"],
        known(serde_json::json!(["anna"]))
    );
    assert_eq!(observations["favorited_by_anyone"], known(true.into()));
    assert_eq!(
        observations["favorited_by_any_requester"],
        known(false.into())
    );

    // Nobody's favourite, everyone observable: a decisive no.
    let observations = favourite_observations(
        built_input(),
        favourite_rows(Vec::new(), &["user-anna"], &["user-anna"]),
        None,
    );
    assert_eq!(
        observations["favorited_by_user_ids"],
        known(serde_json::json!([]))
    );
    assert_eq!(observations["favorited_by_anyone"], known(false.into()));
    assert_eq!(
        observations["favorited_by_any_requester"],
        absent("title_not_requested")
    );
}

/// A series title counts the series, any season and any episode. A season
/// counts the series, itself and its own episodes; an episode the series, its
/// season and itself.
#[test]
fn series_season_and_episode_favourites_roll_up_to_the_title() {
    let rows = || {
        favourite_rows(
            vec![
                favourite(
                    "series",
                    Some("user-anna"),
                    MediaServerSignalKind::Series,
                    None,
                    None,
                ),
                favourite(
                    "season-2",
                    Some("user-boris"),
                    MediaServerSignalKind::Season,
                    Some(2),
                    None,
                ),
                favourite(
                    "episode-1-5",
                    Some("user-cara"),
                    MediaServerSignalKind::Episode,
                    Some(1),
                    Some("episode-1-5"),
                ),
            ],
            &["user-anna", "user-boris", "user-cara"],
            &["user-anna", "user-boris", "user-cara"],
        )
    };
    let ids = |observations: &serde_json::Value| observations["favorited_by_user_ids"].clone();

    let title = favourite_observations(built_input(), rows(), None);
    assert_eq!(
        ids(&title),
        known(serde_json::json!(["user-anna", "user-boris", "user-cara"]))
    );

    let mut season_one = built_input();
    season_one.subject.kind = MaintenanceSubjectKind::Season;
    season_one.subject.season_number = Some(1);
    assert_eq!(
        ids(&favourite_observations(season_one, rows(), None)),
        known(serde_json::json!(["user-anna", "user-cara"]))
    );

    let mut episode_in_two = built_input();
    episode_in_two.subject.kind = MaintenanceSubjectKind::Episode;
    episode_in_two.subject.season_number = Some(2);
    episode_in_two.subject.episode_id = Some("episode-2-1".to_string());
    assert_eq!(
        ids(&favourite_observations(episode_in_two, rows(), None)),
        known(serde_json::json!(["user-anna", "user-boris"]))
    );

    let mut episode_five = built_input();
    episode_five.subject.kind = MaintenanceSubjectKind::Episode;
    episode_five.subject.season_number = Some(1);
    episode_five.subject.episode_id = Some("episode-1-5".to_string());
    assert_eq!(
        ids(&favourite_observations(episode_five, rows(), None)),
        known(serde_json::json!(["user-anna", "user-cara"]))
    );
}

/// A member observed only through Plex could not have been seen favouriting
/// anything, so no favourite fact reads "no" while one is linked. A favourite
/// someone else holds is still a known yes.
#[test]
fn a_plex_only_member_never_reads_as_not_favourited() {
    let requesters = vec!["user-plex".to_string()];
    let observations = favourite_observations(
        built_input(),
        favourite_rows(Vec::new(), &["user-anna", "user-plex"], &["user-anna"]),
        Some(&requesters),
    );
    for fact in FAVOURITE_FACTS {
        assert_eq!(
            observations[fact],
            unknown("favorite_member_unobservable"),
            "{fact}"
        );
    }

    let observations = favourite_observations(
        built_input(),
        favourite_rows(
            vec![favourite(
                "movie",
                Some("user-anna"),
                MediaServerSignalKind::Movie,
                None,
                None,
            )],
            &["user-anna", "user-plex"],
            &["user-anna"],
        ),
        Some(&requesters),
    );
    assert_eq!(observations["favorited_by_anyone"], known(true.into()));
    assert_eq!(
        observations["favorited_by_user_ids"],
        unknown("favorite_member_unobservable")
    );
    assert_eq!(
        observations["favorited_by_any_requester"],
        unknown("favorite_member_unobservable")
    );
}

/// An instance whose enabled servers are all Plex has no favourites: the
/// facts are absent, whatever the watch gate says. With no server at all, or
/// a closed gate, they are unknown for the gate's reason.
#[test]
fn a_plex_only_instance_has_the_favourite_facts_absent() {
    use super::super::facts::{MaintenanceWatchContext, WatchSignalFreshness};
    let plex_only = MaintenanceWatchContext {
        enabled_connection_ids: HashSet::from(["connection-plex".to_string()]),
        ..MaintenanceWatchContext::default()
    };
    let load = favorite_gate(&plex_only).expect("closed");
    assert_eq!(load, FavoriteLoad::Absent("favorites_unsupported"));
    let observations = favourite_observations(built_input(), load, None);
    for fact in FAVOURITE_FACTS {
        assert_eq!(
            observations[fact],
            absent("favorites_unsupported"),
            "{fact}"
        );
    }

    assert_eq!(
        favorite_gate(&MaintenanceWatchContext::default()),
        Some(FavoriteLoad::Unknown("no_media_server_connection"))
    );
    let stale = MaintenanceWatchContext {
        freshness: WatchSignalFreshness::Unavailable("signals_stale"),
        enabled_connection_ids: HashSet::from(["connection-jellyfin".to_string()]),
        favorite_connection_ids: HashSet::from(["connection-jellyfin".to_string()]),
        ..MaintenanceWatchContext::default()
    };
    assert_eq!(
        favorite_gate(&stale),
        Some(FavoriteLoad::Unknown("signals_stale"))
    );
    let open = MaintenanceWatchContext {
        freshness: WatchSignalFreshness::Fresh,
        ..stale
    };
    assert_eq!(favorite_gate(&open), None);
}

/// A rule that names no favourite fact does not load the group, and its facts
/// stay not yet collected.
#[test]
fn unloaded_favourites_stay_not_yet_collected() {
    let observations = favourite_observations(built_input(), FavoriteLoad::NotLoaded, None);
    for fact in FAVOURITE_FACTS {
        assert_eq!(observations[fact], unknown("not_yet_collected"), "{fact}");
    }
    let reads = |source: &str| {
        MaintenanceFactLoads::for_rule(
            &format!("package whatever\nimport rego.v1\n\nunmonitor if {{\n\t{source}\n}}\n"),
            "rule-fixture",
        )
    };
    assert!(reads("input.facts.favorited_by_anyone").favorites);
    assert!(!reads("input.facts.monitored").favorites);
}

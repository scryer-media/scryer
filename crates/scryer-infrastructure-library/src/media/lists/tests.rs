use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use chrono::{Duration, Utc};
use scryer_application::lists::{
    ListExclusionRepository, ListMembershipRepository, ListSubscriptionQuery,
    ListSubscriptionRepository, UserListAccountRepository, UserListPolicyRepository,
};
use scryer_domain::{
    ExternalId, ListAccountCredential, ListCounts, ListExclusion, ListExclusionScope,
    ListMembership, ListMembershipState, ListMode, ListOnLeave, ListPolicy, ListRoute, ListScope,
    ListSource, ListSourceOrigin, ListSubscription, ListSyncRun, ListSyncRunOutcome, ListSyncState,
    ListSyncStatus, MediaFacet, UserListAccount, UserListAccountStatus, UserListPolicy,
};
use sqlx::sqlite::SqlitePoolOptions;

use super::ListStore;
use crate::encryption::EncryptionKey;
use crate::queries::sql_runtime::{SqlArg, SqlRuntime, StoreDatastore};

const OWNER: &str = "member-under-test";
const OTHER_MEMBER: &str = "second-member-under-test";
const LIBRARY: &str = "library-under-test";

async fn test_store(key: Option<EncryptionKey>) -> ListStore {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("in-memory sqlite should open");
    scryer_infrastructure_datastore::migrations::replay_source_catalog_for_fresh_install(
        &pool, None, true,
    )
    .await
    .expect("fresh migrations should apply");
    let datastore = StoreDatastore::Sqlite {
        pool,
        writer_gate: Arc::new(tokio::sync::Mutex::new(())),
    };
    for user in [OWNER, OTHER_MEMBER] {
        SqlRuntime::execute_write(
            &datastore,
            "seed_user",
            "INSERT INTO users (id, username, password_hash, password_change_required, account_kind, status)
             VALUES ({}, {}, {}, {}, {}, {})",
            vec![
                SqlArg::Text(user.to_string()),
                SqlArg::Text(user.to_string()),
                SqlArg::OptText(None),
                SqlArg::Bool(false),
                SqlArg::Text("local".to_string()),
                SqlArg::Text("active".to_string()),
            ],
        )
        .await
        .expect("seed the user row");
    }
    SqlRuntime::execute_write(
        &datastore,
        "seed_library",
        "INSERT INTO libraries (id, facet, name, slug, is_default, created_at, updated_at)
         VALUES ({}, {}, {}, {}, {}, {}, {})",
        vec![
            SqlArg::Text(LIBRARY.to_string()),
            SqlArg::Text("movie".to_string()),
            SqlArg::Text("Under test".to_string()),
            SqlArg::Text("under-test".to_string()),
            SqlArg::Bool(false),
            SqlArg::Timestamp(Utc::now()),
            SqlArg::Timestamp(Utc::now()),
        ],
    )
    .await
    .expect("seed the library row");
    ListStore::new(datastore, Arc::new(RwLock::new(key)))
}

fn subscription(id: &str, scope: ListScope, owner: &str) -> ListSubscription {
    let now = Utc::now();
    ListSubscription {
        id: id.to_string(),
        scope,
        owner_user_id: owner.to_string(),
        source: ListSource {
            provider: "fixture-provider".into(),
            source_type: "user_list".into(),
            params: BTreeMap::from([("list_id".to_string(), "fixture-list-1".to_string())]),
            origin: ListSourceOrigin::ProviderFetch,
        },
        name: "Fixture List".into(),
        provider_url: Some("https://provider.example/lists/fixture-list-1".into()),
        kinds: vec![MediaFacet::Movie],
        enabled: true,
        mode: ListMode::Add,
        routes: vec![ListRoute {
            kind: MediaFacet::Movie,
            library_id: LIBRARY.into(),
            quality_profile_id: Some("profile-1".into()),
            root_folder_id: None,
            monitor_type: "all".into(),
            min_availability: Some("released".into()),
            use_season_folders: None,
            release_numbering: None,
            tags: vec!["fixture".into()],
        }],
        filters: vec![],
        max_per_sync: Some(25),
        on_leave: ListOnLeave::Keep,
        interval_seconds: 12 * 3600,
        sync: ListSyncStatus::default(),
        counts: ListCounts::default(),
        credential_id: None,
        created_at: now,
        updated_at: now,
    }
}

fn membership(
    subscription_id: &str,
    item_key: &str,
    seen_at: chrono::DateTime<Utc>,
) -> ListMembership {
    ListMembership {
        subscription_id: subscription_id.into(),
        item_key: item_key.into(),
        rank: Some(1),
        season: None,
        display_title: None,
        year: None,
        external_ids: vec![ExternalId::new("tmdb", format!("id-{item_key}"))],
        smg_title_id: None,
        title_id: None,
        request_id: None,
        kind: MediaFacet::Movie,
        state: ListMembershipState::Unresolved,
        state_reason: None,
        added_by_list: false,
        first_seen_at: seen_at,
        last_seen_at: seen_at,
        left_at: None,
        left_handled: false,
    }
}

#[tokio::test]
async fn subscription_round_trips_with_routes_and_update_replaces_them() {
    let store = test_store(None).await;
    let created =
        ListSubscriptionRepository::create(&store, subscription("sub-1", ListScope::Public, OWNER))
            .await
            .expect("create");
    let loaded = ListSubscriptionRepository::get_by_id(&store, "sub-1")
        .await
        .expect("get")
        .expect("present");
    assert_eq!(loaded.routes, created.routes);
    assert_eq!(loaded.source, created.source);
    assert_eq!(loaded.max_per_sync, Some(25));
    assert_eq!(loaded.sync.state, ListSyncState::New);

    let mut edited = loaded.clone();
    edited.name = "Renamed Fixture".into();
    edited.routes = vec![ListRoute {
        kind: MediaFacet::Movie,
        library_id: LIBRARY.into(),
        quality_profile_id: None,
        root_folder_id: None,
        monitor_type: "none".into(),
        min_availability: None,
        use_season_folders: Some(true),
        release_numbering: None,
        tags: vec![],
    }];
    let updated = ListSubscriptionRepository::update(&store, edited)
        .await
        .expect("update");
    assert_eq!(updated.name, "Renamed Fixture");
    assert_eq!(updated.routes.len(), 1);
    assert_eq!(updated.routes[0].monitor_type, "none");
    assert_eq!(updated.routes[0].use_season_folders, Some(true));
}

#[tokio::test]
async fn list_filters_by_scope_and_owner_without_leaking_other_members() {
    let store = test_store(None).await;
    ListSubscriptionRepository::create(&store, subscription("public-1", ListScope::Public, OWNER))
        .await
        .expect("create public");
    ListSubscriptionRepository::create(
        &store,
        subscription("personal-owner", ListScope::Personal, OWNER),
    )
    .await
    .expect("create personal");
    ListSubscriptionRepository::create(
        &store,
        subscription("personal-other", ListScope::Personal, OTHER_MEMBER),
    )
    .await
    .expect("create other personal");

    let public: Vec<_> = ListSubscriptionRepository::list(&store, ListSubscriptionQuery::public())
        .await
        .expect("list public")
        .into_iter()
        .map(|subscription| subscription.id)
        .collect();
    assert_eq!(public, vec!["public-1"]);

    let mine: Vec<_> =
        ListSubscriptionRepository::list(&store, ListSubscriptionQuery::personal_for(OWNER))
            .await
            .expect("list personal")
            .into_iter()
            .map(|subscription| subscription.id)
            .collect();
    assert_eq!(mine, vec!["personal-owner"]);
}

#[tokio::test]
async fn due_selection_respects_enabled_next_and_pause() {
    let store = test_store(None).await;
    let now = Utc::now();
    for (id, enabled, next_at, paused_until) in [
        ("never-synced", true, None, None),
        ("due", true, Some(now - Duration::minutes(1)), None),
        ("not-yet", true, Some(now + Duration::hours(1)), None),
        (
            "paused",
            true,
            Some(now - Duration::minutes(1)),
            Some(now + Duration::hours(1)),
        ),
        ("disabled", false, Some(now - Duration::minutes(1)), None),
    ] {
        let mut subscription = subscription(id, ListScope::Public, OWNER);
        subscription.enabled = enabled;
        ListSubscriptionRepository::create(&store, subscription)
            .await
            .expect("create");
        store
            .record_sync(
                id,
                &ListSyncStatus {
                    state: ListSyncState::Ok,
                    next_at,
                    paused_until,
                    ..ListSyncStatus::default()
                },
                &ListCounts::default(),
            )
            .await
            .expect("record sync");
    }
    let mut due: Vec<_> = store
        .list_due(now, 50)
        .await
        .expect("list due")
        .into_iter()
        .map(|subscription| subscription.id)
        .collect();
    due.sort();
    assert_eq!(due, vec!["due", "never-synced"]);
}

#[tokio::test]
async fn record_sync_persists_counts_and_runs_are_newest_first() {
    let store = test_store(None).await;
    ListSubscriptionRepository::create(&store, subscription("sub-1", ListScope::Public, OWNER))
        .await
        .expect("create");
    let counts = ListCounts {
        total: 10,
        in_library: 3,
        added: 2,
        requested: 0,
        held: 1,
        filtered: 2,
        excluded: 1,
        unresolved: 1,
    };
    let now = Utc::now();
    store
        .record_sync(
            "sub-1",
            &ListSyncStatus {
                state: ListSyncState::Fail,
                last_at: Some(now),
                next_at: Some(now + Duration::hours(12)),
                error_message: Some("The list no longer exists or is private".into()),
                error_at: Some(now),
                paused_until: None,
                fetch_fingerprint: Some("etag-1".into()),
            },
            &counts,
        )
        .await
        .expect("record sync");
    let loaded = ListSubscriptionRepository::get_by_id(&store, "sub-1")
        .await
        .expect("get")
        .expect("present");
    assert_eq!(loaded.counts, counts);
    assert_eq!(loaded.sync.state, ListSyncState::Fail);
    assert_eq!(loaded.sync.fetch_fingerprint.as_deref(), Some("etag-1"));

    let mut first = ListSyncRun::started("sub-1", None);
    first.started_at = now - Duration::hours(1);
    first.outcome = ListSyncRunOutcome::Succeeded;
    let mut second = ListSyncRun::started("sub-1", Some("job-run-1".into()));
    second.started_at = now;
    second.outcome = ListSyncRunOutcome::Failed;
    second.error_message = Some("fetch failed".into());
    store
        .record_sync_run(first.clone())
        .await
        .expect("first run");
    store
        .record_sync_run(second.clone())
        .await
        .expect("second run");
    second.finished_at = Some(now + Duration::seconds(5));
    store
        .record_sync_run(second.clone())
        .await
        .expect("finish second run");

    let runs = store.list_sync_runs("sub-1", 10).await.expect("list runs");
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].id, second.id);
    assert_eq!(runs[0].finished_at, second.finished_at);
    assert_eq!(runs[0].outcome, ListSyncRunOutcome::Failed);
    assert_eq!(runs[1].id, first.id);
}

#[tokio::test]
async fn deleting_a_subscription_cascades_to_memberships_routes_and_runs() {
    let store = test_store(None).await;
    ListSubscriptionRepository::create(&store, subscription("sub-1", ListScope::Public, OWNER))
        .await
        .expect("create");
    let now = Utc::now();
    store
        .upsert_many(&[membership("sub-1", "item-a", now)])
        .await
        .expect("upsert");
    store
        .record_sync_run(ListSyncRun::started("sub-1", None))
        .await
        .expect("run");
    ListSubscriptionRepository::delete(&store, "sub-1")
        .await
        .expect("delete");
    assert!(
        ListSubscriptionRepository::get_by_id(&store, "sub-1")
            .await
            .expect("get")
            .is_none()
    );
    assert!(
        store
            .list_by_subscription("sub-1")
            .await
            .expect("memberships")
            .is_empty()
    );
    assert!(
        store
            .list_sync_runs("sub-1", 10)
            .await
            .expect("runs")
            .is_empty()
    );
    let orphan_routes = SqlRuntime::fetch_all(
        store.datastore.read_exec(),
        "SELECT subscription_id FROM list_subscription_routes WHERE subscription_id = {}",
        &[SqlArg::Text("sub-1".into())],
    )
    .await
    .expect("routes");
    assert!(orphan_routes.is_empty());
}

#[tokio::test]
async fn upsert_keeps_first_seen_and_clears_departure_when_item_returns() {
    let store = test_store(None).await;
    ListSubscriptionRepository::create(&store, subscription("sub-1", ListScope::Public, OWNER))
        .await
        .expect("create");
    let first_sync = Utc::now() - Duration::hours(2);
    store
        .upsert_many(&[
            membership("sub-1", "item-a", first_sync),
            membership("sub-1", "item-b", first_sync),
        ])
        .await
        .expect("first upsert");

    // Second sync sees only item-b.
    let second_sync = first_sync + Duration::hours(1);
    let mut refreshed = membership("sub-1", "item-b", second_sync);
    refreshed.display_title = Some("Fixture Title B".into());
    refreshed.year = Some(2031);
    refreshed.state = ListMembershipState::Added;
    refreshed.added_by_list = true;
    refreshed.title_id = Some("title-b".into());
    store
        .upsert_many(&[refreshed])
        .await
        .expect("second upsert");
    let left = store
        .mark_left("sub-1", second_sync, second_sync)
        .await
        .expect("mark left");
    assert_eq!(
        left.iter()
            .map(|row| row.item_key.as_str())
            .collect::<Vec<_>>(),
        vec!["item-a"]
    );

    let rows = store.list_by_subscription("sub-1").await.expect("rows");
    let item_b = rows
        .iter()
        .find(|row| row.item_key == "item-b")
        .expect("item-b");
    assert_eq!(item_b.first_seen_at.timestamp(), first_sync.timestamp());
    assert_eq!(item_b.last_seen_at.timestamp(), second_sync.timestamp());
    assert_eq!(item_b.state, ListMembershipState::Added);
    assert_eq!(item_b.display_title.as_deref(), Some("Fixture Title B"));
    assert_eq!(item_b.year, Some(2031));
    assert!(item_b.left_at.is_none());
    let item_a = rows
        .iter()
        .find(|row| row.item_key == "item-a")
        .expect("item-a");
    assert!(item_a.left_at.is_some());
    assert!(!item_a.left_handled);

    store
        .set_left_handled("sub-1", &["item-a".to_string()])
        .await
        .expect("handled");
    let by_title = store.list_by_title("title-b").await.expect("by title");
    assert_eq!(by_title.len(), 1);
    let by_titles = store
        .list_by_titles(&["title-b".to_string(), "title-missing".to_string()])
        .await
        .expect("by titles");
    assert_eq!(by_titles, by_title);
    assert!(
        store
            .list_by_titles(&[])
            .await
            .expect("no titles")
            .is_empty()
    );

    // Third sync: item-a is back. Its departure is cleared, its first sighting kept.
    let third_sync = second_sync + Duration::hours(1);
    store
        .upsert_many(&[membership("sub-1", "item-a", third_sync)])
        .await
        .expect("third upsert");
    let rows = store.list_by_subscription("sub-1").await.expect("rows");
    let item_a = rows
        .iter()
        .find(|row| row.item_key == "item-a")
        .expect("item-a");
    assert!(item_a.left_at.is_none());
    assert!(!item_a.left_handled);
    assert_eq!(item_a.first_seen_at.timestamp(), first_sync.timestamp());
    // Nothing is newly departed at the third sync's watermark except item-b.
    let left = store
        .mark_left("sub-1", third_sync, third_sync)
        .await
        .expect("mark left");
    assert_eq!(
        left.iter()
            .map(|row| row.item_key.as_str())
            .collect::<Vec<_>>(),
        vec!["item-b"]
    );
}

fn exclusion(id: &str, scope: ListExclusionScope, ids: &[(&str, &str)]) -> ListExclusion {
    ListExclusion {
        id: id.into(),
        kind: MediaFacet::Movie,
        external_ids: ids
            .iter()
            .map(|(source, value)| ExternalId::new(*source, *value))
            .collect(),
        display_title: "Fixture Feature".into(),
        year: Some(2021),
        scope,
        created_by_user_id: Some(OWNER.into()),
        created_at: Utc::now(),
    }
}

#[tokio::test]
async fn exclusions_match_by_id_kind_and_scope() {
    let store = test_store(None).await;
    ListSubscriptionRepository::create(&store, subscription("sub-1", ListScope::Public, OWNER))
        .await
        .expect("create sub-1");
    ListSubscriptionRepository::create(&store, subscription("sub-2", ListScope::Public, OWNER))
        .await
        .expect("create sub-2");
    ListExclusionRepository::create(
        &store,
        exclusion(
            "all",
            ListExclusionScope::AllLists,
            &[("TMDB", "100"), ("imdb", "tt100")],
        ),
    )
    .await
    .expect("all-lists exclusion");
    ListExclusionRepository::create(
        &store,
        exclusion(
            "scoped",
            ListExclusionScope::List {
                subscription_id: "sub-1".into(),
            },
            &[("tmdb", "200")],
        ),
    )
    .await
    .expect("scoped exclusion");

    let ids = |value: &str| vec![ExternalId::new("tmdb", value)];
    let hits = |found: Vec<ListExclusion>| found.into_iter().map(|e| e.id).collect::<Vec<_>>();

    assert_eq!(
        hits(
            store
                .find_matching(MediaFacet::Movie, &ids("100"), Some("sub-2"))
                .await
                .unwrap()
        ),
        vec!["all"]
    );
    assert_eq!(
        hits(
            store
                .find_matching(MediaFacet::Movie, &ids("200"), Some("sub-1"))
                .await
                .unwrap()
        ),
        vec!["scoped"]
    );
    assert!(
        store
            .find_matching(MediaFacet::Movie, &ids("200"), Some("sub-2"))
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .find_matching(MediaFacet::Movie, &ids("200"), None)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .find_matching(MediaFacet::Series, &ids("100"), Some("sub-1"))
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .find_matching(MediaFacet::Movie, &[], Some("sub-1"))
            .await
            .unwrap()
            .is_empty()
    );

    let listed = ListExclusionRepository::list(&store).await.expect("list");
    assert_eq!(
        listed.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
        vec!["all", "scoped"]
    );
    assert_eq!(listed[0].external_ids.len(), 2);

    // Deleting the scoped subscription removes its exclusion but not the all-lists one.
    ListSubscriptionRepository::delete(&store, "sub-1")
        .await
        .expect("delete sub-1");
    let listed = ListExclusionRepository::list(&store)
        .await
        .expect("list after delete");
    assert_eq!(
        listed.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
        vec!["all"]
    );
    ListExclusionRepository::delete(&store, "all")
        .await
        .expect("delete all");
    assert!(
        ListExclusionRepository::list(&store)
            .await
            .unwrap()
            .is_empty()
    );
}

fn account(id: &str, user_id: &str) -> UserListAccount {
    let now = Utc::now();
    UserListAccount {
        id: id.into(),
        user_id: user_id.into(),
        provider: "fixture-provider".into(),
        external_user_id: "provider-user-1".into(),
        username: "fixture_member".into(),
        display_name: Some("Fixture Member".into()),
        credential: ListAccountCredential {
            access_token: "access-token-fixture".into(),
            refresh_token: Some("refresh-token-fixture".into()),
            expires_at: Some(now + Duration::days(7)),
            token_type: Some("bearer".into()),
            scope: Some("public".into()),
        },
        status: UserListAccountStatus::Active,
        error_message: None,
        linked_at: now,
        last_used_at: None,
        last_refresh_at: None,
        updated_at: now,
    }
}

#[tokio::test]
async fn account_credential_is_encrypted_at_rest_and_decrypted_on_read() {
    let store = test_store(Some(EncryptionKey::generate())).await;
    let created = UserListAccountRepository::create(&store, account("acct-1", OWNER))
        .await
        .expect("create");
    let stored = SqlRuntime::fetch_optional(
        store.datastore.read_exec(),
        "SELECT credential_encrypted FROM user_list_accounts WHERE id = {}",
        &[SqlArg::Text("acct-1".into())],
    )
    .await
    .expect("read raw")
    .expect("row")
    .text("credential_encrypted")
    .expect("column");
    assert!(!stored.contains("access-token-fixture"));
    assert!(!stored.contains("refresh-token-fixture"));

    let loaded = UserListAccountRepository::get_by_id(&store, "acct-1")
        .await
        .expect("get")
        .expect("present");
    assert_eq!(loaded.credential, created.credential);

    let mut edited = loaded.clone();
    edited.status = UserListAccountStatus::Expired;
    edited.error_message = Some("reconnect it".into());
    edited.credential.access_token = "rotated-token".into();
    UserListAccountRepository::update(&store, edited)
        .await
        .expect("update");
    let listed = store.list_by_user_id(OWNER).await.expect("list");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].status, UserListAccountStatus::Expired);
    assert_eq!(listed[0].credential.access_token, "rotated-token");
    assert!(
        store
            .list_by_user_id(OTHER_MEMBER)
            .await
            .expect("other")
            .is_empty()
    );

    UserListAccountRepository::delete(&store, "acct-1")
        .await
        .expect("delete");
    assert!(
        UserListAccountRepository::get_by_id(&store, "acct-1")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn duplicate_provider_identity_for_one_member_is_refused() {
    let store = test_store(None).await;
    UserListAccountRepository::create(&store, account("acct-1", OWNER))
        .await
        .expect("create");
    let duplicate = UserListAccountRepository::create(&store, account("acct-2", OWNER)).await;
    assert!(duplicate.is_err());
    // The same provider identity linked by a different member is a different row.
    UserListAccountRepository::create(&store, account("acct-3", OTHER_MEMBER))
        .await
        .expect("other member");
}

#[tokio::test]
async fn policies_upsert_and_default_to_absent() {
    let store = test_store(None).await;
    assert!(store.get(OWNER).await.expect("get").is_none());
    let now = Utc::now();
    store
        .set(UserListPolicy {
            user_id: OWNER.into(),
            policy: ListPolicy::Auto,
            updated_by_user_id: Some(OTHER_MEMBER.into()),
            updated_at: now,
        })
        .await
        .expect("set");
    store
        .set(UserListPolicy {
            user_id: OWNER.into(),
            policy: ListPolicy::None,
            updated_by_user_id: None,
            updated_at: now + Duration::seconds(1),
        })
        .await
        .expect("set again");
    let loaded = store.get(OWNER).await.expect("get").expect("present");
    assert_eq!(loaded.policy, ListPolicy::None);
    assert!(loaded.updated_by_user_id.is_none());
    assert_eq!(
        UserListPolicyRepository::list(&store)
            .await
            .expect("list")
            .len(),
        1
    );
}

#[tokio::test]
async fn deleting_a_member_removes_their_lists_accounts_and_policy() {
    let store = test_store(None).await;
    ListSubscriptionRepository::create(
        &store,
        subscription("personal-1", ListScope::Personal, OWNER),
    )
    .await
    .expect("create");
    UserListAccountRepository::create(&store, account("acct-1", OWNER))
        .await
        .expect("account");
    store
        .set(UserListPolicy {
            user_id: OWNER.into(),
            policy: ListPolicy::Approval,
            updated_by_user_id: None,
            updated_at: Utc::now(),
        })
        .await
        .expect("policy");
    SqlRuntime::execute_write(
        &store.datastore,
        "delete_member",
        "DELETE FROM users WHERE id = {}",
        vec![SqlArg::Text(OWNER.into())],
    )
    .await
    .expect("delete user");
    assert!(
        ListSubscriptionRepository::list(&store, ListSubscriptionQuery::personal_for(OWNER))
            .await
            .unwrap()
            .is_empty()
    );
    assert!(store.list_by_user_id(OWNER).await.unwrap().is_empty());
    assert!(store.get(OWNER).await.unwrap().is_none());
}

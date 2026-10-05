use super::*;
use scryer_application::lists::{
    ListMembershipRepository, ListSubscriptionRepository, UserListAccountRepository,
};
use scryer_application::testing::ListAccountFixtureProvider;
use scryer_domain::{
    ListAccountCredential, ListCounts, ListMembership, ListMembershipState, ListMode, ListOnLeave,
    ListRoute, ListScope, ListSource, ListSourceOrigin, ListSubscription, ListSyncRun,
    ListSyncStatus, UserListAccount, UserListAccountStatus,
};
use scryer_infrastructure_library::media::lists::ListStore;
use std::sync::Arc;

const ACCOUNT_ID: &str = "private-linked-account";
const FOLLOW_ID: &str = "private-follow";
const SECRETS: [&str; 4] = [
    "fixture-access-secret",
    "fixture-refresh-secret",
    "fixture-relay-handle",
    "fixture-app-secret",
];

struct MatrixFixture {
    ctx: TestContext,
    schema: scryer_interface::ApiSchema,
    store: Arc<ListStore>,
    provider: Arc<ListAccountFixtureProvider>,
    owner: User,
    member: User,
    admin: User,
}

async fn fixture() -> MatrixFixture {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let mut owner = User::new_admin("private-owner");
    owner.authorization.app = AppPermissionMask::NONE;
    owner.authorization.default_library = LibraryPermissionMask::from_permissions([
        LibraryPermission::View,
        LibraryPermission::Request,
    ]);
    let mut member = owner.clone();
    member.id = Id::new().0;
    member.username = "another-member".into();
    let admin = User::new_admin("private-admin");
    for actor in [&owner, &member, &admin] {
        ctx.users.create(actor.clone()).await.unwrap();
    }
    ctx.settings_store
        .upsert_setting_json(
            "system",
            "ui.experimental_features_enabled",
            None,
            "true".into(),
            "test",
            None,
        )
        .await
        .unwrap();
    let store = Arc::new(ListStore::new(
        ctx.db.datastore(),
        ctx.db.encryption_key_state(),
    ));
    let provider = Arc::new(ListAccountFixtureProvider::default());
    let app = ctx.app.with_test_overrides(|builder| {
        builder
            .with_list_store(store.clone())
            .with_list_plugin_provider(provider.clone())
    });
    let schema = scryer_interface::build_schema(app, ctx.auth_runtime.clone());
    let now = Utc::now();
    UserListAccountRepository::create(
        store.as_ref(),
        UserListAccount {
            id: ACCOUNT_ID.into(),
            user_id: owner.id.clone(),
            provider: "trakt".into(),
            external_user_id: "private-provider-identity".into(),
            username: "private-provider-member".into(),
            display_name: Some("Private member identity".into()),
            credential: ListAccountCredential {
                access_token: SECRETS[0].into(),
                refresh_token: Some(SECRETS[1].into()),
                refresh_handle: Some(SECRETS[2].into()),
                app_config: Some(BTreeMap::from([(
                    "client_secret".into(),
                    SECRETS[3].into(),
                )])),
                ..Default::default()
            },
            status: UserListAccountStatus::Active,
            error_message: None,
            linked_at: now,
            last_used_at: None,
            last_refresh_at: None,
            updated_at: now,
        },
    )
    .await
    .unwrap();
    ListSubscriptionRepository::create(
        store.as_ref(),
        ListSubscription {
            id: FOLLOW_ID.into(),
            scope: ListScope::Personal,
            owner_user_id: owner.id.clone(),
            source: ListSource {
                provider: "trakt".into(),
                source_type: "watchlist".into(),
                params: BTreeMap::new(),
                origin: ListSourceOrigin::ProviderFetch,
            },
            name: "Private watchlist".into(),
            provider_url: None,
            kinds: vec![MediaFacet::Movie],
            enabled: true,
            mode: ListMode::Request,
            routes: vec![ListRoute {
                kind: MediaFacet::Movie,
                library_id: scryer_domain::default_library_id_for_facet(&MediaFacet::Movie),
                quality_profile_id: None,
                root_folder_id: None,
                monitor_type: "MONITORED".into(),
                min_availability: None,
                use_season_folders: None,
                release_numbering: None,
                tags: vec![],
            }],
            filters: vec![],
            max_per_sync: Some(25),
            on_leave: ListOnLeave::Keep,
            interval_seconds: 3600,
            sync: ListSyncStatus {
                fetch_fingerprint: Some("private-fetch-fingerprint".into()),
                ..Default::default()
            },
            counts: ListCounts::default(),
            credential_id: Some(ACCOUNT_ID.into()),
            created_at: now,
            updated_at: now,
        },
    )
    .await
    .unwrap();
    store
        .upsert_many(&[ListMembership {
            subscription_id: FOLLOW_ID.into(),
            item_key: "private-item".into(),
            rank: Some(1),
            season: None,
            display_title: Some("Private item title".into()),
            year: None,
            external_ids: vec![],
            smg_title_id: None,
            title_id: None,
            request_id: None,
            kind: MediaFacet::Movie,
            state: ListMembershipState::Unresolved,
            state_reason: None,
            added_by_list: false,
            first_seen_at: now,
            last_seen_at: now,
            left_at: None,
            left_handled: false,
        }])
        .await
        .unwrap();
    store
        .record_sync_run(ListSyncRun::started(FOLLOW_ID, None))
        .await
        .unwrap();
    MatrixFixture {
        ctx,
        schema,
        store,
        provider,
        owner,
        member,
        admin,
    }
}
async fn execute(fixture: &MatrixFixture, actor: &User, query: &str) -> Value {
    let response = fixture
        .schema
        .execute(async_graphql::Request::new(query).data(actor.clone()))
        .await;
    let body = serde_json::to_value(response).unwrap();
    let text = body.to_string();
    for secret in SECRETS {
        assert!(
            !text.contains(secret),
            "GraphQL response contains a credential"
        );
    }
    assert!(!text.contains("private-fetch-fingerprint"));
    body
}

#[tokio::test]
async fn graphql_private_list_reads_are_owner_only_even_for_admin() {
    tokio::time::timeout(std::time::Duration::from_secs(60), async {
        let fixture = fixture().await;
        let query = r#"{ myListAccounts { id provider externalUserId username displayName status } myListSubscriptions { id name scope } }"#;
        let owner = execute(&fixture, &fixture.owner, query).await;
        assert_no_errors(&owner);
        assert_eq!(owner["data"]["myListAccounts"][0]["id"], ACCOUNT_ID);
        assert_eq!(owner["data"]["myListSubscriptions"][0]["id"], FOLLOW_ID);
        let own_account = execute(&fixture, &fixture.owner, r#"{ listAccount(id:"private-linked-account") { id externalUserId ownedLists { id } statuses { key } } }"#).await;
        assert_no_errors(&own_account);
        assert_eq!(own_account["data"]["listAccount"]["id"], ACCOUNT_ID);
        assert_eq!(fixture.provider.account_calls(), 1);
        let own_follow = execute(&fixture, &fixture.owner, r#"{ listSubscription(id:"private-follow") { id name scope } listSubscriptionMemberships(id:"private-follow") { totalCount items { itemKey displayTitle } } listSyncRuns(subscriptionId:"private-follow") { id } }"#).await;
        assert_no_errors(&own_follow);
        assert_eq!(own_follow["data"]["listSubscription"]["scope"], "PERSONAL");
        assert_eq!(own_follow["data"]["listSubscriptionMemberships"]["items"][0]["itemKey"], "private-item");
        assert_eq!(own_follow["data"]["listSyncRuns"].as_array().unwrap().len(), 1);
        for actor in [&fixture.member, &fixture.admin] {
            let mine = execute(&fixture, actor, query).await;
            assert_no_errors(&mine);
            assert_eq!(mine["data"]["myListAccounts"], json!([]));
            assert_eq!(mine["data"]["myListSubscriptions"], json!([]));
            let hidden = execute(&fixture, actor, r#"{ listSubscription(id:"private-follow") { id name } }"#).await;
            assert_no_errors(&hidden);
            assert!(hidden["data"]["listSubscription"].is_null());
            for (field, query) in [
                ("listAccount", r#"{ listAccount(id:"private-linked-account") { id username } }"#),
                ("listSubscriptionMemberships", r#"{ listSubscriptionMemberships(id:"private-follow") { totalCount items { itemKey } } }"#),
                ("listSyncRuns", r#"{ listSyncRuns(subscriptionId:"private-follow") { id } }"#),
            ] {
                let body = execute(&fixture, actor, query).await;
                assert_graphql_field_denied(&body, field);
                assert!(body["errors"][0]["message"].as_str().unwrap().contains("not found"), "the owner check must reject this read: {body}");
                assert!(!body.to_string().contains("Private watchlist"));
                assert!(!body.to_string().contains("private-provider-identity"));
                assert!(!body.to_string().contains("private-item"));
            }
        }
        assert_eq!(fixture.provider.account_calls(), 1, "nonowners must be rejected before provider metadata access");
    }).await.expect("bounded GraphQL ownership matrix");
}

#[tokio::test]
async fn graphql_private_list_mutations_reject_nonowners_and_preserve_rows() {
    tokio::time::timeout(std::time::Duration::from_secs(60), async {
        let fixture = fixture().await;
        for actor in [&fixture.member, &fixture.admin] {
            for (field, query) in [
                ("updateListSubscription", r#"mutation { updateListSubscription(id:"private-follow",input:{name:"Unauthorized rename"}) { id name } }"#),
                ("setListSubscriptionEnabled", r#"mutation { setListSubscriptionEnabled(id:"private-follow",enabled:false) { id enabled } }"#),
                ("syncListSubscription", r#"mutation { syncListSubscription(id:"private-follow") { subscriptionIds } }"#),
                ("unsubscribeList", r#"mutation { unsubscribeList(id:"private-follow") }"#),
                ("unlinkListAccount", r#"mutation { unlinkListAccount(id:"private-linked-account") }"#),
            ] {
                let denied = execute(&fixture, actor, query).await;
                assert_graphql_field_denied(&denied, field);
                assert!(denied["errors"][0]["message"].as_str().unwrap().contains("not found"), "the owner check must reject this mutation: {denied}");
                assert!(!denied.to_string().contains("Private watchlist"));
                let account = UserListAccountRepository::get_by_id(fixture.store.as_ref(), ACCOUNT_ID).await.unwrap().unwrap();
                assert_eq!(account.user_id, fixture.owner.id);
                let follow = ListSubscriptionRepository::get_by_id(fixture.store.as_ref(), FOLLOW_ID).await.unwrap().unwrap();
                assert_eq!(follow.name, "Private watchlist");
                assert!(follow.enabled);
            }
        }
        assert_eq!(fixture.provider.account_calls(), 0);
        let disabled = execute(&fixture, &fixture.owner, r#"mutation { setListSubscriptionEnabled(id:"private-follow",enabled:false) { id enabled } }"#).await;
        assert_no_errors(&disabled);
        assert_eq!(disabled["data"]["setListSubscriptionEnabled"]["enabled"], false);
        let unsubscribed = execute(&fixture, &fixture.owner, r#"mutation { unsubscribeList(id:"private-follow") }"#).await;
        assert_no_errors(&unsubscribed);
        assert_eq!(unsubscribed["data"]["unsubscribeList"], FOLLOW_ID);
        assert!(ListSubscriptionRepository::get_by_id(fixture.store.as_ref(), FOLLOW_ID).await.unwrap().is_none());
        let unlinked = execute(&fixture, &fixture.owner, r#"mutation { unlinkListAccount(id:"private-linked-account") }"#).await;
        assert_no_errors(&unlinked);
        assert_eq!(unlinked["data"]["unlinkListAccount"], ACCOUNT_ID);
        assert!(UserListAccountRepository::get_by_id(fixture.store.as_ref(), ACCOUNT_ID).await.unwrap().is_none());
        let empty = execute(&fixture, &fixture.owner, "{ myListAccounts { id } myListSubscriptions { id } }").await;
        assert_no_errors(&empty);
        assert_eq!(empty["data"]["myListAccounts"], json!([]));
        assert_eq!(empty["data"]["myListSubscriptions"], json!([]));
        assert_eq!(fixture.ctx.smg_server.received_requests().await.unwrap().len(), 0, "ownership tests never contact the metadata relay");
    }).await.expect("bounded GraphQL mutation matrix");
}

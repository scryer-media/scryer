use std::collections::HashMap;

use scryer_domain::{
    ListMembershipState, ListMode, ListOnLeave, ListPolicy, ListScope, ListSyncRunOutcome,
    ListSyncState, UserListPolicy,
};
use scryer_plugin_sdk::{PluginError, PluginErrorCode};

use super::*;
use crate::AppError;
use crate::lists::test_support::{
    FixtureResolver, MemoryListStore, RecordedAction, RecordingActions, ScriptedCharts,
    ScriptedLists, ScriptedProvider, at, keys_of, subscription,
};

struct Harness {
    store: MemoryListStore,
    lists: std::sync::Arc<ScriptedLists>,
    resolver: FixtureResolver,
    actions: RecordingActions,
    provider_configs: ListProviderConfigs,
}

impl Harness {
    fn new(subscriptions: Vec<ListSubscription>) -> Self {
        Self {
            store: MemoryListStore::with_subscriptions(subscriptions),
            lists: ScriptedLists::new(),
            resolver: FixtureResolver::default(),
            actions: RecordingActions::default(),
            provider_configs: ListProviderConfigs::default(),
        }
    }

    async fn sync_at(&self, now: DateTime<Utc>) -> ListSyncReport {
        let provider = ScriptedProvider(self.lists.clone());
        let charts = ScriptedCharts::default();
        let context = ListSyncContext {
            subscriptions: &self.store,
            memberships: &self.store,
            exclusions: &self.store,
            accounts: &self.store,
            policies: &self.store,
            plugins: &provider,
            charts: &charts,
            resolver: &self.resolver,
            actions: &self.actions,
            provider_configs: &self.provider_configs,
        };
        sync_due_subscriptions(&context, now, Some("job-run-one".to_string()))
            .await
            .expect("sync pass")
    }
}

fn rate_limited(retry_after_seconds: Option<i64>) -> PluginError {
    PluginError {
        code: PluginErrorCode::RateLimited,
        public_message: "slow down".to_string(),
        debug_message: None,
        retry_after_seconds,
        details: None,
    }
}

#[tokio::test]
async fn a_first_sync_adds_candidates_and_records_counts() {
    let harness = Harness::new(vec![subscription("list-a")]);
    harness.lists.serve("list-a", &["alpha", "beta"]);

    let report = harness.sync_at(at(0)).await;

    assert_eq!(report.synced, 1);
    assert_eq!(report.added, 2);
    let row = harness.store.row("list-a", "alpha");
    assert_eq!(row.state, ListMembershipState::Added);
    assert!(row.added_by_list);
    assert_eq!(row.title_id.as_deref(), Some("title-alpha"));
    let list = harness.store.subscription("list-a");
    assert_eq!(list.sync.state, ListSyncState::Ok);
    assert_eq!(list.counts.added, 2);
    assert_eq!(list.sync.next_at, Some(at(0) + chrono::Duration::hours(6)));
    assert_eq!(
        harness.actions.calls()[0],
        RecordedAction::Add {
            item_key: "alpha".to_string(),
            search: false
        }
    );
}

#[tokio::test]
async fn the_provider_is_built_with_its_server_wide_values() {
    let mut harness = Harness::new(vec![subscription("list-a")]);
    let values = std::collections::BTreeMap::from([(
        "instance_key".to_string(),
        "synthetic-instance-key".to_string(),
    )]);
    harness.provider_configs.insert(
        &crate::lists::test_support::PROVIDER.to_ascii_uppercase(),
        values.clone(),
    );
    harness.lists.serve("list-a", &["alpha"]);

    harness.sync_at(at(0)).await;

    assert_eq!(harness.lists.configs.lock().unwrap().as_slice(), &[values]);
}

#[tokio::test]
async fn a_fetch_failure_records_fail_and_never_marks_anything_left() {
    let mut list = subscription("list-a");
    list.on_leave = ListOnLeave::Unmonitor;
    let harness = Harness::new(vec![list]);
    harness.lists.serve("list-a", &["alpha"]);
    harness.sync_at(at(0)).await;

    harness.lists.fail(
        "list-a",
        PluginError {
            code: PluginErrorCode::Permanent,
            public_message: "gone".to_string(),
            debug_message: None,
            retry_after_seconds: None,
            details: None,
        },
    );
    let report = harness.sync_at(at(6 * 60)).await;

    assert_eq!(report.failed, 1);
    assert_eq!(report.failures.len(), 1);
    let row = harness.store.row("list-a", "alpha");
    assert_eq!(
        row.left_at, None,
        "a failed read must not look like an empty list"
    );
    assert_eq!(row.state, ListMembershipState::Added);
    assert_eq!(
        harness
            .actions
            .calls()
            .iter()
            .filter(|call| !matches!(call, RecordedAction::SyncFailure { .. }))
            .count(),
        1,
        "only the first sync's add ran; no departure action followed the failure"
    );
    let list = harness.store.subscription("list-a");
    assert_eq!(list.sync.state, ListSyncState::Fail);
    assert!(list.sync.error_message.is_some());
    assert_eq!(list.sync.last_at, Some(at(0)), "the last good sync is kept");
    assert_eq!(
        list.counts.added, 1,
        "counts stay as the last good sync wrote them"
    );
    let runs = harness.store.runs.lock().unwrap().clone();
    assert_eq!(
        runs.last().map(|run| run.outcome),
        Some(ListSyncRunOutcome::Failed)
    );
}

fn sync_failures(actions: &RecordingActions) -> Vec<RecordedAction> {
    actions
        .calls()
        .into_iter()
        .filter(|call| matches!(call, RecordedAction::SyncFailure { .. }))
        .collect()
}

#[tokio::test]
async fn a_public_failure_is_announced_once_until_it_changes() {
    let harness = Harness::new(vec![subscription("list-a")]);
    harness.lists.fail("list-a", rate_limited(None));

    harness.sync_at(at(0)).await;
    harness.sync_at(at(6 * 60)).await;

    assert_eq!(
        sync_failures(&harness.actions),
        vec![RecordedAction::SyncFailure {
            subscription_id: "list-a".to_string(),
            class: "rate_limited".to_string(),
        }],
        "a retry that fails the same way is not announced again"
    );

    harness.lists.fail(
        "list-a",
        PluginError {
            code: PluginErrorCode::Permanent,
            public_message: "gone".to_string(),
            debug_message: None,
            retry_after_seconds: None,
            details: None,
        },
    );
    harness.sync_at(at(12 * 60)).await;

    assert_eq!(
        sync_failures(&harness.actions).len(),
        2,
        "a different failure is announced"
    );
}

#[tokio::test]
async fn a_personal_failure_is_not_announced() {
    let list = ListSubscription {
        scope: ListScope::Personal,
        credential_id: Some("missing-account".to_string()),
        ..subscription("list-a")
    };
    let harness = Harness::new(vec![list]);

    let report = harness.sync_at(at(0)).await;

    assert_eq!(report.failed, 1);
    assert!(sync_failures(&harness.actions).is_empty());
}

#[tokio::test]
async fn a_resolver_failure_is_isolated_the_same_way() {
    let mut harness = Harness::new(vec![subscription("list-a")]);
    harness.lists.serve("list-a", &["alpha"]);
    harness.resolver.fail = true;

    let report = harness.sync_at(at(0)).await;

    assert_eq!(report.failed, 1);
    assert!(harness.store.rows("list-a").is_empty());
    assert_eq!(
        harness.actions.calls(),
        sync_failures(&harness.actions),
        "nothing but the failure notice"
    );
}

#[tokio::test]
async fn one_failing_list_does_not_stop_the_next() {
    let harness = Harness::new(vec![subscription("list-a"), subscription("list-b")]);
    harness.lists.fail("list-a", rate_limited(None));
    harness.lists.serve("list-b", &["alpha"]);

    let report = harness.sync_at(at(0)).await;

    assert_eq!(report.considered, 2);
    assert_eq!(report.failed, 1);
    assert_eq!(report.synced, 1);
    assert_eq!(keys_of(&harness.store.rows("list-b")).len(), 1);
}

#[tokio::test]
async fn a_departed_title_is_marked_left_and_its_action_runs_once() {
    let mut list = subscription("list-a");
    list.on_leave = ListOnLeave::Unmonitor;
    list.interval_seconds = 60;
    let harness = Harness::new(vec![list]);
    harness.lists.serve("list-a", &["alpha", "beta"]);
    harness.sync_at(at(0)).await;

    harness.lists.serve("list-a", &["alpha"]);
    harness.sync_at(at(10)).await;

    let beta = harness.store.row("list-a", "beta");
    assert_eq!(beta.left_at, Some(at(10)));
    assert!(beta.left_handled);
    assert!(
        harness
            .actions
            .calls()
            .contains(&RecordedAction::SetMonitored {
                title_id: "title-beta".to_string(),
                monitored: false,
            })
    );
    assert_eq!(harness.store.row("list-a", "alpha").left_at, None);

    harness.sync_at(at(20)).await;
    let unmonitors = harness
        .actions
        .calls()
        .into_iter()
        .filter(|call| matches!(call, RecordedAction::SetMonitored { .. }))
        .count();
    assert_eq!(unmonitors, 1);
}

#[tokio::test]
async fn a_rate_limit_pauses_until_retry_after_capped_at_a_day() {
    let mut list = subscription("list-a");
    list.interval_seconds = 60;
    let harness = Harness::new(vec![list]);
    harness
        .lists
        .fail("list-a", rate_limited(Some(7 * 24 * 3600)));

    harness.sync_at(at(0)).await;

    let list = harness.store.subscription("list-a");
    assert_eq!(
        list.sync.paused_until,
        Some(at(0) + chrono::Duration::seconds(MAX_RATE_LIMIT_PAUSE_SECONDS))
    );
    assert_eq!(list.sync.next_at, list.sync.paused_until);
    assert!(
        harness.sync_at(at(60)).await.considered == 0,
        "paused lists are not due"
    );
}

#[tokio::test]
async fn an_unchanged_list_touches_only_its_timestamps() {
    let mut list = subscription("list-a");
    list.interval_seconds = 60;
    let harness = Harness::new(vec![list]);
    harness.lists.serve("list-a", &["alpha"]);
    harness.sync_at(at(0)).await;

    let report = harness.sync_at(at(10)).await;

    assert_eq!(report.unchanged, 1);
    assert_eq!(harness.actions.calls().len(), 1);
    assert_eq!(harness.store.row("list-a", "alpha").left_at, None);
    assert_eq!(
        harness.store.subscription("list-a").sync.last_at,
        Some(at(10))
    );
}

#[tokio::test]
async fn a_member_with_lists_turned_off_is_not_read() {
    let list = ListSubscription {
        scope: ListScope::Personal,
        ..subscription("list-a")
    };
    let harness = Harness::new(vec![list]);
    harness.store.policies.lock().unwrap().push(UserListPolicy {
        user_id: "owner-one".to_string(),
        policy: ListPolicy::None,
        updated_by_user_id: None,
        updated_at: at(0),
    });
    harness.lists.serve("list-a", &["alpha"]);

    let report = harness.sync_at(at(0)).await;

    assert_eq!(report.off, 1);
    assert!(harness.lists.fetched.lock().unwrap().is_empty());
    assert_eq!(
        harness.store.subscription("list-a").sync.state,
        ListSyncState::Off
    );
}

#[tokio::test]
async fn a_personal_list_without_an_account_fails_without_fetching() {
    let list = ListSubscription {
        scope: ListScope::Personal,
        credential_id: Some("missing-account".to_string()),
        ..subscription("list-a")
    };
    let harness = Harness::new(vec![list]);
    harness.lists.serve("list-a", &["alpha"]);

    let report = harness.sync_at(at(0)).await;

    assert_eq!(report.failed, 1);
    assert!(harness.lists.fetched.lock().unwrap().is_empty());
    assert!(report.failures[0].contains("owner-one"));
}

#[tokio::test]
async fn a_reused_title_is_in_library_not_added_by_the_list() {
    let mut harness = Harness::new(vec![subscription("list-a")]);
    harness.actions.reuse_titles = true;
    harness.lists.serve("list-a", &["alpha"]);

    harness.sync_at(at(0)).await;

    let row = harness.store.row("list-a", "alpha");
    assert_eq!(row.state, ListMembershipState::InLibrary);
    assert!(!row.added_by_list);
}

#[tokio::test]
async fn a_title_already_in_the_library_is_not_added_again() {
    let mut harness = Harness::new(vec![subscription("list-a")]);
    harness.resolver.in_library =
        HashMap::from([("alpha-id".to_string(), "title-existing".to_string())]);
    harness.lists.serve("list-a", &["alpha"]);

    harness.sync_at(at(0)).await;

    assert!(harness.actions.calls().is_empty());
    let row = harness.store.row("list-a", "alpha");
    assert_eq!(row.state, ListMembershipState::InLibrary);
    assert_eq!(row.title_id.as_deref(), Some("title-existing"));
}

#[tokio::test]
async fn capped_items_are_pending_and_acted_on_in_a_later_sync() {
    let mut list = subscription("list-a");
    list.max_per_sync = Some(1);
    list.interval_seconds = 60;
    let harness = Harness::new(vec![list]);
    harness.lists.serve("list-a", &["alpha", "beta"]);

    harness.sync_at(at(0)).await;
    assert_eq!(
        harness.store.row("list-a", "beta").state,
        ListMembershipState::Pending
    );

    // A changed fingerprint makes the provider return the full list again.
    harness.lists.serve("list-a", &["alpha", "beta", "gamma"]);
    harness.sync_at(at(10)).await;
    assert_eq!(
        harness.store.row("list-a", "beta").state,
        ListMembershipState::Added
    );
    assert_eq!(
        harness.store.row("list-a", "gamma").state,
        ListMembershipState::Pending
    );
}

#[tokio::test]
async fn a_personal_add_list_submits_requests_instead() {
    let list = ListSubscription {
        scope: ListScope::Personal,
        mode: ListMode::Search,
        credential_id: None,
        ..subscription("list-a")
    };
    // Exercised through the act step directly: a personal sync needs a
    // linked account, which the privacy tests cover.
    let actions = RecordingActions::default();
    let outcome = crate::lists::act::act_on_candidate(
        &actions,
        &list,
        &crate::lists::test_support::resolved_item("alpha"),
    )
    .await;
    assert_eq!(outcome.state, ListMembershipState::Requested);
    assert_eq!(
        actions.calls(),
        vec![RecordedAction::Request {
            item_key: "alpha".to_string(),
            hold: false
        }]
    );
}

#[tokio::test]
async fn a_refused_add_stays_pending_or_blocked() {
    let mut harness = Harness::new(vec![subscription("list-a"), subscription("list-b")]);
    harness.actions.refuse_adds = Some(|| AppError::Unauthorized("fixture".into()));
    harness.lists.serve("list-a", &["alpha"]);
    harness.lists.serve("list-b", &["beta"]);

    harness.sync_at(at(0)).await;

    let row = harness.store.row("list-a", "alpha");
    assert_eq!(row.state, ListMembershipState::BlockedPermission);
    assert_eq!(row.state_reason.as_deref(), Some("not_permitted"));
    assert!(!row.added_by_list);
}

#[tokio::test]
async fn an_item_listed_twice_is_acted_on_once() {
    let harness = Harness::new(vec![subscription("list-a")]);
    harness.lists.serve("list-a", &["alpha", "beta", "alpha"]);

    let report = harness.sync_at(at(0)).await;

    assert_eq!(report.added, 2);
    let adds = harness
        .actions
        .calls()
        .into_iter()
        .filter(|call| matches!(call, RecordedAction::Add { .. }))
        .count();
    assert_eq!(adds, 2);
}

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

impl Harness {
    /// Sync one subscription as a pass that read it as `read` would.
    async fn sync_one_at(
        &self,
        read: &ListSubscription,
        now: DateTime<Utc>,
    ) -> SubscriptionSyncOutcome {
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
        sync_subscription(&context, read, now, None)
            .await
            .expect("sync one subscription")
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
async fn a_sync_reports_only_the_adds_it_made() {
    let mut list = subscription("list-a");
    list.interval_seconds = 60;
    let harness = Harness::new(vec![list]);
    harness.lists.serve("list-a", &["alpha", "beta"]);
    harness.sync_at(at(0)).await;
    harness.lists.serve("list-a", &["alpha", "beta", "gamma"]);

    let report = harness.sync_at(at(10)).await;

    assert_eq!(report.synced, 1);
    assert_eq!(report.added, 1, "only gamma was added by this sync");
    assert_eq!(harness.store.subscription("list-a").counts.added, 3);
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

/// A personal Request list with a linked account for its owner.
fn personal_request_harness(policy: Option<ListPolicy>) -> Harness {
    let list = ListSubscription {
        scope: ListScope::Personal,
        mode: ListMode::Request,
        credential_id: Some("account-one".to_string()),
        ..subscription("list-a")
    };
    let harness = Harness::new(vec![list]);
    harness
        .store
        .accounts
        .lock()
        .unwrap()
        .push(scryer_domain::UserListAccount {
            id: "account-one".to_string(),
            user_id: "owner-one".to_string(),
            provider: crate::lists::test_support::PROVIDER.to_string(),
            external_user_id: "external-one".to_string(),
            username: "fixture-member".to_string(),
            display_name: None,
            credential: scryer_domain::ListAccountCredential {
                access_token: "fixture-token".to_string(),
                ..scryer_domain::ListAccountCredential::default()
            },
            status: scryer_domain::UserListAccountStatus::Active,
            error_message: None,
            linked_at: at(0),
            last_used_at: None,
            last_refresh_at: None,
            updated_at: at(0),
        });
    if let Some(policy) = policy {
        harness.store.policies.lock().unwrap().push(UserListPolicy {
            user_id: "owner-one".to_string(),
            policy,
            updated_by_user_id: None,
            updated_at: at(0),
        });
    }
    harness.lists.serve("list-a", &["alpha"]);
    harness
}

fn request_holds(harness: &Harness) -> Vec<bool> {
    harness
        .actions
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            RecordedAction::Request { hold, .. } => Some(hold),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn a_member_whose_list_policy_needs_approval_gets_held_requests() {
    for policy in [None, Some(ListPolicy::Approval)] {
        let harness = personal_request_harness(policy);

        harness.sync_at(at(0)).await;

        assert_eq!(request_holds(&harness), vec![true], "policy {policy:?}");
        assert_eq!(
            harness.store.row("list-a", "alpha").state,
            ListMembershipState::Requested
        );
    }
}

#[tokio::test]
async fn a_member_whose_list_policy_auto_approves_gets_evaluated_requests() {
    let harness = personal_request_harness(Some(ListPolicy::Auto));

    harness.sync_at(at(0)).await;

    assert_eq!(request_holds(&harness), vec![false]);
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
        false,
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

fn add_calls(actions: &RecordingActions) -> usize {
    count_calls(actions, |call| matches!(call, RecordedAction::Add { .. }))
}

fn fetch_count(harness: &Harness) -> usize {
    harness.lists.fetched.lock().unwrap().len()
}

fn hourly_list() -> ListSubscription {
    let mut list = subscription("list-a");
    list.interval_seconds = 60;
    list
}

#[tokio::test]
async fn a_refused_add_settles_and_is_not_retried_while_nothing_changes() {
    type Refusal = (fn() -> AppError, &'static str);
    let refusals: [Refusal; 2] = [
        (
            || AppError::NotFound("fixture root folder".into()),
            "not_found",
        ),
        (
            || AppError::Validation("fixture profile".into()),
            "rejected",
        ),
    ];
    for (refusal, reason) in refusals {
        let mut harness = Harness::new(vec![hourly_list()]);
        harness.actions.refuse_adds = Some(refusal);
        harness.lists.serve("list-a", &["alpha"]);

        harness.sync_at(at(0)).await;
        let row = harness.store.row("list-a", "alpha");
        assert_eq!(row.state, ListMembershipState::Rejected, "{reason}");
        assert_eq!(row.state_reason.as_deref(), Some(reason));

        let repeat = harness.sync_at(at(10)).await;
        assert_eq!(repeat.unchanged, 1, "{reason}: nothing forces a full read");
        assert_eq!(add_calls(&harness.actions), 1, "{reason}: tried once");
        assert_eq!(fetch_count(&harness), 2, "one read per pass, no re-read");
        let row = harness.store.row("list-a", "alpha");
        assert_eq!(row.state, ListMembershipState::Rejected);
        assert_eq!(row.state_reason.as_deref(), Some(reason));
    }
}

#[tokio::test]
async fn a_refused_add_keeps_its_reason_through_a_full_read() {
    let mut harness = Harness::new(vec![hourly_list()]);
    harness.actions.refuse_adds = Some(|| AppError::NotFound("fixture root folder".into()));
    harness.lists.serve("list-a", &["alpha"]);
    harness.sync_at(at(0)).await;

    // A changed fingerprint makes the provider return the full list again;
    // the refused item is kept as it was rather than tried again.
    harness.lists.serve("list-a", &["alpha", "beta"]);
    harness.actions.refuse_adds = None;
    harness.sync_at(at(10)).await;

    let alpha = harness.store.row("list-a", "alpha");
    assert_eq!(alpha.state, ListMembershipState::Rejected);
    assert_eq!(alpha.state_reason.as_deref(), Some("not_found"));
    assert_eq!(
        harness.store.row("list-a", "beta").state,
        ListMembershipState::Added
    );
    assert_eq!(add_calls(&harness.actions), 2, "alpha once, beta once");
}

#[tokio::test]
async fn a_passing_add_failure_stays_pending_and_is_retried() {
    let mut harness = Harness::new(vec![hourly_list()]);
    harness.actions.refuse_adds = Some(|| AppError::Repository("fixture timeout".into()));
    harness.lists.serve("list-a", &["alpha"]);

    harness.sync_at(at(0)).await;
    let row = harness.store.row("list-a", "alpha");
    assert_eq!(row.state, ListMembershipState::Pending);
    assert_eq!(row.state_reason.as_deref(), Some("action_failed"));

    harness.actions.refuse_adds = None;
    let retry = harness.sync_at(at(10)).await;
    assert_eq!(retry.synced, 1, "a pending item forces a full read");
    assert_eq!(
        harness.store.row("list-a", "alpha").state,
        ListMembershipState::Added
    );
    assert_eq!(add_calls(&harness.actions), 2);
}

#[tokio::test]
async fn an_edit_after_the_last_sync_tries_a_refused_add_again() {
    let mut harness = Harness::new(vec![hourly_list()]);
    harness.actions.refuse_adds = Some(|| AppError::NotFound("fixture root folder".into()));
    harness.lists.serve("list-a", &["alpha"]);
    harness.sync_at(at(0)).await;

    // The operator fixes the route the add named.
    harness.actions.refuse_adds = None;
    harness
        .store
        .subscriptions
        .lock()
        .unwrap()
        .iter_mut()
        .find(|row| row.id == "list-a")
        .unwrap()
        .updated_at = at(5);

    let report = harness.sync_at(at(10)).await;
    assert_eq!(report.synced, 1);
    let row = harness.store.row("list-a", "alpha");
    assert_eq!(row.state, ListMembershipState::Added);
    assert_eq!(row.state_reason, None);
    assert_eq!(add_calls(&harness.actions), 2);

    let later = harness.sync_at(at(20)).await;
    assert_eq!(later.unchanged, 1, "the edit is processed once");
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

fn count_calls(actions: &RecordingActions, matches: fn(&RecordedAction) -> bool) -> usize {
    actions.calls().iter().filter(|call| matches(call)).count()
}

#[tokio::test]
async fn an_unchanged_capped_list_still_drains_its_pending_items() {
    let mut list = subscription("list-a");
    list.max_per_sync = Some(1);
    list.interval_seconds = 60;
    let harness = Harness::new(vec![list]);
    harness.lists.serve("list-a", &["alpha", "beta", "gamma"]);

    harness.sync_at(at(0)).await;
    assert_eq!(
        harness.store.row("list-a", "beta").state,
        ListMembershipState::Pending
    );

    // The provider answers "unchanged" from here on; pending items remain, so
    // each sync reads the list again and takes the next capped item.
    let second = harness.sync_at(at(10)).await;
    assert_eq!(second.synced, 1);
    assert_eq!(
        harness.store.row("list-a", "beta").state,
        ListMembershipState::Added
    );
    assert_eq!(
        harness.store.row("list-a", "gamma").state,
        ListMembershipState::Pending
    );

    harness.sync_at(at(20)).await;
    assert_eq!(
        harness.store.row("list-a", "gamma").state,
        ListMembershipState::Added
    );

    let fetches_before = harness.lists.fetched.lock().unwrap().len();
    let drained = harness.sync_at(at(30)).await;
    assert_eq!(drained.unchanged, 1, "nothing is left, so the skip applies");
    assert_eq!(
        harness.lists.fetched.lock().unwrap().len(),
        fetches_before + 1,
        "a drained list is not read a second time"
    );
    assert_eq!(
        count_calls(&harness.actions, |call| matches!(
            call,
            RecordedAction::Add { .. }
        )),
        3
    );
}

#[tokio::test]
async fn an_edited_list_is_processed_even_when_unchanged() {
    let mut list = subscription("list-a");
    list.interval_seconds = 60;
    list.routes = Vec::new();
    let harness = Harness::new(vec![list]);
    harness.lists.serve("list-a", &["alpha"]);

    harness.sync_at(at(0)).await;
    assert_eq!(
        harness.store.row("list-a", "alpha").state,
        ListMembershipState::Filtered,
        "no route for the kind yet"
    );

    // An edit after the last sync adds the missing route.
    {
        let mut subscriptions = harness.store.subscriptions.lock().unwrap();
        let edited = subscriptions
            .iter_mut()
            .find(|row| row.id == "list-a")
            .unwrap();
        edited.routes = vec![crate::lists::test_support::route(
            scryer_domain::MediaFacet::Movie,
            crate::lists::test_support::LIBRARY,
        )];
        edited.updated_at = at(5);
    }

    let report = harness.sync_at(at(10)).await;
    assert_eq!(report.synced, 1);
    assert_eq!(
        harness.store.row("list-a", "alpha").state,
        ListMembershipState::Added
    );

    let later = harness.sync_at(at(20)).await;
    assert_eq!(later.unchanged, 1, "the edit was processed once");
}

#[tokio::test]
async fn an_empty_fetch_marks_nobody_left_and_runs_no_leave_action() {
    let mut list = subscription("list-a");
    list.on_leave = ListOnLeave::Unmonitor;
    list.interval_seconds = 60;
    let harness = Harness::new(vec![list]);
    harness.lists.serve("list-a", &["alpha", "beta"]);
    harness.sync_at(at(0)).await;

    harness.lists.serve("list-a", &[]);
    let report = harness.sync_at(at(10)).await;

    assert_eq!(report.departures_acted, 0);
    for key in ["alpha", "beta"] {
        let row = harness.store.row("list-a", key);
        assert_eq!(row.left_at, None, "{key} is still a member");
        assert_eq!(row.state, ListMembershipState::Added);
    }
    assert_eq!(
        count_calls(&harness.actions, |call| matches!(
            call,
            RecordedAction::SetMonitored { .. } | RecordedAction::Departure { .. }
        )),
        0
    );
    let list = harness.store.subscription("list-a");
    assert_eq!(list.sync.state, ListSyncState::Ok);
    assert_eq!(
        list.counts.added, 2,
        "counts stay as the last sync left them"
    );
    let runs = harness.store.runs.lock().unwrap().clone();
    let last = runs.last().expect("the empty sync has a run");
    assert_eq!(last.outcome, ListSyncRunOutcome::Succeeded);
    assert_eq!(
        last.error_message.as_deref(),
        Some(LIST_SYNC_EMPTY_FETCH_NOTE),
        "the run says departures were skipped"
    );
}

#[tokio::test]
async fn a_storage_failure_on_one_list_does_not_stop_the_next() {
    let harness = Harness::new(vec![subscription("list-a"), subscription("list-b")]);
    harness.lists.serve("list-a", &["alpha"]);
    harness.lists.serve("list-b", &["beta"]);
    harness
        .store
        .fail_upserts_for
        .lock()
        .unwrap()
        .insert("list-a".to_string());

    let report = harness.sync_at(at(0)).await;

    assert_eq!(report.considered, 2);
    assert_eq!(report.failed, 1);
    assert_eq!(report.synced, 1);
    assert_eq!(report.failures.len(), 1);
    assert!(report.failures[0].contains("list-a"));
    assert_eq!(
        harness.store.row("list-b", "beta").state,
        ListMembershipState::Added
    );
    let failed = harness.store.subscription("list-a");
    assert_eq!(failed.sync.state, ListSyncState::Fail);
    assert_eq!(
        failed.sync.error_message.as_deref(),
        Some(LIST_SYNC_STORAGE_FAILURE_MESSAGE)
    );
    let runs = harness.store.runs.lock().unwrap().clone();
    let failed_run = runs
        .iter()
        .find(|run| run.subscription_id == "list-a")
        .expect("the failed list has a run");
    assert_eq!(failed_run.outcome, ListSyncRunOutcome::Failed);
}

#[tokio::test]
async fn a_failed_leave_action_is_retried_while_the_list_is_unchanged() {
    let mut list = subscription("list-a");
    list.on_leave = ListOnLeave::Log;
    list.interval_seconds = 60;
    let harness = Harness::new(vec![list]);
    harness.lists.serve("list-a", &["alpha", "beta"]);
    harness.sync_at(at(0)).await;

    harness.lists.serve("list-a", &["alpha"]);
    *harness.actions.fail_departures.lock().unwrap() = 1;
    harness.sync_at(at(10)).await;
    assert!(
        !harness.store.row("list-a", "beta").left_handled,
        "the failed action waits for a retry"
    );

    // Same list again: the provider says "unchanged", but the retry is owed.
    let report = harness.sync_at(at(20)).await;
    assert_eq!(report.synced, 1);
    assert_eq!(report.departures_acted, 1);
    assert!(harness.store.row("list-a", "beta").left_handled);
    assert_eq!(
        count_calls(&harness.actions, |call| matches!(
            call,
            RecordedAction::Departure { .. }
        )),
        2
    );

    let settled = harness.sync_at(at(30)).await;
    assert_eq!(settled.unchanged, 1, "a handled departure is not retried");
}

#[tokio::test]
async fn a_kept_departure_does_not_defeat_the_unchanged_skip() {
    let mut list = subscription("list-a");
    list.interval_seconds = 60;
    let harness = Harness::new(vec![list]);
    harness.lists.serve("list-a", &["alpha", "beta"]);
    harness.sync_at(at(0)).await;
    harness.lists.serve("list-a", &["alpha"]);
    harness.sync_at(at(10)).await;

    let report = harness.sync_at(at(20)).await;

    assert_eq!(report.unchanged, 1);
}

/// Two lists over one library, both listing "alpha": `list-a` adds it with
/// the given on-leave action, and `list-b` (synced first a minute later)
/// finds it already in the library.
async fn title_on_two_lists(on_leave: ListOnLeave) -> Harness {
    let mut adder = subscription("list-a");
    adder.on_leave = on_leave;
    adder.interval_seconds = 60;
    let mut follower = subscription("list-b");
    follower.interval_seconds = 60;
    follower.sync.next_at = Some(at(5));
    let mut harness = Harness::new(vec![adder, follower]);
    harness.lists.serve("list-a", &["alpha", "beta"]);
    harness.lists.serve("list-b", &["alpha", "gamma"]);
    harness.sync_at(at(0)).await;
    harness.resolver.in_library = HashMap::from([
        ("alpha-id".to_string(), "title-alpha".to_string()),
        ("beta-id".to_string(), "title-beta".to_string()),
    ]);
    harness.sync_at(at(5)).await;
    assert!(harness.store.row("list-a", "alpha").added_by_list);
    let follower_row = harness.store.row("list-b", "alpha");
    assert_eq!(follower_row.state, ListMembershipState::InLibrary);
    assert_eq!(follower_row.title_id.as_deref(), Some("title-alpha"));
    assert!(!follower_row.added_by_list);
    harness
}

fn unmonitors_of(actions: &RecordingActions, title_id: &str) -> usize {
    actions
        .calls()
        .iter()
        .filter(|call| {
            matches!(call, RecordedAction::SetMonitored { title_id: id, .. } if id == title_id)
        })
        .count()
}

#[tokio::test]
async fn the_adding_list_acts_once_when_it_drops_the_title_first() {
    let harness = title_on_two_lists(ListOnLeave::Unmonitor).await;

    // The adding list drops the title while the other list still has it.
    harness.lists.serve("list-a", &["beta"]);
    harness.sync_at(at(10)).await;
    let row = harness.store.row("list-a", "alpha");
    assert!(row.left_at.is_some());
    assert!(!row.left_handled, "the action is owed, not cancelled");
    assert_eq!(unmonitors_of(&harness.actions, "title-alpha"), 0);

    // While the guard holds, neither list is read beyond its one fetch.
    let fetches_before = harness.lists.fetched.lock().unwrap().len();
    let quiet = harness.sync_at(at(20)).await;
    assert_eq!(quiet.unchanged, 2);
    assert_eq!(
        harness.lists.fetched.lock().unwrap().len(),
        fetches_before + 2,
        "a held-back departure does not force a full read"
    );
    assert_eq!(unmonitors_of(&harness.actions, "title-alpha"), 0);

    // The other list drops it too. It did not add the title, so it does
    // nothing itself; the adding list's owed action runs at its next sync.
    harness.lists.serve("list-b", &["gamma"]);
    harness.sync_at(at(30)).await;
    assert!(harness.store.row("list-b", "alpha").left_at.is_some());
    harness.sync_at(at(40)).await;
    assert_eq!(unmonitors_of(&harness.actions, "title-alpha"), 1);
    assert!(harness.store.row("list-a", "alpha").left_handled);

    let settled = harness.sync_at(at(50)).await;
    assert_eq!(settled.unchanged, 2);
    assert_eq!(unmonitors_of(&harness.actions, "title-alpha"), 1);
}

#[tokio::test]
async fn the_adding_list_acts_once_when_the_other_list_drops_the_title_first() {
    let harness = title_on_two_lists(ListOnLeave::Unmonitor).await;

    harness.lists.serve("list-b", &["gamma"]);
    harness.sync_at(at(10)).await;
    assert!(harness.store.row("list-b", "alpha").left_handled);
    assert_eq!(unmonitors_of(&harness.actions, "title-alpha"), 0);

    harness.lists.serve("list-a", &["beta"]);
    harness.sync_at(at(20)).await;
    assert_eq!(unmonitors_of(&harness.actions, "title-alpha"), 1);
    assert!(harness.store.row("list-a", "alpha").left_handled);

    harness.sync_at(at(30)).await;
    assert_eq!(unmonitors_of(&harness.actions, "title-alpha"), 1);
}

#[tokio::test]
async fn a_title_another_list_still_wants_is_left_alone() {
    let harness = title_on_two_lists(ListOnLeave::Tag).await;

    harness.lists.serve("list-a", &["beta"]);
    for minutes in [10, 20, 30] {
        harness.sync_at(at(minutes)).await;
    }

    assert!(
        !harness
            .actions
            .calls()
            .iter()
            .any(|call| matches!(call, RecordedAction::Tag { .. })),
        "no tag while the other list keeps the title"
    );
    assert!(!harness.store.row("list-a", "alpha").left_handled);
}

#[tokio::test]
async fn a_failed_owed_action_is_retried_after_the_guard_lifts() {
    let harness = title_on_two_lists(ListOnLeave::Unmonitor).await;
    harness.lists.serve("list-a", &["beta"]);
    harness.sync_at(at(10)).await;
    harness.lists.serve("list-b", &["gamma"]);
    harness.sync_at(at(20)).await;

    *harness.actions.fail_departures.lock().unwrap() = 1;
    harness.sync_at(at(30)).await;
    assert_eq!(unmonitors_of(&harness.actions, "title-alpha"), 1);
    assert!(
        !harness.store.row("list-a", "alpha").left_handled,
        "the failed action waits for a retry"
    );

    let retry = harness.sync_at(at(40)).await;
    assert_eq!(retry.departures_acted, 1);
    assert_eq!(unmonitors_of(&harness.actions, "title-alpha"), 2);
    assert!(harness.store.row("list-a", "alpha").left_handled);

    harness.sync_at(at(50)).await;
    assert_eq!(unmonitors_of(&harness.actions, "title-alpha"), 2);
}

#[tokio::test]
async fn one_run_syncs_every_due_list_across_batches() {
    let count = LIST_SYNC_BATCH_LIMIT * 2 + 7;
    let ids = (0..count)
        .map(|index| format!("list-{index:03}"))
        .collect::<Vec<_>>();
    let harness = Harness::new(ids.iter().map(|id| subscription(id)).collect());
    for id in &ids {
        harness.lists.serve(id, &["alpha"]);
    }

    let report = harness.sync_at(at(0)).await;

    assert_eq!(report.considered, count as u64);
    assert_eq!(report.synced, count as u64);
    assert_eq!(
        harness.store.runs.lock().unwrap().len(),
        count,
        "each list is synced once"
    );
    assert_eq!(
        harness.sync_at(at(0)).await.considered,
        0,
        "nothing is left due"
    );
}

#[tokio::test]
async fn a_list_that_stays_due_is_tried_once_and_does_not_hide_the_rest() {
    // More stuck lists than one batch, all ahead of the healthy ones.
    let stuck = (0..LIST_SYNC_BATCH_LIMIT + 3)
        .map(|index| format!("stuck-{index:03}"))
        .collect::<Vec<_>>();
    let healthy = (0..5)
        .map(|index| format!("healthy-{index}"))
        .collect::<Vec<_>>();
    let harness = Harness::new(
        stuck
            .iter()
            .chain(healthy.iter())
            .map(|id| subscription(id))
            .collect(),
    );
    for id in stuck.iter().chain(healthy.iter()) {
        harness.lists.serve(id, &["alpha"]);
    }
    harness
        .store
        .fail_record_sync_for
        .lock()
        .unwrap()
        .extend(stuck.iter().cloned());

    let report = harness.sync_at(at(0)).await;

    assert_eq!(report.considered, (stuck.len() + healthy.len()) as u64);
    assert_eq!(report.failed, stuck.len() as u64);
    assert_eq!(report.synced, healthy.len() as u64);
    for id in &healthy {
        assert_eq!(
            harness.store.row(id, "alpha").state,
            ListMembershipState::Added
        );
    }
}

#[tokio::test]
async fn a_sync_requested_while_a_sync_runs_survives_that_sync() {
    let harness = Harness::new(vec![subscription("list-a")]);
    harness.lists.serve("list-a", &["alpha"]);
    harness.sync_at(at(0)).await;
    // The pass reads the list when it is next due...
    let read = harness.store.subscription("list-a");
    assert!(read.sync.fetch_fingerprint.is_some());

    // ...and "sync now" makes it due again and drops the fingerprint before
    // that pass finishes.
    let requested = ListSyncStatus {
        next_at: Some(at(400)),
        fetch_fingerprint: None,
        ..read.sync.clone()
    };
    harness
        .store
        .record_sync("list-a", &requested, &read.counts)
        .await
        .expect("sync now");

    harness.sync_one_at(&read, at(401)).await;

    let list = harness.store.subscription("list-a");
    assert_eq!(list.sync.next_at, Some(at(400)), "the request is still due");
    assert_eq!(list.sync.fetch_fingerprint, None);
    assert_eq!(
        list.sync.last_at,
        Some(at(401)),
        "the finished sync is recorded"
    );
}

#[tokio::test]
async fn a_sync_nobody_raced_moves_the_next_sync_on() {
    let harness = Harness::new(vec![subscription("list-a")]);
    harness.lists.serve("list-a", &["alpha"]);
    harness.sync_at(at(0)).await;
    let read = harness.store.subscription("list-a");

    harness.sync_one_at(&read, at(401)).await;

    let list = harness.store.subscription("list-a");
    assert_eq!(
        list.sync.next_at,
        Some(at(401) + chrono::Duration::hours(6))
    );
}

#[tokio::test]
async fn a_list_disabled_while_it_syncs_takes_no_further_action() {
    let mut list = subscription("list-a");
    list.on_leave = ListOnLeave::Unmonitor;
    let harness = Harness::new(vec![list]);
    harness.lists.serve("list-a", &["alpha"]);
    harness.sync_at(at(0)).await;
    let read = harness.store.subscription("list-a");
    let calls_before = harness.actions.calls().len();
    // Disabled after this sync read the list; alpha left and beta is new.
    for row in harness.store.subscriptions.lock().unwrap().iter_mut() {
        row.enabled = false;
    }
    harness.lists.serve("list-a", &["beta"]);

    harness.sync_one_at(&read, at(10)).await;

    assert_eq!(
        harness.actions.calls().len(),
        calls_before,
        "nothing is added and no on-leave action runs"
    );
    assert_eq!(
        harness.store.row("list-a", "beta").state,
        ListMembershipState::Pending,
        "the new item waits for the list to be enabled again"
    );
    assert!(!harness.store.row("list-a", "alpha").left_handled);
}

#[tokio::test]
async fn a_list_unfollowed_while_it_syncs_takes_no_action() {
    let harness = Harness::new(Vec::new());
    harness.lists.serve("list-a", &["alpha"]);

    let outcome = harness.sync_one_at(&subscription("list-a"), at(0)).await;

    assert_eq!(outcome, SubscriptionSyncOutcome::Off);
    assert!(harness.actions.calls().is_empty());
    assert!(harness.store.rows("list-a").is_empty());
}

#[tokio::test]
async fn an_add_is_recorded_even_when_the_sync_breaks_off_after_it() {
    let harness = Harness::new(vec![subscription("list-a")]);
    harness.lists.serve("list-a", &["alpha", "beta"]);
    // The store breaks after the check made before alpha's add.
    *harness.store.subscription_reads_left.lock().unwrap() = Some(1);

    let report = harness.sync_at(at(0)).await;

    assert_eq!(report.failed, 1);
    let row = harness.store.row("list-a", "alpha");
    assert_eq!(row.state, ListMembershipState::Added);
    assert!(row.added_by_list, "the list's add is remembered");
    assert_eq!(row.title_id.as_deref(), Some("title-alpha"));
}

fn movie_exclusion(key: &str) -> scryer_domain::ListExclusion {
    scryer_domain::ListExclusion {
        id: format!("exclusion-{key}"),
        kind: scryer_domain::MediaFacet::Movie,
        external_ids: vec![crate::lists::test_support::tmdb(&format!("{key}-id"))],
        display_title: format!("Fixture Title {key}"),
        year: None,
        scope: scryer_domain::ListExclusionScope::AllLists,
        created_by_user_id: None,
        created_at: at(0),
    }
}

#[tokio::test]
async fn a_title_deleted_without_an_exclusion_is_added_again_on_the_next_sync() {
    let harness = Harness::new(vec![subscription("list-a")]);
    harness.lists.serve("list-a", &["alpha", "beta"]);
    harness.sync_at(at(0)).await;
    assert_eq!(add_calls(&harness.actions), 2);

    harness
        .actions
        .missing_titles
        .lock()
        .unwrap()
        .insert("title-alpha".to_string());
    // The provider reports the list unchanged; the deleted title is still work.
    let report = harness.sync_at(at(6 * 60)).await;

    let added_again = harness
        .actions
        .calls()
        .into_iter()
        .filter(|call| matches!(call, RecordedAction::Add { item_key, .. } if item_key == "alpha"))
        .count();
    assert_eq!(added_again, 2);
    assert_eq!(add_calls(&harness.actions), 3, "beta is still there");
    assert_eq!(report.added, 1);
    let row = harness.store.row("list-a", "alpha");
    assert_eq!(row.state, ListMembershipState::Added);
    assert_eq!(row.title_id.as_deref(), Some("title-alpha"));
    assert!(row.added_by_list);
}

#[tokio::test]
async fn a_title_deleted_with_an_exclusion_stays_gone() {
    let harness = Harness::new(vec![subscription("list-a")]);
    harness.lists.serve("list-a", &["alpha"]);
    harness.sync_at(at(0)).await;

    harness
        .actions
        .missing_titles
        .lock()
        .unwrap()
        .insert("title-alpha".to_string());
    harness
        .store
        .exclusions
        .lock()
        .unwrap()
        .push(movie_exclusion("alpha"));
    harness.sync_at(at(6 * 60)).await;

    assert_eq!(add_calls(&harness.actions), 1);
    let row = harness.store.row("list-a", "alpha");
    assert_eq!(row.state, ListMembershipState::Excluded);
    assert_eq!(row.title_id, None);
    assert!(
        !row.added_by_list,
        "the list no longer owns a deleted title"
    );
}

#[tokio::test]
async fn a_title_the_list_added_that_is_still_present_is_not_added_twice() {
    let harness = Harness::new(vec![subscription("list-a")]);
    harness.lists.serve("list-a", &["alpha"]);
    harness.sync_at(at(0)).await;

    // Unchanged list, title still in the library: nothing to read again.
    harness.sync_at(at(6 * 60)).await;
    // A changed list reads alpha again; its title is still there.
    harness.lists.serve("list-a", &["alpha", "beta"]);
    harness.sync_at(at(12 * 60)).await;

    let alpha_adds = harness
        .actions
        .calls()
        .into_iter()
        .filter(|call| matches!(call, RecordedAction::Add { item_key, .. } if item_key == "alpha"))
        .count();
    assert_eq!(alpha_adds, 1);
    let row = harness.store.row("list-a", "alpha");
    assert_eq!(row.state, ListMembershipState::Added);
    assert_eq!(row.title_id.as_deref(), Some("title-alpha"));
    assert!(row.added_by_list);
}

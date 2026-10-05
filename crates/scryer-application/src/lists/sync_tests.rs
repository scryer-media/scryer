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
async fn a_personal_failure_reaches_the_owner_event_port() {
    let list = ListSubscription {
        scope: ListScope::Personal,
        credential_id: Some("missing-account".to_string()),
        ..subscription("list-a")
    };
    let harness = Harness::new(vec![list]);

    let report = harness.sync_at(at(0)).await;

    assert_eq!(report.failed, 1);
    assert_eq!(
        sync_failures(&harness.actions),
        vec![RecordedAction::SyncFailure {
            subscription_id: "list-a".into(),
            class: "account_required".into()
        }]
    );
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
    assert_eq!(report.failures, vec!["personal list: account_required"]);
}

#[tokio::test]
async fn global_job_report_redacts_private_associations_and_preserves_owner_status() {
    let mut private = subscription("private-subscription-marker");
    private.scope = ListScope::Personal;
    private.owner_user_id = "private-owner-marker".into();
    private.name = "Private subscription name marker".into();
    private.source.provider = "private-provider-marker".into();
    private.credential_id = Some("private-account-marker".into());
    let public = subscription("public-subscription-marker");
    let harness = Harness::new(vec![private.clone(), public.clone()]);
    harness.lists.fail(
        &public.id,
        PluginError {
            code: PluginErrorCode::Permanent,
            public_message: "fixture provider failure".into(),
            debug_message: None,
            retry_after_seconds: None,
            details: None,
        },
    );

    let report = harness.sync_at(at(0)).await;

    assert_eq!(report.failed, 2);
    let serialized = serde_json::to_string(&report).unwrap();
    for marker in [
        &private.id,
        &private.owner_user_id,
        &private.name,
        &private.source.provider,
        private.credential_id.as_ref().unwrap(),
    ] {
        assert!(
            !serialized.contains(marker),
            "global report exposed a private association"
        );
    }
    assert!(
        report
            .failures
            .contains(&"personal list: account_required".into())
    );
    assert!(
        report.failures.contains(&format!(
            "public list {} ({}): failed",
            public.id, public.source.provider
        )),
        "public reports retain their existing association"
    );
    let saved = harness.store.subscription(&private.id);
    assert_eq!(saved.owner_user_id, private.owner_user_id);
    assert_eq!(saved.sync.state, ListSyncState::Fail);
    assert_eq!(
        saved.sync.error_message.as_deref(),
        Some("This list needs a linked private-provider-marker account.")
    );
    assert!(
        sync_failures(&harness.actions).contains(&RecordedAction::SyncFailure {
            subscription_id: private.id,
            class: "account_required".into(),
        }),
        "the owner event port retains the private failure association"
    );
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
async fn personal_manager_add_preserves_search_intent_and_revoked_grants_request() {
    for (mode, search) in [(ListMode::Add, false), (ListMode::Search, true)] {
        let list = ListSubscription {
            scope: ListScope::Personal,
            mode,
            ..subscription("manager-list")
        };
        let item = crate::lists::test_support::resolved_item("alpha");
        let mut actions = RecordingActions {
            owner_manages_titles: true,
            ..Default::default()
        };
        let outcome = crate::lists::act::act_on_candidate(&actions, &list, &item).await;
        assert_eq!(outcome.state, ListMembershipState::Added);
        assert_eq!(
            actions.calls(),
            vec![RecordedAction::Add {
                item_key: "alpha".into(),
                search
            }]
        );
        actions.owner_manages_titles = false;
        let outcome = crate::lists::act::act_on_candidate(&actions, &list, &item).await;
        assert_eq!(outcome.state, ListMembershipState::Requested);
        assert!(matches!(
            actions.calls().last(),
            Some(RecordedAction::Request { hold: false, .. })
        ));
    }
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

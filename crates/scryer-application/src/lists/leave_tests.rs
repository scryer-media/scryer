use scryer_domain::{ListMembershipState, ListOnLeave, ListSubscription};

use super::*;
use crate::lists::test_support::{
    MemoryListStore, RecordedAction, RecordingActions, at, membership, subscription,
};

/// A row the list added that has left the list and not been handled.
fn departed_added(subscription_id: &str, key: &str) -> scryer_domain::ListMembership {
    let mut row = membership(subscription_id, key, ListMembershipState::Added);
    row.title_id = Some(format!("title-{key}"));
    row.added_by_list = true;
    row.left_at = Some(at(30));
    row
}

#[tokio::test]
async fn keep_records_the_departure_and_does_nothing_else() {
    let list = subscription("list-a");
    let store = MemoryListStore::with_subscriptions(vec![list.clone()]);
    store.insert_rows(vec![departed_added("list-a", "alpha")]);
    let actions = RecordingActions::default();

    let report = handle_departures(&list, &store, &store, &actions)
        .await
        .expect("leave step");

    assert_eq!(report.departed, 1);
    assert_eq!(report.acted, 0);
    assert!(actions.calls().is_empty());
    assert!(store.row("list-a", "alpha").left_handled);
}

#[tokio::test]
async fn unmonitor_acts_only_on_titles_the_list_added() {
    let mut list = subscription("list-a");
    list.on_leave = ListOnLeave::Unmonitor;
    let store = MemoryListStore::with_subscriptions(vec![list.clone()]);
    let mut operator_title = membership("list-a", "beta", ListMembershipState::InLibrary);
    operator_title.title_id = Some("title-beta".to_string());
    operator_title.left_at = Some(at(30));
    store.insert_rows(vec![departed_added("list-a", "alpha"), operator_title]);
    let actions = RecordingActions::default();

    let report = handle_departures(&list, &store, &store, &actions)
        .await
        .expect("leave step");

    assert_eq!(report.acted, 1);
    assert_eq!(
        actions.calls(),
        vec![
            RecordedAction::SetMonitored {
                title_id: "title-alpha".to_string(),
                monitored: false,
            },
            RecordedAction::Departure {
                title_id: "title-alpha".to_string(),
                action: ListOnLeave::Unmonitor,
            },
        ]
    );
    assert!(store.row("list-a", "beta").left_handled);
}

#[tokio::test]
async fn another_enabled_list_on_the_same_library_guards_the_title() {
    let mut list = subscription("list-a");
    list.on_leave = ListOnLeave::Unmonitor;
    let other = subscription("list-b");
    let store = MemoryListStore::with_subscriptions(vec![list.clone(), other]);
    let mut still_listed = membership("list-b", "alpha", ListMembershipState::InLibrary);
    still_listed.title_id = Some("title-alpha".to_string());
    store.insert_rows(vec![departed_added("list-a", "alpha"), still_listed]);
    let actions = RecordingActions::default();

    let report = handle_departures(&list, &store, &store, &actions)
        .await
        .expect("leave step");

    assert_eq!(report.guarded, 1);
    assert!(actions.calls().is_empty());
    assert!(store.row("list-a", "alpha").left_handled);
}

#[tokio::test]
async fn a_disabled_list_does_not_guard_the_title() {
    let mut list = subscription("list-a");
    list.on_leave = ListOnLeave::Tag;
    let mut other = subscription("list-b");
    other.enabled = false;
    let store = MemoryListStore::with_subscriptions(vec![list.clone(), other]);
    let mut still_listed = membership("list-b", "alpha", ListMembershipState::InLibrary);
    still_listed.title_id = Some("title-alpha".to_string());
    store.insert_rows(vec![departed_added("list-a", "alpha"), still_listed]);
    let actions = RecordingActions::default();

    handle_departures(&list, &store, &store, &actions)
        .await
        .expect("leave step");

    assert_eq!(
        actions.calls(),
        vec![
            RecordedAction::Tag {
                title_id: "title-alpha".to_string(),
                tag: LEFT_LIST_TAG.to_string(),
            },
            RecordedAction::Departure {
                title_id: "title-alpha".to_string(),
                action: ListOnLeave::Tag,
            },
        ]
    );
}

#[tokio::test]
async fn a_failed_action_stays_unhandled_and_is_retried() {
    let mut list = subscription("list-a");
    list.on_leave = ListOnLeave::Log;
    let store = MemoryListStore::with_subscriptions(vec![list.clone()]);
    store.insert_rows(vec![departed_added("list-a", "alpha")]);
    let actions = RecordingActions {
        fail_departures: std::sync::Mutex::new(1),
        ..RecordingActions::default()
    };

    let first = handle_departures(&list, &store, &store, &actions)
        .await
        .expect("leave step");
    assert_eq!(first.failed, 1);
    assert!(!store.row("list-a", "alpha").left_handled);

    let second = handle_departures(&list, &store, &store, &actions)
        .await
        .expect("leave step");
    assert_eq!(second.acted, 1);
    assert!(store.row("list-a", "alpha").left_handled);

    let third = handle_departures(&list, &store, &store, &actions)
        .await
        .expect("leave step");
    assert_eq!(
        third.departed, 0,
        "a handled departure is not acted on twice"
    );
    assert_eq!(actions.calls().len(), 2);
}

#[tokio::test]
async fn log_records_the_departure_as_its_whole_action() {
    let mut list = subscription("list-a");
    list.on_leave = ListOnLeave::Log;
    let store = MemoryListStore::with_subscriptions(vec![list.clone()]);
    store.insert_rows(vec![departed_added("list-a", "alpha")]);
    let actions = RecordingActions::default();

    let report = handle_departures(&list, &store, &store, &actions)
        .await
        .expect("leave step");

    assert_eq!(report.acted, 1);
    assert_eq!(
        actions.calls(),
        vec![RecordedAction::Departure {
            title_id: "title-alpha".to_string(),
            action: ListOnLeave::Log,
        }]
    );
    assert!(store.row("list-a", "alpha").left_handled);
}

#[tokio::test]
async fn a_failed_departure_record_does_not_repeat_the_tag() {
    let mut list = subscription("list-a");
    list.on_leave = ListOnLeave::Tag;
    let store = MemoryListStore::with_subscriptions(vec![list.clone()]);
    store.insert_rows(vec![departed_added("list-a", "alpha")]);
    let actions = RecordingActions::default();
    // Let the tag succeed, then fail the record that follows it.
    let recording = FailingDepartureRecord(actions);

    let report = handle_departures(&list, &store, &store, &recording)
        .await
        .expect("leave step");

    assert_eq!(report.acted, 1, "the tag ran, so the row is handled");
    assert_eq!(report.failed, 0);
    assert!(store.row("list-a", "alpha").left_handled);
}

/// Passes every call through, failing only the departure record.
struct FailingDepartureRecord(RecordingActions);

#[async_trait::async_trait]
impl ListActions for FailingDepartureRecord {
    async fn add_title(
        &self,
        subscription: &ListSubscription,
        route: &scryer_domain::ListRoute,
        item: &crate::lists::resolve::ResolvedItem,
        search: bool,
    ) -> crate::AppResult<crate::lists::act::AddedTitle> {
        self.0.add_title(subscription, route, item, search).await
    }

    async fn submit_request(
        &self,
        subscription: &ListSubscription,
        route: &scryer_domain::ListRoute,
        item: &crate::lists::resolve::ResolvedItem,
        hold: bool,
    ) -> crate::AppResult<String> {
        self.0.submit_request(subscription, route, item, hold).await
    }

    async fn set_title_monitored(&self, title_id: &str, monitored: bool) -> crate::AppResult<()> {
        self.0.set_title_monitored(title_id, monitored).await
    }

    async fn tag_title(&self, title_id: &str, tag: &str) -> crate::AppResult<()> {
        self.0.tag_title(title_id, tag).await
    }

    async fn record_departure(
        &self,
        _subscription: &ListSubscription,
        _title_id: &str,
        _action: ListOnLeave,
    ) -> crate::AppResult<()> {
        Err(crate::AppError::Repository("fixture failure".into()))
    }

    async fn record_sync_failure(
        &self,
        subscription: &ListSubscription,
        failure: &crate::lists::fetch::ListFailure,
    ) -> crate::AppResult<()> {
        self.0.record_sync_failure(subscription, failure).await
    }
}

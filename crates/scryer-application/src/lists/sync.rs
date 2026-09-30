//! Sync: one scheduled pass over every due subscription.
//!
//! Per subscription the order is fixed: fetch, resolve, evaluate, act, handle
//! departures, persist. The isolation rule follows the media-server signal
//! sweep: a fetch or resolve failure records `fail` with a plain-words message
//! and stops there. Departures are computed only from a list that was
//! actually read, so a broken provider can never make every title "leave",
//! and a fetch that returns no items skips departures entirely.
//! One subscription failing never stops the next one.
//!
//! A provider's "unchanged" answer skips the rest of the sync, unless the list
//! still has work: settings edited since its last sync, items the per-sync cap
//! left pending, a title the list added that has since been deleted, or an
//! on-leave action that has not run. Then the whole list is read again and
//! processed.
//!
//! Subscriptions run one after another, so an instance never has two fetches
//! in flight against the same provider.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Duration, Utc};
use scryer_domain::{
    ListCounts, ListMembership, ListMembershipState, ListPolicy, ListScope, ListSubscription,
    ListSyncRun, ListSyncRunOutcome, ListSyncState, ListSyncStatus,
};
use serde::Serialize;

use super::act::{ListActions, act_on_candidate};
use super::evaluate::{ItemDecision, count_states, edited_since_last_sync, evaluate};
use super::fetch::{ListChartSource, ListFailure, ListFailureClass, fetch_list};
use super::leave::{LeaveReport, handle_departures, has_runnable_leave_action};
use super::plugin::ListPluginProvider;
use super::ports::{
    ListExclusionRepository, ListMembershipRepository, ListSubscriptionRepository,
    UserListAccountRepository, UserListPolicyRepository,
};
use super::privacy::{credential_for, job_failure_label};
use super::provider_settings::ListProviderConfigs;
use super::resolve::{ListItemResolver, resolve_items};
use crate::AppResult;

/// How many due subscriptions one read of the due set takes on. A run keeps
/// reading batches until nothing it has not yet tried is due.
pub const LIST_SYNC_BATCH_LIMIT: usize = 50;

/// Shown on a sync whose fetch came back with no items. An empty answer is
/// not evidence that anything left the list, so nothing was marked as leaving.
pub const LIST_SYNC_EMPTY_FETCH_NOTE: &str =
    "The provider returned no items, so no title was treated as having left the list.";

/// Shown on a list whose sync stopped on a storage error.
pub const LIST_SYNC_STORAGE_FAILURE_MESSAGE: &str =
    "Scryer could not save this list's sync. It will retry at the next interval.";

/// The longest a provider's `Retry-After` may pause a subscription.
pub const MAX_RATE_LIMIT_PAUSE_SECONDS: i64 = 24 * 3600;

/// Everything one sync reads and writes through.
pub struct ListSyncContext<'a> {
    pub subscriptions: &'a dyn ListSubscriptionRepository,
    pub memberships: &'a dyn ListMembershipRepository,
    pub exclusions: &'a dyn ListExclusionRepository,
    pub accounts: &'a dyn UserListAccountRepository,
    pub policies: &'a dyn UserListPolicyRepository,
    pub plugins: &'a dyn ListPluginProvider,
    pub charts: &'a dyn ListChartSource,
    pub resolver: &'a dyn ListItemResolver,
    pub actions: &'a dyn ListActions,
    /// Each provider's server-wide values, read once per pass.
    pub provider_configs: &'a ListProviderConfigs,
}

/// The outcome of one subscription's sync.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SubscriptionSyncOutcome {
    Synced {
        /// The outcomes of the adds and requests this sync made. Items an
        /// earlier sync settled are not in it.
        acted: ListCounts,
        departures_acted: u64,
    },
    Unchanged,
    /// The owner's list policy is off and nothing was read, or the list was
    /// unfollowed while it synced and nothing more was done.
    Off,
    Failed(ListFailure),
}

/// Aggregate outcome of one job run. Personal failures are named by owner,
/// provider and error class only.
#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
pub struct ListSyncReport {
    pub considered: u64,
    pub synced: u64,
    pub unchanged: u64,
    pub off: u64,
    pub failed: u64,
    pub added: u64,
    pub requested: u64,
    pub held: u64,
    pub departures_acted: u64,
    pub failures: Vec<String>,
}

pub fn list_sync_summary(report: &ListSyncReport) -> String {
    format!(
        "{} list(s) considered: {} synced, {} unchanged, {} off, {} failed; {} added, {} requested, {} held",
        report.considered,
        report.synced,
        report.unchanged,
        report.off,
        report.failed,
        report.added,
        report.requested,
        report.held
    )
}

pub fn log_list_sync_report(report: &ListSyncReport) {
    tracing::info!(
        considered = report.considered,
        synced = report.synced,
        unchanged = report.unchanged,
        off = report.off,
        failed = report.failed,
        added = report.added,
        requested = report.requested,
        held = report.held,
        departures_acted = report.departures_acted,
        "list sync finished"
    );
}

/// Sync every subscription due at `now`. Only reading the due set can fail the
/// pass; any error inside one subscription's sync is counted as that
/// subscription's failure and the pass continues.
///
/// The due set is read in batches until it holds nothing this run has not
/// tried. Every outcome moves a list's next sync past `now`, so a synced list
/// leaves the due set; one whose outcome could not be saved stays due and is
/// tried once per run only. Each read asks for that many more rows, so lists
/// stuck at the front of the due set never hide the ones behind them.
pub async fn sync_due_subscriptions(
    context: &ListSyncContext<'_>,
    now: DateTime<Utc>,
    job_run_id: Option<String>,
) -> AppResult<ListSyncReport> {
    let mut report = ListSyncReport::default();
    let mut tried = HashSet::new();
    loop {
        let due = context
            .subscriptions
            .list_due(now, tried.len().saturating_add(LIST_SYNC_BATCH_LIMIT))
            .await?
            .into_iter()
            .filter(|subscription| !tried.contains(&subscription.id))
            .take(LIST_SYNC_BATCH_LIMIT)
            .collect::<Vec<_>>();
        if due.is_empty() {
            return Ok(report);
        }
        for subscription in due {
            tried.insert(subscription.id.clone());
            sync_one_due(context, &subscription, now, job_run_id.clone(), &mut report).await;
        }
    }
}

async fn sync_one_due(
    context: &ListSyncContext<'_>,
    subscription: &ListSubscription,
    now: DateTime<Utc>,
    job_run_id: Option<String>,
    report: &mut ListSyncReport,
) {
    report.considered += 1;
    let outcome = match sync_subscription(context, subscription, now, job_run_id.clone()).await {
        Ok(outcome) => outcome,
        // A store error on one list is that list's failure: it is
        // recorded against the list and the pass moves on.
        Err(error) => {
            tracing::warn!(
                subscription_id = %subscription.id,
                error = %error,
                "list sync failed on a storage error; continuing with the next list"
            );
            record_storage_failure(context, subscription, now, job_run_id.clone()).await
        }
    };
    match outcome {
        SubscriptionSyncOutcome::Synced {
            acted,
            departures_acted,
        } => {
            report.synced += 1;
            report.added += acted.added;
            report.requested += acted.requested;
            report.held += acted.held;
            report.departures_acted += departures_acted;
        }
        SubscriptionSyncOutcome::Unchanged => report.unchanged += 1,
        SubscriptionSyncOutcome::Off => report.off += 1,
        SubscriptionSyncOutcome::Failed(failure) => {
            report.failed += 1;
            report
                .failures
                .push(job_failure_label(subscription, &failure));
        }
    }
}

/// Sync one subscription. Repository errors propagate; provider, gateway and
/// action failures are recorded on the subscription and returned as an
/// outcome.
pub async fn sync_subscription(
    context: &ListSyncContext<'_>,
    subscription: &ListSubscription,
    now: DateTime<Utc>,
    job_run_id: Option<String>,
) -> AppResult<SubscriptionSyncOutcome> {
    let mut run = ListSyncRun::started(subscription.id.clone(), job_run_id);
    run.started_at = now;

    let owner_policy = match subscription.scope {
        ListScope::Personal => Some(
            context
                .policies
                .get(&subscription.owner_user_id)
                .await?
                .map(|policy| policy.policy)
                .unwrap_or_default(),
        ),
        ListScope::Public => None,
    };
    if owner_policy == Some(ListPolicy::None) {
        let status = ListSyncStatus {
            state: ListSyncState::Off,
            next_at: Some(next_sync_at(subscription, now)),
            ..subscription.sync.clone()
        };
        context
            .subscriptions
            .record_sync_outcome(
                &subscription.id,
                &subscription.sync,
                &status,
                &subscription.counts,
            )
            .await?;
        run.outcome = ListSyncRunOutcome::Skipped;
        run.counts = subscription.counts;
        finish_run(context, run, now).await?;
        return Ok(SubscriptionSyncOutcome::Off);
    }

    let credential = match subscription.scope {
        ListScope::Personal => {
            let account = match &subscription.credential_id {
                Some(account_id) => context.accounts.get_by_id(account_id).await?,
                None => None,
            };
            match credential_for(subscription, account.as_ref()) {
                Ok(credential) => Some(credential),
                Err(failure) => {
                    return record_failure(context, subscription, run, now, failure).await;
                }
            }
        }
        ListScope::Public => None,
    };

    let config = context
        .provider_configs
        .for_provider(context.plugins, &subscription.source.provider);
    let mut fetched = match fetch_list(
        subscription,
        context.plugins,
        context.charts,
        credential.clone(),
        &config,
    )
    .await
    {
        Ok(fetched) => fetched,
        Err(failure) => return record_failure(context, subscription, run, now, failure).await,
    };

    if fetched.unchanged {
        if !has_unfinished_work(context, subscription).await? {
            return record_unchanged(context, subscription, run, now, fetched.fingerprint).await;
        }
        // The provider's "unchanged" carries no items. Work is still left, so
        // read the whole list again rather than treat it as empty.
        let mut whole = subscription.clone();
        whole.sync.fetch_fingerprint = None;
        fetched = match fetch_list(&whole, context.plugins, context.charts, credential, &config)
            .await
        {
            Ok(fetched) => fetched,
            Err(failure) => return record_failure(context, subscription, run, now, failure).await,
        };
        if fetched.unchanged {
            return record_unchanged(context, subscription, run, now, fetched.fingerprint).await;
        }
    }

    // The same item listed twice keeps its first position only, and is acted
    // on once.
    fetched.dedupe();
    if fetched.items.is_empty() {
        return record_empty_fetch(context, subscription, run, now, fetched.fingerprint).await;
    }
    let resolved = match resolve_items(subscription, fetched.items, context.resolver).await {
        Ok(resolved) => resolved,
        Err(_) => {
            let failure = ListFailure::new(ListFailureClass::Unavailable, "The metadata service");
            return record_failure(context, subscription, run, now, failure).await;
        }
    };

    let existing = context
        .memberships
        .list_by_subscription(&subscription.id)
        .await?
        .into_iter()
        .map(|row| (row.item_key.clone(), row))
        .collect::<HashMap<_, _>>();
    let exclusions = context.exclusions.list().await?;
    // A title the list added and that has since been deleted from the library
    // is weighed again as if new; only an exclusion keeps it out.
    let deleted = deleted_additions(context.actions, &resolved, &existing).await;
    let evaluated = evaluate(
        subscription,
        resolved,
        &exclusions,
        &without_rows(&existing, &deleted),
    );

    // A member whose list policy needs approval gets requests that wait for
    // review, whatever their grants and the request rules would allow.
    let hold_requests = owner_policy == Some(ListPolicy::Approval);
    let mut rows = Vec::with_capacity(evaluated.len());
    let mut acted_states = Vec::new();
    // Set once the list is found disabled mid-sync: no further add, request
    // or on-leave action runs for it.
    let mut stopped = false;
    for evaluated in evaluated {
        let previous = existing.get(&evaluated.item.item.item_key);
        let mut row = membership_row(subscription, &evaluated.item, previous, now);
        if deleted.contains(&row.item_key) {
            // The title the list added is gone, so the row no longer names it
            // and the list no longer owns anything through it.
            row.title_id = None;
            row.added_by_list = false;
        }
        match evaluated.decision {
            ItemDecision::Excluded => row.state = ListMembershipState::Excluded,
            ItemDecision::Filtered { reason } => {
                row.state = ListMembershipState::Filtered;
                row.state_reason = Some(reason);
            }
            ItemDecision::InLibrary { title_id } => {
                row.state = ListMembershipState::InLibrary;
                row.title_id = Some(title_id);
            }
            ItemDecision::Keep { state } => {
                row.state = state;
                // The reason belongs to the settled outcome and stays with
                // it; evaluation reads it to tell a refused add from a
                // rejected request.
                row.state_reason = previous.and_then(|row| row.state_reason.clone());
            }
            ItemDecision::Unresolved => row.state = ListMembershipState::Unresolved,
            ItemDecision::Deferred => row.state = ListMembershipState::Pending,
            ItemDecision::Candidate => {
                if !stopped {
                    match subscription_standing(context, &subscription.id).await? {
                        SubscriptionStanding::Active => {}
                        SubscriptionStanding::Disabled => stopped = true,
                        SubscriptionStanding::Gone => return Ok(SubscriptionSyncOutcome::Off),
                    }
                }
                if stopped {
                    // Left for the sync after the list is enabled again.
                    row.state = ListMembershipState::Pending;
                    rows.push(row);
                    continue;
                }
                let outcome = act_on_candidate(
                    context.actions,
                    subscription,
                    &evaluated.item,
                    hold_requests,
                )
                .await;
                acted_states.push(outcome.state);
                row.state = outcome.state;
                row.state_reason = outcome.reason;
                row.added_by_list |= outcome.added_by_list;
                if outcome.title_id.is_some() {
                    row.title_id = outcome.title_id;
                }
                if outcome.request_id.is_some() {
                    row.request_id = outcome.request_id;
                }
                // The action is done; record what it did before acting on the
                // next item, so a sync cut short keeps `added_by_list` and
                // the title and request it names.
                context
                    .memberships
                    .upsert_many(std::slice::from_ref(&row))
                    .await?;
            }
        }
        rows.push(row);
    }

    if !rows.is_empty() {
        context.memberships.upsert_many(&rows).await?;
    }
    // Every present row was just stamped `last_seen_at = now`; anything older
    // left the list in this sync.
    context
        .memberships
        .mark_left(&subscription.id, now, now)
        .await?;
    if !stopped {
        match subscription_standing(context, &subscription.id).await? {
            SubscriptionStanding::Active => {}
            SubscriptionStanding::Disabled => stopped = true,
            SubscriptionStanding::Gone => return Ok(SubscriptionSyncOutcome::Off),
        }
    }
    let leave = if stopped {
        LeaveReport::default()
    } else {
        handle_departures(
            subscription,
            context.memberships,
            context.subscriptions,
            context.actions,
        )
        .await?
    };

    let counts = count_states(rows.iter().map(|row| row.state));
    let status = ListSyncStatus {
        state: ListSyncState::Ok,
        last_at: Some(now),
        next_at: Some(next_sync_at(subscription, now)),
        error_message: None,
        error_at: None,
        paused_until: None,
        fetch_fingerprint: fetched.fingerprint,
    };
    context
        .subscriptions
        .record_sync_outcome(&subscription.id, &subscription.sync, &status, &counts)
        .await?;
    run.counts = counts;
    finish_run(context, run, now).await?;
    Ok(SubscriptionSyncOutcome::Synced {
        acted: count_states(acted_states),
        departures_acted: leave.acted,
    })
}

/// The item keys whose membership says the list added a title that is no
/// longer in the library. Only rows still on the list and marked `Added` are
/// checked, and only when the item did not already match that same title. A
/// lookup that fails counts as the title being present, so a passing storage
/// error never re-adds anything.
pub(super) async fn deleted_additions(
    actions: &dyn ListActions,
    items: &[super::resolve::ResolvedItem],
    existing: &HashMap<String, ListMembership>,
) -> HashSet<String> {
    let mut deleted = HashSet::new();
    for item in items {
        let Some(row) = existing.get(&item.item.item_key) else {
            continue;
        };
        if row.left_at.is_some() || row.state != ListMembershipState::Added {
            continue;
        }
        let Some(title_id) = row.title_id.as_deref() else {
            continue;
        };
        if item.library_title_id.as_deref() == Some(title_id) {
            continue;
        }
        if !actions.title_exists(title_id).await.unwrap_or(true) {
            deleted.insert(row.item_key.clone());
        }
    }
    deleted
}

/// `existing` without the rows named in `keys`.
pub(super) fn without_rows(
    existing: &HashMap<String, ListMembership>,
    keys: &HashSet<String>,
) -> HashMap<String, ListMembership> {
    existing
        .iter()
        .filter(|(key, _)| !keys.contains(*key))
        .map(|(key, row)| (key.clone(), row.clone()))
        .collect()
}

/// Whether a list may still act, read again from the store: a sync runs long
/// enough for its list to be disabled or unfollowed underneath it.
enum SubscriptionStanding {
    Active,
    Disabled,
    Gone,
}

async fn subscription_standing(
    context: &ListSyncContext<'_>,
    subscription_id: &str,
) -> AppResult<SubscriptionStanding> {
    Ok(
        match context.subscriptions.get_by_id(subscription_id).await? {
            Some(current) if current.enabled => SubscriptionStanding::Active,
            Some(_) => SubscriptionStanding::Disabled,
            None => SubscriptionStanding::Gone,
        },
    )
}

fn membership_row(
    subscription: &ListSubscription,
    item: &super::resolve::ResolvedItem,
    previous: Option<&ListMembership>,
    now: DateTime<Utc>,
) -> ListMembership {
    // A row that left and came back starts over: its earlier outcome belonged
    // to the earlier appearance.
    let carried = previous.filter(|row| row.left_at.is_none());
    ListMembership {
        subscription_id: subscription.id.clone(),
        item_key: item.item.item_key.clone(),
        rank: item.item.rank.map(i64::from),
        season: item.item.season,
        display_title: item
            .item
            .title
            .as_deref()
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .map(str::to_string),
        year: item.item.year,
        external_ids: item.external_ids.clone(),
        smg_title_id: item.smg_title_id,
        title_id: carried.and_then(|row| row.title_id.clone()),
        request_id: carried.and_then(|row| row.request_id.clone()),
        kind: item
            .kind
            .clone()
            .or_else(|| subscription.kinds.first().cloned())
            .unwrap_or_default(),
        state: ListMembershipState::Unresolved,
        state_reason: None,
        added_by_list: previous.is_some_and(|row| row.added_by_list),
        first_seen_at: previous.map(|row| row.first_seen_at).unwrap_or(now),
        last_seen_at: now,
        left_at: None,
        left_handled: false,
    }
}

fn next_sync_at(subscription: &ListSubscription, now: DateTime<Utc>) -> DateTime<Utc> {
    now + Duration::seconds(subscription.interval_seconds.max(60))
}

/// Whether a list the provider reports as unchanged still has work a sync
/// must do: settings edited since its last sync, items the per-sync cap left
/// pending, a title the list added that has since been deleted, or a
/// departure whose on-leave action has not run and could run now. A departure
/// another list still holds back is not work yet.
async fn has_unfinished_work(
    context: &ListSyncContext<'_>,
    subscription: &ListSubscription,
) -> AppResult<bool> {
    if edited_since_last_sync(subscription) {
        return Ok(true);
    }
    let rows = context
        .memberships
        .list_by_subscription(&subscription.id)
        .await?;
    if rows
        .iter()
        .any(|row| row.left_at.is_none() && row.state == ListMembershipState::Pending)
    {
        return Ok(true);
    }
    // A title the list added was deleted from the library: the next sync
    // weighs it again, so the list must be read.
    for row in &rows {
        if row.left_at.is_none()
            && row.state == ListMembershipState::Added
            && let Some(title_id) = row.title_id.as_deref()
            && !context.actions.title_exists(title_id).await.unwrap_or(true)
        {
            return Ok(true);
        }
    }
    has_runnable_leave_action(
        subscription,
        &rows,
        context.memberships,
        context.subscriptions,
    )
    .await
}

/// Record a sync the provider answered with "unchanged": only the timestamps
/// move.
async fn record_unchanged(
    context: &ListSyncContext<'_>,
    subscription: &ListSubscription,
    mut run: ListSyncRun,
    now: DateTime<Utc>,
    fingerprint: Option<String>,
) -> AppResult<SubscriptionSyncOutcome> {
    let status = ListSyncStatus {
        state: ListSyncState::Ok,
        last_at: Some(now),
        next_at: Some(next_sync_at(subscription, now)),
        error_message: None,
        error_at: None,
        paused_until: None,
        fetch_fingerprint: fingerprint.or_else(|| subscription.sync.fetch_fingerprint.clone()),
    };
    context
        .subscriptions
        .record_sync_outcome(
            &subscription.id,
            &subscription.sync,
            &status,
            &subscription.counts,
        )
        .await?;
    run.counts = subscription.counts;
    finish_run(context, run, now).await?;
    Ok(SubscriptionSyncOutcome::Unchanged)
}

/// Record a fetch that returned no items. Departures are skipped entirely:
/// no member is marked as having left and no on-leave action runs, and the
/// run says so. Memberships and counts stay as the last sync left them.
async fn record_empty_fetch(
    context: &ListSyncContext<'_>,
    subscription: &ListSubscription,
    mut run: ListSyncRun,
    now: DateTime<Utc>,
    fingerprint: Option<String>,
) -> AppResult<SubscriptionSyncOutcome> {
    let status = ListSyncStatus {
        state: ListSyncState::Ok,
        last_at: Some(now),
        next_at: Some(next_sync_at(subscription, now)),
        error_message: None,
        error_at: None,
        paused_until: None,
        fetch_fingerprint: fingerprint,
    };
    context
        .subscriptions
        .record_sync_outcome(
            &subscription.id,
            &subscription.sync,
            &status,
            &subscription.counts,
        )
        .await?;
    run.counts = subscription.counts;
    run.error_message = Some(LIST_SYNC_EMPTY_FETCH_NOTE.to_string());
    finish_run(context, run, now).await?;
    Ok(SubscriptionSyncOutcome::Synced {
        acted: ListCounts::default(),
        departures_acted: 0,
    })
}

/// Record a sync that stopped on a storage error, as far as the store still
/// allows. Both writes are best effort: the store that just failed may fail
/// again, and the pass must go on either way.
async fn record_storage_failure(
    context: &ListSyncContext<'_>,
    subscription: &ListSubscription,
    now: DateTime<Utc>,
    job_run_id: Option<String>,
) -> SubscriptionSyncOutcome {
    let status = ListSyncStatus {
        state: ListSyncState::Fail,
        error_message: Some(LIST_SYNC_STORAGE_FAILURE_MESSAGE.to_string()),
        error_at: Some(now),
        next_at: Some(next_sync_at(subscription, now)),
        ..subscription.sync.clone()
    };
    if let Err(error) = context
        .subscriptions
        .record_sync_outcome(
            &subscription.id,
            &subscription.sync,
            &status,
            &subscription.counts,
        )
        .await
    {
        tracing::warn!(
            subscription_id = %subscription.id,
            error = %error,
            "could not record a list sync storage failure"
        );
    }
    let mut run = ListSyncRun::started(subscription.id.clone(), job_run_id);
    run.started_at = now;
    run.outcome = ListSyncRunOutcome::Failed;
    run.counts = subscription.counts;
    run.error_message = Some(LIST_SYNC_STORAGE_FAILURE_MESSAGE.to_string());
    if let Err(error) = finish_run(context, run, now).await {
        tracing::warn!(
            subscription_id = %subscription.id,
            error = %error,
            "could not record a failed list sync run"
        );
    }
    SubscriptionSyncOutcome::Failed(ListFailure {
        class: ListFailureClass::Failed,
        message: LIST_SYNC_STORAGE_FAILURE_MESSAGE.to_string(),
    })
}

/// Record a failed fetch or resolve. Counts, memberships and titles are left
/// exactly as the last good sync wrote them.
async fn record_failure(
    context: &ListSyncContext<'_>,
    subscription: &ListSubscription,
    mut run: ListSyncRun,
    now: DateTime<Utc>,
    failure: ListFailure,
) -> AppResult<SubscriptionSyncOutcome> {
    let next_at = next_sync_at(subscription, now);
    let paused_until = match failure.class {
        ListFailureClass::RateLimited {
            retry_after_seconds: Some(seconds),
        } => {
            let retry_at = now
                + Duration::seconds(
                    i64::try_from(seconds)
                        .unwrap_or(i64::MAX)
                        .min(MAX_RATE_LIMIT_PAUSE_SECONDS),
                );
            (retry_at > next_at).then_some(retry_at)
        }
        _ => None,
    };
    let status = ListSyncStatus {
        state: ListSyncState::Fail,
        error_message: Some(failure.message.clone()),
        error_at: Some(now),
        next_at: Some(paused_until.unwrap_or(next_at)),
        paused_until,
        ..subscription.sync.clone()
    };
    context
        .subscriptions
        .record_sync_outcome(
            &subscription.id,
            &subscription.sync,
            &status,
            &subscription.counts,
        )
        .await?;
    run.outcome = ListSyncRunOutcome::Failed;
    run.counts = subscription.counts;
    run.error_message = Some(failure.message.clone());
    finish_run(context, run, now).await?;

    // Tell the operator once per failure, not once per retry. Personal
    // failures surface only in the member's own view.
    let newly_failing = subscription.sync.state != ListSyncState::Fail
        || subscription.sync.error_message.as_deref() != Some(failure.message.as_str());
    if newly_failing
        && !subscription.is_personal()
        && let Err(error) = context
            .actions
            .record_sync_failure(subscription, &failure)
            .await
    {
        tracing::warn!(
            subscription_id = %subscription.id,
            error = %error,
            "could not record a list sync failure"
        );
    }
    Ok(SubscriptionSyncOutcome::Failed(failure))
}

async fn finish_run(
    context: &ListSyncContext<'_>,
    mut run: ListSyncRun,
    now: DateTime<Utc>,
) -> AppResult<()> {
    run.finished_at = Some(now.max(Utc::now()));
    context.subscriptions.record_sync_run(run).await?;
    Ok(())
}

#[cfg(test)]
#[path = "sync_tests.rs"]
mod tests;

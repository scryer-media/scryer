//! Sync: one scheduled pass over every due subscription.
//!
//! Per subscription the order is fixed: fetch, resolve, evaluate, act, handle
//! departures, persist. The isolation rule follows the media-server signal
//! sweep: a fetch or resolve failure records `fail` with a plain-words message
//! and stops there. Departures are computed only from a list that was
//! actually read, so a broken provider can never make every title "leave".
//! One subscription failing never stops the next one.
//!
//! Subscriptions run one after another, so an instance never has two fetches
//! in flight against the same provider.

use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use scryer_domain::{
    ListCounts, ListMembership, ListMembershipState, ListPolicy, ListScope, ListSubscription,
    ListSyncRun, ListSyncRunOutcome, ListSyncState, ListSyncStatus,
};
use serde::Serialize;

use super::act::{ListActions, act_on_candidate};
use super::evaluate::{ItemDecision, count_states, evaluate};
use super::fetch::{ListChartSource, ListFailure, ListFailureClass, fetch_list};
use super::leave::handle_departures;
use super::plugin::ListPluginProvider;
use super::ports::{
    ListExclusionRepository, ListMembershipRepository, ListSubscriptionRepository,
    UserListAccountRepository, UserListPolicyRepository,
};
use super::privacy::{credential_for, job_failure_label};
use super::provider_settings::ListProviderConfigs;
use super::resolve::{ListItemResolver, resolve_items};
use crate::AppResult;

/// How many due subscriptions one tick takes on.
pub const LIST_SYNC_BATCH_LIMIT: usize = 50;

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
        counts: ListCounts,
        departures_acted: u64,
    },
    Unchanged,
    /// The owner's list policy is off; nothing was read.
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

/// Sync every subscription due at `now`.
pub async fn sync_due_subscriptions(
    context: &ListSyncContext<'_>,
    now: DateTime<Utc>,
    job_run_id: Option<String>,
) -> AppResult<ListSyncReport> {
    let due = context
        .subscriptions
        .list_due(now, LIST_SYNC_BATCH_LIMIT)
        .await?;
    let mut report = ListSyncReport::default();
    for subscription in due {
        report.considered += 1;
        let outcome = sync_subscription(context, &subscription, now, job_run_id.clone()).await?;
        match outcome {
            SubscriptionSyncOutcome::Synced {
                counts,
                departures_acted,
            } => {
                report.synced += 1;
                report.added += counts.added;
                report.requested += counts.requested;
                report.held += counts.held;
                report.departures_acted += departures_acted;
            }
            SubscriptionSyncOutcome::Unchanged => report.unchanged += 1,
            SubscriptionSyncOutcome::Off => report.off += 1,
            SubscriptionSyncOutcome::Failed(failure) => {
                report.failed += 1;
                report
                    .failures
                    .push(job_failure_label(&subscription, &failure));
            }
        }
    }
    Ok(report)
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

    if subscription.scope == ListScope::Personal
        && context
            .policies
            .get(&subscription.owner_user_id)
            .await?
            .map(|policy| policy.policy)
            .unwrap_or_default()
            == ListPolicy::None
    {
        let status = ListSyncStatus {
            state: ListSyncState::Off,
            next_at: Some(next_sync_at(subscription, now)),
            ..subscription.sync.clone()
        };
        context
            .subscriptions
            .record_sync(&subscription.id, &status, &subscription.counts)
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
        credential,
        &config,
    )
    .await
    {
        Ok(fetched) => fetched,
        Err(failure) => return record_failure(context, subscription, run, now, failure).await,
    };

    if fetched.unchanged {
        let status = ListSyncStatus {
            state: ListSyncState::Ok,
            last_at: Some(now),
            next_at: Some(next_sync_at(subscription, now)),
            error_message: None,
            error_at: None,
            paused_until: None,
            fetch_fingerprint: fetched
                .fingerprint
                .or_else(|| subscription.sync.fetch_fingerprint.clone()),
        };
        context
            .subscriptions
            .record_sync(&subscription.id, &status, &subscription.counts)
            .await?;
        run.counts = subscription.counts;
        finish_run(context, run, now).await?;
        return Ok(SubscriptionSyncOutcome::Unchanged);
    }

    // The same item listed twice keeps its first position only, and is acted
    // on once.
    fetched.dedupe();
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
    let evaluated = evaluate(subscription, resolved, &exclusions, &existing);

    let mut rows = Vec::with_capacity(evaluated.len());
    for evaluated in evaluated {
        let previous = existing.get(&evaluated.item.item.item_key);
        let mut row = membership_row(subscription, &evaluated.item, previous, now);
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
            ItemDecision::Keep { state } => row.state = state,
            ItemDecision::Unresolved => row.state = ListMembershipState::Unresolved,
            ItemDecision::Deferred => row.state = ListMembershipState::Pending,
            ItemDecision::Candidate => {
                let outcome =
                    act_on_candidate(context.actions, subscription, &evaluated.item).await;
                row.state = outcome.state;
                row.state_reason = outcome.reason;
                row.added_by_list |= outcome.added_by_list;
                if outcome.title_id.is_some() {
                    row.title_id = outcome.title_id;
                }
                if outcome.request_id.is_some() {
                    row.request_id = outcome.request_id;
                }
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
    let leave = handle_departures(
        subscription,
        context.memberships,
        context.subscriptions,
        context.actions,
    )
    .await?;

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
        .record_sync(&subscription.id, &status, &counts)
        .await?;
    run.counts = counts;
    finish_run(context, run, now).await?;
    Ok(SubscriptionSyncOutcome::Synced {
        counts,
        departures_acted: leave.acted,
    })
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
        .record_sync(&subscription.id, &status, &subscription.counts)
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

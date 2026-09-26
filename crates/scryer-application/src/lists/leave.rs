//! Leave: what happens to a title when it drops off a list.
//!
//! Only titles this list added are ever touched, and only when no other
//! enabled subscription still lists the title for the same library. The guard
//! is scope-blind on purpose (a member's personal list keeps a title a public
//! list dropped) and reveals nothing, because it only answers "is someone
//! else still asking for this".
//!
//! The actions are `Keep` (nothing), `Log` (a recorded departure),
//! `Unmonitor`, and `Tag` (the registered `left-list` tag). `Unmonitor` and
//! `Tag` are recorded as departures too, so the title's history says why it
//! changed. There is no removal: taking a title out of the
//! library because a list no longer wants it is a maintenance rule's
//! decision, with its own preview and approval.

use std::collections::HashMap;

use scryer_domain::{ListMembership, ListOnLeave, ListSubscription};

use super::act::ListActions;
use super::ports::{ListMembershipRepository, ListSubscriptionRepository};
use crate::AppResult;

/// The tag applied by the `Tag` on-leave action.
pub const LEFT_LIST_TAG: &str = "left-list";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LeaveReport {
    /// Departed rows looked at.
    pub departed: u64,
    /// Rows whose on-leave action ran.
    pub acted: u64,
    /// Rows skipped because another enabled list still wants the title.
    pub guarded: u64,
    /// Rows whose action failed; left unhandled and retried next sync.
    pub failed: u64,
}

/// Run the on-leave action for every departed, not-yet-handled row of the
/// subscription. Called only after a successful fetch and resolve.
pub async fn handle_departures(
    subscription: &ListSubscription,
    memberships: &dyn ListMembershipRepository,
    subscriptions: &dyn ListSubscriptionRepository,
    actions: &dyn ListActions,
) -> AppResult<LeaveReport> {
    let departed = memberships
        .list_by_subscription(&subscription.id)
        .await?
        .into_iter()
        .filter(|row| row.left_at.is_some() && !row.left_handled)
        .collect::<Vec<_>>();

    let mut report = LeaveReport {
        departed: departed.len() as u64,
        ..LeaveReport::default()
    };
    let mut handled = Vec::new();
    let mut other_subscriptions: HashMap<String, Option<ListSubscription>> = HashMap::new();

    for row in departed {
        let title_id = match (&row.title_id, row.added_by_list, subscription.on_leave) {
            (Some(title_id), true, on_leave) if on_leave != ListOnLeave::Keep => title_id.clone(),
            // Keep, a title the list did not add, or no title at all: the
            // departure is recorded on the row and nothing else happens.
            _ => {
                handled.push(row.item_key);
                continue;
            }
        };

        if still_wanted_elsewhere(
            subscription,
            &row,
            &title_id,
            memberships,
            subscriptions,
            &mut other_subscriptions,
        )
        .await?
        {
            report.guarded += 1;
            handled.push(row.item_key);
            continue;
        }

        let on_leave = subscription.on_leave;
        let result = match on_leave {
            ListOnLeave::Keep => Ok(()),
            ListOnLeave::Log => {
                actions
                    .record_departure(subscription, &title_id, on_leave)
                    .await
            }
            ListOnLeave::Unmonitor => actions.set_title_monitored(&title_id, false).await,
            ListOnLeave::Tag => actions.tag_title(&title_id, LEFT_LIST_TAG).await,
        };
        if result.is_ok()
            && matches!(on_leave, ListOnLeave::Unmonitor | ListOnLeave::Tag)
            && let Err(error) = actions
                .record_departure(subscription, &title_id, on_leave)
                .await
        {
            // The action itself ran; retrying it to get the record written
            // would unmonitor or tag the title a second time.
            tracing::warn!(
                subscription_id = %subscription.id,
                title_id = %title_id,
                error = %error,
                "could not record a list departure"
            );
        }
        match result {
            Ok(()) => {
                report.acted += 1;
                handled.push(row.item_key);
            }
            Err(_) => report.failed += 1,
        }
    }

    if !handled.is_empty() {
        memberships
            .set_left_handled(&subscription.id, &handled)
            .await?;
    }
    Ok(report)
}

/// Whether any other enabled subscription, of either scope, still lists this
/// title and routes its kind to the same library.
async fn still_wanted_elsewhere(
    subscription: &ListSubscription,
    row: &ListMembership,
    title_id: &str,
    memberships: &dyn ListMembershipRepository,
    subscriptions: &dyn ListSubscriptionRepository,
    cache: &mut HashMap<String, Option<ListSubscription>>,
) -> AppResult<bool> {
    let library_id = subscription
        .route_for(row.kind.clone())
        .map(|route| route.library_id.clone());
    for other in memberships.list_by_title(title_id).await? {
        if other.subscription_id == subscription.id || other.left_at.is_some() {
            continue;
        }
        if !cache.contains_key(&other.subscription_id) {
            let loaded = subscriptions.get_by_id(&other.subscription_id).await?;
            cache.insert(other.subscription_id.clone(), loaded);
        }
        let Some(Some(other_subscription)) = cache.get(&other.subscription_id) else {
            continue;
        };
        if !other_subscription.enabled {
            continue;
        }
        let other_library = other_subscription
            .route_for(other.kind)
            .map(|route| route.library_id.as_str());
        // Without a known library on this side, any enabled list keeping the
        // title is enough to leave it alone.
        if library_id.is_none() || other_library == library_id.as_deref() {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
#[path = "leave_tests.rs"]
mod tests;

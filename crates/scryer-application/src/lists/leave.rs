//! Leave: what happens to a title when it drops off a list.
//!
//! Only titles this list added are ever touched, and only when no other
//! enabled subscription still lists the title for the same library. The guard
//! is scope-blind on purpose (a member's personal list keeps a title a public
//! list dropped) and reveals nothing, because it only answers "is someone
//! else still asking for this".
//!
//! A departure the guard holds back stays unhandled. Its action is still
//! owed: once no other enabled list wants the title (the other list dropped
//! it too, was disabled, or was deleted), the next sync of this list runs it.
//! A list that did not add the title never acts on it, so a title on two
//! lists gets the on-leave action of the list that added it exactly once,
//! whichever list drops it first.
//!
//! The actions are `Keep` (nothing), `Log` (a recorded departure),
//! `Unmonitor`, and `Tag` (the registered `left-list` tag). `Unmonitor` and
//! `Tag` are recorded as departures too, so the title's history says why it
//! changed. There is no removal: taking a title out of the
//! library because a list no longer wants it is a maintenance rule's
//! decision, with its own preview and approval.

use std::collections::HashMap;

use scryer_domain::{ListMembership, ListMembershipState, ListOnLeave, ListSubscription};

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
    /// Rows whose title was no longer in the library; marked handled with
    /// nothing to act on.
    pub gone: u64,
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
    let mut guard = LeaveGuard::load(subscription, &departed, memberships, subscriptions).await?;
    let mut handled = Vec::new();

    for row in departed {
        let Some(title_id) = acting_title(subscription, &row).map(str::to_string) else {
            // Keep, a title the list did not add, or no title at all: the
            // departure is recorded on the row and nothing else happens.
            handled.push(row.item_key);
            continue;
        };

        if guard.holds(subscription, &row, &title_id).await? {
            // The action is owed, not cancelled: the row stays unhandled so
            // it runs once the other list lets go of the title as well.
            report.guarded += 1;
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
            // A title that is no longer in the library has nothing left to
            // unmonitor, tag or record against, so the departure is done;
            // retrying it would fail the same way on every sync. Nothing is
            // removed here: the title was already gone.
            Err(_) if !title_still_exists(actions, &title_id).await => {
                report.gone += 1;
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

/// Whether a title whose on-leave action failed is still in the library. A
/// lookup that fails counts as present, so the departure stays owed and is
/// retried rather than written off on a passing storage error.
async fn title_still_exists(actions: &dyn ListActions, title_id: &str) -> bool {
    actions.title_exists(title_id).await.unwrap_or(true)
}

/// The title a departed row's on-leave action would touch: only a title this
/// list added, and only when the list's on-leave action is not `Keep`.
fn acting_title<'a>(subscription: &ListSubscription, row: &'a ListMembership) -> Option<&'a str> {
    match (&row.title_id, row.added_by_list, subscription.on_leave) {
        (Some(title_id), true, on_leave) if on_leave != ListOnLeave::Keep => Some(title_id),
        _ => None,
    }
}

/// Whether `row` departed and its on-leave action has not run yet: it failed,
/// the sync stopped before reaching it, or another list still wanted the
/// title at the time.
pub fn awaits_leave_action(subscription: &ListSubscription, row: &ListMembership) -> bool {
    row.left_at.is_some() && !row.left_handled && acting_title(subscription, row).is_some()
}

/// Whether any of `rows` has an on-leave action that a sync could run now:
/// it awaits its action and no other enabled list still wants the title.
/// Such a list must be processed again even when its provider reports no
/// change. A departure another list still holds back does not count, so it
/// never forces the list to be read again while that stays true.
pub async fn has_runnable_leave_action(
    subscription: &ListSubscription,
    rows: &[ListMembership],
    memberships: &dyn ListMembershipRepository,
    subscriptions: &dyn ListSubscriptionRepository,
) -> AppResult<bool> {
    let waiting = rows
        .iter()
        .filter(|row| awaits_leave_action(subscription, row))
        .cloned()
        .collect::<Vec<_>>();
    if waiting.is_empty() {
        return Ok(false);
    }
    let mut guard = LeaveGuard::load(subscription, &waiting, memberships, subscriptions).await?;
    for row in &waiting {
        let Some(title_id) = acting_title(subscription, row) else {
            continue;
        };
        if !guard.holds(subscription, row, title_id).await? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Answers "does another enabled list still want this title" for a batch of
/// departed rows. Every title's memberships are read in one query up front;
/// the other subscriptions are read once each, on first use.
struct LeaveGuard<'a> {
    subscriptions: &'a dyn ListSubscriptionRepository,
    by_title: HashMap<String, Vec<ListMembership>>,
    loaded: HashMap<String, Option<ListSubscription>>,
}

impl<'a> LeaveGuard<'a> {
    async fn load(
        subscription: &ListSubscription,
        departed: &[ListMembership],
        memberships: &dyn ListMembershipRepository,
        subscriptions: &'a dyn ListSubscriptionRepository,
    ) -> AppResult<Self> {
        let mut title_ids = departed
            .iter()
            .filter_map(|row| acting_title(subscription, row))
            .map(str::to_string)
            .collect::<Vec<_>>();
        title_ids.sort();
        title_ids.dedup();
        let mut by_title: HashMap<String, Vec<ListMembership>> = HashMap::new();
        if !title_ids.is_empty() {
            for row in memberships.list_by_titles(&title_ids).await? {
                if let Some(title_id) = row.title_id.clone() {
                    by_title.entry(title_id).or_default().push(row);
                }
            }
        }
        Ok(Self {
            subscriptions,
            by_title,
            loaded: HashMap::new(),
        })
    }

    /// Whether any other enabled subscription, of either scope, still lists
    /// this title and routes its kind to the same library.
    async fn holds(
        &mut self,
        subscription: &ListSubscription,
        row: &ListMembership,
        title_id: &str,
    ) -> AppResult<bool> {
        let library_id = subscription
            .route_for(row.kind.clone())
            .map(|route| route.library_id.clone());
        let others = self.by_title.get(title_id).cloned().unwrap_or_default();
        for other in others {
            if other.subscription_id == subscription.id || other.left_at.is_some() {
                continue;
            }
            // A list that filters or excludes the title still shows it, but
            // does not want it in the library, so it holds nothing back.
            if matches!(
                other.state,
                ListMembershipState::Filtered | ListMembershipState::Excluded
            ) {
                continue;
            }
            if !self.loaded.contains_key(&other.subscription_id) {
                let loaded = self.subscriptions.get_by_id(&other.subscription_id).await?;
                self.loaded.insert(other.subscription_id.clone(), loaded);
            }
            let Some(Some(other_subscription)) = self.loaded.get(&other.subscription_id) else {
                continue;
            };
            if !other_subscription.enabled {
                continue;
            }
            let other_library = other_subscription
                .route_for(other.kind)
                .map(|route| route.library_id.as_str());
            // Without a known library on this side, any enabled list keeping
            // the title is enough to leave it alone.
            if library_id.is_none() || other_library == library_id.as_deref() {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

#[cfg(test)]
#[path = "leave_tests.rs"]
mod tests;

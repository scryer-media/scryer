//! Act: turn one candidate into a library add, a request, or a Discover row.
//!
//! The engine never writes titles or requests itself. Every effect goes
//! through [`ListActions`], whose production implementation calls the
//! existing use cases (title creation, wanted search, request submission,
//! monitoring, tagging, events), so a list add is exactly an operator add with the
//! same checks. There is no removal operation on this port: lists never
//! delete a title.

use async_trait::async_trait;
use scryer_domain::{ListMembershipState, ListMode, ListOnLeave, ListRoute, ListSubscription};

use super::fetch::ListFailure;
use super::resolve::ResolvedItem;
use crate::{AppError, AppResult};

/// A title the add action settled on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AddedTitle {
    pub title_id: String,
    /// False when an existing title was reused: the list did not add it, so
    /// the on-leave step must never act on it.
    pub created: bool,
}

/// The effects the sync engine may cause.
#[async_trait]
pub trait ListActions: Send + Sync {
    /// Create the title monitored in the route's library and, when `search`
    /// is set, start the same wanted search an approved request starts.
    async fn add_title(
        &self,
        subscription: &ListSubscription,
        route: &ListRoute,
        item: &ResolvedItem,
        search: bool,
    ) -> AppResult<AddedTitle>;

    /// Whether the subscription's owner may manage titles in the route's
    /// library. Manage Titles shadows Request, so such an owner's Request
    /// list adds its titles instead of requesting them.
    async fn owner_manages_titles(
        &self,
        subscription: &ListSubscription,
        route: &ListRoute,
    ) -> AppResult<bool>;

    /// Submit a media request owned by the subscription's owner. `hold` asks
    /// for a request that waits for review whatever the owner's grants and
    /// the request rules would allow. Returns the request id.
    async fn submit_request(
        &self,
        subscription: &ListSubscription,
        route: &ListRoute,
        item: &ResolvedItem,
        hold: bool,
    ) -> AppResult<String>;

    async fn set_title_monitored(&self, title_id: &str, monitored: bool) -> AppResult<()>;

    async fn tag_title(&self, title_id: &str, tag: &str) -> AppResult<()>;

    /// Whether the title is still in the library. Only read after an
    /// on-leave action failed, to tell a title that is gone from a failure
    /// worth retrying.
    async fn title_exists(&self, title_id: &str) -> AppResult<bool>;

    /// Record that a title the list added has left it, and which on-leave
    /// action ran. For `Log` this record is the whole action.
    async fn record_departure(
        &self,
        subscription: &ListSubscription,
        title_id: &str,
        action: ListOnLeave,
    ) -> AppResult<()>;

    /// Tell the operator a sync failed. Called once when a subscription
    /// starts failing or its failure changes, not on every failed retry.
    async fn record_sync_failure(
        &self,
        subscription: &ListSubscription,
        failure: &ListFailure,
    ) -> AppResult<()>;
}

/// What acting on one candidate settled.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActOutcome {
    pub state: ListMembershipState,
    pub title_id: Option<String>,
    pub request_id: Option<String>,
    pub added_by_list: bool,
    pub reason: Option<String>,
}

impl ActOutcome {
    fn state(state: ListMembershipState) -> Self {
        Self {
            state,
            title_id: None,
            request_id: None,
            added_by_list: false,
            reason: None,
        }
    }
}

/// Act on one candidate according to the subscription's mode. A failure is
/// recorded on the outcome and never aborts the sync. A passing failure
/// leaves the item `Pending`, tried again next sync; a refusal settles it as
/// `Rejected` (see [`failed_action_state`]); and it is `BlockedPermission`
/// when the owner may not add or request into the routed library.
///
/// `hold_requests` makes a request the Request mode submits wait for review,
/// as a personal list does for a member whose list policy needs approval.
pub async fn act_on_candidate(
    actions: &dyn ListActions,
    subscription: &ListSubscription,
    item: &ResolvedItem,
    hold_requests: bool,
) -> ActOutcome {
    let Some(route) = item
        .kind
        .clone()
        .and_then(|kind| subscription.route_for(kind))
    else {
        // Evaluation filters routeless kinds first; this is only reachable if
        // the routes changed underneath the evaluation.
        return ActOutcome {
            reason: Some(super::evaluate::NO_ROUTE_REASON.to_string()),
            ..ActOutcome::state(ListMembershipState::Filtered)
        };
    };

    // A personal list never adds straight into a library on its mode alone:
    // whatever mode it carries, it acts as a Request list, so its titles are
    // added only for an owner who could add them by hand.
    let mode = match subscription.mode {
        ListMode::Search | ListMode::Add if subscription.is_personal() => ListMode::Request,
        mode => mode,
    };
    let result = match mode {
        ListMode::Search | ListMode::Add => {
            add_outcome(actions, subscription, route, item, mode == ListMode::Search).await
        }
        // Hold parks every item for review, whoever owns the list.
        ListMode::Hold => actions
            .submit_request(subscription, route, item, true)
            .await
            .map(|request_id| ActOutcome {
                request_id: Some(request_id),
                ..ActOutcome::state(ListMembershipState::Held)
            }),
        ListMode::Request => {
            // Manage Titles shadows Request: an owner who may add the title
            // themselves gets it added, searched as an approved request would
            // be, rather than a request that only they could approve.
            match actions.owner_manages_titles(subscription, route).await {
                Ok(true) => add_outcome(actions, subscription, route, item, true).await,
                Ok(false) => actions
                    .submit_request(subscription, route, item, hold_requests)
                    .await
                    .map(|request_id| ActOutcome {
                        request_id: Some(request_id),
                        ..ActOutcome::state(ListMembershipState::Requested)
                    }),
                Err(error) => Err(error),
            }
        }
        ListMode::Discover => Ok(ActOutcome::state(ListMembershipState::Discover)),
    };

    result.unwrap_or_else(|error| ActOutcome {
        reason: Some(action_failure_reason(&error)),
        ..ActOutcome::state(failed_action_state(&error))
    })
}

/// Where a failed action leaves its item.
///
/// A refusal (the add or request is invalid, or something it names does not
/// exist) comes back the same way however often the same item is tried, so
/// it settles as `Rejected` rather than `Pending`. A `Pending` item makes
/// every sync read the whole list again to retry it; a settled one does not.
/// Evaluation reopens it once the item or the list's settings change. Every
/// other failure (network, rate limit, gateway or storage trouble) may pass,
/// so the item stays `Pending` and is retried next sync.
fn failed_action_state(error: &AppError) -> ListMembershipState {
    match error {
        AppError::Unauthorized(_) => ListMembershipState::BlockedPermission,
        AppError::Validation(_) | AppError::NotFound(_) => ListMembershipState::Rejected,
        _ => ListMembershipState::Pending,
    }
}

const REFUSED_REASON: &str = "rejected";
const NOT_FOUND_REASON: &str = "not_found";

/// Add the candidate's title to the route's library. A title the library
/// already held is `InLibrary`, and the list did not add it.
async fn add_outcome(
    actions: &dyn ListActions,
    subscription: &ListSubscription,
    route: &ListRoute,
    item: &ResolvedItem,
    search: bool,
) -> AppResult<ActOutcome> {
    let added = actions.add_title(subscription, route, item, search).await?;
    Ok(if added.created {
        ActOutcome {
            title_id: Some(added.title_id),
            added_by_list: true,
            ..ActOutcome::state(ListMembershipState::Added)
        }
    } else {
        ActOutcome {
            title_id: Some(added.title_id),
            ..ActOutcome::state(ListMembershipState::InLibrary)
        }
    })
}

/// A short, stable reason for a failed action. The error text is not copied:
/// it can name the title, and the membership row is shown to the list's
/// viewers.
fn action_failure_reason(error: &AppError) -> String {
    match error {
        AppError::Unauthorized(_) => "not_permitted",
        AppError::Validation(_) => REFUSED_REASON,
        AppError::NotFound(_) => NOT_FOUND_REASON,
        _ => "action_failed",
    }
    .to_string()
}

/// Whether a membership reason records an action this engine tried and was
/// refused, as opposed to a reviewer rejecting the request it made.
pub(super) fn is_refused_action_reason(reason: &str) -> bool {
    matches!(reason, REFUSED_REASON | NOT_FOUND_REASON)
}

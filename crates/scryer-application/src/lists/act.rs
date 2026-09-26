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

    /// Submit a media request owned by the subscription's owner. `hold` asks
    /// for a request that waits for review whatever the owner's grants allow.
    /// Returns the request id.
    async fn submit_request(
        &self,
        subscription: &ListSubscription,
        route: &ListRoute,
        item: &ResolvedItem,
        hold: bool,
    ) -> AppResult<String>;

    async fn set_title_monitored(&self, title_id: &str, monitored: bool) -> AppResult<()>;

    async fn tag_title(&self, title_id: &str, tag: &str) -> AppResult<()>;

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
/// recorded on the outcome and never aborts the sync: the item stays
/// `Pending` and is tried again next sync, or `BlockedPermission` when the
/// owner may not add or request into the routed library.
pub async fn act_on_candidate(
    actions: &dyn ListActions,
    subscription: &ListSubscription,
    item: &ResolvedItem,
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

    // A personal list never adds straight into a library: whatever mode it
    // carries, its titles go through the owner's own request.
    let mode = match subscription.mode {
        ListMode::Search | ListMode::Add if subscription.is_personal() => ListMode::Request,
        mode => mode,
    };
    let result = match mode {
        ListMode::Search | ListMode::Add => actions
            .add_title(subscription, route, item, mode == ListMode::Search)
            .await
            .map(|added| {
                if added.created {
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
                }
            }),
        ListMode::Hold | ListMode::Request => {
            let hold = mode == ListMode::Hold;
            actions
                .submit_request(subscription, route, item, hold)
                .await
                .map(|request_id| ActOutcome {
                    request_id: Some(request_id),
                    ..ActOutcome::state(if hold {
                        ListMembershipState::Held
                    } else {
                        ListMembershipState::Requested
                    })
                })
        }
        ListMode::Discover => Ok(ActOutcome::state(ListMembershipState::Discover)),
    };

    result.unwrap_or_else(|error| ActOutcome {
        reason: Some(action_failure_reason(&error)),
        ..ActOutcome::state(match error {
            AppError::Unauthorized(_) => ListMembershipState::BlockedPermission,
            _ => ListMembershipState::Pending,
        })
    })
}

/// A short, stable reason for a failed action. The error text is not copied:
/// it can name the title, and the membership row is shown to the list's
/// viewers.
fn action_failure_reason(error: &AppError) -> String {
    match error {
        AppError::Unauthorized(_) => "not_permitted",
        AppError::Validation(_) => "rejected",
        AppError::NotFound(_) => "not_found",
        _ => "action_failed",
    }
    .to_string()
}

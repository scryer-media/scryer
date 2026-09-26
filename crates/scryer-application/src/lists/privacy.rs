//! Owner checks for personal list data, in one place.
//!
//! A personal subscription, its account, its preview and its sync errors
//! belong to one member. Nobody else reads them: not another member, and not
//! an administrator. There is no admin projection of personal list data, so
//! these checks have no permission escape hatch. A caller who is not the
//! owner gets "not found", the same answer as for a row that does not exist,
//! so the check does not confirm that someone else's list is there.
//!
//! The one path that reads across owners is the sync job itself, which runs as
//! the system and reports personal failures by owner id, provider and error
//! class only ([`job_failure_label`]).

use scryer_domain::{ListScope, ListSubscription, User, UserListAccount};
use scryer_plugin_sdk::ListCredential;

use super::fetch::{ListFailure, ListFailureClass};
use crate::{AppError, AppResult};

fn not_found() -> AppError {
    AppError::NotFound("list subscription not found".into())
}

/// Whether `actor` may see `subscription` at all. Public subscriptions are
/// governed by app permissions elsewhere; personal ones only by ownership.
pub fn can_view_subscription(actor: &User, subscription: &ListSubscription) -> bool {
    match subscription.scope {
        ListScope::Public => true,
        ListScope::Personal => actor.id == subscription.owner_user_id,
    }
}

/// Fail with "not found" unless `actor` may see `subscription`.
pub fn ensure_subscription_visible(actor: &User, subscription: &ListSubscription) -> AppResult<()> {
    if can_view_subscription(actor, subscription) {
        Ok(())
    } else {
        Err(not_found())
    }
}

/// Drop every personal subscription `actor` does not own.
pub fn visible_subscriptions(
    actor: &User,
    subscriptions: Vec<ListSubscription>,
) -> Vec<ListSubscription> {
    subscriptions
        .into_iter()
        .filter(|subscription| can_view_subscription(actor, subscription))
        .collect()
}

/// Fail with "not found" unless `actor` owns `account`.
pub fn ensure_account_owner(actor: &User, account: &UserListAccount) -> AppResult<()> {
    if actor.id == account.user_id {
        Ok(())
    } else {
        Err(AppError::NotFound("list account not found".into()))
    }
}

/// The credential a personal subscription reads with, or why there is none.
///
/// The account must belong to the subscription's owner and to its provider,
/// and must be active. A mismatch is treated as a missing account rather than
/// an error that names whose account it was.
pub fn credential_for(
    subscription: &ListSubscription,
    account: Option<&UserListAccount>,
) -> Result<ListCredential, ListFailure> {
    let provider = subscription.source.provider.as_str();
    let account = account
        .filter(|account| account.user_id == subscription.owner_user_id)
        .filter(|account| account.provider.eq_ignore_ascii_case(provider))
        .ok_or_else(|| ListFailure::new(ListFailureClass::AccountRequired, provider))?;
    match account.status {
        scryer_domain::UserListAccountStatus::Active => Ok(ListCredential {
            access_token: account.credential.access_token.clone(),
            token_type: account.credential.token_type.clone(),
            external_user_id: Some(account.external_user_id.clone()),
            username: Some(account.username.clone()),
        }),
        _ => Err(ListFailure::new(ListFailureClass::Unauthorized, provider)),
    }
}

/// How a job summary names one subscription's failure. A personal list is
/// named by its owner id and provider only; its name and items never appear.
pub fn job_failure_label(subscription: &ListSubscription, failure: &ListFailure) -> String {
    match subscription.scope {
        ListScope::Public => format!(
            "public list {} ({}): {}",
            subscription.id,
            subscription.source.provider,
            failure.class.as_str()
        ),
        ListScope::Personal => format!(
            "personal list of user {} ({}): {}",
            subscription.owner_user_id,
            subscription.source.provider,
            failure.class.as_str()
        ),
    }
}

#[cfg(test)]
#[path = "privacy_tests.rs"]
mod tests;

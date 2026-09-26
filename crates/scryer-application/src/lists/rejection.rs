//! Remembering a rejected list request.
//!
//! When a reviewer rejects a request a list submitted, the list must not
//! submit it again on its next sync. A public list gets a list-scoped
//! exclusion, which every later sync honours first. A personal list gets no
//! exclusion: exclusions are instance-level and a member's list is theirs, so
//! the membership row moves to `Rejected` instead, a settled state the engine
//! keeps until the item leaves the list and comes back.
//!
//! Nothing here touches the title or the request itself.

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use scryer_domain::{
    Id, ListExclusion, ListExclusionScope, ListMembershipState, ListScope, MediaRequest,
    MediaRequestOrigin,
};

use super::ports::{ListExclusionRepository, ListMembershipRepository, ListSubscriptionRepository};
use crate::AppResult;

/// What remembering one rejection wrote.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RejectionRecord {
    pub exclusions_created: u64,
    pub memberships_excluded: u64,
    pub memberships_rejected: u64,
}

/// Remember that `request` was rejected by `rejected_by_user_id`.
///
/// The request's own origin names one subscription; a request several lists
/// submitted into also carries rows on the others, found through their
/// membership rows. Each public subscription involved gets its own
/// list-scoped exclusion, unless an exclusion already covers the title there.
pub async fn remember_rejected_request(
    subscriptions: &dyn ListSubscriptionRepository,
    memberships: &dyn ListMembershipRepository,
    exclusions: &dyn ListExclusionRepository,
    request: &MediaRequest,
    rejected_by_user_id: &str,
    now: DateTime<Utc>,
) -> AppResult<RejectionRecord> {
    let mut record = RejectionRecord::default();
    let rows = memberships.list_by_request(&request.id).await?;
    if request.origin == MediaRequestOrigin::Manual && rows.is_empty() {
        return Ok(record);
    }

    let mut subscription_ids = BTreeSet::new();
    if let Some(id) = request.origin.subscription_id() {
        subscription_ids.insert(id.to_string());
    }
    subscription_ids.extend(rows.iter().map(|row| row.subscription_id.clone()));

    let mut public_ids = BTreeSet::new();
    for subscription_id in &subscription_ids {
        let Some(subscription) = subscriptions.get_by_id(subscription_id).await? else {
            continue;
        };
        if subscription.scope != ListScope::Public {
            continue;
        }
        public_ids.insert(subscription.id.clone());
        let existing = exclusions
            .find_matching(
                request.facet.clone(),
                &request.external_ids,
                Some(&subscription.id),
            )
            .await?;
        if !existing.is_empty() || request.external_ids.is_empty() {
            continue;
        }
        exclusions
            .create(ListExclusion {
                id: Id::new().0,
                kind: request.facet.clone(),
                external_ids: request.external_ids.clone(),
                display_title: request.title.clone(),
                year: request.year,
                scope: ListExclusionScope::List {
                    subscription_id: subscription.id.clone(),
                },
                created_by_user_id: Some(rejected_by_user_id.to_string()),
                created_at: now,
            })
            .await?;
        record.exclusions_created += 1;
    }

    // Only rows still on their list move: a departed row keeps its history,
    // and refreshing it would mark it present again.
    let mut updated = Vec::new();
    for mut row in rows.into_iter().filter(|row| row.left_at.is_none()) {
        if public_ids.contains(&row.subscription_id) {
            row.state = ListMembershipState::Excluded;
            record.memberships_excluded += 1;
        } else if subscription_ids.contains(&row.subscription_id) {
            row.state = ListMembershipState::Rejected;
            record.memberships_rejected += 1;
        } else {
            continue;
        }
        row.state_reason = Some("request_rejected".to_string());
        updated.push(row);
    }
    if !updated.is_empty() {
        memberships.upsert_many(&updated).await?;
    }
    Ok(record)
}

#[cfg(test)]
#[path = "rejection_tests.rs"]
pub(crate) mod tests;

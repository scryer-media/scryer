//! Evaluate: resolved items + exclusions + filters + existing memberships →
//! one decision per item. Pure, so the preview and the sync can share it and
//! "what the panel shows is what the sync does" holds by construction.
//!
//! Precedence per item, first match wins:
//!
//! 1. an exclusion covering the item → `Excluded` (never added, whatever else
//!    is true);
//! 2. a filter rejecting it, or no routing card for its kind → `Filtered`
//!    (filters are re-evaluated every sync, so a relaxed filter lets the item
//!    through next time);
//! 3. the title is already in a library of its kind → `InLibrary`;
//! 4. an earlier sync already settled it (added, requested, held, rejected,
//!    recorded for Discover) → keep that state;
//! 5. the gateway could not resolve it → `Unresolved`, retried next sync;
//! 6. otherwise it is a candidate.
//!
//! The per-sync cap is applied last, to candidates only, in list order: the
//! first `max_per_sync` are acted on and the rest stay `Pending` for the next
//! sync. A capped item is not filtered, so the filtered count stays honest.

use std::collections::HashMap;

use scryer_domain::{
    ListCounts, ListExclusion, ListFilter, ListMembership, ListMembershipState, ListSubscription,
};

use super::resolve::ResolvedItem;

/// Why an item was filtered. Stored on the membership row.
pub const NO_ROUTE_REASON: &str = "no_route";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ItemDecision {
    Excluded,
    Filtered {
        reason: String,
    },
    InLibrary {
        title_id: String,
    },
    /// An earlier sync's settled outcome, carried forward.
    Keep {
        state: ListMembershipState,
    },
    Unresolved,
    /// Act on this item in this sync.
    Candidate,
    /// A candidate beyond this sync's cap.
    Deferred,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EvaluatedItem {
    pub item: ResolvedItem,
    pub decision: ItemDecision,
}

/// Decide every item. `memberships` is keyed by item key.
pub fn evaluate(
    subscription: &ListSubscription,
    items: Vec<ResolvedItem>,
    exclusions: &[ListExclusion],
    memberships: &HashMap<String, ListMembership>,
) -> Vec<EvaluatedItem> {
    let mut remaining_cap = subscription.max_per_sync.map(|cap| cap as usize);
    items
        .into_iter()
        .map(|item| {
            let mut decision = decide(subscription, &item, exclusions, memberships);
            if decision == ItemDecision::Candidate
                && let Some(remaining) = remaining_cap.as_mut()
            {
                if *remaining == 0 {
                    decision = ItemDecision::Deferred;
                } else {
                    *remaining -= 1;
                }
            }
            EvaluatedItem { item, decision }
        })
        .collect()
}

fn decide(
    subscription: &ListSubscription,
    item: &ResolvedItem,
    exclusions: &[ListExclusion],
    memberships: &HashMap<String, ListMembership>,
) -> ItemDecision {
    if let Some(kind) = item.kind.clone()
        && exclusions
            .iter()
            .any(|exclusion| exclusion.matches(kind.clone(), &item.external_ids, &subscription.id))
    {
        return ItemDecision::Excluded;
    }

    if let Some(reason) = filter_reason(subscription, item) {
        return ItemDecision::Filtered { reason };
    }

    let existing = memberships
        .get(&item.item.item_key)
        .filter(|existing| existing.left_at.is_none() && existing.state.is_settled());

    if let Some(title_id) = &item.library_title_id {
        // A title this list added is in the library because the list put it
        // there; it stays counted as added rather than turning into a
        // pre-existing title on the next sync.
        if let Some(existing) = existing
            && existing.state == ListMembershipState::Added
        {
            return ItemDecision::Keep {
                state: ListMembershipState::Added,
            };
        }
        return ItemDecision::InLibrary {
            title_id: title_id.clone(),
        };
    }

    if let Some(existing) = existing {
        return ItemDecision::Keep {
            state: existing.state,
        };
    }

    if !item.resolved {
        return ItemDecision::Unresolved;
    }

    ItemDecision::Candidate
}

/// The first filter the item fails, if any. A kind without a routing card is
/// filtered with [`NO_ROUTE_REASON`].
pub fn filter_reason(subscription: &ListSubscription, item: &ResolvedItem) -> Option<String> {
    let kind = item.kind.clone()?;
    if subscription.route_for(kind).is_none() {
        return Some(NO_ROUTE_REASON.to_string());
    }
    subscription
        .filters
        .iter()
        .find(|filter| !passes(filter, item))
        .map(filter_label)
}

fn passes(filter: &ListFilter, item: &ResolvedItem) -> bool {
    let item = &item.item;
    match filter {
        // A rating's scale names the source that rated the item, such as
        // `tmdb` or `imdb`, and its value is on that source's own scale. A
        // filter matches only a rating from the source it names.
        ListFilter::RatingAtLeast { scale, value } => {
            item.provider_rating.as_ref().is_some_and(|rating| {
                rating.scale.trim().eq_ignore_ascii_case(scale.trim()) && rating.value >= *value
            })
        }
        ListFilter::ReleaseYear { from, to } => match item.year {
            Some(year) => from.is_none_or(|from| year >= from) && to.is_none_or(|to| year <= to),
            None => true,
        },
        ListFilter::ExcludeGenres { genres } => !item.genres.iter().any(|genre| {
            genres
                .iter()
                .any(|excluded| excluded.eq_ignore_ascii_case(genre))
        }),
        ListFilter::Format { formats } => item.format.as_ref().is_none_or(|format| {
            formats
                .iter()
                .any(|allowed| allowed.eq_ignore_ascii_case(format))
        }),
        ListFilter::Language { languages } => item.language.as_ref().is_none_or(|language| {
            languages
                .iter()
                .any(|allowed| allowed.eq_ignore_ascii_case(language))
        }),
        ListFilter::ReleasedOnly => item.released != Some(false),
        // These need facts the fetched item does not carry (the member's
        // streaming services, credits, franchise order). Until the engine has
        // them they pass rather than silently dropping every item.
        ListFilter::SkipOnMyStreamingServices
        | ListFilter::DirectorCreditsOnly
        | ListFilter::NotSequelWithoutBase => true,
    }
}

fn filter_label(filter: &ListFilter) -> String {
    match filter {
        ListFilter::RatingAtLeast { .. } => "rating",
        ListFilter::ReleaseYear { .. } => "release_year",
        ListFilter::ExcludeGenres { .. } => "genre",
        ListFilter::Format { .. } => "format",
        ListFilter::Language { .. } => "language",
        ListFilter::SkipOnMyStreamingServices => "streaming_service",
        ListFilter::ReleasedOnly => "unreleased",
        ListFilter::DirectorCreditsOnly => "director_credit",
        ListFilter::NotSequelWithoutBase => "sequel_without_base",
    }
    .to_string()
}

/// Counts for one sync, from the final state of every present item.
pub fn count_states(states: impl IntoIterator<Item = ListMembershipState>) -> ListCounts {
    let mut counts = ListCounts::default();
    for state in states {
        counts.total += 1;
        match state {
            ListMembershipState::InLibrary => counts.in_library += 1,
            ListMembershipState::Added => counts.added += 1,
            ListMembershipState::Requested => counts.requested += 1,
            ListMembershipState::Held => counts.held += 1,
            ListMembershipState::Filtered => counts.filtered += 1,
            ListMembershipState::Excluded => counts.excluded += 1,
            ListMembershipState::Unresolved => counts.unresolved += 1,
            ListMembershipState::Rejected
            | ListMembershipState::BlockedPermission
            | ListMembershipState::Discover
            | ListMembershipState::Pending => {}
        }
    }
    counts
}

#[cfg(test)]
#[path = "evaluate_tests.rs"]
mod tests;

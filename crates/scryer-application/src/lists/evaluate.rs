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
//!    recorded for Discover) → keep that state. An add or request that was
//!    refused is kept only until the item's ids or the list's settings
//!    change, and then it is a candidate again. A title the list added and
//!    that was later deleted from the library is not settled: the sync leaves
//!    that row out, so the item is weighed again as if new;
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
    let mut candidate_ids = std::collections::HashSet::new();
    items
        .into_iter()
        .map(|item| {
            let mut decision = decide(subscription, &item, exclusions, memberships);
            if decision == ItemDecision::Candidate
                && let Some(id) = item.smg_title_id
                && !candidate_ids.insert(id)
            {
                decision = ItemDecision::Filtered {
                    reason: "duplicate_target".into(),
                };
            }
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
    if let Some(kind) = if item.series_movie.is_some() {
        Some(scryer_domain::MediaFacet::Movie)
    } else {
        item.kind.clone()
    } && exclusions
        .iter()
        .any(|exclusion| exclusion.matches(kind.clone(), &item.external_ids, &subscription.id))
    {
        return ItemDecision::Excluded;
    }

    // Filters govern admission, not cleanup protection for an existing link.
    if let Some(previous) = memberships.get(&item.item.item_key)
        && previous.left_at.is_none()
        && previous.title_id.is_some()
        && !matches!(
            previous.state,
            ListMembershipState::Filtered | ListMembershipState::Excluded
        )
    {
        return ItemDecision::Keep {
            state: previous.state,
        };
    }

    if item.resolution_reason.is_some() {
        return ItemDecision::Unresolved;
    }
    if let Some(reason) = filter_reason(subscription, item) {
        return ItemDecision::Filtered { reason };
    }

    let existing = memberships.get(&item.item.item_key).filter(|existing| {
        existing.left_at.is_none()
            && existing.state.is_settled()
            && !refusal_may_have_lifted(subscription, item, existing)
    });

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

/// Whether the list's settings were edited after its last sync, so that sync
/// has not seen them yet. A list that never synced counts as edited.
pub(super) fn edited_since_last_sync(subscription: &ListSubscription) -> bool {
    subscription
        .sync
        .last_at
        .is_none_or(|last_at| subscription.updated_at > last_at)
}

/// Whether an item whose add or request was refused should be tried again.
/// Retrying the same item against the same settings would only be refused
/// again, so it waits for something that could change the answer: the
/// list's settings were edited (a route, library or profile the refusal
/// named), or the item now resolves to different ids.
fn refusal_may_have_lifted(
    subscription: &ListSubscription,
    item: &ResolvedItem,
    row: &ListMembership,
) -> bool {
    row.state == ListMembershipState::Rejected
        && row
            .state_reason
            .as_deref()
            .is_some_and(super::act::is_refused_action_reason)
        && (edited_since_last_sync(subscription)
            || row.external_ids != item.external_ids
            || row.smg_title_id != item.smg_title_id)
}

/// The first filter the item fails, if any. A kind without a routing card is
/// filtered with [`NO_ROUTE_REASON`].
pub fn filter_reason(subscription: &ListSubscription, item: &ResolvedItem) -> Option<String> {
    let kind = item.kind.clone()?;
    if !subscription.kinds.is_empty() && !subscription.kinds.contains(&kind) {
        return Some("media_type_not_included".into());
    }
    if subscription.route_for(kind).is_none() {
        return Some(NO_ROUTE_REASON.to_string());
    }
    subscription
        .filters
        .iter()
        .find(|filter| !passes(filter, item))
        .map(|filter| {
            let missing = match filter {
                ListFilter::Ratings { minimums, .. } => minimums.iter().any(|minimum| {
                    !item.facts.as_ref().is_some_and(|facts| {
                        facts.ratings.iter().any(|rating| {
                            rating_source(&rating.source).map(|value| value.0)
                                == rating_source(&minimum.source).map(|value| value.0)
                                && usable_rating(rating)
                        })
                    })
                }),
                ListFilter::RatingAtLeast { scale, .. } => {
                    !item.facts.as_ref().is_some_and(|facts| {
                        facts.ratings.iter().any(|rating| {
                            rating_source(&rating.source).map(|value| value.0)
                                == rating_source(scale).map(|value| value.0)
                                && usable_rating(rating)
                        })
                    })
                }
                ListFilter::ReleaseYear { .. } => {
                    item.facts.as_ref().and_then(|facts| facts.year).is_none()
                }
                ListFilter::Language { .. } => item
                    .facts
                    .as_ref()
                    .and_then(|facts| facts.original_language.as_ref())
                    .is_none(),
                ListFilter::ReleasedOnly => item
                    .facts
                    .as_ref()
                    .and_then(|facts| facts.release_date)
                    .is_none(),
                ListFilter::ExcludeCanonicalTags { .. } => item.facts.is_none(),
                _ => false,
            };
            let label = filter_label(filter);
            if missing {
                format!("missing_{label}")
            } else {
                label
            }
        })
}

fn passes(filter: &ListFilter, item: &ResolvedItem) -> bool {
    let facts = item.facts.as_ref();
    match filter {
        ListFilter::MonitorSpecials { facet, enabled } => {
            item.kind.as_ref() != Some(facet) || *enabled || item.item.season != Some(0)
        }
        // These are the existing per-episode monitoring policies, not a reason
        // to reject an entire series that contains some filler or recap episodes.
        ListFilter::FillerPolicy { .. } | ListFilter::RecapPolicy { .. } => true,
        // A rating's scale names the source that rated the item, such as
        // `tmdb` or `imdb`, and its value is on that source's own scale. A
        // filter matches only a rating from the source it names.
        ListFilter::Ratings {
            facet,
            match_any,
            minimums,
        } => {
            if item.kind.as_ref() != Some(facet) || minimums.is_empty() {
                return true;
            }
            let mut checks = minimums
                .iter()
                .map(|minimum| rating_passes(facts, &minimum.source, minimum.value));
            if *match_any {
                checks.any(|passes| passes)
            } else {
                checks.all(|passes| passes)
            }
        }
        ListFilter::ExcludeCanonicalTags {
            facet,
            keys,
            unresolved_labels,
        } => {
            if item.kind.as_ref() != Some(facet)
                || (keys.is_empty() && unresolved_labels.is_empty())
            {
                return true;
            }
            unresolved_labels.is_empty()
                && facts
                    .is_some_and(|facts| !facts.canonical_keys.iter().any(|key| keys.contains(key)))
        }
        ListFilter::RatingAtLeast { scale, value } => rating_passes(facts, scale, *value),
        ListFilter::ReleaseYear { from, to } => match facts.and_then(|facts| facts.year) {
            Some(year) => from.is_none_or(|from| year >= from) && to.is_none_or(|to| year <= to),
            None => from.is_none() && to.is_none(),
        },
        // Legacy labels must be resolved through the vocabulary before evaluation.
        ListFilter::ExcludeGenres { genres } => genres.is_empty(),
        ListFilter::Format { formats } => item.item.format.as_ref().is_none_or(|format| {
            formats
                .iter()
                .any(|allowed| allowed.eq_ignore_ascii_case(format))
        }),
        ListFilter::Language { languages } => {
            languages.is_empty()
                || facts
                    .and_then(|facts| facts.original_language.as_ref())
                    .is_some_and(|language| {
                        languages.iter().any(|allowed| {
                            crate::normalize_search_language_code(language).is_some_and(
                                |language| {
                                    crate::normalize_search_language_code(allowed).as_ref()
                                        == Some(&language)
                                },
                            )
                        })
                    })
        }
        ListFilter::ReleasedOnly => facts
            .and_then(|facts| facts.release_date)
            .is_some_and(|date| date <= chrono::Utc::now().date_naive()),
        // These need facts the fetched item does not carry (the member's
        // streaming services, credits, franchise order). Until the engine has
        // them they pass rather than silently dropping every item.
        ListFilter::SkipOnMyStreamingServices
        | ListFilter::DirectorCreditsOnly
        | ListFilter::NotSequelWithoutBase => true,
    }
}

pub fn rating_source(source: &str) -> Option<(&'static str, f64)> {
    Some(match source.trim().to_ascii_lowercase().as_str() {
        "imdb" => ("imdb", 10.0),
        "tmdb" => ("tmdb", 10.0),
        "tvdb" | "thetvdb" => ("tvdb", 10.0),
        "trakt" => ("trakt", 10.0),
        "mal" | "myanimelist" | "myanimelist.net" => ("mal", 10.0),
        "anilist" => ("anilist", 100.0),
        "anidb" => ("anidb", 10.0),
        "letterboxd" => ("letterboxd", 5.0),
        "tomatoes" | "rottentomatoes" => ("tomatoes", 100.0),
        "audience" | "popcorn" | "popcornmeter" => ("audience", 100.0),
        "metacritic" => ("metacritic", 100.0),
        "mcuser" | "metacriticuser" => ("mcuser", 10.0),
        "mdblist" => ("mdblist", 100.0),
        _ => return None,
    })
}

fn usable_rating(rating: &scryer_domain::TitleExternalRating) -> bool {
    rating.normalized.is_finite()
        && (0.0..=10.0).contains(&rating.normalized)
        // A default zero with no supplied score is missing, not a zero-star rating.
        && (rating.normalized > 0.0
            || rating.value.is_some_and(f64::is_finite)
            || rating.score.is_some_and(f64::is_finite))
}

fn rating_passes(
    facts: Option<&super::resolve::ListMetadataFacts>,
    source: &str,
    minimum: f64,
) -> bool {
    let Some((source, scale)) = rating_source(source) else {
        return false;
    };
    if !minimum.is_finite() || !(0.0..=scale).contains(&minimum) {
        return false;
    }
    facts.is_some_and(|facts| {
        facts.ratings.iter().any(|rating| {
            rating_source(&rating.source).is_some_and(|(name, _)| name == source)
                && usable_rating(rating)
                && rating.normalized / 10.0 >= minimum / scale
        })
    })
}

fn filter_label(filter: &ListFilter) -> String {
    match filter {
        ListFilter::MonitorSpecials { .. } => "specials",
        ListFilter::FillerPolicy { .. } => "filler",
        ListFilter::RecapPolicy { .. } => "recap",
        ListFilter::Ratings { .. } => "rating",
        ListFilter::ExcludeCanonicalTags { .. } => "genre",
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

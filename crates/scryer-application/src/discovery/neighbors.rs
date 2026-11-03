//! Related titles for any discovery anchor, owned or not, so a graph view can
//! expand a node and recognise the neighbours the viewer already holds.

use super::*;
use crate::catalog_workflow::{
    TITLE_MORE_LIKE_THIS_BACKGROUND_REFRESH_HOURS, TITLE_MORE_LIKE_THIS_HYDRATION_LIMIT,
};
use crate::error_reasons;
use crate::ports::{DiscoveryNeighborCacheRecord, DiscoveryNeighborItemRecord};

/// Neighbours served when the caller names no limit.
pub const DISCOVERY_NEIGHBORS_DEFAULT_LIMIT: i64 = 24;

/// One anchor to expand.
#[derive(Clone, Debug, PartialEq)]
pub struct DiscoveryNeighborsQuery {
    /// The anchor's discovery target key, such as `tmdb:movie:<id>`.
    pub target_key: String,
    pub facet: MediaFacet,
    /// Most neighbours to return. The gateway is asked for at most
    /// [`DISCOVERY_NEIGHBORS_DEFAULT_LIMIT`] (24), so a larger limit serves no
    /// more than that.
    pub limit: i64,
}

/// An anchor's related titles and the neighbourhoods they fall into.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DiscoveryNeighborsView {
    /// Related titles, best first, each naming its group.
    pub items: Vec<DiscoveryNeighborItemRecord>,
    /// The groups that hold at least one of `items`, in the gateway's order.
    /// `size` counts only the served items. Empty when the gateway did not
    /// group the results.
    pub groups: Vec<crate::DiscoveryRelatedGroup>,
}

/// The anchor's target key in canonical form and the gateway subject that
/// names it. Refuses a key outside the vocabulary discovery keys targets on,
/// and a movie or series key that contradicts `facet`. Anime takes both:
/// anime series are series targets and anime films are movie targets.
pub(super) fn discovery_neighbor_subject(
    target_key: &str,
    facet: &MediaFacet,
) -> AppResult<(String, DiscoverySubjectInput)> {
    let unsupported = || {
        AppError::validation_refused(
            error_reasons::discovery::NEIGHBOR_TARGET_KEY_UNSUPPORTED,
            format!("{target_key:?} is not a discovery target key"),
        )
    };
    let mut parts = target_key.trim().split(':');
    let (Some(source), Some(kind), Some(value), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(unsupported());
    };
    let source = source.trim().to_ascii_lowercase();
    let kind = kind.trim().to_ascii_lowercase();
    let value = value.trim();
    let normalized_value = match (source.as_str(), kind.as_str()) {
        ("tvdb" | "tmdb", "movie" | "series") | ("mal" | "anidb" | "anilist", "anime") => {
            parse_positive_i32(value).map(|id| id.to_string())
        }
        ("imdb", "title") => crate::normalize::normalize_imdb_id(value),
        _ => None,
    }
    .ok_or_else(unsupported)?;
    let facet_agrees = match kind.as_str() {
        "movie" => matches!(facet, MediaFacet::Movie | MediaFacet::Anime),
        "series" => matches!(facet, MediaFacet::Series | MediaFacet::Anime),
        _ => true,
    };
    if !facet_agrees {
        return Err(AppError::validation_refused(
            error_reasons::discovery::NEIGHBOR_TARGET_KEY_FACET_MISMATCH,
            format!("{target_key:?} does not name a {} title", facet.as_str()),
        ));
    }

    let canonical_key = format!("{source}:{kind}:{normalized_value}");
    let id = parse_positive_i32(&normalized_value);
    let mut subject = DiscoverySubjectInput {
        key: Some(canonical_key.clone()),
        kind: Some(facet.as_str().to_string()),
        facet: Some(facet.as_str().to_string()),
        external_ids: vec![DiscoveryExternalIdInput {
            source: source.clone(),
            value: normalized_value.clone(),
        }],
        ..Default::default()
    };
    match source.as_str() {
        "tvdb" => subject.tvdb_id = id,
        "tmdb" => subject.tmdb_id = id,
        "mal" => subject.mal_id = id,
        "anidb" => subject.anidb_id = id,
        _ => {}
    }
    Ok((canonical_key, subject))
}

impl AppUseCase {
    /// Titles related to one anchor, owned or not. A result that a title in a
    /// library the viewer can read stands for carries that title's id in
    /// `resolved_title_id`; nothing is dropped for being owned. The
    /// gateway's answer is cached per anchor and metadata language on the
    /// same schedule as a title's more-like-this rows.
    ///
    /// The gate matches the library graph's and runs before the key is
    /// checked: with personalized discovery off, the facet not visible, or no
    /// library of the facet the actor may view, the answer is empty and never
    /// a refusal.
    pub async fn discovery_neighbors(
        &self,
        actor: &User,
        query: DiscoveryNeighborsQuery,
    ) -> AppResult<DiscoveryNeighborsView> {
        let visibility = self.discovery_visibility(actor).await?;
        if !visibility.allows_facet(&query.facet) {
            return Ok(DiscoveryNeighborsView::default());
        }
        let effective_library_ids = self
            .discovery_effective_library_ids(actor, &query.facet, &[])
            .await?;
        if effective_library_ids.is_empty() || !self.personalized_discovery_enabled().await? {
            return Ok(DiscoveryNeighborsView::default());
        }
        let (target_key, subject) = discovery_neighbor_subject(&query.target_key, &query.facet)?;
        let requested_limit = query
            .limit
            .clamp(0, TITLE_MORE_LIKE_THIS_HYDRATION_LIMIT as i64)
            as usize;
        if requested_limit == 0 {
            return Ok(DiscoveryNeighborsView::default());
        }

        let cached = self.discovery_neighbor_items(&target_key, subject).await?;
        let mut items = cached.items;
        items.truncate(requested_limit);
        let readable_library_ids = self
            .authorized_library_ids(actor, None, LibraryPermission::View)
            .await?
            .into_iter()
            .collect::<HashSet<_>>();
        let records = items
            .iter()
            .map(|neighbor| neighbor.item.clone())
            .collect::<Vec<_>>();
        let ownership = self
            .discovery_items_readable_ownership(&records, &readable_library_ids)
            .await?;
        for (neighbor, owned) in items.iter_mut().zip(ownership) {
            neighbor.item.resolved_title_id = owned.readable_title_id;
            neighbor.item.owned_in_input = false;
        }
        let groups = discovery_neighbor_served_groups(&mut items, cached.groups);
        Ok(DiscoveryNeighborsView { items, groups })
    }

    /// The anchor's related titles from the cache while it is fresh, else
    /// from the gateway, which then refreshes the cache. A failed cache write
    /// is logged and the caller gets the gateway's answer as mapped.
    ///
    /// An entry with related titles but no groups stays fresh only until the
    /// next read: the gateway groups whenever it can, so such an answer is
    /// one whose grouping was skipped (or one cached before grouping). The
    /// next read asks the gateway again, and serves the cached entry if that
    /// ask fails.
    async fn discovery_neighbor_items(
        &self,
        target_key: &str,
        subject: DiscoverySubjectInput,
    ) -> AppResult<DiscoveryNeighborCacheRecord> {
        let language = self.metadata_language().await;
        let now = self.runtime.environment.now();
        let ttl = chrono::Duration::hours(TITLE_MORE_LIKE_THIS_BACKGROUND_REFRESH_HOURS);
        let mut ungrouped = None;
        if let Some(cached) = self
            .services
            .library
            .discovery
            .get_discovery_neighbor_cache(target_key, &language)
            .await?
            && now.signed_duration_since(cached.fetched_at) < ttl
        {
            if !discovery_neighbor_cache_ungrouped(&cached) {
                return Ok(cached);
            }
            ungrouped = Some(cached);
        }

        let input = crate::TitleRecommendationsInput {
            subject,
            query: String::new(),
            limit: TITLE_MORE_LIKE_THIS_HYDRATION_LIMIT as i32,
            language: language.clone(),
            include_unresolved: true,
        };
        let recommendations = match self
            .services
            .library
            .metadata_gateway
            .title_recommendations(&input)
            .await
        {
            Ok(recommendations) => recommendations,
            Err(error) => match ungrouped {
                Some(cached) => {
                    warn!(
                        target_key,
                        error = %error,
                        "failed to refresh ungrouped discovery neighbors; serving the cached entry"
                    );
                    return Ok(cached);
                }
                None => return Err(error),
            },
        };
        let mut items = title_more_like_this_item_records(
            target_key,
            &[target_key.to_string()],
            &recommendations.results,
            TITLE_MORE_LIKE_THIS_HYDRATION_LIMIT,
            now,
        )?;
        let group_of = recommendations
            .results
            .iter()
            .filter_map(|result| Some((result.target_key.trim(), result.group_id?)))
            .collect::<HashMap<_, _>>();
        let run_id = format!("neighbors:{target_key}");
        for item in &mut items {
            item.id = format!("{run_id}:{}", item.target_key);
            item.run_id = run_id.clone();
            item.base_generation_id = None;
        }
        let items = items
            .into_iter()
            .map(|item| DiscoveryNeighborItemRecord {
                group_id: group_of.get(item.target_key.as_str()).copied(),
                item,
            })
            .collect();
        let record = DiscoveryNeighborCacheRecord {
            target_key: target_key.to_string(),
            language,
            items,
            groups: recommendations.groups,
            fetched_at: now,
        };
        let discovery = &self.services.library.discovery;
        if let Err(error) = discovery
            .replace_discovery_neighbor_cache(&record, now - ttl)
            .await
        {
            warn!(
                target_key,
                error = %error,
                "failed to cache discovery neighbors"
            );
            return Ok(record);
        }
        // Read the entry back so a first expansion serves exactly what a
        // cached one will: the store enriches and screens what it keeps. A
        // failed read-back still serves the gateway's answer as mapped.
        match discovery
            .get_discovery_neighbor_cache(target_key, &record.language)
            .await
        {
            Ok(Some(cached)) => Ok(cached),
            Ok(None) => Ok(record),
            Err(error) => {
                warn!(
                    target_key,
                    error = %error,
                    "failed to read back cached discovery neighbors"
                );
                Ok(record)
            }
        }
    }
}

/// Whether a cache entry has related titles but no groups to place them in.
fn discovery_neighbor_cache_ungrouped(record: &DiscoveryNeighborCacheRecord) -> bool {
    !record.items.is_empty() && record.groups.is_empty()
}

/// The groups that hold at least one served item, in the gateway's order,
/// each sized by its served items. An item naming a group the gateway did
/// not return is ungrouped, so every served `group_id` names a served group.
pub(super) fn discovery_neighbor_served_groups(
    items: &mut [DiscoveryNeighborItemRecord],
    groups: Vec<crate::DiscoveryRelatedGroup>,
) -> Vec<crate::DiscoveryRelatedGroup> {
    let known = groups.iter().map(|group| group.id).collect::<HashSet<_>>();
    let mut served = HashMap::<i64, i64>::new();
    for neighbor in items.iter_mut() {
        match neighbor.group_id {
            Some(id) if known.contains(&id) => *served.entry(id).or_default() += 1,
            Some(_) => neighbor.group_id = None,
            None => {}
        }
    }
    groups
        .into_iter()
        .filter_map(|mut group| {
            group.size = *served.get(&group.id)?;
            Some(group)
        })
        .collect()
}

#[cfg(test)]
#[path = "neighbors_tests.rs"]
mod tests;

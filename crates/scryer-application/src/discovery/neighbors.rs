//! Related titles for any discovery anchor, owned or not, so a graph view can
//! expand a node and recognise the neighbours the viewer already holds.

use super::*;
use crate::catalog_workflow::{
    TITLE_MORE_LIKE_THIS_BACKGROUND_REFRESH_HOURS, TITLE_MORE_LIKE_THIS_HYDRATION_LIMIT,
};
use crate::error_reasons;
use crate::ports::DiscoveryNeighborCacheRecord;

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
    /// library of the facet the actor may view, the answer is an empty list
    /// and never a refusal.
    pub async fn discovery_neighbors(
        &self,
        actor: &User,
        query: DiscoveryNeighborsQuery,
    ) -> AppResult<Vec<DiscoveryItemRecord>> {
        let visibility = self.discovery_visibility(actor).await?;
        if !visibility.allows_facet(&query.facet) {
            return Ok(Vec::new());
        }
        let effective_library_ids = self
            .discovery_effective_library_ids(actor, &query.facet, &[])
            .await?;
        if effective_library_ids.is_empty() || !self.personalized_discovery_enabled().await? {
            return Ok(Vec::new());
        }
        let (target_key, subject) = discovery_neighbor_subject(&query.target_key, &query.facet)?;
        let requested_limit = query
            .limit
            .clamp(0, TITLE_MORE_LIKE_THIS_HYDRATION_LIMIT as i64)
            as usize;
        if requested_limit == 0 {
            return Ok(Vec::new());
        }

        let mut items = self.discovery_neighbor_items(&target_key, subject).await?;
        items.truncate(requested_limit);
        let readable_library_ids = self
            .authorized_library_ids(actor, None, LibraryPermission::View)
            .await?
            .into_iter()
            .collect::<HashSet<_>>();
        let ownership = self
            .discovery_items_readable_ownership(&items, &readable_library_ids)
            .await?;
        for (item, owned) in items.iter_mut().zip(ownership) {
            item.resolved_title_id = owned.readable_title_id;
            item.owned_in_input = false;
        }
        Ok(items)
    }

    /// The anchor's related titles from the cache while it is fresh, else
    /// from the gateway, which then refreshes the cache. A failed cache write
    /// is logged and the caller gets the gateway's answer as mapped.
    async fn discovery_neighbor_items(
        &self,
        target_key: &str,
        subject: DiscoverySubjectInput,
    ) -> AppResult<Vec<DiscoveryItemRecord>> {
        let language = self.metadata_language().await;
        let now = self.runtime.environment.now();
        let ttl = chrono::Duration::hours(TITLE_MORE_LIKE_THIS_BACKGROUND_REFRESH_HOURS);
        if let Some(cached) = self
            .services
            .library
            .discovery
            .get_discovery_neighbor_cache(target_key, &language)
            .await?
            && now.signed_duration_since(cached.fetched_at) < ttl
        {
            return Ok(cached.items);
        }

        let input = crate::TitleRecommendationsInput {
            subject,
            query: String::new(),
            limit: TITLE_MORE_LIKE_THIS_HYDRATION_LIMIT as i32,
            language: language.clone(),
            include_unresolved: true,
        };
        let recommendations = self
            .services
            .library
            .metadata_gateway
            .title_recommendations(&input)
            .await?
            .results;
        let mut items = title_more_like_this_item_records(
            target_key,
            &[target_key.to_string()],
            &recommendations,
            TITLE_MORE_LIKE_THIS_HYDRATION_LIMIT,
            now,
        )?;
        let run_id = format!("neighbors:{target_key}");
        for item in &mut items {
            item.id = format!("{run_id}:{}", item.target_key);
            item.run_id = run_id.clone();
            item.base_generation_id = None;
        }
        let record = DiscoveryNeighborCacheRecord {
            target_key: target_key.to_string(),
            language,
            items,
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
            return Ok(record.items);
        }
        // Read the entry back so a first expansion serves exactly what a
        // cached one will: the store enriches and screens what it keeps. A
        // failed read-back still serves the gateway's answer as mapped.
        match discovery
            .get_discovery_neighbor_cache(target_key, &record.language)
            .await
        {
            Ok(Some(cached)) => Ok(cached.items),
            Ok(None) => Ok(record.items),
            Err(error) => {
                warn!(
                    target_key,
                    error = %error,
                    "failed to read back cached discovery neighbors"
                );
                Ok(record.items)
            }
        }
    }
}

#[cfg(test)]
#[path = "neighbors_tests.rs"]
mod tests;

//! What a list route asks of the titles it lands.
//!
//! A route's options reach a title two ways: a list add creates the title
//! directly, and a Request or Hold list files a request that a reviewer or a
//! rule approves later. Both read the route through here, so a title the list
//! brought in carries the same settings whichever way it arrived.

use scryer_domain::{
    ListFilter, ListRoute, ListSubscription, MediaFacet, MediaRequest, MediaRequestOrigin,
    NewTitle, RELEASE_NUMBERING_TAG_PREFIX, ReleaseNumbering,
};

use crate::AppUseCase;
use crate::media_requests::{
    TITLE_MONITOR_TYPE_TAG_PREFIX, TITLE_QUALITY_PROFILE_TAG_PREFIX,
    normalize_requested_monitor_type,
};

const SEASON_FOLDER_TAG_PREFIX: &str = "scryer:season-folder:";

#[cfg(test)]
mod episode_policy_tests {
    use super::*;

    #[test]
    fn list_episode_policies_reuse_title_overrides_and_keep_facets_isolated() {
        let filters = vec![
            ListFilter::MonitorSpecials {
                facet: MediaFacet::Series,
                enabled: false,
            },
            ListFilter::MonitorSpecials {
                facet: MediaFacet::Anime,
                enabled: true,
            },
            ListFilter::FillerPolicy {
                facet: MediaFacet::Anime,
                skip: true,
            },
            ListFilter::RecapPolicy {
                facet: MediaFacet::Anime,
                skip: false,
            },
        ];
        assert_eq!(
            episode_policy_tags(&filters, &MediaFacet::Series),
            vec!["scryer:monitor-specials:false"]
        );
        assert_eq!(
            episode_policy_tags(&filters, &MediaFacet::Anime),
            vec![
                "scryer:filler-policy:skip_filler",
                "scryer:monitor-specials:true",
                "scryer:recap-policy:download_all",
            ]
        );
        assert!(episode_policy_tags(&filters, &MediaFacet::Movie).is_empty());
        assert!(episode_policy_tags(&[], &MediaFacet::Anime).is_empty());
    }

    #[test]
    fn list_episode_policy_json_round_trip_preserves_explicit_download_all() {
        let filters = vec![ListFilter::FillerPolicy {
            facet: MediaFacet::Anime,
            skip: false,
        }];
        let json = serde_json::to_string(&filters).unwrap();
        let restored: Vec<ListFilter> = serde_json::from_str(&json).unwrap();
        assert_eq!(filters, restored);
        assert_eq!(
            episode_policy_tags(&restored, &MediaFacet::Anime),
            vec!["scryer:filler-policy:download_all"]
        );
    }
}

/// Reuse the title overrides consumed by the ordinary metadata hydration and
/// wanted policies. Absence preserves library inheritance.
pub(crate) fn episode_policy_tags(filters: &[ListFilter], facet: &MediaFacet) -> Vec<String> {
    let mut tags = std::collections::BTreeMap::new();
    for filter in filters {
        let (scope, prefix, value) = match filter {
            ListFilter::MonitorSpecials { facet, enabled } => {
                (facet, "scryer:monitor-specials:", enabled.to_string())
            }
            ListFilter::FillerPolicy { facet, skip } if *facet == MediaFacet::Anime => (
                facet,
                "scryer:filler-policy:",
                if *skip { "skip_filler" } else { "download_all" }.into(),
            ),
            ListFilter::RecapPolicy { facet, skip } if *facet == MediaFacet::Anime => (
                facet,
                "scryer:recap-policy:",
                if *skip { "skip_recap" } else { "download_all" }.into(),
            ),
            _ => continue,
        };
        if scope == facet && *facet != MediaFacet::Movie {
            tags.insert(prefix, value);
        }
    }
    tags.into_iter()
        .map(|(prefix, value)| format!("{prefix}{value}"))
        .collect()
}

/// The route's monitor type in the title vocabulary, or `None` when it names
/// none or one the title cannot carry.
pub(crate) fn route_monitor_type(route: &ListRoute) -> Option<String> {
    normalize_requested_monitor_type(&route.kind, Some(route.monitor_type.clone()))
        .ok()
        .flatten()
}

/// The structured tags for the folder layout and episode numbering the route
/// asks for. A request carries its own quality profile and monitor type, so
/// these are the options a request alone would lose.
pub(crate) fn route_layout_tags(route: &ListRoute) -> Vec<String> {
    let mut tags = Vec::new();
    if route.kind != MediaFacet::Movie {
        if let Some(enabled) = route.use_season_folders {
            let value = if enabled { "enabled" } else { "disabled" };
            tags.push(format!("{SEASON_FOLDER_TAG_PREFIX}{value}"));
        }
        // `Auto` is the absence of the tag.
        let numbering = route.release_numbering.as_deref().map_or(
            ReleaseNumbering::Auto,
            ReleaseNumbering::from_str_or_default,
        );
        if numbering != ReleaseNumbering::Auto {
            tags.push(format!(
                "{RELEASE_NUMBERING_TAG_PREFIX}{}",
                numbering.as_str()
            ));
        }
    }
    tags
}

/// Every structured tag a title the route adds is created with: its quality
/// profile, monitor type, folder layout and numbering.
pub(crate) fn route_option_tags(route: &ListRoute) -> Vec<String> {
    let mut tags = Vec::new();
    if let Some(profile_id) = route
        .quality_profile_id
        .as_deref()
        .map(str::trim)
        .filter(|profile_id| !profile_id.is_empty())
    {
        tags.push(format!("{TITLE_QUALITY_PROFILE_TAG_PREFIX}{profile_id}"));
    }
    if let Some(monitor_type) = route_monitor_type(route) {
        tags.push(format!("{TITLE_MONITOR_TYPE_TAG_PREFIX}{monitor_type}"));
    }
    tags.extend(route_layout_tags(route));
    tags
}

/// The route a list request came through, while its list still routes the
/// request's kind into the request's library. A manual request, an
/// unfollowed list, or a route since moved to another library has none.
pub(crate) fn route_for_request(
    subscription: &ListSubscription,
    facet: &MediaFacet,
    library_id: &str,
) -> Option<ListRoute> {
    subscription
        .route_for(facet.clone())
        .filter(|route| route.library_id == library_id)
        .cloned()
}

impl AppUseCase {
    /// The route of the list a request came from. Best effort: a request
    /// whose list cannot be read is approved as a manual one would be.
    pub(crate) async fn list_route_for_request(
        &self,
        origin: &MediaRequestOrigin,
        facet: &MediaFacet,
        library_id: &str,
    ) -> Option<ListRoute> {
        self.list_route_and_filters_for_request(origin, facet, library_id)
            .await
            .map(|(route, _)| route)
    }

    async fn list_route_and_filters_for_request(
        &self,
        origin: &MediaRequestOrigin,
        facet: &MediaFacet,
        library_id: &str,
    ) -> Option<(ListRoute, Vec<ListFilter>)> {
        let subscription_id = origin.subscription_id()?;
        match self
            .services
            .lists
            .subscriptions
            .get_by_id(subscription_id)
            .await
        {
            Ok(subscription) => subscription.and_then(|subscription| {
                route_for_request(&subscription, facet, library_id)
                    .map(|route| (route, subscription.filters))
            }),
            Err(error) => {
                tracing::warn!(
                    subscription_id,
                    error = %error,
                    "could not read the list a request came from"
                );
                None
            }
        }
    }

    /// Add the labels a list request's route applies to `tags`, the tags the
    /// request carries to its title. They sit beside the policy's tags, so a
    /// reviewer sees them on the request and may remove them at approval.
    /// Narrowed to labels the registry defines, like the policy's own: a
    /// label deleted since the list was set up never blocks the approval.
    pub(crate) async fn add_list_request_tags(
        &self,
        origin: &MediaRequestOrigin,
        facet: &MediaFacet,
        library_id: &str,
        tags: &mut Vec<String>,
    ) {
        let Some(route) = self.list_route_for_request(origin, facet, library_id).await else {
            return;
        };
        let labels = route
            .tags
            .iter()
            .map(|label| label.trim().to_lowercase())
            .filter(|label| !label.is_empty() && !crate::is_reserved_title_tag(label))
            .collect::<Vec<_>>();
        if labels.is_empty() {
            return;
        }
        let undefined = match self.undefined_title_tag_labels(&labels).await {
            Ok(undefined) => undefined,
            Err(error) => {
                tracing::warn!(error = %error, "could not read the tag registry for a list request");
                return;
            }
        };
        for label in labels {
            if !undefined.contains(&label) && !tags.contains(&label) {
                tags.push(label);
            }
        }
    }

    /// Give a list request's title the folder and numbering its route asks
    /// for. The request already carries the route's quality profile and
    /// monitor type, and its labels ride in its tags.
    pub(crate) async fn apply_list_route_to_request_title(
        &self,
        request: &MediaRequest,
        title: &mut NewTitle,
    ) {
        let Some((route, filters)) = self
            .list_route_and_filters_for_request(
                &request.origin,
                &request.facet,
                &request.library_id,
            )
            .await
        else {
            return;
        };
        for tag in route_layout_tags(&route)
            .into_iter()
            .chain(episode_policy_tags(&filters, &route.kind))
        {
            if !title.tags.contains(&tag) {
                title.tags.push(tag);
            }
        }
        if route.kind == MediaFacet::Movie {
            title.min_availability = route.min_availability.clone();
        }
        // A root removed from the library since the list was set up falls
        // back to the library default rather than failing the approval.
        if let Some(root_folder_id) = route.root_folder_id.as_deref() {
            match self
                .services
                .catalog
                .libraries
                .get_by_id(&request.library_id)
                .await
            {
                Ok(Some(library)) if library.roots.iter().any(|root| root.id == root_folder_id) => {
                    title.root_folder_id = Some(root_folder_id.to_string());
                }
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(
                        request_id = %request.id,
                        error = %error,
                        "could not read the library for a list request's root folder"
                    );
                }
            }
        }
    }
}

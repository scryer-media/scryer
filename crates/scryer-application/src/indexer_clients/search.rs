//! Thin client authorization and routing over the interactive raw-search core.

use super::*;
use crate::catalog::interactive_release_search::{
    RawSearchSource,
    scope_filter::{RawSearchResultScope, category_contains},
};
use crate::{RawSearchPage, RawTextSearchRequest};
use scryer_domain::{IndexerConfig, IndexerProtocolCapability, IndexerProviderCapabilities};
use scryer_plugin_sdk::{PluginNabOperation, PluginNabSearch};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct IndexerClientSearchRequest {
    pub protocol: IndexerClientProtocol,
    pub operation: PluginNabOperation,
    pub query: String,
    pub parameters: BTreeMap<String, String>,
    pub categories: Vec<String>,
    pub offset: usize,
    /// Aggregate page size only. Upstream defaults stay with each adapter.
    pub limit: usize,
}

struct ResolvedSearch {
    sources: Vec<RawSearchSource>,
    identity: String,
}

impl AppUseCase {
    pub async fn search_indexer_client(
        &self,
        authenticated: &AuthenticatedIndexerClient,
        mut request: IndexerClientSearchRequest,
    ) -> AppResult<RawSearchPage> {
        self.revalidate_indexer_client(authenticated).await?;
        if !authenticated
            .client
            .configuration
            .protocols
            .contains(&request.protocol)
        {
            return Err(AppError::unauthorized_reason(
                reasons::PROTOCOL_FORBIDDEN,
                "This protocol is not enabled for the indexer client",
            ));
        }
        request.categories = normalize_categories(&request.categories)?;
        if request.parameters.keys().any(|name| {
            matches!(
                name.to_ascii_lowercase().as_str(),
                "apikey" | "t" | "q" | "cat" | "o" | "offset" | "limit" | "cursor"
            )
        }) {
            return Err(AppError::validation_refused(
                reasons::SEARCH_UNSUPPORTED,
                "Transport and authentication parameters cannot be search constraints",
            ));
        }
        let resolved = self
            .resolve_indexer_client_search(authenticated, &request)
            .await?;
        let identity = resolved.identity.clone();
        let response = self
            .search_scoped_raw_page(
                authenticated.principal(),
                resolved.identity,
                resolved.sources,
                request.offset,
                request.limit,
            )
            .await;
        // The same checks apply to cache hits and new source reads. A grant,
        // key or source change while waiting cannot leak an old scoped page.
        self.revalidate_indexer_client(authenticated).await?;
        if self
            .resolve_indexer_client_search(authenticated, &request)
            .await?
            .identity
            != identity
        {
            return Err(AppError::validation_refused(
                reasons::SOURCE_CHANGED,
                "Indexer configuration changed; retry the search",
            ));
        }
        response
    }

    async fn resolve_indexer_client_search(
        &self,
        authenticated: &AuthenticatedIndexerClient,
        request: &IndexerClientSearchRequest,
    ) -> AppResult<ResolvedSearch> {
        let provider = self
            .services
            .integrations
            .plugin_provider
            .available()
            .ok_or_else(|| {
                AppError::temporary_unavailable_reason(
                    reasons::UNAVAILABLE,
                    "Indexer providers are unavailable",
                    None,
                )
            })?;
        let configs = self
            .services
            .integrations
            .indexer_configs
            .list(None)
            .await?;
        let mut sources = Vec::new();
        let mut identities = Vec::new();
        let mut compatible = false;
        for grant in &authenticated.client.configuration.grants {
            let Some(config) = configs.iter().find(|config| config.id == grant.indexer_id) else {
                continue;
            };
            if !config.is_enabled
                || provider
                    .management_capabilities_for_provider(&config.provider_type)
                    .supports_managed_children_sync
            {
                continue;
            }
            let capabilities = provider.capabilities_for_provider(&config.provider_type);
            let protocol = protocol_kind(request.protocol);
            if !capabilities
                .protocols
                .iter()
                .any(|kind| *kind == protocol || *kind == IndexerProtocolCapability::Mixed)
            {
                continue;
            }
            let parent = config
                .managed_parent_config_id
                .as_deref()
                .and_then(|id| configs.iter().find(|candidate| candidate.id == id));
            identities.push(serde_json::json!({
                "source": config.id,
                "identity": search_identity(config, provider.as_ref()),
                "parent": parent.map(|parent| search_identity(parent, provider.as_ref())),
                "capabilities": capabilities,
                "grant": grant.categories,
            }));
            // Capability discoveries are not connection edits. Include the
            // granted source in the identity even when its current caps exclude
            // this operation; a refresh must not split an existing page walk.
            let caps = crate::cached_indexer_caps(config, self.runtime.environment.now());
            let Some(mut raw_request) = source_request(request, &capabilities, &caps) else {
                continue;
            };
            compatible = true;
            let Some(categories) =
                intersect_categories(&request.categories, grant.categories.as_deref())
            else {
                continue;
            };
            let result_categories = (!categories.is_empty()).then(|| categories.clone());
            raw_request.categories = categories;
            sources.push(RawSearchSource {
                id: config.id.clone(),
                paged: capabilities
                    .limits
                    .as_ref()
                    .is_some_and(|limits| limits.paged_search),
                result_scope: RawSearchResultScope {
                    protocol: Some(protocol),
                    allow_untyped_protocol: capabilities
                        .protocols
                        .iter()
                        .all(|kind| *kind == protocol),
                    categories: result_categories,
                    allow_uncategorized: grant.categories.is_none(),
                },
                request: raw_request,
            });
        }
        if !compatible && !authenticated.client.configuration.grants.is_empty() {
            return Err(AppError::validation_refused(
                reasons::SEARCH_UNSUPPORTED,
                "No granted indexer supports this search operation and parameter combination",
            ));
        }
        // Grant ordering is canonical at write time; sort source identities as
        // well so a repository's return order never splits an offset walk.
        identities.sort_by(|left, right| left["source"].as_str().cmp(&right["source"].as_str()));
        let identity = crate::helpers::blake3_identity_hex(
            crate::helpers::HashDomain::RawSearchScope,
            serde_json::json!({
                "protocol": request.protocol, "operation": request.operation,
                "query": request.query, "parameters": request.parameters,
                "categories": request.categories, "sources": identities,
            })
            .to_string(),
        );
        Ok(ResolvedSearch { sources, identity })
    }
}

fn search_identity(
    config: &IndexerConfig,
    provider: &dyn crate::IndexerPluginProvider,
) -> serde_json::Value {
    let mut identity = crate::indexer_search_identity(
        config,
        provider.search_semantics_version_for_provider(&config.provider_type),
    );
    // Caps discovery updates source knowledge, not operator configuration.
    // Preserve the shared connection/search projection without its snapshot.
    if let Some(identity) = identity.as_object_mut() {
        identity.remove("caps");
    }
    serde_json::json!({
        "enabled": config.is_enabled,
        "search": identity,
    })
}

pub(super) fn protocol_kind(protocol: IndexerClientProtocol) -> IndexerProtocolCapability {
    match protocol {
        IndexerClientProtocol::Newznab => IndexerProtocolCapability::Usenet,
        IndexerClientProtocol::Torznab => IndexerProtocolCapability::Torrent,
    }
}

fn source_request(
    request: &IndexerClientSearchRequest,
    capabilities: &IndexerProviderCapabilities,
    resolved: &crate::ResolvedIndexerCaps,
) -> Option<RawTextSearchRequest> {
    if !capabilities.raw_nab_search {
        // Native JSON adapters already understand raw text. Wire format is
        // irrelevant, but dropping a TV/music/book constraint is never valid.
        return (request.operation == PluginNabOperation::Search
            && request.parameters.is_empty()
            && !request.query.trim().is_empty()
            && capabilities.query_param.is_some())
        .then(|| RawTextSearchRequest {
            query: request.query.clone(),
            ..Default::default()
        });
    }
    let caps = resolved.snapshot.as_ref();
    let node = caps.and_then(|caps| match request.operation {
        PluginNabOperation::Search => caps.search.as_ref(),
        PluginNabOperation::Tv => caps.tv_search.as_ref(),
        PluginNabOperation::Movie => caps.movie_search.as_ref(),
        PluginNabOperation::Music => caps.music_search.as_ref().or(caps.audio_search.as_ref()),
        PluginNabOperation::Book => caps.book_search.as_ref(),
    });
    let supported = node.map_or_else(
        // Missing searching data is unknown, not a negative capability claim.
        // The adapter attempts the complete typed request without weakening it.
        || !caps.is_some_and(|caps| caps.searching_present),
        |node| {
            node.available
                && (request.query.is_empty()
                    || node.supported_params.iter().any(|param| param == "q"))
                && request
                    .parameters
                    .keys()
                    .all(|param| node.supported_params.contains(param))
        },
    );
    // Stale or missing discovery cannot refuse a request before the source's
    // existing accounted resolver runs. Its Nab adapter validates the exact
    // operation against the resolved caps afterward, without a broader fallback.
    let discovery_pending = matches!(
        resolved.source,
        crate::IndexerCapsSource::Stale | crate::IndexerCapsSource::Permissive
    );
    if !supported && !discovery_pending {
        return None;
    }
    Some(RawTextSearchRequest {
        query: request.query.clone(),
        nab: Some(PluginNabSearch {
            operation: request.operation,
            parameters: request.parameters.clone(),
        }),
        recent_paging: request.operation == PluginNabOperation::Search
            && request.query.trim().is_empty(),
        ..Default::default()
    })
}

fn normalize_categories(categories: &[String]) -> AppResult<Vec<String>> {
    let mut normalized = categories
        .iter()
        .map(|value| {
            if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(AppError::validation_refused(
                    reasons::CATEGORY_INVALID,
                    "Compatibility category identifiers must be numeric",
                ));
            }
            value
                .parse::<u32>()
                .map(|value| value.to_string())
                .map_err(|_| {
                    AppError::validation_refused(
                        reasons::CATEGORY_INVALID,
                        "Compatibility category identifier is out of range",
                    )
                })
        })
        .collect::<AppResult<Vec<_>>>()?;
    normalized.sort();
    normalized.dedup();
    Ok(normalized)
}

fn intersect_categories(requested: &[String], allowed: Option<&[String]>) -> Option<Vec<String>> {
    let Some(allowed) = allowed else {
        return Some(requested.to_vec());
    };
    // String-coded categories remain a raw-UI facility. They cannot be silently
    // sent as a Nab numeric category or converted to an unconstrained request.
    let allowed = allowed
        .iter()
        .filter(|value| value.parse::<u32>().is_ok())
        .collect::<Vec<_>>();
    let mut intersection = if requested.is_empty() {
        allowed.into_iter().cloned().collect::<Vec<_>>()
    } else {
        requested
            .iter()
            .flat_map(|requested| {
                allowed.iter().filter_map(move |allowed| {
                    if category_contains(allowed, requested) {
                        Some(requested.clone())
                    } else if category_contains(requested, allowed) {
                        Some((*allowed).clone())
                    } else {
                        None
                    }
                })
            })
            .collect::<Vec<_>>()
    };
    intersection.sort();
    intersection.dedup();
    (!intersection.is_empty()).then_some(intersection)
}

#[cfg(test)]
mod tests;

use super::*;
use crate::catalog::interactive_release_search::{
    RAW_SEARCH_PAGE_SIZE, scope_filter::category_contains,
};
use scryer_domain::{
    IndexerCapsSearchNode, IndexerCapsSnapshot, IndexerCategoryDescriptor,
    IndexerCategoryValueKind, IndexerProtocolCapability, IndexerProviderCapabilities,
};
use std::collections::BTreeMap;

impl AppUseCase {
    /// Read-only discovery for an authenticated client. Only the normal source
    /// execution path may refresh caps; this projection never makes a request.
    pub async fn indexer_client_caps(
        &self,
        authenticated: &AuthenticatedIndexerClient,
        protocol: IndexerClientProtocol,
    ) -> AppResult<IndexerCapsSnapshot> {
        self.revalidate_indexer_client(authenticated).await?;
        if !authenticated
            .client
            .configuration
            .protocols
            .contains(&protocol)
        {
            return Err(AppError::unauthorized_reason(
                reasons::PROTOCOL_FORBIDDEN,
                "This protocol is not enabled for the indexer client",
            ));
        }
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
        let mut configs = self
            .services
            .integrations
            .indexer_configs
            .list(None)
            .await?;
        configs.sort_by(|left, right| left.id.cmp(&right.id));
        let mut caps = empty_caps();
        let mut categories = BTreeMap::new();
        for config in configs {
            let Some(grant) = authenticated
                .client
                .configuration
                .grants
                .iter()
                .find(|grant| grant.indexer_id == config.id)
            else {
                continue;
            };
            if !config.is_enabled
                || grant.categories.as_ref().is_some_and(Vec::is_empty)
                || provider
                    .management_capabilities_for_provider(&config.provider_type)
                    .supports_managed_children_sync
            {
                continue;
            }
            let declared = provider.capabilities_for_provider(&config.provider_type);
            let required = super::search::protocol_kind(protocol);
            if !declared
                .protocols
                .iter()
                .any(|kind| *kind == required || *kind == IndexerProtocolCapability::Mixed)
            {
                continue;
            }
            let cached = crate::cached_indexer_caps(&config, self.runtime.environment.now());
            merge_source(
                &mut caps,
                &mut categories,
                &declared,
                cached.snapshot.as_ref(),
                grant.categories.as_deref(),
            );
        }
        caps.categories.categories = categories.into_values().collect();
        if !caps.categories.categories.is_empty() {
            caps.categories.value_kinds = vec![IndexerCategoryValueKind::Numeric];
        }
        self.revalidate_indexer_client(authenticated).await?;
        Ok(caps)
    }
}

fn empty_caps() -> IndexerCapsSnapshot {
    IndexerCapsSnapshot {
        parser_version: scryer_domain::INDEXER_CAPS_PARSER_VERSION,
        searching_present: true,
        limits_default: Some(RAW_SEARCH_PAGE_SIZE as i64),
        limits_max: Some(RAW_SEARCH_PAGE_SIZE as i64),
        search: Some(Default::default()),
        tv_search: Some(Default::default()),
        movie_search: Some(Default::default()),
        music_search: Some(Default::default()),
        book_search: Some(Default::default()),
        ..Default::default()
    }
}

fn merge_node(target: &mut Option<IndexerCapsSearchNode>, source: Option<&IndexerCapsSearchNode>) {
    let Some(source) = source.filter(|node| node.available) else {
        return;
    };
    let target = target.get_or_insert_with(Default::default);
    target.available = true;
    target
        .supported_params
        .extend(source.supported_params.iter().cloned());
    target.supported_params.sort();
    target.supported_params.dedup();
}

fn merge_source(
    caps: &mut IndexerCapsSnapshot,
    categories: &mut BTreeMap<u32, IndexerCategoryDescriptor>,
    declared: &IndexerProviderCapabilities,
    cached: Option<&IndexerCapsSnapshot>,
    allowed: Option<&[String]>,
) {
    // A grant containing only string category codes is not executable on Nab.
    if allowed.is_some_and(|values| !values.iter().any(|value| numeric_id(value).is_some())) {
        return;
    }
    if declared.raw_nab_search {
        if let Some(cached) = cached.filter(|cached| cached.searching_present) {
            merge_node(&mut caps.search, cached.search.as_ref());
            merge_node(&mut caps.tv_search, cached.tv_search.as_ref());
            merge_node(&mut caps.movie_search, cached.movie_search.as_ref());
            merge_node(
                &mut caps.music_search,
                cached
                    .music_search
                    .as_ref()
                    .or(cached.audio_search.as_ref()),
            );
            merge_node(&mut caps.book_search, cached.book_search.as_ref());
        } else if declared.query_param.is_some() || declared.rss {
            merge_node(
                &mut caps.search,
                Some(&IndexerCapsSearchNode {
                    available: true,
                    supported_params: if declared.query_param.is_some() {
                        vec!["q".into()]
                    } else {
                        Vec::new()
                    },
                    search_engine: None,
                }),
            );
        }
    } else if declared.query_param.is_some() {
        // A native JSON adapter's exact executable surface is generic text.
        merge_node(
            &mut caps.search,
            Some(&IndexerCapsSearchNode {
                available: true,
                supported_params: vec!["q".into()],
                search_engine: None,
            }),
        );
    }
    let model = cached
        .map(|cached| &cached.categories)
        .filter(|model| !model.categories.is_empty())
        .or(declared.category_model.as_ref());
    for category in model.into_iter().flat_map(|model| &model.categories) {
        if category.value_kind != IndexerCategoryValueKind::Numeric {
            continue;
        }
        let Some(id) = numeric_id(&category.value) else {
            continue;
        };
        let value = id.to_string();
        if allowed
            .is_some_and(|allowed| !allowed.iter().any(|grant| category_contains(grant, &value)))
        {
            continue;
        }
        // Match existing category overlays: first occurrence wins, preserving
        // the source label; custom identifiers never become synthetic ranges.
        categories
            .entry(id)
            .or_insert_with(|| IndexerCategoryDescriptor {
                value,
                ..category.clone()
            });
    }
}

fn numeric_id(value: &str) -> Option<u32> {
    (!value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| value.parse().ok())
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> IndexerProviderCapabilities {
        IndexerProviderCapabilities {
            raw_nab_search: true,
            query_param: Some("q".into()),
            ..Default::default()
        }
    }

    #[test]
    fn cached_modes_union_without_inventing_typed_modes_from_provider_ids() {
        let mut caps = empty_caps();
        let mut categories = BTreeMap::new();
        let mut declared = source();
        declared
            .supported_ids
            .insert("movie".into(), vec!["imdb_id".into()]);
        merge_source(&mut caps, &mut categories, &declared, None, None);
        assert!(caps.search.as_ref().unwrap().available);
        assert!(!caps.movie_search.as_ref().unwrap().available);
        let mut cached = IndexerCapsSnapshot {
            searching_present: true,
            movie_search: Some(IndexerCapsSearchNode {
                available: true,
                supported_params: vec!["q".into(), "imdbid".into()],
                search_engine: None,
            }),
            ..Default::default()
        };
        merge_source(&mut caps, &mut categories, &declared, Some(&cached), None);
        cached.movie_search.as_mut().unwrap().supported_params = vec!["tmdbid".into()];
        merge_source(&mut caps, &mut categories, &declared, Some(&cached), None);
        assert_eq!(
            caps.movie_search.unwrap().supported_params,
            ["imdbid", "q", "tmdbid"]
        );
        assert!(!caps.tv_search.unwrap().available);
        assert_eq!(
            (caps.limits_default, caps.limits_max),
            (Some(100), Some(100))
        );
    }

    #[test]
    fn categories_obey_grants_keep_custom_ids_and_never_advertise_string_codes() {
        let mut cached = IndexerCapsSnapshot::default();
        for (value, kind) in [
            ("4000", IndexerCategoryValueKind::Numeric),
            ("4010", IndexerCategoryValueKind::Numeric),
            ("6000", IndexerCategoryValueKind::Numeric),
            ("100123", IndexerCategoryValueKind::Numeric),
            ("100124", IndexerCategoryValueKind::Numeric),
            ("9001", IndexerCategoryValueKind::String),
        ] {
            cached
                .categories
                .categories
                .push(IndexerCategoryDescriptor {
                    value: value.into(),
                    label: Some(value.into()),
                    value_kind: kind,
                    facets: Vec::new(),
                });
        }
        let mut caps = empty_caps();
        let mut categories = BTreeMap::new();
        merge_source(
            &mut caps,
            &mut categories,
            &source(),
            Some(&cached),
            Some(&["4000".into(), "100123".into()]),
        );
        assert_eq!(
            categories.keys().copied().collect::<Vec<_>>(),
            [4000, 4010, 100123]
        );
        let mut empty = empty_caps();
        merge_source(
            &mut empty,
            &mut BTreeMap::new(),
            &source(),
            Some(&cached),
            Some(&[]),
        );
        assert!(!empty.search.unwrap().available);
    }
}

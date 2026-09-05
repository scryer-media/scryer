//! The test indexer's plugin descriptor.
//!
//! Shared by the `test-indexer` component (whose `describe` export returns it)
//! and by `cargo xtask build-test-plugin-fixture`, which embeds it as the
//! artifact's top-level descriptor custom section: the loader identifies WASI
//! Preview 2 indexer components solely by that section, exactly as it does for
//! shipped plugins.

use scryer_plugin_sdk::{
    ConfigFieldDef, ConfigFieldRole, ConfigFieldType, ConfigFieldValueSource, IndexerCapabilities,
    IndexerDescriptor, IndexerSourceKind, PluginDescriptor, ProviderDescriptor, SDK_VERSION,
    current_sdk_constraint,
};
use std::collections::HashMap;

pub fn descriptor() -> PluginDescriptor {
    PluginDescriptor {
        id: "test".to_string(),
        name: "Test Indexer".to_string(),
        version: "0.1.0".to_string(),
        sdk_version: SDK_VERSION.to_string(),
        sdk_constraint: current_sdk_constraint(),
        socket_permissions: vec![],
        provider: ProviderDescriptor::Indexer(IndexerDescriptor {
            provider_type: "test".to_string(),
            provider_aliases: vec![],
            provider_profiles: vec![],
            source_kind: IndexerSourceKind::Generic,
            capabilities: IndexerCapabilities {
                supported_ids: HashMap::from([("movie".into(), vec!["imdb_id".into()])]),
                query_param: Some("q".into()),
                search: true,
                imdb_search: true,
                tvdb_search: false,
                ..IndexerCapabilities::default()
            },
            scoring_policies: vec![],
            config_fields: vec![ConfigFieldDef {
                key: "base_url".to_string(),
                label: "Base URL".to_string(),
                field_type: ConfigFieldType::String,
                required: true,
                default_value: None,
                value_source: ConfigFieldValueSource::User,
                role: Some(ConfigFieldRole::ConnectionUrl),
                host_binding: None,
                options: vec![],
                help_text: None,
                ..Default::default()
            }],
            allowed_hosts: vec![],
            rate_limit_seconds: None,
            search_semantics_version: Some(1),
            strategy_plan: None,
        }),
    }
}

use extism_pdk::*;
use scryer_plugin_sdk::*;
use std::collections::HashMap;

#[plugin_fn]
pub fn scryer_describe(_input: String) -> FnResult<String> {
    let descriptor = PluginDescriptor {
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
            }],
            allowed_hosts: vec![],
            rate_limit_seconds: None,
            search_semantics_version: Some(1),
            strategy_plan: None,
        }),
    };
    Ok(serde_json::to_string(&descriptor)?)
}

#[plugin_fn]
pub fn scryer_indexer_search(input: String) -> FnResult<String> {
    let req: PluginSearchRequest = serde_json::from_str(&input)?;
    let limit = if req.limit == 0 { 10 } else { req.limit };

    let results = vec![PluginSearchResult {
        title: format!("{} 2024 2160p WEB-DL H.265", req.query),
        link: Some("https://example.com/details/12345".to_string()),
        download_url: Some("https://example.com/download/12345.nzb".to_string()),
        size_bytes: Some(8_000_000_000),
        published_at: Some("2024-06-15T00:00:00Z".to_string()),
        grabs: Some(42),
        languages: vec!["English".to_string()],
        provider_extra: HashMap::new(),
        thumbs_up: None,
        thumbs_down: None,
        subtitles: vec![],
        password_hint: None,
        protected: None,
        guid: None,
        info_url: None,
        ..PluginSearchResult::default()
    }];

    Ok(serde_json::to_string(&PluginResult::Ok(
        PluginSearchResponse {
            results: results.into_iter().take(limit).collect(),
            ..PluginSearchResponse::default()
        },
    ))?)
}

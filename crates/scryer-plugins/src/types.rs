pub use scryer_plugin_sdk::*;

pub(crate) fn config_field_to_domain(field: &ConfigFieldDef) -> scryer_domain::ConfigFieldDef {
    scryer_domain::ConfigFieldDef {
        key: field.key.clone(),
        label: field.label.clone(),
        field_type: match field.field_type {
            ConfigFieldType::String => scryer_domain::ConfigFieldType::String,
            ConfigFieldType::Password => scryer_domain::ConfigFieldType::Password,
            ConfigFieldType::Multiline => scryer_domain::ConfigFieldType::Multiline,
            ConfigFieldType::Bool => scryer_domain::ConfigFieldType::Bool,
            ConfigFieldType::Select => scryer_domain::ConfigFieldType::Select,
            ConfigFieldType::FilteredSelect => scryer_domain::ConfigFieldType::FilteredSelect,
            ConfigFieldType::Number => scryer_domain::ConfigFieldType::Number,
            ConfigFieldType::Path => scryer_domain::ConfigFieldType::Path,
            ConfigFieldType::Tag => scryer_domain::ConfigFieldType::Tag,
        },
        required: field.required,
        default_value: field.default_value.clone(),
        value_source: match field.value_source {
            ConfigFieldValueSource::User => scryer_domain::ConfigFieldValueSource::User,
            ConfigFieldValueSource::HostBinding => {
                scryer_domain::ConfigFieldValueSource::HostBinding
            }
        },
        role: field.role.map(|role| match role {
            ConfigFieldRole::ConnectionUrl => scryer_domain::ConfigFieldRole::ConnectionUrl,
        }),
        host_binding: field.host_binding.map(host_binding_to_domain),
        options: field
            .options
            .iter()
            .map(|option| scryer_domain::ConfigFieldOption {
                value: option.value.clone(),
                label: option.label.clone(),
                config_overrides: option.config_overrides.clone(),
            })
            .collect(),
        help_text: field.help_text.clone(),
        visible_when: field.visible_when.as_ref().map(field_condition_to_domain),
        required_when: field.required_when.as_ref().map(field_condition_to_domain),
        advanced: field.advanced,
    }
}

pub(crate) fn field_condition_to_domain(
    condition: &FieldCondition,
) -> scryer_domain::FieldCondition {
    scryer_domain::FieldCondition {
        key: condition.key.clone(),
        op: match condition.op {
            ConditionOp::Eq => scryer_domain::ConditionOp::Eq,
            ConditionOp::Ne => scryer_domain::ConditionOp::Ne,
            ConditionOp::In => scryer_domain::ConditionOp::In,
            ConditionOp::NotIn => scryer_domain::ConditionOp::NotIn,
            ConditionOp::NonEmpty => scryer_domain::ConditionOp::NonEmpty,
        },
        values: condition.values.clone(),
    }
}

pub(crate) fn config_fields_to_domain(
    fields: &[ConfigFieldDef],
) -> Vec<scryer_domain::ConfigFieldDef> {
    fields.iter().map(config_field_to_domain).collect()
}

pub(crate) fn host_binding_to_domain(
    binding: PluginHostBindingId,
) -> scryer_domain::PluginHostBindingId {
    match binding {
        PluginHostBindingId::SmgOpenSubtitlesApiKey => {
            scryer_domain::PluginHostBindingId::SmgOpenSubtitlesApiKey
        }
    }
}

pub(crate) fn indexer_capabilities_to_domain(
    capabilities: &IndexerCapabilities,
) -> scryer_domain::IndexerProviderCapabilities {
    scryer_domain::IndexerProviderCapabilities {
        rss: capabilities.rss,
        supported_ids: capabilities.supported_ids.clone(),
        deduplicates_aliases: capabilities.deduplicates_aliases,
        season_param: capabilities.season_param.clone(),
        episode_param: capabilities.episode_param.clone(),
        query_param: capabilities.query_param.clone(),
        supported_query_facets: capabilities.supported_query_facets.clone(),
        search: capabilities.search,
        imdb_search: capabilities.imdb_search,
        tvdb_search: capabilities.tvdb_search,
        anidb_search: capabilities.anidb_search,
        protocols: capabilities
            .protocols
            .iter()
            .copied()
            .map(indexer_protocol_to_domain)
            .collect(),
        feed_modes: capabilities
            .feed_modes
            .iter()
            .copied()
            .map(indexer_feed_mode_to_domain)
            .collect(),
        search_inputs: capabilities
            .search_inputs
            .iter()
            .copied()
            .map(indexer_search_input_to_domain)
            .collect(),
        supported_external_ids: capabilities.supported_external_ids.clone(),
        category_model: capabilities
            .category_model
            .as_ref()
            .map(indexer_category_model_to_domain),
        limits: capabilities
            .limits
            .as_ref()
            .map(indexer_limit_capabilities_to_domain),
        torrent: capabilities
            .torrent
            .as_ref()
            .map(indexer_torrent_capabilities_to_domain),
        response_features: capabilities
            .response_features
            .as_ref()
            .map(indexer_response_features_to_domain),
    }
}

fn indexer_protocol_to_domain(
    protocol: IndexerProtocol,
) -> scryer_domain::IndexerProtocolCapability {
    match protocol {
        IndexerProtocol::Usenet => scryer_domain::IndexerProtocolCapability::Usenet,
        IndexerProtocol::Torrent => scryer_domain::IndexerProtocolCapability::Torrent,
        IndexerProtocol::Mixed => scryer_domain::IndexerProtocolCapability::Mixed,
        IndexerProtocol::Unknown => scryer_domain::IndexerProtocolCapability::Unknown,
    }
}

fn indexer_feed_mode_to_domain(mode: IndexerFeedMode) -> scryer_domain::IndexerFeedModeCapability {
    match mode {
        IndexerFeedMode::Recent => scryer_domain::IndexerFeedModeCapability::Recent,
        IndexerFeedMode::Rss => scryer_domain::IndexerFeedModeCapability::Rss,
        IndexerFeedMode::AutomaticSearch => {
            scryer_domain::IndexerFeedModeCapability::AutomaticSearch
        }
        IndexerFeedMode::InteractiveSearch => {
            scryer_domain::IndexerFeedModeCapability::InteractiveSearch
        }
    }
}

fn indexer_search_input_to_domain(
    input: IndexerSearchInput,
) -> scryer_domain::IndexerSearchInputCapability {
    match input {
        IndexerSearchInput::TextQuery => scryer_domain::IndexerSearchInputCapability::TextQuery,
        IndexerSearchInput::TitleQuery => scryer_domain::IndexerSearchInputCapability::TitleQuery,
        IndexerSearchInput::IdQuery => scryer_domain::IndexerSearchInputCapability::IdQuery,
        IndexerSearchInput::AggregateIdQuery => {
            scryer_domain::IndexerSearchInputCapability::AggregateIdQuery
        }
        IndexerSearchInput::Season => scryer_domain::IndexerSearchInputCapability::Season,
        IndexerSearchInput::Episode => scryer_domain::IndexerSearchInputCapability::Episode,
        IndexerSearchInput::AbsoluteEpisode => {
            scryer_domain::IndexerSearchInputCapability::AbsoluteEpisode
        }
        IndexerSearchInput::AirDate => scryer_domain::IndexerSearchInputCapability::AirDate,
        IndexerSearchInput::SpecialEpisodeTitle => {
            scryer_domain::IndexerSearchInputCapability::SpecialEpisodeTitle
        }
        IndexerSearchInput::Category => scryer_domain::IndexerSearchInputCapability::Category,
        IndexerSearchInput::Offset => scryer_domain::IndexerSearchInputCapability::Offset,
        IndexerSearchInput::Limit => scryer_domain::IndexerSearchInputCapability::Limit,
    }
}

fn indexer_category_model_to_domain(
    model: &IndexerCategoryModel,
) -> scryer_domain::IndexerCategoryModel {
    scryer_domain::IndexerCategoryModel {
        value_kinds: model
            .value_kinds
            .iter()
            .copied()
            .map(indexer_category_value_kind_to_domain)
            .collect(),
        separate_anime_categories: model.separate_anime_categories,
        provider_category_metadata: model.provider_category_metadata,
        categories: model
            .categories
            .iter()
            .map(indexer_category_descriptor_to_domain)
            .collect(),
    }
}

fn indexer_category_descriptor_to_domain(
    descriptor: &IndexerCategoryDescriptor,
) -> scryer_domain::IndexerCategoryDescriptor {
    scryer_domain::IndexerCategoryDescriptor {
        value: descriptor.value.clone(),
        label: descriptor.label.clone(),
        value_kind: indexer_category_value_kind_to_domain(descriptor.value_kind),
        facets: descriptor.facets.clone(),
    }
}

fn indexer_category_value_kind_to_domain(
    kind: IndexerCategoryValueKind,
) -> scryer_domain::IndexerCategoryValueKind {
    match kind {
        IndexerCategoryValueKind::Numeric => scryer_domain::IndexerCategoryValueKind::Numeric,
        IndexerCategoryValueKind::String => scryer_domain::IndexerCategoryValueKind::String,
    }
}

fn indexer_limit_capabilities_to_domain(
    limits: &IndexerLimitCapabilities,
) -> scryer_domain::IndexerLimitCapabilities {
    scryer_domain::IndexerLimitCapabilities {
        page_size: limits.page_size,
        max_page_size: limits.max_page_size,
        max_pages: limits.max_pages,
        rate_limit_hint_seconds: limits.rate_limit_hint_seconds,
        api_quota_supported: limits.api_quota_supported,
        grab_quota_supported: limits.grab_quota_supported,
    }
}

fn indexer_torrent_capabilities_to_domain(
    torrent: &IndexerTorrentCapabilities,
) -> scryer_domain::IndexerTorrentCapabilities {
    scryer_domain::IndexerTorrentCapabilities {
        reports_seeders: torrent.reports_seeders,
        reports_peers: torrent.reports_peers,
        reports_leechers: torrent.reports_leechers,
        reports_info_hash: torrent.reports_info_hash,
        reports_magnet_uri: torrent.reports_magnet_uri,
        reports_volume_factors: torrent.reports_volume_factors,
        supports_private_tracker_flags: torrent.supports_private_tracker_flags,
        supports_seed_requirements: torrent.supports_seed_requirements,
    }
}

fn indexer_response_features_to_domain(
    features: &IndexerResponseFeatures,
) -> scryer_domain::IndexerResponseFeatures {
    scryer_domain::IndexerResponseFeatures {
        languages: features.languages,
        subtitles: features.subtitles,
        grabs: features.grabs,
        votes: features.votes,
        comments: features.comments,
        info_url: features.info_url,
        guid: features.guid,
        raw_provider_metadata: features.raw_provider_metadata,
        password_hint: features.password_hint,
        protection_hint: features.protection_hint,
    }
}

pub(crate) fn tagged_alias_to_sdk(alias: scryer_domain::TaggedAlias) -> TaggedAlias {
    TaggedAlias {
        name: alias.name,
        language: alias.language,
    }
}

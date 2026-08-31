use std::collections::HashMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    ConfigFieldDef, IndexerCapabilities, IndexerDescriptor, IndexerSourceKind, PluginDescriptor,
    ProviderDescriptor,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IndexerProtocol {
    Usenet,
    Torrent,
    Mixed,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IndexerFeedMode {
    Recent,
    Rss,
    AutomaticSearch,
    InteractiveSearch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IndexerSearchInput {
    TextQuery,
    TitleQuery,
    IdQuery,
    AggregateIdQuery,
    Season,
    Episode,
    AbsoluteEpisode,
    AirDate,
    SpecialEpisodeTitle,
    Category,
    Offset,
    Limit,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IndexerCategoryValueKind {
    Numeric,
    #[default]
    String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct IndexerCategoryDescriptor {
    pub value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default)]
    pub value_kind: IndexerCategoryValueKind,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub facets: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct IndexerCategoryModel {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub value_kinds: Vec<IndexerCategoryValueKind>,
    #[serde(default)]
    pub separate_anime_categories: bool,
    #[serde(default)]
    pub provider_category_metadata: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<IndexerCategoryDescriptor>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct IndexerLimitCapabilities {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_size: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_page_size: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_pages: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_limit_hint_seconds: Option<u32>,
    #[serde(default)]
    pub api_quota_supported: bool,
    #[serde(default)]
    pub grab_quota_supported: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct IndexerTorrentCapabilities {
    #[serde(default)]
    pub reports_seeders: bool,
    #[serde(default)]
    pub reports_peers: bool,
    #[serde(default)]
    pub reports_leechers: bool,
    #[serde(default)]
    pub reports_info_hash: bool,
    #[serde(default)]
    pub reports_magnet_uri: bool,
    #[serde(default)]
    pub reports_volume_factors: bool,
    #[serde(default)]
    pub supports_private_tracker_flags: bool,
    #[serde(default)]
    pub supports_seed_requirements: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct IndexerResponseFeatures {
    #[serde(default)]
    pub languages: bool,
    #[serde(default)]
    pub subtitles: bool,
    #[serde(default)]
    pub grabs: bool,
    #[serde(default)]
    pub votes: bool,
    #[serde(default)]
    pub comments: bool,
    #[serde(default)]
    pub info_url: bool,
    #[serde(default)]
    pub guid: bool,
    #[serde(default)]
    pub raw_provider_metadata: bool,
    #[serde(default)]
    pub password_hint: bool,
    #[serde(default)]
    pub protection_hint: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PluginSearchRequestKind {
    Recent,
    #[default]
    Search,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PluginSearchOrigin {
    Rss,
    Automatic,
    #[default]
    Interactive,
    Manual,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PluginSearchSubjectKind {
    Title,
    Movie,
    Episode,
    Season,
    Collection,
    AnimeEpisode,
    Special,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PluginSearchQueryKind {
    Text,
    Title,
    Id,
    AggregateId,
    #[default]
    Fallback,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PluginSearchContext {
    #[serde(default)]
    pub request_kind: PluginSearchRequestKind,
    #[serde(default)]
    pub search_origin: PluginSearchOrigin,
    #[serde(default)]
    pub subject_kind: PluginSearchSubjectKind,
    #[serde(default)]
    pub query_kind: PluginSearchQueryKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub air_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub episode_title: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scene_titles: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clean_titles: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_index: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_count: Option<u32>,
}

pub fn normalize_external_id_key(key: &str) -> String {
    match key.trim().to_ascii_lowercase().as_str() {
        "imdb" | "imdb_id" | "imdbid" => "imdb_id".to_string(),
        "tmdb" | "tmdb_id" | "tmdbid" => "tmdb_id".to_string(),
        "tvdb" | "tvdb_id" | "tvdbid" => "tvdb_id".to_string(),
        "tvmaze" | "tvmaze_id" | "tvmazeid" => "tvmaze_id".to_string(),
        "tvrage" | "tvrage_id" | "rid" | "rageid" => "tvrage_id".to_string(),
        "anidb" | "anidb_id" | "anidbid" | "aid" => "anidb_id".to_string(),
        other => other.to_string(),
    }
}

pub fn normalize_external_ids<I, K, V>(pairs: I) -> HashMap<String, String>
where
    I: IntoIterator<Item = (K, V)>,
    K: AsRef<str>,
    V: Into<String>,
{
    let mut normalized = HashMap::new();
    for (key, value) in pairs {
        let value = value.into();
        if value.trim().is_empty() {
            continue;
        }
        normalized.insert(normalize_external_id_key(key.as_ref()), value);
    }
    normalized
}

pub fn normalize_info_hash(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            value
                .chars()
                .filter(|ch| ch.is_ascii_hexdigit())
                .collect::<String>()
                .to_ascii_lowercase()
        })
        .filter(|value| matches!(value.len(), 40 | 64))
}

pub fn derive_indexer_flags(
    download_volume_factor: Option<f64>,
    upload_volume_factor: Option<f64>,
    tags: &[String],
    protected: Option<bool>,
) -> Vec<String> {
    let mut flags = Vec::new();
    if download_volume_factor.is_some_and(|value| (value - 0.0).abs() < f64::EPSILON) {
        flags.push("freeleech".to_string());
    }
    if upload_volume_factor.is_some_and(|value| (value - 0.0).abs() < f64::EPSILON) {
        flags.push("neutral_upload".to_string());
    }
    if protected.unwrap_or(false) {
        flags.push("protected".to_string());
    }
    for tag in tags {
        let normalized = tag.trim().to_ascii_lowercase();
        if !normalized.is_empty() && !flags.iter().any(|existing| existing == &normalized) {
            flags.push(normalized);
        }
    }
    flags
}

pub fn usenet_result(
    title: impl Into<String>,
    download_url: Option<String>,
) -> crate::PluginSearchResult {
    crate::PluginSearchResult {
        title: title.into(),
        download_url,
        source_kind: Some(IndexerSourceKind::Usenet),
        protocol: Some(IndexerProtocol::Usenet),
        ..crate::PluginSearchResult::default()
    }
}

pub fn torrent_result(
    title: impl Into<String>,
    download_url: Option<String>,
) -> crate::PluginSearchResult {
    crate::PluginSearchResult {
        title: title.into(),
        download_url,
        source_kind: Some(IndexerSourceKind::Torrent),
        protocol: Some(IndexerProtocol::Torrent),
        ..crate::PluginSearchResult::default()
    }
}

pub fn indexer_capability_fixtures() -> Vec<PluginDescriptor> {
    vec![
        fixture_descriptor(
            "newznab",
            IndexerSourceKind::Usenet,
            IndexerCapabilities {
                rss: true,
                supported_ids: HashMap::from([
                    ("movie".into(), vec!["imdb_id".into(), "tmdb_id".into()]),
                    (
                        "series".into(),
                        vec!["tvdb_id".into(), "tvmaze_id".into(), "tvrage_id".into()],
                    ),
                    ("anime".into(), vec!["tvdb_id".into(), "anidb_id".into()]),
                ]),
                season_param: Some("season".into()),
                episode_param: Some("ep".into()),
                query_param: Some("q".into()),
                search: true,
                imdb_search: true,
                tvdb_search: true,
                anidb_search: true,
                protocols: vec![IndexerProtocol::Usenet],
                feed_modes: vec![
                    IndexerFeedMode::Rss,
                    IndexerFeedMode::AutomaticSearch,
                    IndexerFeedMode::InteractiveSearch,
                ],
                search_inputs: vec![
                    IndexerSearchInput::TitleQuery,
                    IndexerSearchInput::IdQuery,
                    IndexerSearchInput::Category,
                    IndexerSearchInput::Season,
                    IndexerSearchInput::Episode,
                    IndexerSearchInput::Limit,
                ],
                supported_external_ids: vec![
                    "imdb_id".into(),
                    "tmdb_id".into(),
                    "tvdb_id".into(),
                    "tvmaze_id".into(),
                    "tvrage_id".into(),
                    "anidb_id".into(),
                ],
                category_model: Some(IndexerCategoryModel {
                    value_kinds: vec![IndexerCategoryValueKind::Numeric],
                    separate_anime_categories: true,
                    provider_category_metadata: true,
                    ..IndexerCategoryModel::default()
                }),
                limits: Some(IndexerLimitCapabilities {
                    page_size: Some(100),
                    max_page_size: Some(100),
                    max_pages: Some(10),
                    api_quota_supported: true,
                    grab_quota_supported: true,
                    ..IndexerLimitCapabilities::default()
                }),
                response_features: Some(IndexerResponseFeatures {
                    languages: true,
                    grabs: true,
                    comments: true,
                    info_url: true,
                    guid: true,
                    raw_provider_metadata: true,
                    password_hint: true,
                    protection_hint: true,
                    ..IndexerResponseFeatures::default()
                }),
                ..IndexerCapabilities::default()
            },
        ),
        fixture_descriptor(
            "torznab",
            IndexerSourceKind::Torrent,
            IndexerCapabilities {
                rss: true,
                supported_ids: HashMap::from([
                    ("movie".into(), vec!["imdb_id".into(), "tmdb_id".into()]),
                    (
                        "series".into(),
                        vec!["tvdb_id".into(), "tvmaze_id".into(), "tvrage_id".into()],
                    ),
                    ("anime".into(), vec!["tvdb_id".into(), "anidb_id".into()]),
                ]),
                season_param: Some("season".into()),
                episode_param: Some("ep".into()),
                query_param: Some("q".into()),
                search: true,
                imdb_search: true,
                tvdb_search: true,
                anidb_search: true,
                protocols: vec![IndexerProtocol::Torrent],
                feed_modes: vec![
                    IndexerFeedMode::Rss,
                    IndexerFeedMode::AutomaticSearch,
                    IndexerFeedMode::InteractiveSearch,
                ],
                search_inputs: vec![
                    IndexerSearchInput::TitleQuery,
                    IndexerSearchInput::IdQuery,
                    IndexerSearchInput::AggregateIdQuery,
                    IndexerSearchInput::Category,
                    IndexerSearchInput::Season,
                    IndexerSearchInput::Episode,
                    IndexerSearchInput::AbsoluteEpisode,
                    IndexerSearchInput::Limit,
                ],
                supported_external_ids: vec![
                    "imdb_id".into(),
                    "tmdb_id".into(),
                    "tvdb_id".into(),
                    "tvmaze_id".into(),
                    "tvrage_id".into(),
                    "anidb_id".into(),
                ],
                category_model: Some(IndexerCategoryModel {
                    value_kinds: vec![IndexerCategoryValueKind::Numeric],
                    separate_anime_categories: true,
                    provider_category_metadata: true,
                    ..IndexerCategoryModel::default()
                }),
                limits: Some(IndexerLimitCapabilities {
                    page_size: Some(100),
                    max_page_size: Some(100),
                    max_pages: Some(10),
                    rate_limit_hint_seconds: Some(2),
                    api_quota_supported: true,
                    grab_quota_supported: true,
                }),
                torrent: Some(IndexerTorrentCapabilities {
                    reports_seeders: true,
                    reports_peers: true,
                    reports_leechers: true,
                    reports_info_hash: true,
                    reports_magnet_uri: true,
                    reports_volume_factors: true,
                    supports_private_tracker_flags: true,
                    supports_seed_requirements: true,
                }),
                response_features: Some(IndexerResponseFeatures {
                    languages: true,
                    subtitles: true,
                    grabs: true,
                    votes: true,
                    comments: true,
                    info_url: true,
                    guid: true,
                    raw_provider_metadata: true,
                    protection_hint: true,
                    ..IndexerResponseFeatures::default()
                }),
                ..IndexerCapabilities::default()
            },
        ),
        fixture_descriptor(
            "torrent_rss",
            IndexerSourceKind::Torrent,
            IndexerCapabilities {
                rss: true,
                protocols: vec![IndexerProtocol::Torrent],
                feed_modes: vec![IndexerFeedMode::Recent, IndexerFeedMode::Rss],
                search_inputs: vec![
                    IndexerSearchInput::TextQuery,
                    IndexerSearchInput::Category,
                    IndexerSearchInput::Season,
                    IndexerSearchInput::Episode,
                    IndexerSearchInput::Limit,
                ],
                category_model: Some(IndexerCategoryModel {
                    value_kinds: vec![IndexerCategoryValueKind::String],
                    provider_category_metadata: true,
                    ..IndexerCategoryModel::default()
                }),
                limits: Some(IndexerLimitCapabilities {
                    page_size: Some(200),
                    max_page_size: Some(200),
                    rate_limit_hint_seconds: Some(2),
                    ..IndexerLimitCapabilities::default()
                }),
                torrent: Some(IndexerTorrentCapabilities {
                    reports_seeders: true,
                    reports_peers: true,
                    reports_leechers: true,
                    reports_info_hash: true,
                    reports_magnet_uri: true,
                    reports_volume_factors: true,
                    supports_private_tracker_flags: true,
                    supports_seed_requirements: true,
                }),
                response_features: Some(IndexerResponseFeatures {
                    languages: true,
                    grabs: true,
                    info_url: true,
                    guid: true,
                    raw_provider_metadata: true,
                    ..IndexerResponseFeatures::default()
                }),
                ..IndexerCapabilities::default()
            },
        ),
        fixture_descriptor(
            "id_only_anime_indexer",
            IndexerSourceKind::Generic,
            IndexerCapabilities {
                rss: true,
                supported_ids: HashMap::from([
                    ("anime".into(), vec!["anidb_id".into()]),
                    ("movie".into(), vec!["anidb_id".into()]),
                ]),
                deduplicates_aliases: true,
                query_param: Some("q".into()),
                search: true,
                anidb_search: true,
                protocols: vec![IndexerProtocol::Mixed],
                feed_modes: vec![
                    IndexerFeedMode::Rss,
                    IndexerFeedMode::AutomaticSearch,
                    IndexerFeedMode::InteractiveSearch,
                ],
                search_inputs: vec![
                    IndexerSearchInput::TitleQuery,
                    IndexerSearchInput::IdQuery,
                    IndexerSearchInput::AbsoluteEpisode,
                    IndexerSearchInput::Episode,
                    IndexerSearchInput::Limit,
                ],
                supported_external_ids: vec!["anidb_id".into()],
                limits: Some(IndexerLimitCapabilities {
                    page_size: Some(75),
                    max_page_size: Some(75),
                    max_pages: Some(14),
                    ..IndexerLimitCapabilities::default()
                }),
                torrent: Some(IndexerTorrentCapabilities {
                    reports_seeders: true,
                    reports_leechers: true,
                    reports_info_hash: true,
                    reports_magnet_uri: true,
                    ..IndexerTorrentCapabilities::default()
                }),
                response_features: Some(IndexerResponseFeatures {
                    info_url: true,
                    raw_provider_metadata: true,
                    ..IndexerResponseFeatures::default()
                }),
                ..IndexerCapabilities::default()
            },
        ),
        fixture_descriptor(
            "private_tracker",
            IndexerSourceKind::Torrent,
            IndexerCapabilities {
                rss: true,
                supported_ids: HashMap::from([
                    ("movie".into(), vec!["imdb_id".into()]),
                    ("series".into(), vec!["tvdb_id".into()]),
                ]),
                query_param: Some("q".into()),
                search: true,
                imdb_search: true,
                tvdb_search: true,
                protocols: vec![IndexerProtocol::Torrent],
                feed_modes: vec![
                    IndexerFeedMode::Rss,
                    IndexerFeedMode::AutomaticSearch,
                    IndexerFeedMode::InteractiveSearch,
                ],
                search_inputs: vec![
                    IndexerSearchInput::TitleQuery,
                    IndexerSearchInput::IdQuery,
                    IndexerSearchInput::Category,
                    IndexerSearchInput::Limit,
                ],
                supported_external_ids: vec!["imdb_id".into(), "tvdb_id".into()],
                category_model: Some(IndexerCategoryModel {
                    value_kinds: vec![IndexerCategoryValueKind::Numeric],
                    provider_category_metadata: true,
                    ..IndexerCategoryModel::default()
                }),
                torrent: Some(IndexerTorrentCapabilities {
                    reports_seeders: true,
                    reports_peers: true,
                    reports_leechers: true,
                    reports_info_hash: true,
                    reports_magnet_uri: true,
                    reports_volume_factors: true,
                    supports_private_tracker_flags: true,
                    supports_seed_requirements: true,
                }),
                response_features: Some(IndexerResponseFeatures {
                    languages: true,
                    subtitles: true,
                    grabs: true,
                    comments: true,
                    info_url: true,
                    guid: true,
                    raw_provider_metadata: true,
                    protection_hint: true,
                    ..IndexerResponseFeatures::default()
                }),
                ..IndexerCapabilities::default()
            },
        ),
        fixture_descriptor(
            "api_backed_tracker",
            IndexerSourceKind::Torrent,
            IndexerCapabilities {
                rss: false,
                supported_ids: HashMap::from([
                    ("movie".into(), vec!["imdb_id".into(), "tmdb_id".into()]),
                    ("series".into(), vec!["tvdb_id".into(), "tvmaze_id".into()]),
                ]),
                query_param: Some("q".into()),
                search: true,
                imdb_search: true,
                tvdb_search: true,
                protocols: vec![IndexerProtocol::Torrent],
                feed_modes: vec![
                    IndexerFeedMode::AutomaticSearch,
                    IndexerFeedMode::InteractiveSearch,
                ],
                search_inputs: vec![
                    IndexerSearchInput::TitleQuery,
                    IndexerSearchInput::IdQuery,
                    IndexerSearchInput::AggregateIdQuery,
                    IndexerSearchInput::Category,
                    IndexerSearchInput::Offset,
                    IndexerSearchInput::Limit,
                ],
                supported_external_ids: vec![
                    "imdb_id".into(),
                    "tmdb_id".into(),
                    "tvdb_id".into(),
                    "tvmaze_id".into(),
                    "provider_release_id".into(),
                ],
                limits: Some(IndexerLimitCapabilities {
                    page_size: Some(50),
                    max_page_size: Some(100),
                    max_pages: Some(20),
                    rate_limit_hint_seconds: Some(1),
                    api_quota_supported: true,
                    ..IndexerLimitCapabilities::default()
                }),
                torrent: Some(IndexerTorrentCapabilities {
                    reports_seeders: true,
                    reports_peers: true,
                    reports_info_hash: true,
                    reports_magnet_uri: true,
                    reports_volume_factors: true,
                    ..IndexerTorrentCapabilities::default()
                }),
                response_features: Some(IndexerResponseFeatures {
                    votes: true,
                    comments: true,
                    info_url: true,
                    guid: true,
                    raw_provider_metadata: true,
                    ..IndexerResponseFeatures::default()
                }),
                ..IndexerCapabilities::default()
            },
        ),
    ]
}

fn fixture_descriptor(
    provider_type: &str,
    source_kind: IndexerSourceKind,
    capabilities: IndexerCapabilities,
) -> PluginDescriptor {
    PluginDescriptor {
        id: format!("{provider_type}_fixture"),
        name: format!("{provider_type} fixture"),
        version: "0.0.0".to_string(),
        sdk_version: crate::SDK_VERSION.to_string(),
        sdk_constraint: crate::current_sdk_constraint(),
        socket_permissions: vec![],
        provider: ProviderDescriptor::Indexer(IndexerDescriptor {
            provider_type: provider_type.to_string(),
            provider_aliases: vec![],
            provider_profiles: vec![],
            search_semantics_version: Some(1),
            strategy_plan: None,
            source_kind,
            capabilities,
            scoring_policies: Vec::<crate::PluginScoringPolicy>::new(),
            config_fields: Vec::<ConfigFieldDef>::new(),
            allowed_hosts: Vec::new(),
            rate_limit_seconds: None,
        }),
    }
}

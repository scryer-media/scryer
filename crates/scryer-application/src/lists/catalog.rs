//! The provider catalog: what can be followed, and how a source is named.
//!
//! Installed list-provider plugins describe themselves through their manifest.
//! The metadata gateway adds two things no plugin serves: the public charts it
//! ingests, and public IMDb lists it proxies. Both are merged into the
//! manifest of the provider they belong to, so the Lists page shows one tile
//! per provider whatever serves it.
//!
//! A gateway chart is named by its source type alone
//! (`smg_chart:{chart_key}:{scope}`), so a follow needs no parameters; a public
//! IMDb list is the `imdb` provider's `user_list` source with a `list_id`.

use std::collections::{BTreeMap, HashMap};

use regex::Regex;
use scryer_domain::{LIST_SOURCE_IMDB_LIST_ID_PARAM, ListSource, ListSourceOrigin, MediaFacet};
pub use scryer_plugin_sdk::{
    ListAuthBadge, ListMediaKind, ListNoteTone, ListProviderGroup, ListProviderItem,
    ListProviderNote, ListProviderTile, ListSourceParam, ListSourceParamType, ListUrlPattern,
    ListUrlPatternCapture,
};
use scryer_plugin_sdk::{ListProviderDescriptor, PluginDescriptor, ProviderDescriptor};

use super::gateway::ListChartCatalogEntry;
use super::provider_settings::{ListProviderSettingField, declared_server_fields};
use crate::{AppError, AppResult};

/// Source types of gateway charts start with this, then `{chart_key}:{scope}`.
pub const LIST_SOURCE_TYPE_SMG_CHART_PREFIX: &str = "smg_chart:";
/// The provider and source type of a proxied public IMDb list.
pub const LIST_PROVIDER_IMDB: &str = "imdb";
pub const LIST_SOURCE_TYPE_IMDB_USER_LIST: &str = "user_list";
/// How often a gateway chart is re-read. Charts are ingested daily.
pub const LIST_SMG_CHART_INTERVAL_SECONDS: u64 = 12 * 60 * 60;
/// How often a proxied IMDb list is re-read.
pub const LIST_IMDB_LIST_INTERVAL_SECONDS: u64 = 6 * 60 * 60;

const IMDB_LIST_URL_PATTERN: &str =
    r"^https?://(?:www\.|m\.)?imdb\.com/list/(?P<list_id>ls\d{6,12})/?(?:[?#].*)?$";

/// One provider as the Lists page shows it.
#[derive(Clone, Debug)]
pub struct ListProviderManifest {
    pub provider_type: String,
    pub name: String,
    pub summary: Option<String>,
    pub blurb: Option<String>,
    pub tile: Option<ListProviderTile>,
    pub coverage: Vec<MediaFacet>,
    pub groups: Vec<ListProviderGroup>,
    pub notes: Vec<ListProviderNote>,
    pub url_patterns: Vec<ListUrlPattern>,
    /// Server-wide settings the provider declares. `is_set` is filled only
    /// for callers who manage lists; values are never carried here.
    pub config_fields: Vec<ListProviderSettingField>,
}

pub fn facet_of(kind: ListMediaKind) -> MediaFacet {
    match kind {
        ListMediaKind::Movie => MediaFacet::Movie,
        ListMediaKind::Series => MediaFacet::Series,
        ListMediaKind::Anime => MediaFacet::Anime,
    }
}

pub fn list_kind_of(kind: &MediaFacet) -> ListMediaKind {
    match kind {
        MediaFacet::Movie => ListMediaKind::Movie,
        MediaFacet::Series => ListMediaKind::Series,
        MediaFacet::Anime => ListMediaKind::Anime,
    }
}

fn gateway_kinds(kinds: &[String]) -> Vec<ListMediaKind> {
    let mut out = Vec::new();
    for kind in kinds {
        let kind = match kind.trim().to_ascii_lowercase().as_str() {
            "movie" => ListMediaKind::Movie,
            "series" | "tv" | "show" => ListMediaKind::Series,
            "anime" => ListMediaKind::Anime,
            _ => continue,
        };
        if !out.contains(&kind) {
            out.push(kind);
        }
    }
    out
}

/// A plain display name for a provider no plugin names.
fn provider_display_name(provider_type: &str) -> String {
    match provider_type {
        "tmdb" => "TMDb".to_string(),
        "imdb" => "IMDb".to_string(),
        "trakt" => "Trakt".to_string(),
        "anilist" => "AniList".to_string(),
        "mal" => "MyAnimeList".to_string(),
        "simkl" => "Simkl".to_string(),
        "tvdb" => "TheTVDB".to_string(),
        "mdblist" => "MDBList".to_string(),
        other => {
            let mut chars = other.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect(),
                None => String::new(),
            }
        }
    }
}

pub fn smg_chart_source_type(chart_key: &str, scope: &str) -> String {
    format!("{LIST_SOURCE_TYPE_SMG_CHART_PREFIX}{chart_key}:{scope}")
}

/// The chart key and scope a gateway chart source type names.
pub fn parse_smg_chart_source_type(source_type: &str) -> Option<(String, String)> {
    let rest = source_type.strip_prefix(LIST_SOURCE_TYPE_SMG_CHART_PREFIX)?;
    let (chart_key, scope) = rest.rsplit_once(':')?;
    let (chart_key, scope) = (chart_key.trim(), scope.trim());
    (!chart_key.is_empty() && !scope.is_empty()).then(|| (chart_key.to_string(), scope.to_string()))
}

fn list_descriptors(
    plugins: &[PluginDescriptor],
) -> Vec<(&PluginDescriptor, ListProviderDescriptor)> {
    plugins
        .iter()
        .filter_map(|plugin| match &plugin.provider {
            ProviderDescriptor::ListProvider(list) => Some((plugin, list.clone())),
            _ => None,
        })
        .collect()
}

fn manifest_from_descriptor(
    plugin: &PluginDescriptor,
    list: ListProviderDescriptor,
) -> ListProviderManifest {
    ListProviderManifest {
        config_fields: declared_server_fields(plugin),
        name: plugin.name.clone(),
        provider_type: list.provider_type.to_ascii_lowercase(),
        summary: list.summary,
        blurb: list.blurb,
        tile: list.tile,
        coverage: list.coverage.into_iter().map(facet_of).collect(),
        groups: list.groups,
        notes: list.notes,
        url_patterns: list.url_patterns,
    }
}

fn bare_manifest(provider_type: &str) -> ListProviderManifest {
    ListProviderManifest {
        provider_type: provider_type.to_string(),
        name: provider_display_name(provider_type),
        summary: None,
        blurb: None,
        tile: None,
        coverage: Vec::new(),
        groups: Vec::new(),
        notes: Vec::new(),
        url_patterns: Vec::new(),
        config_fields: Vec::new(),
    }
}

fn imdb_list_group() -> ListProviderGroup {
    ListProviderGroup {
        label: "Public lists".to_string(),
        auth_badge: ListAuthBadge::NoAccountNeedsValue,
        items: vec![ListProviderItem {
            id: "imdb:user_list".to_string(),
            name: "Public IMDb list".to_string(),
            description: Some("Any public IMDb list, by its link or its ls id.".to_string()),
            kinds: vec![ListMediaKind::Movie, ListMediaKind::Series],
            source_type: LIST_SOURCE_TYPE_IMDB_USER_LIST.to_string(),
            params: vec![ListSourceParam {
                key: LIST_SOURCE_IMDB_LIST_ID_PARAM.to_string(),
                label: "List ID".to_string(),
                param_type: ListSourceParamType::Text,
                options: Vec::new(),
                required: true,
            }],
            personal: false,
            default_interval_seconds: LIST_IMDB_LIST_INTERVAL_SECONDS,
        }],
    }
}

fn imdb_list_url_pattern() -> ListUrlPattern {
    ListUrlPattern {
        pattern: IMDB_LIST_URL_PATTERN.to_string(),
        source_type: LIST_SOURCE_TYPE_IMDB_USER_LIST.to_string(),
        captures: vec![ListUrlPatternCapture {
            group: LIST_SOURCE_IMDB_LIST_ID_PARAM.to_string(),
            param: LIST_SOURCE_IMDB_LIST_ID_PARAM.to_string(),
        }],
    }
}

fn add_coverage(manifest: &mut ListProviderManifest, kinds: &[ListMediaKind]) {
    for kind in kinds {
        let facet = facet_of(*kind);
        if !manifest.coverage.contains(&facet) {
            manifest.coverage.push(facet);
        }
    }
}

/// Merge installed plugins and the gateway's charts into one manifest per
/// provider, sorted by provider type.
pub fn merge_provider_catalog(
    plugins: &[PluginDescriptor],
    charts: &[ListChartCatalogEntry],
) -> Vec<ListProviderManifest> {
    let mut manifests: BTreeMap<String, ListProviderManifest> = BTreeMap::new();
    for (plugin, descriptor) in list_descriptors(plugins) {
        let manifest = manifest_from_descriptor(plugin, descriptor);
        manifests
            .entry(manifest.provider_type.clone())
            .or_insert(manifest);
    }

    let mut chart_groups: BTreeMap<String, Vec<ListProviderItem>> = BTreeMap::new();
    for chart in charts {
        let provider = chart.provider.trim().to_ascii_lowercase();
        if provider.is_empty() {
            continue;
        }
        chart_groups
            .entry(provider)
            .or_default()
            .push(ListProviderItem {
                id: format!("smg:{}:{}", chart.chart_key, chart.scope),
                name: chart.label.clone(),
                description: None,
                kinds: gateway_kinds(&chart.kinds),
                source_type: smg_chart_source_type(&chart.chart_key, &chart.scope),
                params: Vec::new(),
                personal: false,
                default_interval_seconds: LIST_SMG_CHART_INTERVAL_SECONDS,
            });
    }
    for (provider, items) in chart_groups {
        let manifest = manifests
            .entry(provider.clone())
            .or_insert_with(|| bare_manifest(&provider));
        for item in &items {
            add_coverage(manifest, &item.kinds);
        }
        manifest.groups.insert(
            0,
            ListProviderGroup {
                label: "Charts".to_string(),
                auth_badge: ListAuthBadge::NoAccount,
                items,
            },
        );
    }

    let imdb = manifests
        .entry(LIST_PROVIDER_IMDB.to_string())
        .or_insert_with(|| bare_manifest(LIST_PROVIDER_IMDB));
    add_coverage(imdb, &[ListMediaKind::Movie, ListMediaKind::Series]);
    if !imdb
        .groups
        .iter()
        .flat_map(|group| &group.items)
        .any(|item| item.source_type == LIST_SOURCE_TYPE_IMDB_USER_LIST)
    {
        imdb.groups.push(imdb_list_group());
    }
    if !imdb
        .url_patterns
        .iter()
        .any(|pattern| pattern.source_type == LIST_SOURCE_TYPE_IMDB_USER_LIST)
    {
        imdb.url_patterns.push(imdb_list_url_pattern());
    }

    manifests.into_values().collect()
}

/// The manifest item a source type names, if any.
pub fn find_item<'a>(
    manifest: &'a ListProviderManifest,
    source_type: &str,
) -> Option<&'a ListProviderItem> {
    manifest
        .groups
        .iter()
        .flat_map(|group| &group.items)
        .find(|item| item.source_type == source_type)
}

/// A URL the catalog recognises: which provider, which source, which params.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecognizedListUrl {
    pub provider: String,
    pub source_type: String,
    pub params: BTreeMap<String, String>,
}

/// Match `url` against every provider's URL patterns, in catalog order. A
/// pattern that does not compile is skipped: a bad plugin manifest must not
/// break recognition for everyone else.
pub fn recognize_url(manifests: &[ListProviderManifest], url: &str) -> Option<RecognizedListUrl> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }
    for manifest in manifests {
        for pattern in &manifest.url_patterns {
            let Ok(regex) = Regex::new(&pattern.pattern) else {
                continue;
            };
            let Some(captures) = regex.captures(url) else {
                continue;
            };
            let mut params = BTreeMap::new();
            for capture in &pattern.captures {
                if let Some(value) = captures.name(&capture.group) {
                    params.insert(capture.param.clone(), value.as_str().to_string());
                }
            }
            return Some(RecognizedListUrl {
                provider: manifest.provider_type.clone(),
                source_type: pattern.source_type.clone(),
                params,
            });
        }
    }
    None
}

/// A source a follow or a preview names, checked against the catalog.
#[derive(Clone, Debug)]
pub struct ClassifiedSource {
    pub source: ListSource,
    pub name: String,
    pub kinds: Vec<MediaFacet>,
    pub interval_seconds: u64,
}

fn check_required_params(
    item: &ListProviderItem,
    params: &BTreeMap<String, String>,
) -> AppResult<()> {
    for param in &item.params {
        let value = params.get(&param.key).map(|value| value.trim());
        if param.required && value.is_none_or(str::is_empty) {
            return Err(AppError::Validation(format!("{} is required", param.label)));
        }
        if param.param_type == ListSourceParamType::Enum
            && let Some(value) = value.filter(|value| !value.is_empty())
            && !param.options.is_empty()
            && !param.options.iter().any(|option| option == value)
        {
            return Err(AppError::Validation(format!(
                "{} must be one of the listed options",
                param.label
            )));
        }
    }
    Ok(())
}

/// Name the origin and the manifest facts of a public source.
///
/// A gateway chart must be in the gateway's catalog; an IMDb list needs an
/// `ls` id; anything else must be a non-personal item of an installed
/// provider whose fetch does not need a member's account.
pub fn classify_public_source(
    manifests: &[ListProviderManifest],
    charts: &[ListChartCatalogEntry],
    member_only_providers: &[String],
    provider: &str,
    source_type: &str,
    params: &BTreeMap<String, String>,
) -> AppResult<ClassifiedSource> {
    let provider = provider.trim().to_ascii_lowercase();
    let source_type = source_type.trim();
    let params = params
        .iter()
        .map(|(key, value)| (key.trim().to_string(), value.trim().to_string()))
        .filter(|(key, value)| !key.is_empty() && !value.is_empty())
        .collect::<BTreeMap<_, _>>();

    if let Some((chart_key, scope)) = parse_smg_chart_source_type(source_type) {
        let chart = charts
            .iter()
            .find(|chart| {
                chart.provider.eq_ignore_ascii_case(&provider)
                    && chart.chart_key == chart_key
                    && chart.scope == scope
            })
            .ok_or_else(|| AppError::Validation("that chart is not available".to_string()))?;
        return Ok(ClassifiedSource {
            source: ListSource {
                provider,
                source_type: source_type.to_string(),
                params: BTreeMap::new(),
                origin: ListSourceOrigin::SmgChart { chart_key, scope },
            },
            name: chart.label.clone(),
            kinds: gateway_kinds(&chart.kinds)
                .into_iter()
                .map(facet_of)
                .collect(),
            interval_seconds: LIST_SMG_CHART_INTERVAL_SECONDS,
        });
    }

    if provider == LIST_PROVIDER_IMDB && source_type == LIST_SOURCE_TYPE_IMDB_USER_LIST {
        let list_id = params
            .get(LIST_SOURCE_IMDB_LIST_ID_PARAM)
            .cloned()
            .unwrap_or_default();
        if !is_imdb_list_id(&list_id) {
            return Err(AppError::Validation(
                "an IMDb list id looks like ls followed by digits".to_string(),
            ));
        }
        return Ok(ClassifiedSource {
            source: ListSource {
                provider,
                source_type: source_type.to_string(),
                params: BTreeMap::from([(LIST_SOURCE_IMDB_LIST_ID_PARAM.to_string(), list_id)]),
                origin: ListSourceOrigin::SmgImdbList,
            },
            name: "IMDb list".to_string(),
            kinds: vec![MediaFacet::Movie, MediaFacet::Series],
            interval_seconds: LIST_IMDB_LIST_INTERVAL_SECONDS,
        });
    }

    let manifest = manifests
        .iter()
        .find(|manifest| manifest.provider_type == provider)
        .ok_or_else(|| AppError::Validation("that list provider is not installed".to_string()))?;
    let item = find_item(manifest, source_type)
        .ok_or_else(|| AppError::Validation("that provider has no such list".to_string()))?;
    if item.personal || member_only_providers.iter().any(|entry| entry == &provider) {
        return Err(AppError::Validation(
            "that list needs a member's own account and cannot be followed for everyone"
                .to_string(),
        ));
    }
    check_required_params(item, &params)?;
    Ok(ClassifiedSource {
        source: ListSource {
            provider,
            source_type: source_type.to_string(),
            params,
            origin: ListSourceOrigin::ProviderFetch,
        },
        name: item.name.clone(),
        kinds: item.kinds.iter().copied().map(facet_of).collect(),
        interval_seconds: item.default_interval_seconds,
    })
}

pub fn is_imdb_list_id(value: &str) -> bool {
    let value = value.trim();
    value.len() > 2
        && value.starts_with("ls")
        && value[2..]
            .chars()
            .all(|character| character.is_ascii_digit())
}

/// Providers whose plugin refuses a fetch without a member's credential.
pub fn member_only_providers(plugins: &[PluginDescriptor]) -> Vec<String> {
    list_descriptors(plugins)
        .into_iter()
        .filter(|(_, descriptor)| descriptor.capabilities.requires_member_credential)
        .map(|(_, descriptor)| descriptor.provider_type.to_ascii_lowercase())
        .collect()
}

/// Chart labels keyed by `(provider, chart_key, scope)`, for naming.
pub fn chart_labels(charts: &[ListChartCatalogEntry]) -> HashMap<(String, String, String), String> {
    charts
        .iter()
        .map(|chart| {
            (
                (
                    chart.provider.to_ascii_lowercase(),
                    chart.chart_key.clone(),
                    chart.scope.clone(),
                ),
                chart.label.clone(),
            )
        })
        .collect()
}

#[cfg(test)]
#[path = "catalog_tests.rs"]
mod tests;

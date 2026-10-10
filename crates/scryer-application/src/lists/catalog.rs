//! The provider catalog: what can be followed, and how a source is named.
//!
//! Installed list-provider plugins describe themselves through their manifest.
//! The metadata gateway adds the public charts it ingests, which no plugin
//! serves. They are merged into the manifest of the provider they belong to,
//! so the Lists page shows one tile per provider whatever serves it.
//!
//! A gateway chart is named by its source type alone
//! (`smg_chart:{chart_key}:{scope}`), so a follow needs no parameters.

use std::collections::{BTreeMap, HashMap};

use regex::Regex;
use scryer_domain::{ListSource, ListSourceOrigin, MediaFacet};
pub use scryer_plugin_sdk::{
    ListAuthBadge, ListMediaKind, ListNoteTone, ListProviderGroup, ListProviderItem,
    ListProviderNote, ListProviderTile, ListSourceParam, ListSourceParamType, ListUrlPattern,
    ListUrlPatternCapture,
};
use scryer_plugin_sdk::{ListProviderDescriptor, PluginDescriptor, ProviderDescriptor};

use super::gateway::ListChartCatalogEntry;
use super::provider_settings::{ListProviderSettingField, declared_server_fields};
use super::refusal::{self, refused};
use crate::AppResult;

/// Source types of gateway charts start with this, then `{chart_key}:{scope}`.
pub const LIST_SOURCE_TYPE_SMG_CHART_PREFIX: &str = "smg_chart:";
/// How often a gateway chart is re-read. Charts are ingested daily.
pub const LIST_SMG_CHART_INTERVAL_SECONDS: u64 = 12 * 60 * 60;

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

/// Media enums are driven by Include, not a second independent source control.
pub(crate) const INCLUDE_MEDIA_PARAM: &str = "__include__";

pub(crate) fn is_media_param(param: &ListSourceParam) -> bool {
    param.param_type == ListSourceParamType::Enum
        && matches!(param.key.as_str(), "type" | "kind")
        && !param.options.is_empty()
        && param.options.iter().all(|option| {
            matches!(
                option.as_str(),
                "all" | "movie" | "movies" | "series" | "shows" | "anime"
            )
        })
}

pub(crate) fn media_param<'a>(
    descriptor: &'a PluginDescriptor,
    source_type: &str,
) -> Option<&'a ListSourceParam> {
    descriptor
        .list_provider()?
        .groups
        .iter()
        .flat_map(|group| &group.items)
        .find(|item| item.source_type == source_type)?
        .params
        .iter()
        .find(|param| is_media_param(param))
}

pub(crate) fn use_include_media_selection(
    source: &mut ListSource,
    descriptors: &[PluginDescriptor],
) {
    if let Some(param) = descriptors
        .iter()
        .find(|descriptor| {
            descriptor
                .list_provider()
                .is_some_and(|list| list.provider_type == source.provider)
        })
        .and_then(|descriptor| media_param(descriptor, &source.source_type))
    {
        source
            .params
            .insert(param.key.clone(), INCLUDE_MEDIA_PARAM.into());
    }
}

fn check_required_params(
    item: &ListProviderItem,
    params: &BTreeMap<String, String>,
) -> AppResult<()> {
    for param in &item.params {
        let value = params.get(&param.key).map(|value| value.trim());
        if is_media_param(param) && value == Some(INCLUDE_MEDIA_PARAM) {
            continue;
        }
        if param.required && value.is_none_or(str::is_empty) {
            return Err(refused(
                refusal::PARAM_REQUIRED,
                format!("{} is required", param.label),
            ));
        }
        if param.param_type == ListSourceParamType::Enum
            && let Some(value) = value.filter(|value| !value.is_empty())
            && !param.options.is_empty()
            && !param.options.iter().any(|option| option == value)
        {
            return Err(refused(
                refusal::PARAM_INVALID,
                format!("{} must be one of the listed options", param.label),
            ));
        }
    }
    Ok(())
}

/// Name the origin and the manifest facts of a public source.
///
/// A gateway chart must be in the gateway's catalog; anything else must be a
/// non-personal item of an installed provider whose fetch does not need a
/// member's account.
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
            .ok_or_else(|| refused(refusal::CHART_UNAVAILABLE, "that chart is not available"))?;
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

    let manifest = manifests
        .iter()
        .find(|manifest| manifest.provider_type == provider)
        .ok_or_else(|| {
            refused(
                refusal::PROVIDER_NOT_INSTALLED,
                "that list provider is not installed",
            )
        })?;
    let item = find_item(manifest, source_type).ok_or_else(|| {
        refused(
            refusal::SOURCE_NOT_OFFERED,
            "that provider has no such list",
        )
    })?;
    if item.personal || member_only_providers.iter().any(|entry| entry == &provider) {
        return Err(refused(
            refusal::MEMBER_ACCOUNT_ONLY,
            "that list needs a member's own account and cannot be followed for everyone",
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

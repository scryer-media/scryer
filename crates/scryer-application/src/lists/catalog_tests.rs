use std::collections::BTreeMap;

use scryer_domain::{ListSourceOrigin, MediaFacet};
use scryer_plugin_sdk::{
    ListAuthBadge, ListMediaKind, ListProviderCapabilities, ListProviderDescriptor,
    ListProviderGroup, ListProviderItem, ListSourceParam, ListSourceParamType, ListUrlPattern,
    ListUrlPatternCapture, PluginDescriptor, ProviderDescriptor,
};

use super::*;
use crate::lists::gateway::ListChartCatalogEntry;

fn chart(provider: &str, key: &str, scope: &str) -> ListChartCatalogEntry {
    ListChartCatalogEntry {
        provider: provider.to_string(),
        chart_key: key.to_string(),
        scope: scope.to_string(),
        label: format!("Fixture chart {key}"),
        kinds: vec!["movie".to_string()],
        refreshed_at: None,
        item_count: 10,
    }
}

fn plugin(provider_type: &str, personal: bool, member_only: bool) -> PluginDescriptor {
    let descriptor = ListProviderDescriptor {
        provider_type: provider_type.to_string(),
        provider_aliases: Vec::new(),
        summary: Some("Fixture provider".to_string()),
        blurb: None,
        tile: None,
        brand_url_template: None,
        coverage: vec![ListMediaKind::Series],
        auth: Default::default(),
        groups: vec![ListProviderGroup {
            label: "Lists".to_string(),
            auth_badge: ListAuthBadge::NoAccountNeedsValue,
            items: vec![ListProviderItem {
                id: format!("{provider_type}:user_list"),
                name: "Fixture user list".to_string(),
                description: None,
                kinds: vec![ListMediaKind::Series],
                source_type: "user_list".to_string(),
                params: vec![ListSourceParam {
                    key: "slug".to_string(),
                    label: "Slug".to_string(),
                    param_type: ListSourceParamType::Text,
                    options: Vec::new(),
                    required: true,
                }],
                personal,
                default_interval_seconds: 3600,
            }],
        }],
        notes: Vec::new(),
        url_patterns: vec![ListUrlPattern {
            pattern: r"^https://lists\.example\.test/(?P<slug>[a-z0-9-]+)$".to_string(),
            source_type: "user_list".to_string(),
            captures: vec![ListUrlPatternCapture {
                group: "slug".to_string(),
                param: "slug".to_string(),
            }],
        }],
        capabilities: ListProviderCapabilities {
            requires_member_credential: member_only,
            ..Default::default()
        },
        config_fields: Vec::new(),
        default_base_url: None,
        allowed_hosts: Vec::new(),
        rate_limit_seconds: None,
    };
    PluginDescriptor {
        id: format!("{provider_type}-plugin"),
        name: "Fixture Lists".to_string(),
        version: "1.0.0".to_string(),
        sdk_version: "3.0.0".to_string(),
        sdk_constraint: String::new(),
        socket_permissions: Vec::new(),
        provider: ProviderDescriptor::ListProvider(descriptor),
    }
}

fn imdb_charts() -> Vec<ListChartCatalogEntry> {
    vec![
        chart("imdb", "imdb.top250.movies", "global"),
        chart("imdb", "imdb.moviemeter.movies", "global"),
        chart("IMDb", "imdb.toptv.series", "global"),
        chart("imdb", "imdb.tvmeter.series", "global"),
    ]
}

fn provider_types(manifests: &[ListProviderManifest]) -> Vec<&str> {
    manifests
        .iter()
        .map(|manifest| manifest.provider_type.as_str())
        .collect()
}

#[test]
fn charts_join_their_provider() {
    let manifests = merge_provider_catalog(
        &[plugin("fixturelists", false, false)],
        &[
            chart("fixturelists", "top", "global"),
            chart("tmdb", "popular", "movie"),
        ],
    );
    assert_eq!(provider_types(&manifests), vec!["fixturelists", "tmdb"]);

    let fixture = &manifests[0];
    assert_eq!(fixture.name, "Fixture Lists");
    assert_eq!(fixture.groups[0].label, "Charts");
    assert_eq!(
        fixture.groups[0].items[0].source_type,
        "smg_chart:top:global"
    );
    assert!(fixture.coverage.contains(&MediaFacet::Movie));
    assert!(fixture.coverage.contains(&MediaFacet::Series));

    let tmdb = &manifests[1];
    assert_eq!(tmdb.name, "TMDb");
    assert!(find_item(tmdb, "smg_chart:popular:movie").is_some());
}

#[test]
fn imdb_is_offered_only_as_the_gateway_charts() {
    let manifests = merge_provider_catalog(
        &[plugin("fixturelists", false, false)],
        &[chart("tmdb", "popular", "movie")],
    );
    assert_eq!(
        provider_types(&manifests),
        vec!["fixturelists", "tmdb"],
        "IMDb has no tile until the gateway serves its charts"
    );

    let mut charts = imdb_charts();
    charts.push(chart("tmdb", "popular", "movie"));
    let manifests = merge_provider_catalog(&[plugin("fixturelists", false, false)], &charts);
    assert_eq!(
        provider_types(&manifests),
        vec!["fixturelists", "imdb", "tmdb"]
    );
    let imdb = &manifests[1];
    assert_eq!(imdb.name, "IMDb");
    let items = imdb
        .groups
        .iter()
        .flat_map(|group| &group.items)
        .map(|item| item.source_type.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        items,
        vec![
            "smg_chart:imdb.top250.movies:global",
            "smg_chart:imdb.moviemeter.movies:global",
            "smg_chart:imdb.toptv.series:global",
            "smg_chart:imdb.tvmeter.series:global",
        ],
        "every chart, and nothing but charts"
    );
    assert!(
        imdb.url_patterns.is_empty(),
        "no imdb.com link is recognised"
    );
}

#[test]
fn chart_source_types_round_trip() {
    let source_type = smg_chart_source_type("top:rated", "global");
    assert_eq!(
        parse_smg_chart_source_type(&source_type),
        Some(("top:rated".to_string(), "global".to_string()))
    );
    assert_eq!(parse_smg_chart_source_type("user_list"), None);
    assert_eq!(parse_smg_chart_source_type("smg_chart:top:"), None);
}

#[test]
fn urls_are_recognised_by_manifest_patterns() {
    let manifests = merge_provider_catalog(&[plugin("fixturelists", false, false)], &[]);
    let recognized = recognize_url(&manifests, "https://lists.example.test/fixture-one").unwrap();
    assert_eq!(recognized.provider, "fixturelists");
    assert_eq!(recognized.source_type, "user_list");
    assert_eq!(
        recognized.params.get("slug").map(String::as_str),
        Some("fixture-one")
    );

    let with_imdb_charts =
        merge_provider_catalog(&[plugin("fixturelists", false, false)], &imdb_charts());
    assert!(
        recognize_url(&with_imdb_charts, "https://www.imdb.com/list/ls000000123/").is_none(),
        "an IMDb list link is not something Scryer follows"
    );

    assert!(recognize_url(&manifests, "https://unknown.example.test/x").is_none());
}

#[test]
fn public_sources_are_classified_by_origin() {
    let plugins = [plugin("fixturelists", false, false)];
    let charts = [chart("tmdb", "popular", "movie")];
    let manifests = merge_provider_catalog(&plugins, &charts);
    let member_only = member_only_providers(&plugins);

    let chart = classify_public_source(
        &manifests,
        &charts,
        &member_only,
        "tmdb",
        "smg_chart:popular:movie",
        &BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(
        chart.source.origin,
        ListSourceOrigin::SmgChart {
            chart_key: "popular".to_string(),
            scope: "movie".to_string()
        }
    );
    assert_eq!(chart.kinds, vec![MediaFacet::Movie]);

    let fetched = classify_public_source(
        &manifests,
        &charts,
        &member_only,
        "fixturelists",
        "user_list",
        &BTreeMap::from([("slug".to_string(), "fixture-one".to_string())]),
    )
    .unwrap();
    assert_eq!(fetched.source.origin, ListSourceOrigin::ProviderFetch);
    assert_eq!(fetched.interval_seconds, 3600);
    assert_eq!(fetched.kinds, vec![MediaFacet::Series]);
}

#[test]
fn public_sources_reject_what_they_cannot_follow() {
    let charts = [chart("tmdb", "popular", "movie")];
    let open = [plugin("fixturelists", false, false)];
    let manifests = merge_provider_catalog(&open, &charts);
    let no_member_only = member_only_providers(&open);

    let unknown_chart = classify_public_source(
        &manifests,
        &charts,
        &no_member_only,
        "tmdb",
        "smg_chart:missing:movie",
        &BTreeMap::new(),
    );
    assert!(unknown_chart.is_err());

    let bad_imdb = classify_public_source(
        &manifests,
        &charts,
        &no_member_only,
        "imdb",
        "user_list",
        &BTreeMap::from([("list_id".to_string(), "ur12345".to_string())]),
    );
    assert!(bad_imdb.is_err());

    let missing_param = classify_public_source(
        &manifests,
        &charts,
        &no_member_only,
        "fixturelists",
        "user_list",
        &BTreeMap::new(),
    );
    assert!(missing_param.is_err());

    let personal = [plugin("fixturelists", true, false)];
    let manifests = merge_provider_catalog(&personal, &charts);
    let personal_item = classify_public_source(
        &manifests,
        &charts,
        &member_only_providers(&personal),
        "fixturelists",
        "user_list",
        &BTreeMap::from([("slug".to_string(), "fixture-one".to_string())]),
    );
    assert!(personal_item.is_err());

    let member_only = [plugin("fixturelists", false, true)];
    let manifests = merge_provider_catalog(&member_only, &charts);
    let member_only_item = classify_public_source(
        &manifests,
        &charts,
        &member_only_providers(&member_only),
        "fixturelists",
        "user_list",
        &BTreeMap::from([("slug".to_string(), "fixture-one".to_string())]),
    );
    assert!(member_only_item.is_err());
}

#[test]
fn imdb_charts_can_be_followed_but_imdb_lists_cannot() {
    let plugins = [plugin("fixturelists", false, false)];
    let charts = imdb_charts();
    let manifests = merge_provider_catalog(&plugins, &charts);
    let member_only = member_only_providers(&plugins);

    for provider in ["imdb", "IMDb"] {
        let classified = classify_public_source(
            &manifests,
            &charts,
            &member_only,
            provider,
            &smg_chart_source_type("imdb.top250.movies", "global"),
            &BTreeMap::new(),
        )
        .expect("an IMDb chart is followable");
        assert_eq!(classified.source.provider, "imdb");
        assert_eq!(
            classified.source.origin,
            ListSourceOrigin::SmgChart {
                chart_key: "imdb.top250.movies".to_string(),
                scope: "global".to_string(),
            }
        );
    }

    let list = classify_public_source(
        &manifests,
        &charts,
        &member_only,
        "imdb",
        "user_list",
        &BTreeMap::from([("list_id".to_string(), "ls000000123".to_string())]),
    );
    assert!(
        matches!(&list, Err(AppError::Validation(message)) if message == "that provider has no such list"),
        "an IMDb list is refused: {list:?}"
    );
}

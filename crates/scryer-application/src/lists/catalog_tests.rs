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

#[test]
fn charts_join_their_provider_and_imdb_lists_are_always_offered() {
    let manifests = merge_provider_catalog(
        &[plugin("fixturelists", false, false)],
        &[
            chart("fixturelists", "top", "global"),
            chart("tmdb", "popular", "movie"),
        ],
    );
    let names = manifests
        .iter()
        .map(|manifest| manifest.provider_type.as_str())
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["fixturelists", "imdb", "tmdb"]);

    let fixture = &manifests[0];
    assert_eq!(fixture.name, "Fixture Lists");
    assert_eq!(fixture.groups[0].label, "Charts");
    assert_eq!(
        fixture.groups[0].items[0].source_type,
        "smg_chart:top:global"
    );
    assert!(fixture.coverage.contains(&MediaFacet::Movie));
    assert!(fixture.coverage.contains(&MediaFacet::Series));

    let tmdb = &manifests[2];
    assert_eq!(tmdb.name, "TMDb");
    assert!(find_item(tmdb, "smg_chart:popular:movie").is_some());

    let imdb = &manifests[1];
    assert!(find_item(imdb, LIST_SOURCE_TYPE_IMDB_USER_LIST).is_some());
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

    let imdb = recognize_url(&manifests, "https://www.imdb.com/list/ls000000123/").unwrap();
    assert_eq!(imdb.provider, "imdb");
    assert_eq!(
        imdb.params
            .get(LIST_SOURCE_IMDB_LIST_ID_PARAM)
            .map(String::as_str),
        Some("ls000000123")
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

    let imdb = classify_public_source(
        &manifests,
        &charts,
        &member_only,
        "imdb",
        "user_list",
        &BTreeMap::from([("list_id".to_string(), "ls000000123".to_string())]),
    )
    .unwrap();
    assert_eq!(imdb.source.origin, ListSourceOrigin::SmgImdbList);

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

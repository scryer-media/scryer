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
fn charts_join_their_provider_and_withheld_providers_are_left_out() {
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

    // The public IMDb list source stays built, so offering IMDb again is a
    // one-line change; it is just never shown.
    let with_withheld = merge_provider_catalog_with_withheld(
        &[plugin("fixturelists", false, false)],
        &[chart("tmdb", "popular", "movie")],
    );
    let imdb = with_withheld
        .iter()
        .find(|manifest| manifest.provider_type == LIST_PROVIDER_IMDB)
        .expect("the IMDb source is still built");
    assert!(find_item(imdb, LIST_SOURCE_TYPE_IMDB_USER_LIST).is_some());
}

#[test]
fn gateway_imdb_charts_never_reach_the_catalog() {
    let mut charts = imdb_charts();
    charts.push(chart("tmdb", "popular", "movie"));
    let manifests = merge_provider_catalog(&[plugin("fixturelists", false, false)], &charts);

    assert_eq!(provider_types(&manifests), vec!["fixturelists", "tmdb"]);
    let items = manifests
        .iter()
        .flat_map(|manifest| &manifest.groups)
        .flat_map(|group| &group.items)
        .collect::<Vec<_>>();
    assert!(
        items
            .iter()
            .all(|item| !item.id.contains("imdb") && !item.source_type.contains("imdb")),
        "no IMDb chart or public IMDb list is offered: {items:?}"
    );
    assert!(
        manifests
            .iter()
            .flat_map(|manifest| &manifest.url_patterns)
            .all(|pattern| !pattern.pattern.contains("imdb")),
        "no imdb.com link is recognised"
    );
}

#[test]
fn every_withheld_provider_is_matched_whatever_its_case() {
    assert!(is_withheld_list_provider("imdb"));
    assert!(is_withheld_list_provider(" IMDb "));
    assert!(!is_withheld_list_provider("tmdb"));
    assert!(!is_withheld_list_provider("fixturelists"));
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

    assert!(
        recognize_url(&manifests, "https://www.imdb.com/list/ls000000123/").is_none(),
        "the shown catalog does not recognise a withheld provider's link"
    );
    let with_withheld =
        merge_provider_catalog_with_withheld(&[plugin("fixturelists", false, false)], &[]);
    let imdb = recognize_url(&with_withheld, "https://www.imdb.com/list/ls000000123/").unwrap();
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

fn refused_as_withheld(result: AppResult<ClassifiedSource>) {
    match result {
        Err(AppError::Validation(message)) => assert_eq!(
            message, "IMDb lists are not available in Scryer right now",
            "unexpected refusal"
        ),
        other => panic!("expected the IMDb refusal, got {other:?}"),
    }
}

#[test]
fn withheld_providers_cannot_be_followed_even_when_the_gateway_lists_them() {
    let plugins = [plugin("fixturelists", false, false)];
    let charts = imdb_charts();
    let member_only = member_only_providers(&plugins);
    // Classified against the full catalog too, so the refusal does not rest
    // on the provider merely being missing from what is shown.
    for manifests in [
        merge_provider_catalog(&plugins, &charts),
        merge_provider_catalog_with_withheld(&plugins, &charts),
    ] {
        for provider in ["imdb", "IMDb"] {
            refused_as_withheld(classify_public_source(
                &manifests,
                &charts,
                &member_only,
                provider,
                &smg_chart_source_type("imdb.top250.movies", "global"),
                &BTreeMap::new(),
            ));
        }
        refused_as_withheld(classify_public_source(
            &manifests,
            &charts,
            &member_only,
            LIST_PROVIDER_IMDB,
            LIST_SOURCE_TYPE_IMDB_USER_LIST,
            &BTreeMap::from([(
                scryer_domain::LIST_SOURCE_IMDB_LIST_ID_PARAM.to_string(),
                "ls000000123".to_string(),
            )]),
        ));
    }
}

use std::sync::Arc;

use base64::Engine as _;
use scryer_application::{AppError, MetadataGateway, TitleExternalRef};
use scryer_domain::ExternalId;
use serde_json::json;
use wiremock::matchers::{body_string_contains, method, path, query_param};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use super::super::{InstanceAuth, MetadataGatewayClient, MtlsState, SmgEnrollmentConfig};
use super::{OP_LIST_CHART_CATALOG, OP_LIST_CHART_ITEMS, OP_LIST_IMDB_USER_LIST};

fn unsigned_client(endpoint: String) -> MetadataGatewayClient {
    MetadataGatewayClient::new_without_enrollment_store(
        endpoint,
        SmgEnrollmentConfig {
            registration_secret: None,
        },
    )
}

async fn signed_client(endpoint: String) -> MetadataGatewayClient {
    let client = MetadataGatewayClient::new_without_enrollment_store(
        endpoint,
        SmgEnrollmentConfig {
            registration_secret: Some("fixture-secret".to_string()),
        },
    );
    *client.mtls_state.write().await = MtlsState::Enrolled {
        client: scryer_outbound_http::smg_reqwest_client(),
        auth: InstanceAuth::Pq {
            instance_id: Arc::new("fixture-instance".to_string()),
            seed_b64: Arc::new(base64::engine::general_purpose::STANDARD.encode([7u8; 32])),
            key_id: Arc::new("fixture-key".to_string()),
            enrollment_generation: Some(1),
        },
    };
    client
}

fn query_variables(request: &Request) -> serde_json::Value {
    let raw = request
        .url
        .query_pairs()
        .find(|(key, _)| key == "variables")
        .map(|(_, value)| value.into_owned())
        .expect("APQ GET carries variables");
    serde_json::from_str(&raw).expect("variables are JSON")
}

fn chart_entry(rank: i64, title_id: Option<i64>, imdb: &str) -> serde_json::Value {
    json!({
        "rank": rank,
        "title_id": title_id,
        "resolved": title_id.is_some(),
        "kind": if title_id.is_some() { "movie" } else { "unknown" },
        "external_ids": [
            {"source": "imdb", "kind": "title", "id": imdb, "key": format!("imdb:title:{imdb}")}
        ],
        "display_title": format!("Fixture Ranked Title {rank}"),
        "year": 2031,
        "poster_url": if rank == 1 { json!("https://images.invalid/one.jpg") } else { json!("") }
    })
}

#[tokio::test]
async fn chart_catalog_reads_every_public_chart() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/graphql"))
        .and(query_param("operationName", OP_LIST_CHART_CATALOG))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": {"listChartCatalog": [{
                "provider": "tmdb",
                "chart_key": "tmdb.popular.movie",
                "scope": "global",
                "label": "Popular movies",
                "kinds": ["movie"],
                "refreshed_at": "2031-01-01T00:00:00Z",
                "item_count": 100
            }]}
        })))
        .expect(1)
        .mount(&server)
        .await;
    let client = unsigned_client(format!("{}/graphql", server.uri()));

    let catalog = client.list_chart_catalog("eng").await.expect("catalog");

    assert_eq!(catalog.len(), 1);
    assert_eq!(catalog[0].chart_key, "tmdb.popular.movie");
    assert_eq!(catalog[0].kinds, vec!["movie".to_string()]);
    assert_eq!(catalog[0].item_count, 100);
    let requests = server.received_requests().await.expect("captured");
    assert_eq!(query_variables(&requests[0]), json!({"language": "eng"}));
}

#[tokio::test]
async fn chart_items_keep_chart_order_and_map_ids() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/graphql"))
        .and(query_param("operationName", OP_LIST_CHART_ITEMS))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": {"listChartItems": [
                chart_entry(1, Some(7001), "tt0000701"),
                chart_entry(2, Some(7002), "tt0000702")
            ]}
        })))
        .expect(1)
        .mount(&server)
        .await;
    let client = unsigned_client(format!("{}/graphql", server.uri()));

    let items = client
        .list_chart_items("tmdb", "tmdb.popular.movie", "global", 250, "eng")
        .await
        .expect("chart items");

    assert_eq!(
        items.iter().map(|item| item.rank).collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert_eq!(items[0].title_id, Some(7001));
    assert_eq!(
        items[0].poster_url.as_deref(),
        Some("https://images.invalid/one.jpg")
    );
    assert_eq!(items[1].poster_url, None, "a blank poster is no poster");
    assert_eq!(items[0].external_ids[0].source, "imdb");
    assert_eq!(items[0].external_ids[0].value, "tt0000701");
    let requests = server.received_requests().await.expect("captured");
    assert_eq!(
        query_variables(&requests[0]),
        json!({
            "provider": "tmdb",
            "chartKey": "tmdb.popular.movie",
            "scope": "global",
            "limit": 250,
            "language": "eng"
        })
    );
}

#[tokio::test]
async fn an_unknown_chart_is_an_error_not_an_empty_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/graphql"))
        .and(query_param("operationName", OP_LIST_CHART_ITEMS))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "errors": [{"message": "unknown chart"}]
        })))
        .mount(&server)
        .await;
    let client = unsigned_client(format!("{}/graphql", server.uri()));

    let error = client
        .list_chart_items("tmdb", "tmdb.missing", "global", 250, "eng")
        .await
        .expect_err("unknown chart");
    assert!(matches!(&error, AppError::Repository(message) if message == "unknown chart"));
}

#[tokio::test]
async fn an_imdb_list_is_a_signed_post() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .and(body_string_contains(format!(
            "\"operationName\":\"{OP_LIST_IMDB_USER_LIST}\""
        )))
        .and(body_string_contains("\"listId\":\"ls000000001\""))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": {"listImdbUserList": [
                chart_entry(1, Some(7001), "tt0000701"),
                chart_entry(2, None, "tt0000799")
            ]}
        })))
        .expect(1)
        .mount(&server)
        .await;
    let client = signed_client(format!("{}/graphql", server.uri())).await;

    let items = client
        .list_imdb_user_list("ls000000001")
        .await
        .expect("imdb list");

    assert_eq!(items.len(), 2);
    assert!(!items[1].resolved);
    assert_eq!(items[1].kind, "unknown");
    let requests = server.received_requests().await.expect("captured");
    let signed = requests
        .iter()
        .find(|request| request.method.as_str() == "POST")
        .expect("post captured");
    assert_eq!(
        signed
            .headers
            .get("x-scryer-key-id")
            .and_then(|value| value.to_str().ok()),
        Some("fixture-key")
    );
    assert!(signed.headers.get("x-scryer-signature").is_some());
}

#[tokio::test]
async fn an_imdb_list_needs_an_enrolled_instance() {
    let server = MockServer::start().await;
    let client = unsigned_client(format!("{}/graphql", server.uri()));

    client
        .list_imdb_user_list("ls000000001")
        .await
        .expect_err("no instance auth");
    assert!(
        server
            .received_requests()
            .await
            .expect("captured")
            .is_empty()
    );
}

fn alias_ref(source: &str, value: &str) -> TitleExternalRef {
    TitleExternalRef {
        smg_id: None,
        external_ids: vec![ExternalId {
            source: source.to_string(),
            kind: None,
            value: value.to_string(),
        }],
    }
}

#[tokio::test]
async fn resolve_titles_sends_alias_sources_and_rebases_batches() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/graphql"))
        .and(query_param(
            "operationName",
            super::super::OP_RESOLVE_TITLES,
        ))
        .respond_with(|request: &Request| {
            let variables = query_variables(request);
            let count = variables["refs"].as_array().map(Vec::len).unwrap_or(0);
            let items = (0..count)
                .map(|index| {
                    json!({
                        "ref_index": index,
                        "resolved": true,
                        "title_id": 9000 + index as i64,
                        "kind": variables["kind"],
                        "primary_source": "anilist",
                        "redirected_from": null,
                        "created": false,
                        "external_ids": [],
                        "reason": ""
                    })
                })
                .collect::<Vec<_>>();
            ResponseTemplate::new(200).set_body_json(json!({"data": {"resolveTitles": items}}))
        })
        .expect(2)
        .mount(&server)
        .await;
    let client = unsigned_client(format!("{}/graphql", server.uri()));
    let mut refs = (0..super::METADATA_GATEWAY_MAX_TITLE_BULK_BATCH + 2)
        .map(|n| alias_ref("anilist", &format!("{}", 100 + n)))
        .collect::<Vec<_>>();
    refs[0].smg_id = Some(55);

    let resolutions = client
        .resolve_titles(&refs, "anime", true)
        .await
        .expect("resolve");

    assert_eq!(resolutions.len(), refs.len());
    assert_eq!(
        resolutions.last().map(|resolution| resolution.ref_index),
        Some(refs.len() - 1)
    );
    let requests = server.received_requests().await.expect("captured");
    let first = query_variables(&requests[0]);
    assert_eq!(first["kind"], "anime");
    assert_eq!(first["createMissing"], true);
    assert_eq!(
        first["refs"][0],
        json!({"id": 55, "externalIds": [{"source": "anilist", "id": "100"}]})
    );
}

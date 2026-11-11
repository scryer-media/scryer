use super::*;
use scryer_infrastructure_metadata::discovery::store::DiscoveryStore;

const LIBRARY_GRAPH_QUERY: &str = r#"
    query LibraryGraph($input: LibraryGraphInput!) {
      libraryGraph(input: $input) {
        generatedAt
        subjectCount
        truncated
        nodes { subjectKey targetKey clusterId }
        edges { from to weight kinds }
        clusters { id labelKey label medoid size }
      }
    }
"#;

/// A test context whose app reads discovery from the real SQLite store, as
/// production wires it, instead of the null repository.
async fn library_graph_context() -> TestContext {
    let mut ctx = TestContext::new().await;
    let discovery = std::sync::Arc::new(DiscoveryStore::new(ctx.db.datastore()));
    let app = ctx
        .app
        .with_test_overrides(|builder| builder.with_discovery_store(discovery));
    ctx.schema = scryer_interface::build_schema(app.clone(), ctx.auth_runtime.clone());
    ctx.app = app;
    ctx
}

/// One stored graph node: its key (subject and target alike), the titles
/// behind it and its cluster.
struct SeedNode<'a> {
    key: &'a str,
    title_ids: &'a [&'a str],
    cluster_id: i64,
}

/// Stores a complete movie graph run with the given children, as the sync
/// job's commit does.
async fn seed_graph_run(
    ctx: &TestContext,
    run_id: &str,
    nodes: &[SeedNode<'_>],
    edges: &[(&str, &str, f64)],
    clusters: &[(i64, Option<&str>, &str, &str, i64)],
    omitted_subject_keys: &[&str],
) {
    use scryer_application::{
        DiscoveryRepository, LibraryGraphClusterRecord, LibraryGraphEdgeRecord,
        LibraryGraphNodeRecord, LibraryGraphRunRecord, LibraryGraphSnapshotRecord,
    };
    let stamp = chrono::DateTime::parse_from_rfc3339("2026-06-25T00:00:00Z")
        .expect("stamp")
        .with_timezone(&chrono::Utc);
    let snapshot = LibraryGraphSnapshotRecord {
        run: LibraryGraphRunRecord {
            id: run_id.to_string(),
            facet: "movie".to_string(),
            axis: "SEMANTIC".to_string(),
            status: "complete".to_string(),
            subject_count: (nodes.len() + omitted_subject_keys.len()) as i64,
            submitted_count: nodes.len() as i64,
            generated_at: Some("2026-06-25T00:00:01Z".to_string()),
            error_code: None,
            error_text: None,
            retry_after_seconds: None,
            omitted_subject_keys: omitted_subject_keys
                .iter()
                .map(|key| key.to_string())
                .collect(),
            started_at: stamp,
            completed_at: Some(stamp),
            created_at: stamp,
            updated_at: stamp,
        },
        nodes: nodes
            .iter()
            .map(|node| LibraryGraphNodeRecord {
                subject_key: node.key.to_string(),
                target_key: node.key.to_string(),
                cluster_id: node.cluster_id,
                title_ids: node.title_ids.iter().map(|id| id.to_string()).collect(),
            })
            .collect(),
        edges: edges
            .iter()
            .map(|(from_key, to_key, weight)| LibraryGraphEdgeRecord {
                from_key: from_key.to_string(),
                to_key: to_key.to_string(),
                weight: *weight,
                kinds: vec!["semantic".to_string()],
            })
            .collect(),
        clusters: clusters
            .iter()
            .map(
                |(cluster_id, label_key, label, medoid, size)| LibraryGraphClusterRecord {
                    cluster_id: *cluster_id,
                    label_key: label_key.map(str::to_string),
                    label: label.to_string(),
                    medoid: medoid.to_string(),
                    size: *size,
                },
            )
            .collect(),
    };
    DiscoveryStore::new(ctx.db.datastore())
        .commit_library_graph(&snapshot)
        .await
        .expect("seed library graph run");
}

async fn seed_movie_graph(ctx: &TestContext, owned_title_id: &str) {
    seed_graph_run(
        ctx,
        "graph-run-1",
        &[
            SeedNode {
                key: "tmdb:movie:9001",
                title_ids: &[owned_title_id],
                cluster_id: 0,
            },
            SeedNode {
                key: "tmdb:movie:9002",
                title_ids: &["synthetic-title-not-in-any-library"],
                cluster_id: 0,
            },
        ],
        &[("tmdb:movie:9001", "tmdb:movie:9002", 0.9)],
        &[(
            0,
            Some("synthetic"),
            "Synthetic Cluster",
            "tmdb:movie:9002",
            2,
        )],
        // Left out of the build, and not a subject of any visible title.
        &["tmdb:movie:9003"],
    )
    .await;
}

async fn create_graph_library(ctx: &TestContext, name: &str, root_path: &str) -> (String, String) {
    let body = gql(
        ctx,
        r#"mutation($input: CreateLibraryInput!) {
            createLibrary(input: $input) { id roots { id } }
        }"#,
        json!({
            "input": {
                "facet": "MOVIE",
                "name": name,
                "roots": [{ "path": root_path, "isDefault": true }]
            }
        }),
    )
    .await;
    assert_no_errors(&body);
    let library = &body["data"]["createLibrary"];
    (
        library["id"].as_str().expect("library id").to_string(),
        library["roots"][0]["id"]
            .as_str()
            .expect("library root id")
            .to_string(),
    )
}

async fn add_graph_title(
    ctx: &TestContext,
    name: &str,
    tmdb_id: &str,
    (library_id, root_id): &(String, String),
) -> String {
    let body = gql(
        ctx,
        r#"mutation($input: AddTitleInput!) {
            addTitle(input: $input) { title { id } }
        }"#,
        json!({
            "input": {
                "name": name,
                "facet": "MOVIE",
                "libraryId": library_id,
                "monitored": true,
                "tags": [],
                "externalIds": [{ "source": "tmdb", "value": tmdb_id }],
                "options": { "rootFolderId": root_id }
            }
        }),
    )
    .await;
    assert_no_errors(&body);
    body["data"]["addTitle"]["title"]["id"]
        .as_str()
        .expect("graph title id")
        .to_string()
}

/// A viewer who may view and request in exactly one library and holds no app
/// permission that could widen that.
fn graph_viewer(library_id: &str) -> User {
    User {
        id: Id::new().0,
        username: "library-graph-viewer".to_string(),
        password_hash: None,
        password_change_required: false,
        account_kind: Default::default(),
        authorization: UserAuthorization {
            app: AppPermissionMask::NONE,
            libraries: HashMap::from([(
                library_id.to_string(),
                LibraryPermissionMask::from_permissions([
                    LibraryPermission::View,
                    LibraryPermission::Request,
                ]),
            )]),
            default_library: LibraryPermissionMask::NONE,
            actor_capabilities: scryer_domain::ActorCapabilityMask::MANAGE_OWN_ACCOUNT,
            login_status: Default::default(),
            loaded: true,
        },
    }
}

fn library_graph_query(library_ids: Option<&[&str]>) -> String {
    let library_ids = library_ids
        .map(|ids| format!(", libraryIds: {}", json!(ids)))
        .unwrap_or_default();
    format!(
        r#"{{
          libraryGraph(input: {{ facet: MOVIE{library_ids} }}) {{
            subjectCount
            truncated
            nodes {{ subjectKey targetKey clusterId }}
            edges {{ from to weight kinds }}
            clusters {{ id labelKey label medoid size }}
          }}
        }}"#
    )
}

#[tokio::test]
async fn graphql_library_graph_is_null_before_a_graph_is_built() {
    let ctx = library_graph_context().await;

    let body = gql(
        &ctx,
        LIBRARY_GRAPH_QUERY,
        json!({ "input": { "facet": "MOVIE" } }),
    )
    .await;

    assert_no_errors(&body);
    assert!(body["data"]["libraryGraph"].is_null(), "{body}");
}

#[tokio::test]
async fn graphql_library_graph_serves_only_nodes_and_edges_of_visible_titles() {
    let ctx = library_graph_context().await;
    let owned_title_id = add_test_title(&ctx, "Synthetic Graph Movie", "MOVIE").await;
    seed_movie_graph(&ctx, &owned_title_id).await;

    let body = gql(
        &ctx,
        LIBRARY_GRAPH_QUERY,
        json!({ "input": { "facet": "MOVIE" } }),
    )
    .await;

    assert_no_errors(&body);
    let graph = &body["data"]["libraryGraph"];
    assert_eq!(graph["generatedAt"], "2026-06-25T00:00:01Z", "{body}");
    assert_eq!(
        graph["nodes"],
        json!([{ "subjectKey": "tmdb:movie:9001", "targetKey": "tmdb:movie:9001", "clusterId": 0 }])
    );
    // The only edge reaches a title outside every library, so it is dropped.
    assert_eq!(graph["edges"], json!([]));
    // The stored medoid is hidden; the one visible member replaces it.
    assert_eq!(
        graph["clusters"],
        json!([{
            "id": 0,
            "labelKey": "synthetic",
            "label": "Synthetic Cluster",
            "medoid": "tmdb:movie:9001",
            "size": 1
        }])
    );
    // The build left a subject out, but not one of this viewer's.
    assert_eq!(graph["truncated"], false);
    assert_eq!(graph["subjectCount"], 1);
}

#[tokio::test]
async fn graphql_library_graph_answers_unchanged_only_for_the_same_view() {
    const QUERY: &str = r#"
        query LibraryGraph($input: LibraryGraphInput!) {
          libraryGraph(input: $input) {
            generatedAt
            viewKey
            unchanged
            subjectCount
            truncated
            nodes { subjectKey targetKey clusterId }
            edges { from to weight kinds }
            clusters { id labelKey label medoid size }
          }
        }
    "#;
    let ctx = library_graph_context().await;

    // Before any build, a held key still reads as "not built yet".
    let body = gql(
        &ctx,
        QUERY,
        json!({ "input": { "facet": "MOVIE", "knownViewKey": "v1:synthetic" } }),
    )
    .await;
    assert_no_errors(&body);
    assert!(body["data"]["libraryGraph"].is_null(), "{body}");

    let owned_title_id = add_test_title(&ctx, "Emberfall", "MOVIE").await;
    seed_movie_graph(&ctx, &owned_title_id).await;
    let full = gql(&ctx, QUERY, json!({ "input": { "facet": "MOVIE" } })).await;
    assert_no_errors(&full);
    assert_eq!(full["data"]["libraryGraph"]["unchanged"], false, "{full}");
    let view_key = full["data"]["libraryGraph"]["viewKey"]
        .as_str()
        .expect("view key")
        .to_string();
    assert!(!view_key.is_empty());

    // Another key gets the same full graph as a read without one.
    let stale = gql(
        &ctx,
        QUERY,
        json!({ "input": { "facet": "MOVIE", "knownViewKey": "v1:synthetic" } }),
    )
    .await;
    assert_no_errors(&stale);
    assert_eq!(stale["data"], full["data"]);

    let unchanged = gql(
        &ctx,
        QUERY,
        json!({ "input": { "facet": "MOVIE", "knownViewKey": view_key } }),
    )
    .await;
    assert_no_errors(&unchanged);
    assert_eq!(
        unchanged["data"]["libraryGraph"],
        json!({
            "generatedAt": "2026-06-25T00:00:01Z",
            "viewKey": view_key,
            "unchanged": true,
            "subjectCount": 0,
            "truncated": false,
            "nodes": [],
            "edges": [],
            "clusters": []
        })
    );
}

#[tokio::test]
async fn graphql_library_graph_is_not_unchanged_when_the_viewers_libraries_change() {
    const QUERY: &str = r#"
        query LibraryGraph($input: LibraryGraphInput!) {
          libraryGraph(input: $input) { viewKey unchanged subjectCount nodes { subjectKey } }
        }
    "#;
    let ctx = library_graph_context().await;
    let first_library =
        create_graph_library(&ctx, "Synthetic Ashgrove Movies", "/library-graph-key/one").await;
    let second_library =
        create_graph_library(&ctx, "Synthetic Duskmere Movies", "/library-graph-key/two").await;
    let first = add_graph_title(&ctx, "Synthetic Ashgrove", "9401", &first_library).await;
    let second = add_graph_title(&ctx, "Synthetic Duskmere", "9402", &second_library).await;
    seed_graph_run(
        &ctx,
        "graph-run-view-key",
        &[
            SeedNode {
                key: "tmdb:movie:9401",
                title_ids: &[&first],
                cluster_id: -1,
            },
            SeedNode {
                key: "tmdb:movie:9402",
                title_ids: &[&second],
                cluster_id: -1,
            },
        ],
        &[],
        &[],
        &[],
    )
    .await;
    let mut viewer = graph_viewer(&first_library.0);
    let read = |viewer: User, known: Option<String>| {
        let ctx = &ctx;
        async move {
            let mut input = json!({ "facet": "MOVIE" });
            if let Some(known) = known {
                input["knownViewKey"] = json!(known);
            }
            let request = async_graphql::Request::new(QUERY)
                .variables(async_graphql::Variables::from_json(
                    json!({ "input": input }),
                ))
                .data(viewer);
            let body = serde_json::to_value(ctx.schema.execute(request).await).expect("json");
            assert_no_errors(&body);
            body["data"]["libraryGraph"].clone()
        }
    };

    let narrow = read(viewer.clone(), None).await;
    assert_eq!(narrow["subjectCount"], 1, "{narrow}");
    let narrow_key = narrow["viewKey"].as_str().expect("view key").to_string();
    assert_eq!(
        read(viewer.clone(), Some(narrow_key.clone())).await["unchanged"],
        true
    );

    // The viewer is given the second library; the graph is not rebuilt, but
    // the held view is no longer what this viewer sees.
    viewer.authorization.libraries.insert(
        second_library.0.clone(),
        LibraryPermissionMask::from_permissions([LibraryPermission::View]),
    );
    let wide = read(viewer.clone(), Some(narrow_key.clone())).await;
    assert_eq!(wide["unchanged"], false, "{wide}");
    assert_eq!(wide["subjectCount"], 2, "{wide}");
    assert_ne!(wide["viewKey"], json!(narrow_key));
}

#[tokio::test]
async fn graphql_library_graph_never_shows_a_scoped_viewer_another_librarys_titles() {
    let ctx = library_graph_context().await;
    let visible_library = create_graph_library(
        &ctx,
        "Synthetic Visible Movies",
        "/library-graph-rbac/visible",
    )
    .await;
    let hidden_library = create_graph_library(
        &ctx,
        "Synthetic Hidden Movies",
        "/library-graph-rbac/hidden",
    )
    .await;
    let visible_one =
        add_graph_title(&ctx, "Synthetic Visible One", "9101", &visible_library).await;
    let visible_two =
        add_graph_title(&ctx, "Synthetic Visible Two", "9102", &visible_library).await;
    let hidden = add_graph_title(&ctx, "Synthetic Hidden Medoid", "9201", &hidden_library).await;
    // The hidden title is the cluster's medoid, its label is that title's
    // name (no label key), it has edges to both visible titles, and the build
    // left out a hidden subject.
    seed_graph_run(
        &ctx,
        "graph-run-rbac",
        &[
            SeedNode {
                key: "tmdb:movie:9101",
                title_ids: &[&visible_one],
                cluster_id: 0,
            },
            SeedNode {
                key: "tmdb:movie:9102",
                title_ids: &[&visible_two],
                cluster_id: 0,
            },
            SeedNode {
                key: "tmdb:movie:9201",
                title_ids: &[&hidden],
                cluster_id: 0,
            },
        ],
        &[
            ("tmdb:movie:9101", "tmdb:movie:9102", 0.8),
            ("tmdb:movie:9101", "tmdb:movie:9201", 0.9),
            ("tmdb:movie:9102", "tmdb:movie:9201", 0.7),
        ],
        &[(0, None, "Synthetic Hidden Medoid", "tmdb:movie:9201", 3)],
        &["tmdb:movie:9202"],
    )
    .await;
    let viewer = graph_viewer(&visible_library.0);

    let expected = json!({
        "subjectCount": 2,
        "truncated": false,
        "nodes": [
            { "subjectKey": "tmdb:movie:9101", "targetKey": "tmdb:movie:9101", "clusterId": 0 },
            { "subjectKey": "tmdb:movie:9102", "targetKey": "tmdb:movie:9102", "clusterId": 0 }
        ],
        "edges": [
            { "from": "tmdb:movie:9101", "to": "tmdb:movie:9102", "weight": 0.8, "kinds": ["semantic"] }
        ],
        "clusters": [{
            "id": 0,
            "labelKey": null,
            "label": "Synthetic Visible One",
            "medoid": "tmdb:movie:9101",
            "size": 2
        }]
    });
    for library_ids in [None, Some(&[visible_library.0.as_str()][..])] {
        let body = schema_exec(
            &ctx,
            &library_graph_query(library_ids),
            Some(viewer.clone()),
        )
        .await;
        assert_no_errors(&body);
        assert_eq!(body["data"]["libraryGraph"], expected, "{library_ids:?}");
        let payload = body.to_string();
        for leaked in [
            "9201",
            "9202",
            hidden.as_str(),
            "Synthetic Hidden Medoid",
            hidden_library.0.as_str(),
        ] {
            assert!(!payload.contains(leaked), "{leaked} leaked: {payload}");
        }
    }

    // Asking for the hidden library alone reads as no graph, not as its titles.
    let body = schema_exec(
        &ctx,
        &library_graph_query(Some(&[hidden_library.0.as_str()])),
        Some(viewer.clone()),
    )
    .await;
    assert_no_errors(&body);
    assert!(body["data"]["libraryGraph"].is_null(), "{body}");

    // The admin scoping to the hidden library sees that title and its stored
    // medoid and label as they are.
    let body = gql(
        &ctx,
        LIBRARY_GRAPH_QUERY,
        json!({ "input": { "facet": "MOVIE", "libraryIds": [hidden_library.0] } }),
    )
    .await;
    assert_no_errors(&body);
    let graph = &body["data"]["libraryGraph"];
    assert_eq!(
        graph["nodes"],
        json!([{ "subjectKey": "tmdb:movie:9201", "targetKey": "tmdb:movie:9201", "clusterId": 0 }])
    );
    assert_eq!(graph["clusters"][0]["label"], "Synthetic Hidden Medoid");
    assert_eq!(graph["edges"], json!([]));
}

#[tokio::test]
async fn graphql_library_graph_is_truncated_only_when_a_visible_subject_was_left_out() {
    let ctx = library_graph_context().await;
    let library = create_graph_library(
        &ctx,
        "Synthetic Truncated Movies",
        "/library-graph-truncated",
    )
    .await;
    let kept = add_graph_title(&ctx, "Synthetic Kept Movie", "9301", &library).await;
    add_graph_title(&ctx, "Synthetic Omitted Movie", "9302", &library).await;
    seed_graph_run(
        &ctx,
        "graph-run-truncated",
        &[SeedNode {
            key: "tmdb:movie:9301",
            title_ids: &[&kept],
            cluster_id: -1,
        }],
        &[],
        &[],
        &["tmdb:movie:9302"],
    )
    .await;

    let body = schema_exec(
        &ctx,
        &library_graph_query(None),
        Some(graph_viewer(&library.0)),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["libraryGraph"]["truncated"], true, "{body}");
    assert_eq!(body["data"]["libraryGraph"]["subjectCount"], 2, "{body}");
}

#[tokio::test]
async fn graphql_discovery_neighbors_is_empty_for_a_viewer_outside_the_facet() {
    let ctx = library_graph_context().await;
    let library = create_graph_library(
        &ctx,
        "Synthetic Neighbor Movies",
        "/library-graph-neighbors",
    )
    .await;

    // A movie-only viewer asking about series gets nothing, whatever the key.
    let body = schema_exec(
        &ctx,
        r#"{ discoveryNeighbors(input: { targetKey: "plex:series:9001", facet: SERIES }) { items { targetKey groupId } groups { id label size } } }"#,
        Some(graph_viewer(&library.0)),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(
        body["data"]["discoveryNeighbors"],
        json!({ "items": [], "groups": [] }),
        "{body}"
    );

    // The same viewer is refused an unusable key on its own facet.
    let body = schema_exec(
        &ctx,
        r#"{ discoveryNeighbors(input: { targetKey: "plex:movie:9001", facet: MOVIE }) { items { targetKey } } }"#,
        Some(graph_viewer(&library.0)),
    )
    .await;
    assert_eq!(
        body["errors"][0]["extensions"]["reason"], "DISCOVERY_NEIGHBOR_TARGET_KEY_UNSUPPORTED",
        "{body}"
    );
}

#[tokio::test]
async fn graphql_discovery_neighbors_refuses_an_unknown_anchor_with_its_reason() {
    let ctx = library_graph_context().await;

    let body = gql(
        &ctx,
        r#"
        query DiscoveryNeighbors($input: DiscoveryNeighborsInput!) {
          discoveryNeighbors(input: $input) { items { targetKey resolvedTitleId groupId } groups { id labelKey label medoid size } }
        }
        "#,
        json!({ "input": { "targetKey": "plex:movie:9001", "facet": "MOVIE" } }),
    )
    .await;

    assert_eq!(
        body["errors"][0]["extensions"]["reason"], "DISCOVERY_NEIGHBOR_TARGET_KEY_UNSUPPORTED",
        "{body}"
    );
}

/// A cached related title, as the neighbour cache stores one.
fn cached_neighbor_item(
    target_key: &str,
    display_title: &str,
) -> scryer_application::DiscoveryItemRecord {
    let stamp = "2026-06-25T00:00:00Z";
    serde_json::from_value(json!({
        "id": format!("neighbors:tmdb:movie:9101:{target_key}"),
        "run_id": "neighbors:tmdb:movie:9101",
        "source_run_kind": "title_more_like_this",
        "sort_index": 0,
        "target_key": target_key,
        "target_kind": "movie",
        "resolved": false,
        "display_title": display_title,
        "sort_title": display_title,
        "poster_url": format!("https://images.example.test/{target_key}.jpg"),
        "canonical_tags": [],
        "is_adult": false,
        "content_ratings": [],
        "rating_sources": [],
        "external_ratings": [],
        "external_ids": [],
        "status_tags": [],
        "source_tags": [],
        "sources": [],
        "relation_types": [],
        "relation_subtypes": [],
        "chart_signals": [],
        "provider_signals": [],
        "rank_components": [],
        "matched_subject_keys": [],
        "matched_subject_titles": [],
        "matched_subject_count": 0,
        "library_provenance": [],
        "owned_in_input": false,
        "person_ids": [],
        "facet_terms": [],
        "context_terms": [],
        "change_subject_keys": [],
        "removed_subject_keys": [],
        "created_at": stamp,
        "updated_at": stamp,
    }))
    .expect("synthetic neighbour item")
}

#[tokio::test]
async fn graphql_discovery_neighbors_serves_the_gateway_groups() {
    use scryer_application::DiscoveryRepository;

    let ctx = library_graph_context().await;
    let library = create_graph_library(
        &ctx,
        "Synthetic Grouped Movies",
        "/library-graph-neighbor-groups",
    )
    .await;
    let store = DiscoveryStore::new(ctx.db.datastore());
    let fetched_at = chrono::Utc::now();
    let neighbor = |target_key: &str, display_title: &str, group_id: Option<i64>| {
        scryer_application::DiscoveryNeighborItemRecord {
            item: cached_neighbor_item(target_key, display_title),
            group_id,
        }
    };
    store
        .replace_discovery_neighbor_cache(
            &scryer_application::DiscoveryNeighborCacheRecord {
                target_key: "tmdb:movie:9101".to_string(),
                language: "eng".to_string(),
                items: vec![
                    neighbor("tmdb:movie:9102", "Synthetic Grouped One", Some(0)),
                    neighbor("tmdb:movie:9103", "Synthetic Grouped Two", Some(1)),
                    neighbor("tmdb:movie:9104", "Synthetic Grouped Three", Some(0)),
                    neighbor("tmdb:movie:9105", "Synthetic Ungrouped", None),
                ],
                groups: vec![
                    scryer_application::DiscoveryRelatedGroup {
                        id: 0,
                        label_key: Some("canonical:theme:heist".to_string()),
                        label: "Heist".to_string(),
                        medoid: "tmdb:movie:9102".to_string(),
                        size: 5,
                    },
                    scryer_application::DiscoveryRelatedGroup {
                        id: 1,
                        label_key: None,
                        label: "Synthetic Grouped Two".to_string(),
                        medoid: "tmdb:movie:9103".to_string(),
                        size: 1,
                    },
                ],
                fetched_at,
                regroup_attempted: false,
            },
            fetched_at - chrono::Duration::hours(24),
        )
        .await
        .expect("seed neighbour cache");

    let body = schema_exec(
        &ctx,
        r#"{ discoveryNeighbors(input: { targetKey: "tmdb:movie:9101", facet: MOVIE }) {
              items { targetKey groupId }
              groups { id labelKey label medoid size }
            } }"#,
        Some(graph_viewer(&library.0)),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(
        body["data"]["discoveryNeighbors"],
        json!({
            "items": [
                { "targetKey": "tmdb:movie:9102", "groupId": 0 },
                { "targetKey": "tmdb:movie:9103", "groupId": 1 },
                { "targetKey": "tmdb:movie:9104", "groupId": 0 },
                { "targetKey": "tmdb:movie:9105", "groupId": null },
            ],
            "groups": [
                { "id": 0, "labelKey": "canonical:theme:heist", "label": "Heist", "medoid": "tmdb:movie:9102", "size": 2 },
                { "id": 1, "labelKey": null, "label": "Synthetic Grouped Two", "medoid": "tmdb:movie:9103", "size": 1 },
            ],
        }),
        "{body}"
    );
}

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
/// job's commit would.
async fn seed_graph_run(
    ctx: &TestContext,
    run_id: &str,
    nodes: &[SeedNode<'_>],
    edges: &[(&str, &str, f64)],
    clusters: &[(i64, Option<&str>, &str, &str, i64)],
    omitted_subject_keys: &[&str],
) {
    let stamp = "2026-06-25T00:00:00Z";
    sqlx::query(
        "INSERT INTO discovery_library_graph_runs
            (id, facet, axis, status, subject_count, submitted_count, generated_at,
             omitted_subject_keys_json, started_at, completed_at, created_at, updated_at)
         VALUES (?, 'movie', 'SEMANTIC', 'complete', ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(run_id)
    .bind((nodes.len() + omitted_subject_keys.len()) as i64)
    .bind(nodes.len() as i64)
    .bind("2026-06-25T00:00:01Z")
    .bind(json!(omitted_subject_keys).to_string())
    .bind(stamp)
    .bind(stamp)
    .bind(stamp)
    .bind(stamp)
    .execute(ctx.db.pool())
    .await
    .expect("seed library graph run");
    for node in nodes {
        sqlx::query(
            "INSERT INTO discovery_library_graph_nodes
                (run_id, subject_key, target_key, cluster_id, title_ids_json)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(run_id)
        .bind(node.key)
        .bind(node.key)
        .bind(node.cluster_id)
        .bind(json!(node.title_ids).to_string())
        .execute(ctx.db.pool())
        .await
        .expect("seed library graph node");
    }
    for (from_key, to_key, weight) in edges {
        sqlx::query(
            "INSERT INTO discovery_library_graph_edges (run_id, from_key, to_key, weight, kinds_json)
             VALUES (?, ?, ?, ?, '[\"semantic\"]')",
        )
        .bind(run_id)
        .bind(from_key)
        .bind(to_key)
        .bind(weight)
        .execute(ctx.db.pool())
        .await
        .expect("seed library graph edge");
    }
    for (cluster_id, label_key, label, medoid, size) in clusters {
        sqlx::query(
            "INSERT INTO discovery_library_graph_clusters
                (run_id, cluster_id, label_key, label, medoid, size)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(run_id)
        .bind(cluster_id)
        .bind(label_key)
        .bind(label)
        .bind(medoid)
        .bind(size)
        .execute(ctx.db.pool())
        .await
        .expect("seed library graph cluster");
    }
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
        r#"{ discoveryNeighbors(input: { targetKey: "plex:series:9001", facet: SERIES }) { targetKey } }"#,
        Some(graph_viewer(&library.0)),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["discoveryNeighbors"], json!([]), "{body}");

    // The same viewer is refused an unusable key on its own facet.
    let body = schema_exec(
        &ctx,
        r#"{ discoveryNeighbors(input: { targetKey: "plex:movie:9001", facet: MOVIE }) { targetKey } }"#,
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
          discoveryNeighbors(input: $input) { targetKey resolvedTitleId }
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

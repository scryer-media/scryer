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

async fn seed_movie_graph(ctx: &TestContext, owned_title_id: &str) {
    let stamp = "2026-06-25T00:00:00Z";
    sqlx::query(
        "INSERT INTO discovery_library_graph_runs
            (id, facet, axis, status, subject_count, submitted_count, generated_at,
             started_at, completed_at, created_at, updated_at)
         VALUES ('graph-run-1', 'movie', 'SEMANTIC', 'complete', 3, 2, ?, ?, ?, ?, ?)",
    )
    .bind("2026-06-25T00:00:01Z")
    .bind(stamp)
    .bind(stamp)
    .bind(stamp)
    .bind(stamp)
    .execute(ctx.db.pool())
    .await
    .expect("seed library graph run");
    for (subject_key, title_id) in [
        ("tmdb:movie:9001", owned_title_id),
        ("tmdb:movie:9002", "synthetic-title-not-in-any-library"),
    ] {
        sqlx::query(
            "INSERT INTO discovery_library_graph_nodes
                (run_id, subject_key, target_key, cluster_id, title_ids_json)
             VALUES ('graph-run-1', ?, ?, 0, ?)",
        )
        .bind(subject_key)
        .bind(subject_key)
        .bind(json!([title_id]).to_string())
        .execute(ctx.db.pool())
        .await
        .expect("seed library graph node");
    }
    sqlx::query(
        "INSERT INTO discovery_library_graph_edges (run_id, from_key, to_key, weight, kinds_json)
         VALUES ('graph-run-1', 'tmdb:movie:9001', 'tmdb:movie:9002', 0.9, '[\"semantic\"]')",
    )
    .execute(ctx.db.pool())
    .await
    .expect("seed library graph edge");
    sqlx::query(
        "INSERT INTO discovery_library_graph_clusters
            (run_id, cluster_id, label_key, label, medoid, size)
         VALUES ('graph-run-1', 0, 'synthetic', 'Synthetic Cluster', 'tmdb:movie:9002', 2)",
    )
    .execute(ctx.db.pool())
    .await
    .expect("seed library graph cluster");
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
    assert_eq!(graph["truncated"], true);
    assert_eq!(graph["subjectCount"], 1);
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

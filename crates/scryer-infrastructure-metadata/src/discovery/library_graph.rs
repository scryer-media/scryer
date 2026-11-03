//! Storage for the per-facet library graph: one run row per build, with the
//! build's nodes, edges and clusters keyed by the run id.

use scryer_application::{
    AppResult, LibraryGraphClusterRecord, LibraryGraphEdgeRecord, LibraryGraphNodeRecord,
    LibraryGraphPruneReport, LibraryGraphRunRecord, LibraryGraphSnapshotRecord,
};
use std::sync::Arc;

use crate::queries::sql_runtime::{
    SqlArg, SqlExec, SqlRow, SqlRuntime, SqlTx, StoreDatastore, repo_err,
};

/// Status of a run whose graph is stored and served.
pub(crate) const LIBRARY_GRAPH_RUN_COMPLETE: &str = "complete";

const RUN_COLUMNS: &[&str] = &[
    "id",
    "facet",
    "axis",
    "status",
    "subject_count",
    "submitted_count",
    "generated_at",
    "error_code",
    "error_text",
    "started_at",
    "completed_at",
    "created_at",
    "updated_at",
];

const NODE_COLUMNS: &[&str] = &[
    "run_id",
    "subject_key",
    "target_key",
    "cluster_id",
    "title_ids_json",
];

const EDGE_COLUMNS: &[&str] = &["run_id", "from_key", "to_key", "weight", "kinds_json"];

const CLUSTER_COLUMNS: &[&str] = &[
    "run_id",
    "cluster_id",
    "label_key",
    "label",
    "medoid",
    "size",
];

const CHILD_TABLES: &[&str] = &[
    "discovery_library_graph_nodes",
    "discovery_library_graph_edges",
    "discovery_library_graph_clusters",
];

fn insert_prefix(table: &str, columns: &[&str]) -> String {
    format!("INSERT INTO {table} ({})", columns.join(", "))
}

fn run_args(run: &LibraryGraphRunRecord) -> Vec<SqlArg> {
    vec![
        SqlArg::Text(run.id.clone()),
        SqlArg::Text(run.facet.clone()),
        SqlArg::Text(run.axis.clone()),
        SqlArg::Text(run.status.clone()),
        SqlArg::I64(run.subject_count),
        SqlArg::I64(run.submitted_count),
        SqlArg::OptText(run.generated_at.clone()),
        SqlArg::OptText(run.error_code.clone()),
        SqlArg::OptText(run.error_text.clone()),
        SqlArg::Timestamp(run.started_at),
        SqlArg::OptTimestamp(run.completed_at),
        SqlArg::Timestamp(run.created_at),
        SqlArg::Timestamp(run.updated_at),
    ]
}

fn run_from_row(row: &SqlRow) -> AppResult<LibraryGraphRunRecord> {
    Ok(LibraryGraphRunRecord {
        id: row.text("id")?,
        facet: row.text("facet")?,
        axis: row.text("axis")?,
        status: row.text("status")?,
        subject_count: row.i64("subject_count")?,
        submitted_count: row.i64("submitted_count")?,
        generated_at: row.opt_text("generated_at")?,
        error_code: row.opt_text("error_code")?,
        error_text: row.opt_text("error_text")?,
        started_at: row.timestamp("started_at")?,
        completed_at: row.opt_timestamp("completed_at")?,
        created_at: row.timestamp("created_at")?,
        updated_at: row.timestamp("updated_at")?,
    })
}

fn string_list_json(values: &[String]) -> AppResult<String> {
    serde_json::to_string(values).map_err(repo_err)
}

fn string_list_from_json(raw: &str) -> AppResult<Vec<String>> {
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(raw).map_err(repo_err)
}

fn node_from_row(row: &SqlRow) -> AppResult<LibraryGraphNodeRecord> {
    Ok(LibraryGraphNodeRecord {
        subject_key: row.text("subject_key")?,
        target_key: row.text("target_key")?,
        cluster_id: row.i64("cluster_id")?,
        title_ids: string_list_from_json(&row.text("title_ids_json")?)?,
    })
}

fn edge_from_row(row: &SqlRow) -> AppResult<LibraryGraphEdgeRecord> {
    Ok(LibraryGraphEdgeRecord {
        from_key: row.text("from_key")?,
        to_key: row.text("to_key")?,
        weight: row.opt_f64("weight")?.unwrap_or_default(),
        kinds: string_list_from_json(&row.text("kinds_json")?)?,
    })
}

fn cluster_from_row(row: &SqlRow) -> AppResult<LibraryGraphClusterRecord> {
    Ok(LibraryGraphClusterRecord {
        cluster_id: row.i64("cluster_id")?,
        label_key: row.opt_text("label_key")?,
        label: row.text("label")?,
        medoid: row.text("medoid")?,
        size: row.i64("size")?,
    })
}

async fn insert_run_tx(tx: &mut SqlTx<'_>, run: &LibraryGraphRunRecord) -> AppResult<()> {
    SqlRuntime::execute_batch_insert(
        tx,
        &insert_prefix("discovery_library_graph_runs", RUN_COLUMNS),
        RUN_COLUMNS.len(),
        vec![run_args(run)],
        "",
    )
    .await?;
    Ok(())
}

/// Deletes every run for `facet` other than `keep_run_id` that matches
/// `status_filter` (all statuses when `None`), children first.
async fn delete_other_runs_tx(
    tx: &mut SqlTx<'_>,
    facet: &str,
    keep_run_id: &str,
    status_filter: Option<&str>,
) -> AppResult<()> {
    let (status_clause, status_args) = match status_filter {
        Some(status) => (" AND status = {}", vec![SqlArg::Text(status.to_string())]),
        None => ("", Vec::new()),
    };
    let mut args = vec![
        SqlArg::Text(facet.to_string()),
        SqlArg::Text(keep_run_id.to_string()),
    ];
    args.extend(status_args);
    for table in CHILD_TABLES {
        SqlRuntime::execute(
            SqlExec::Tx(tx),
            &format!(
                "DELETE FROM {table} WHERE run_id IN (SELECT id FROM discovery_library_graph_runs \
                 WHERE facet = {{}} AND id <> {{}}{status_clause})"
            ),
            &args,
        )
        .await?;
    }
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        &format!(
            "DELETE FROM discovery_library_graph_runs WHERE facet = {{}} AND id <> {{}}{status_clause}"
        ),
        &args,
    )
    .await?;
    Ok(())
}

pub(crate) async fn record_failed_library_graph_run(
    datastore: &StoreDatastore,
    run: &LibraryGraphRunRecord,
) -> AppResult<()> {
    let run = Arc::new(run.clone());
    SqlRuntime::run_in_transaction(datastore, "record_failed_library_graph_run", move |tx| {
        let run = Arc::clone(&run);
        Box::pin(async move {
            // Only the newest failure is worth keeping next to the last
            // complete graph.
            delete_other_runs_tx(tx, &run.facet, &run.id, Some(&run.status)).await?;
            SqlRuntime::execute(
                SqlExec::Tx(tx),
                "DELETE FROM discovery_library_graph_runs WHERE id = {}",
                &[SqlArg::Text(run.id.clone())],
            )
            .await?;
            insert_run_tx(tx, &run).await
        })
    })
    .await
}

pub(crate) async fn commit_library_graph(
    datastore: &StoreDatastore,
    snapshot: &LibraryGraphSnapshotRecord,
) -> AppResult<()> {
    let snapshot = Arc::new(snapshot.clone());
    SqlRuntime::run_in_transaction(datastore, "commit_library_graph", move |tx| {
        let snapshot = Arc::clone(&snapshot);
        Box::pin(async move {
            let run_id = snapshot.run.id.clone();
            insert_run_tx(tx, &snapshot.run).await?;

            let mut node_rows = Vec::with_capacity(snapshot.nodes.len());
            for node in &snapshot.nodes {
                node_rows.push(vec![
                    SqlArg::Text(run_id.clone()),
                    SqlArg::Text(node.subject_key.clone()),
                    SqlArg::Text(node.target_key.clone()),
                    SqlArg::I64(node.cluster_id),
                    SqlArg::Text(string_list_json(&node.title_ids)?),
                ]);
            }
            SqlRuntime::execute_batch_insert(
                tx,
                &insert_prefix("discovery_library_graph_nodes", NODE_COLUMNS),
                NODE_COLUMNS.len(),
                node_rows,
                "ON CONFLICT DO NOTHING",
            )
            .await?;

            let mut edge_rows = Vec::with_capacity(snapshot.edges.len());
            for edge in &snapshot.edges {
                edge_rows.push(vec![
                    SqlArg::Text(run_id.clone()),
                    SqlArg::Text(edge.from_key.clone()),
                    SqlArg::Text(edge.to_key.clone()),
                    SqlArg::F64(edge.weight),
                    SqlArg::Text(string_list_json(&edge.kinds)?),
                ]);
            }
            SqlRuntime::execute_batch_insert(
                tx,
                &insert_prefix("discovery_library_graph_edges", EDGE_COLUMNS),
                EDGE_COLUMNS.len(),
                edge_rows,
                "ON CONFLICT DO NOTHING",
            )
            .await?;

            let cluster_rows = snapshot
                .clusters
                .iter()
                .map(|cluster| {
                    vec![
                        SqlArg::Text(run_id.clone()),
                        SqlArg::I64(cluster.cluster_id),
                        SqlArg::OptText(cluster.label_key.clone()),
                        SqlArg::Text(cluster.label.clone()),
                        SqlArg::Text(cluster.medoid.clone()),
                        SqlArg::I64(cluster.size),
                    ]
                })
                .collect();
            SqlRuntime::execute_batch_insert(
                tx,
                &insert_prefix("discovery_library_graph_clusters", CLUSTER_COLUMNS),
                CLUSTER_COLUMNS.len(),
                cluster_rows,
                "ON CONFLICT DO NOTHING",
            )
            .await?;

            // The new graph is in place; everything older for the facet goes,
            // including any failure recorded since the last complete build.
            delete_other_runs_tx(tx, &snapshot.run.facet, &run_id, None).await
        })
    })
    .await
}

async fn latest_run(
    datastore: &StoreDatastore,
    facet: &str,
    status: Option<&str>,
) -> AppResult<Option<LibraryGraphRunRecord>> {
    let mut args = vec![SqlArg::Text(facet.to_string())];
    let status_clause = match status {
        Some(status) => {
            args.push(SqlArg::Text(status.to_string()));
            " AND status = {}"
        }
        None => "",
    };
    let row = SqlRuntime::fetch_optional(
        datastore.read_exec(),
        &format!(
            "SELECT {} FROM discovery_library_graph_runs WHERE facet = {{}}{status_clause} \
             ORDER BY started_at DESC, id DESC LIMIT 1",
            RUN_COLUMNS.join(", ")
        ),
        &args,
    )
    .await?;
    row.as_ref().map(run_from_row).transpose()
}

pub(crate) async fn get_latest_library_graph_run(
    datastore: &StoreDatastore,
    facet: &str,
) -> AppResult<Option<LibraryGraphRunRecord>> {
    latest_run(datastore, facet, None).await
}

pub(crate) async fn get_latest_library_graph(
    datastore: &StoreDatastore,
    facet: &str,
) -> AppResult<Option<LibraryGraphSnapshotRecord>> {
    let Some(run) = latest_run(datastore, facet, Some(LIBRARY_GRAPH_RUN_COMPLETE)).await? else {
        return Ok(None);
    };
    let args = [SqlArg::Text(run.id.clone())];
    let nodes = SqlRuntime::fetch_all(
        datastore.read_exec(),
        "SELECT subject_key, target_key, cluster_id, title_ids_json \
         FROM discovery_library_graph_nodes WHERE run_id = {} ORDER BY subject_key",
        &args,
    )
    .await?
    .iter()
    .map(node_from_row)
    .collect::<AppResult<Vec<_>>>()?;
    let edges = SqlRuntime::fetch_all(
        datastore.read_exec(),
        "SELECT from_key, to_key, weight, kinds_json \
         FROM discovery_library_graph_edges WHERE run_id = {} ORDER BY from_key, to_key",
        &args,
    )
    .await?
    .iter()
    .map(edge_from_row)
    .collect::<AppResult<Vec<_>>>()?;
    let clusters = SqlRuntime::fetch_all(
        datastore.read_exec(),
        "SELECT cluster_id, label_key, label, medoid, size \
         FROM discovery_library_graph_clusters WHERE run_id = {} ORDER BY cluster_id",
        &args,
    )
    .await?
    .iter()
    .map(cluster_from_row)
    .collect::<AppResult<Vec<_>>>()?;
    Ok(Some(LibraryGraphSnapshotRecord {
        run,
        nodes,
        edges,
        clusters,
    }))
}

pub(crate) async fn prune_library_graph_title(
    datastore: &StoreDatastore,
    facet: &str,
    subject_key: &str,
    title_id: &str,
) -> AppResult<LibraryGraphPruneReport> {
    let facet: Arc<str> = Arc::from(facet);
    let subject_key: Arc<str> = Arc::from(subject_key);
    let title_id: Arc<str> = Arc::from(title_id);
    SqlRuntime::run_in_transaction(datastore, "prune_library_graph_title", move |tx| {
        let facet = Arc::clone(&facet);
        let subject_key = Arc::clone(&subject_key);
        let title_id = Arc::clone(&title_id);
        Box::pin(async move {
            let mut report = LibraryGraphPruneReport::default();
            let Some(run_row) = SqlRuntime::fetch_optional(
                SqlExec::Tx(tx),
                "SELECT id FROM discovery_library_graph_runs WHERE facet = {} AND status = {} \
                 ORDER BY started_at DESC, id DESC LIMIT 1",
                &[
                    SqlArg::Text(facet.to_string()),
                    SqlArg::Text(LIBRARY_GRAPH_RUN_COMPLETE.to_string()),
                ],
            )
            .await?
            else {
                return Ok(report);
            };
            let run_id = run_row.text("id")?;
            let Some(node_row) = SqlRuntime::fetch_optional(
                SqlExec::Tx(tx),
                "SELECT subject_key, target_key, cluster_id, title_ids_json \
                 FROM discovery_library_graph_nodes WHERE run_id = {} AND subject_key = {}",
                &[
                    SqlArg::Text(run_id.clone()),
                    SqlArg::Text(subject_key.to_string()),
                ],
            )
            .await?
            else {
                return Ok(report);
            };
            let node = node_from_row(&node_row)?;
            let remaining = node
                .title_ids
                .iter()
                .filter(|id| id.as_str() != title_id.as_ref())
                .cloned()
                .collect::<Vec<_>>();
            if !remaining.is_empty() {
                // Another owned title still stands for this subject.
                if remaining.len() != node.title_ids.len() {
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "UPDATE discovery_library_graph_nodes SET title_ids_json = {} \
                         WHERE run_id = {} AND subject_key = {}",
                        &[
                            SqlArg::Text(string_list_json(&remaining)?),
                            SqlArg::Text(run_id.clone()),
                            SqlArg::Text(node.subject_key.clone()),
                        ],
                    )
                    .await?;
                }
                return Ok(report);
            }

            report.removed_nodes = SqlRuntime::execute(
                SqlExec::Tx(tx),
                "DELETE FROM discovery_library_graph_nodes WHERE run_id = {} AND subject_key = {}",
                &[
                    SqlArg::Text(run_id.clone()),
                    SqlArg::Text(node.subject_key.clone()),
                ],
            )
            .await?;

            if node.cluster_id >= 0 {
                SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "UPDATE discovery_library_graph_clusters SET size = size - 1 \
                     WHERE run_id = {} AND cluster_id = {} AND size > 0",
                    &[SqlArg::Text(run_id.clone()), SqlArg::I64(node.cluster_id)],
                )
                .await?;
                SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "DELETE FROM discovery_library_graph_clusters \
                     WHERE run_id = {} AND cluster_id = {} AND size <= 0",
                    &[SqlArg::Text(run_id.clone()), SqlArg::I64(node.cluster_id)],
                )
                .await?;
            }

            // Edges join targets, not subjects: they stay while another node
            // still resolves to the same target.
            let target_still_present = SqlRuntime::fetch_optional(
                SqlExec::Tx(tx),
                "SELECT subject_key FROM discovery_library_graph_nodes \
                 WHERE run_id = {} AND target_key = {} LIMIT 1",
                &[
                    SqlArg::Text(run_id.clone()),
                    SqlArg::Text(node.target_key.clone()),
                ],
            )
            .await?
            .is_some();
            if !target_still_present {
                report.removed_edges = SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "DELETE FROM discovery_library_graph_edges \
                     WHERE run_id = {} AND (from_key = {} OR to_key = {})",
                    &[
                        SqlArg::Text(run_id.clone()),
                        SqlArg::Text(node.target_key.clone()),
                        SqlArg::Text(node.target_key.clone()),
                    ],
                )
                .await?;
            }
            Ok(report)
        })
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone, Utc};
    use scryer_application::DiscoveryRepository;
    use scryer_infrastructure_datastore::SqliteServices;

    use crate::discovery::store::DiscoveryStore;

    async fn sqlite_store(name: &str) -> (DiscoveryStore, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "scryer_library_graph_{name}_{}.db",
            uuid::Uuid::new_v4()
        ));
        let services = SqliteServices::new(path.to_string_lossy())
            .await
            .expect("sqlite services should initialize");
        (DiscoveryStore::new(services.datastore()), path)
    }

    fn run(id: &str, facet: &str, status: &str, minute: i64) -> LibraryGraphRunRecord {
        let at = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap() + Duration::minutes(minute);
        LibraryGraphRunRecord {
            id: id.to_string(),
            facet: facet.to_string(),
            axis: "SEMANTIC".to_string(),
            status: status.to_string(),
            subject_count: 6_000,
            submitted_count: 5_000,
            generated_at: Some("2026-01-01T00:00:00Z".to_string()),
            error_code: None,
            error_text: None,
            started_at: at,
            completed_at: Some(at),
            created_at: at,
            updated_at: at,
        }
    }

    fn node(subject: u32, cluster_id: i64, titles: &[&str]) -> LibraryGraphNodeRecord {
        LibraryGraphNodeRecord {
            subject_key: format!("tmdb:movie:{subject}"),
            target_key: format!("tmdb:movie:{subject}"),
            cluster_id,
            title_ids: titles.iter().map(|title| title.to_string()).collect(),
        }
    }

    fn edge(from: u32, to: u32, weight: f64) -> LibraryGraphEdgeRecord {
        LibraryGraphEdgeRecord {
            from_key: format!("tmdb:movie:{from}"),
            to_key: format!("tmdb:movie:{to}"),
            weight,
            kinds: vec!["semantic".to_string(), "relation:sequel".to_string()],
        }
    }

    fn snapshot(run_id: &str, minute: i64) -> LibraryGraphSnapshotRecord {
        LibraryGraphSnapshotRecord {
            run: run(run_id, "movie", LIBRARY_GRAPH_RUN_COMPLETE, minute),
            nodes: vec![
                node(1, 0, &["title-a"]),
                node(2, 0, &["title-b", "title-b2"]),
                node(3, 1, &["title-c"]),
                node(4, -1, &["title-d"]),
            ],
            edges: vec![edge(1, 2, 0.9), edge(1, 3, 0.6), edge(2, 3, 0.75)],
            clusters: vec![
                LibraryGraphClusterRecord {
                    cluster_id: 0,
                    label_key: Some("canonical:genre:heist".to_string()),
                    label: "Heist".to_string(),
                    medoid: "tmdb:movie:1".to_string(),
                    size: 2,
                },
                LibraryGraphClusterRecord {
                    cluster_id: 1,
                    label_key: None,
                    label: "Synthetic Title C".to_string(),
                    medoid: "tmdb:movie:3".to_string(),
                    size: 1,
                },
            ],
        }
    }

    #[tokio::test]
    async fn library_graph_round_trips_and_replaces_the_previous_run() {
        let (store, path) = sqlite_store("round_trip").await;
        let first = snapshot("library-graph-first", 0);
        store.commit_library_graph(&first).await.expect("commit");
        assert_eq!(
            store.get_latest_library_graph("movie").await.expect("read"),
            Some(first.clone())
        );
        assert_eq!(
            store.get_latest_library_graph("anime").await.expect("read"),
            None
        );

        // A failure keeps the complete graph and is the newest run.
        let mut failed = run("library-graph-failed", "movie", "failed", 5);
        failed.generated_at = None;
        failed.error_code = Some("LIBRARY_GRAPH_GATEWAY_QUEUE_FULL".to_string());
        failed.error_text = Some("at capacity".to_string());
        store
            .record_failed_library_graph_run(&failed)
            .await
            .expect("record failure");
        assert_eq!(
            store.get_latest_library_graph("movie").await.expect("read"),
            Some(first.clone())
        );
        assert_eq!(
            store
                .get_latest_library_graph_run("movie")
                .await
                .expect("read"),
            Some(failed)
        );

        // A new complete run replaces both.
        let mut second = snapshot("library-graph-second", 10);
        second.edges.truncate(1);
        store.commit_library_graph(&second).await.expect("commit");
        assert_eq!(
            store.get_latest_library_graph("movie").await.expect("read"),
            Some(second.clone())
        );
        assert_eq!(
            store
                .get_latest_library_graph_run("movie")
                .await
                .expect("read")
                .map(|run| run.id),
            Some("library-graph-second".to_string())
        );
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn incremental_change_updates_one_subjects_edges_without_touching_the_rest() {
        let (store, path) = sqlite_store("prune").await;
        let original = snapshot("library-graph-prune", 0);
        store.commit_library_graph(&original).await.expect("commit");

        // One of two titles behind a subject goes: the node stays.
        let report = store
            .prune_library_graph_title("movie", "tmdb:movie:2", "title-b2")
            .await
            .expect("prune");
        assert_eq!(report, LibraryGraphPruneReport::default());
        let after_first = store
            .get_latest_library_graph("movie")
            .await
            .expect("read")
            .expect("graph");
        assert_eq!(after_first.edges, original.edges);
        assert_eq!(after_first.nodes[1].title_ids, vec!["title-b".to_string()]);

        // The last title behind subject 1 goes: its node and edges go, its
        // cluster shrinks, everything else is untouched.
        let report = store
            .prune_library_graph_title("movie", "tmdb:movie:1", "title-a")
            .await
            .expect("prune");
        assert_eq!(report.removed_nodes, 1);
        assert_eq!(report.removed_edges, 2);
        let after = store
            .get_latest_library_graph("movie")
            .await
            .expect("read")
            .expect("graph");
        assert_eq!(after.run, original.run);
        assert_eq!(after.nodes, after_first.nodes[1..].to_vec());
        assert_eq!(after.edges, vec![original.edges[2].clone()]);
        assert_eq!(after.clusters[0].size, 1);
        assert_eq!(after.clusters[1], original.clusters[1]);

        // The last member of a cluster goes: the cluster goes with it.
        store
            .prune_library_graph_title("movie", "tmdb:movie:3", "title-c")
            .await
            .expect("prune");
        let after = store
            .get_latest_library_graph("movie")
            .await
            .expect("read")
            .expect("graph");
        assert_eq!(
            after
                .clusters
                .iter()
                .map(|cluster| cluster.cluster_id)
                .collect::<Vec<_>>(),
            vec![0]
        );
        assert!(after.edges.is_empty());

        // An unknown subject or facet is a no-op.
        assert_eq!(
            store
                .prune_library_graph_title("movie", "tmdb:movie:99", "title-z")
                .await
                .expect("prune"),
            LibraryGraphPruneReport::default()
        );
        assert_eq!(
            store
                .prune_library_graph_title("anime", "tmdb:movie:2", "title-b")
                .await
                .expect("prune"),
            LibraryGraphPruneReport::default()
        );
        let _ = std::fs::remove_file(path);
    }
}

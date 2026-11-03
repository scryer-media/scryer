//! Storage for the per-facet library graph: one run row per build, with the
//! build's nodes, edges and clusters keyed by the run id.

use scryer_application::{
    AppError, AppResult, LibraryGraphClusterRecord, LibraryGraphEdgeRecord, LibraryGraphNodeRecord,
    LibraryGraphRunRecord, LibraryGraphSnapshotRecord, error_reasons,
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
    "retry_after_seconds",
    "omitted_subject_keys_json",
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

fn run_args(run: &LibraryGraphRunRecord) -> AppResult<Vec<SqlArg>> {
    Ok(vec![
        SqlArg::Text(run.id.clone()),
        SqlArg::Text(run.facet.clone()),
        SqlArg::Text(run.axis.clone()),
        SqlArg::Text(run.status.clone()),
        SqlArg::I64(run.subject_count),
        SqlArg::I64(run.submitted_count),
        SqlArg::OptText(run.generated_at.clone()),
        SqlArg::OptText(run.error_code.clone()),
        SqlArg::OptText(run.error_text.clone()),
        SqlArg::OptI64(run.retry_after_seconds),
        SqlArg::Text(string_list_json(&run.omitted_subject_keys)?),
        SqlArg::Timestamp(run.started_at),
        SqlArg::OptTimestamp(run.completed_at),
        SqlArg::Timestamp(run.created_at),
        SqlArg::Timestamp(run.updated_at),
    ])
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
        retry_after_seconds: row.opt_i64("retry_after_seconds")?,
        omitted_subject_keys: string_list_from_json(&row.text("omitted_subject_keys_json")?)?,
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
        vec![run_args(run)?],
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
    read_latest_library_graph(datastore, facet, || async {}).await
}

pub(crate) async fn has_complete_library_graph(
    datastore: &StoreDatastore,
    facet: &str,
) -> AppResult<bool> {
    Ok(
        latest_run(datastore, facet, Some(LIBRARY_GRAPH_RUN_COMPLETE))
            .await?
            .is_some(),
    )
}

pub(crate) async fn delete_library_graph_runs(
    datastore: &StoreDatastore,
    facet: &str,
) -> AppResult<()> {
    let facet: Arc<str> = Arc::from(facet);
    SqlRuntime::run_in_transaction(datastore, "delete_library_graph_runs", move |tx| {
        let facet = Arc::clone(&facet);
        Box::pin(async move {
            let args = [SqlArg::Text(facet.to_string())];
            for table in CHILD_TABLES {
                SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    &format!(
                        "DELETE FROM {table} WHERE run_id IN \
                         (SELECT id FROM discovery_library_graph_runs WHERE facet = {{}})"
                    ),
                    &args,
                )
                .await?;
            }
            SqlRuntime::execute(
                SqlExec::Tx(tx),
                "DELETE FROM discovery_library_graph_runs WHERE facet = {}",
                &args,
            )
            .await?;
            Ok(())
        })
    })
    .await
}

/// Reads the newest complete graph of a facet without a transaction, so a
/// commit can replace the run between the run read and the child reads.
/// After the child reads the newest complete run id is read again; when it
/// changed, the whole read is repeated once. `between_reads` runs after each
/// attempt's child reads, which is where tests land a concurrent commit.
async fn read_latest_library_graph<F, Fut>(
    datastore: &StoreDatastore,
    facet: &str,
    between_reads: F,
) -> AppResult<Option<LibraryGraphSnapshotRecord>>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    const ATTEMPTS: usize = 2;
    for _ in 0..ATTEMPTS {
        let Some(run) = latest_run(datastore, facet, Some(LIBRARY_GRAPH_RUN_COMPLETE)).await?
        else {
            return Ok(None);
        };
        let snapshot = read_library_graph_children(datastore, run).await?;
        between_reads().await;
        let current = latest_run(datastore, facet, Some(LIBRARY_GRAPH_RUN_COMPLETE))
            .await?
            .map(|run| run.id);
        if current.as_deref() == Some(snapshot.run.id.as_str()) {
            return Ok(Some(snapshot));
        }
    }
    Err(AppError::temporary_unavailable_reason(
        error_reasons::library_graph::READ_SUPERSEDED,
        "the library graph was replaced while it was being read; try again",
        None,
    ))
}

async fn read_library_graph_children(
    datastore: &StoreDatastore,
    run: LibraryGraphRunRecord,
) -> AppResult<LibraryGraphSnapshotRecord> {
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
    Ok(LibraryGraphSnapshotRecord {
        run,
        nodes,
        edges,
        clusters,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone, Utc};
    use scryer_application::DiscoveryRepository;
    use scryer_infrastructure_datastore::SqliteServices;

    use crate::discovery::store::DiscoveryStore;

    async fn sqlite_store(name: &str) -> (DiscoveryStore, std::path::PathBuf) {
        let (store, _, path) = sqlite_store_and_datastore(name).await;
        (store, path)
    }

    async fn sqlite_store_and_datastore(
        name: &str,
    ) -> (DiscoveryStore, StoreDatastore, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "scryer_library_graph_{name}_{}.db",
            uuid::Uuid::new_v4()
        ));
        let services = SqliteServices::new(path.to_string_lossy())
            .await
            .expect("sqlite services should initialize");
        (
            DiscoveryStore::new(services.datastore()),
            services.datastore(),
            path,
        )
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
            retry_after_seconds: None,
            omitted_subject_keys: vec!["tmdb:movie:7".to_string(), "tmdb:movie:8".to_string()],
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
        failed.retry_after_seconds = Some(900);
        failed.omitted_subject_keys = Vec::new();
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
    async fn completeness_and_facet_deletion_follow_the_stored_runs() {
        let (store, path) = sqlite_store("delete").await;
        assert!(
            !store
                .has_complete_library_graph("movie")
                .await
                .expect("read")
        );
        let mut failed = run("library-graph-failed", "movie", "failed", 0);
        failed.generated_at = None;
        store
            .record_failed_library_graph_run(&failed)
            .await
            .expect("record failure");
        assert!(
            !store
                .has_complete_library_graph("movie")
                .await
                .expect("read")
        );
        store
            .commit_library_graph(&snapshot("library-graph-movie", 5))
            .await
            .expect("commit");
        let mut anime = snapshot("library-graph-anime", 5);
        anime.run.facet = "anime".to_string();
        store.commit_library_graph(&anime).await.expect("commit");
        assert!(
            store
                .has_complete_library_graph("movie")
                .await
                .expect("read")
        );

        store
            .delete_library_graph_runs("movie")
            .await
            .expect("delete");
        assert!(
            !store
                .has_complete_library_graph("movie")
                .await
                .expect("read")
        );
        assert_eq!(
            store
                .get_latest_library_graph_run("movie")
                .await
                .expect("read"),
            None
        );
        // Another facet's graph is untouched.
        assert_eq!(
            store.get_latest_library_graph("anime").await.expect("read"),
            Some(anime)
        );
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn a_graph_replaced_mid_read_is_read_again_from_the_new_run() {
        let (store, datastore, path) = sqlite_store_and_datastore("torn_read").await;
        let first = snapshot("library-graph-first", 0);
        store.commit_library_graph(&first).await.expect("commit");
        let mut second = snapshot("library-graph-second", 10);
        second.edges.truncate(1);

        // The first attempt's child reads land, then a commit replaces the
        // run: the read starts over and returns the new graph whole.
        let committed = std::sync::atomic::AtomicBool::new(false);
        let read = read_latest_library_graph(&datastore, "movie", || async {
            if !committed.swap(true, std::sync::atomic::Ordering::SeqCst) {
                store.commit_library_graph(&second).await.expect("commit");
            }
        })
        .await
        .expect("read");
        assert_eq!(read, Some(second.clone()));

        // Replaced on every attempt: a coded refusal, never a torn graph.
        let minute = std::sync::atomic::AtomicI64::new(20);
        let error = read_latest_library_graph(&datastore, "movie", || async {
            let at = minute.fetch_add(10, std::sync::atomic::Ordering::SeqCst);
            store
                .commit_library_graph(&snapshot(&format!("library-graph-{at}"), at))
                .await
                .expect("commit");
        })
        .await
        .expect_err("a graph replaced on every attempt cannot be read");
        assert_eq!(
            error.reason(),
            Some(error_reasons::library_graph::READ_SUPERSEDED)
        );
        let _ = std::fs::remove_file(path);
    }
}

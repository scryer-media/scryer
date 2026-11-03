//! Storage for the per-facet library graph: one run row per build, with the
//! build's nodes, edges and clusters in one compressed payload keyed by the
//! run id (see [`super::library_graph_payload`]).
//!
//! A facet keeps at most two runs: its newest complete run and, after it, the
//! newest failed one. Committing a graph removes every other run of the facet
//! together with its payload.

use scryer_application::{AppError, AppResult, LibraryGraphRunRecord, LibraryGraphSnapshotRecord};
use std::sync::Arc;

use super::library_graph_payload::{decode_library_graph_payload, encode_library_graph_payload};
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

/// Deletes the runs of `facet` that `run_filter` selects (a SQL condition on
/// the run row with its own placeholders, bound from `filter_args`), payloads
/// first.
async fn delete_runs_tx(
    tx: &mut SqlTx<'_>,
    facet: &str,
    run_filter: &str,
    filter_args: Vec<SqlArg>,
) -> AppResult<()> {
    let mut args = vec![SqlArg::Text(facet.to_string())];
    args.extend(filter_args);
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        &format!(
            "DELETE FROM discovery_library_graph_payloads WHERE run_id IN \
             (SELECT id FROM discovery_library_graph_runs WHERE facet = {{}}{run_filter})"
        ),
        &args,
    )
    .await?;
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        &format!("DELETE FROM discovery_library_graph_runs WHERE facet = {{}}{run_filter}"),
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
            // complete graph; a rewrite of this run replaces it.
            delete_runs_tx(
                tx,
                &run.facet,
                " AND (status = {} OR id = {})",
                vec![
                    SqlArg::Text(run.status.clone()),
                    SqlArg::Text(run.id.clone()),
                ],
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
    let payload =
        encode_library_graph_payload(&snapshot.nodes, &snapshot.edges, &snapshot.clusters)?;
    let run = Arc::new(snapshot.run.clone());
    let payload = Arc::new(payload);
    SqlRuntime::run_in_transaction(datastore, "commit_library_graph", move |tx| {
        let run = Arc::clone(&run);
        let payload = Arc::clone(&payload);
        Box::pin(async move {
            // The new graph replaces everything older for the facet,
            // including any failure recorded since the last complete build.
            delete_runs_tx(tx, &run.facet, "", Vec::new()).await?;
            insert_run_tx(tx, &run).await?;
            SqlRuntime::execute(
                SqlExec::Tx(tx),
                "INSERT INTO discovery_library_graph_payloads (run_id, payload) VALUES ({}, {})",
                &[
                    SqlArg::Text(run.id.clone()),
                    SqlArg::OptBytes(Some(payload.as_ref().clone())),
                ],
            )
            .await?;
            Ok(())
        })
    })
    .await
}

pub(crate) async fn get_latest_library_graph_run(
    datastore: &StoreDatastore,
    facet: &str,
) -> AppResult<Option<LibraryGraphRunRecord>> {
    let row = SqlRuntime::fetch_optional(
        datastore.read_exec(),
        &format!(
            "SELECT {} FROM discovery_library_graph_runs WHERE facet = {{}} \
             ORDER BY started_at DESC, id DESC LIMIT 1",
            RUN_COLUMNS.join(", ")
        ),
        &[SqlArg::Text(facet.to_string())],
    )
    .await?;
    row.as_ref().map(run_from_row).transpose()
}

/// The newest complete graph of a facet. The run row and its payload are read
/// in one statement, so a commit landing alongside cannot pair a run with
/// another run's graph.
pub(crate) async fn get_latest_library_graph(
    datastore: &StoreDatastore,
    facet: &str,
) -> AppResult<Option<LibraryGraphSnapshotRecord>> {
    let columns = RUN_COLUMNS
        .iter()
        .map(|column| format!("r.{column}"))
        .collect::<Vec<_>>()
        .join(", ");
    let row = SqlRuntime::fetch_optional(
        datastore.read_exec(),
        &format!(
            "SELECT {columns}, p.payload FROM discovery_library_graph_runs r \
             JOIN discovery_library_graph_payloads p ON p.run_id = r.id \
             WHERE r.facet = {{}} AND r.status = {{}} \
             ORDER BY r.started_at DESC, r.id DESC LIMIT 1"
        ),
        &[
            SqlArg::Text(facet.to_string()),
            SqlArg::Text(LIBRARY_GRAPH_RUN_COMPLETE.to_string()),
        ],
    )
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let run = run_from_row(&row)?;
    let payload = row
        .opt_bytes("payload")?
        .ok_or_else(|| AppError::Repository("library graph payload is missing".to_string()))?;
    let payload = decode_library_graph_payload(&payload)?;
    Ok(Some(LibraryGraphSnapshotRecord {
        run,
        nodes: payload.nodes,
        edges: payload.edges,
        clusters: payload.clusters,
    }))
}

/// Whether the facet has a complete graph stored: a complete run together
/// with its payload.
pub(crate) async fn has_complete_library_graph(
    datastore: &StoreDatastore,
    facet: &str,
) -> AppResult<bool> {
    let row = SqlRuntime::fetch_optional(
        datastore.read_exec(),
        "SELECT r.id FROM discovery_library_graph_runs r \
         JOIN discovery_library_graph_payloads p ON p.run_id = r.id \
         WHERE r.facet = {} AND r.status = {} LIMIT 1",
        &[
            SqlArg::Text(facet.to_string()),
            SqlArg::Text(LIBRARY_GRAPH_RUN_COMPLETE.to_string()),
        ],
    )
    .await?;
    Ok(row.is_some())
}

pub(crate) async fn delete_library_graph_runs(
    datastore: &StoreDatastore,
    facet: &str,
) -> AppResult<()> {
    let facet: Arc<str> = Arc::from(facet);
    SqlRuntime::run_in_transaction(datastore, "delete_library_graph_runs", move |tx| {
        let facet = Arc::clone(&facet);
        Box::pin(async move { delete_runs_tx(tx, &facet, "", Vec::new()).await })
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone, Utc};
    use scryer_application::{
        DiscoveryRepository, LibraryGraphClusterRecord, LibraryGraphEdgeRecord,
        LibraryGraphNodeRecord,
    };
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

    async fn count(datastore: &StoreDatastore, sql: &str) -> i64 {
        SqlRuntime::fetch_optional(datastore.read_exec(), sql, &[])
            .await
            .expect("count")
            .expect("row")
            .i64("n")
            .expect("n")
    }

    #[tokio::test]
    async fn a_facet_keeps_only_its_newest_graph_and_newest_failure() {
        let (store, datastore, path) = sqlite_store_and_datastore("retention").await;
        for (index, minute) in [0, 10, 20].into_iter().enumerate() {
            store
                .commit_library_graph(&snapshot(&format!("library-graph-{index}"), minute))
                .await
                .expect("commit");
            let mut failed = run(
                &format!("library-graph-failed-{index}"),
                "movie",
                "failed",
                minute + 5,
            );
            failed.generated_at = None;
            store
                .record_failed_library_graph_run(&failed)
                .await
                .expect("record failure");
            // Recording the same failure again replaces it.
            store
                .record_failed_library_graph_run(&failed)
                .await
                .expect("record failure again");
        }
        assert_eq!(
            count(
                &datastore,
                "SELECT COUNT(*) AS n FROM discovery_library_graph_runs"
            )
            .await,
            2
        );
        assert_eq!(
            count(
                &datastore,
                "SELECT COUNT(*) AS n FROM discovery_library_graph_payloads"
            )
            .await,
            1
        );
        assert_eq!(
            store
                .get_latest_library_graph("movie")
                .await
                .expect("read")
                .map(|graph| graph.run.id),
            Some("library-graph-2".to_string())
        );
        store
            .delete_library_graph_runs("movie")
            .await
            .expect("delete");
        assert_eq!(
            count(
                &datastore,
                "SELECT COUNT(*) AS n FROM discovery_library_graph_payloads"
            )
            .await,
            0
        );
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn a_complete_run_without_its_graph_reads_as_no_graph_so_it_is_rebuilt() {
        let (store, datastore, path) = sqlite_store_and_datastore("no_payload").await;
        let orphan = run(
            "library-graph-orphan",
            "movie",
            LIBRARY_GRAPH_RUN_COMPLETE,
            0,
        );
        let orphan = Arc::new(orphan);
        SqlRuntime::run_in_transaction(&datastore, "seed_orphan_run", move |tx| {
            let orphan = Arc::clone(&orphan);
            Box::pin(async move { insert_run_tx(tx, &orphan).await })
        })
        .await
        .expect("seed run");
        assert_eq!(
            store.get_latest_library_graph("movie").await.expect("read"),
            None
        );
        assert!(
            !store
                .has_complete_library_graph("movie")
                .await
                .expect("read")
        );
        let _ = std::fs::remove_file(path);
    }

    /// Deterministic synthetic graph of `subjects` nodes, each linked to
    /// `neighbors` others, with kind mixes shaped like the gateway's.
    fn synthetic_large_graph(
        run_id: &str,
        subjects: u32,
        neighbors: u32,
    ) -> LibraryGraphSnapshotRecord {
        let mut state: u64 = 0x5eed_1234_abcd_ef01;
        let mut next = move || {
            state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^ (z >> 31)
        };
        let key = |n: u32| format!("tmdb:movie:{}", 100_000 + n * 37);
        let structural = (0..48u128)
            .map(|i| format!("structure:{}", uuid::Uuid::from_u128((1 << 120) + i)))
            .collect::<Vec<_>>();
        let themes = (0..64)
            .map(|i| format!("affinity:theme:synthetic-theme-{i}"))
            .collect::<Vec<_>>();
        let nodes = (0..subjects)
            .map(|n| LibraryGraphNodeRecord {
                subject_key: key(n),
                target_key: key(n),
                cluster_id: if n % 23 == 0 { -1 } else { i64::from(n % 180) },
                title_ids: vec![uuid::Uuid::from_u128(u128::from(n) * 7919 + 1).to_string()],
            })
            .collect::<Vec<_>>();
        let mut seen = std::collections::HashSet::new();
        let mut edges = Vec::new();
        for from in 0..subjects {
            for _ in 0..neighbors {
                let to = (next() % u64::from(subjects)) as u32;
                if to == from {
                    continue;
                }
                let (a, b) = (key(from), key(to));
                let (a, b) = if a <= b { (a, b) } else { (b, a) };
                if !seen.insert((a.clone(), b.clone())) {
                    continue;
                }
                let roll = next() % 100;
                let mut kinds = vec!["semantic".to_string()];
                if roll < 20 {
                    kinds.push("relation:list".to_string());
                }
                if roll % 3 == 0 {
                    kinds.push(themes[(next() % 64) as usize].clone());
                }
                if roll % 7 == 0 {
                    kinds.push("relation:recommendation".to_string());
                }
                if roll % 11 == 0 {
                    kinds.push(structural[(next() % 48) as usize].clone());
                }
                let weight = 0.55 + (next() % 1_000_000_000) as f64 / 1_000_000_000.0 * 0.45;
                edges.push(LibraryGraphEdgeRecord {
                    from_key: a,
                    to_key: b,
                    weight,
                    kinds,
                });
            }
        }
        let clusters = (0..180)
            .map(|c| LibraryGraphClusterRecord {
                cluster_id: c,
                label_key: (c % 2 == 0).then(|| format!("canonical:genre:synthetic-{c}")),
                label: format!("Synthetic Cluster {c}"),
                medoid: key((c as u32 + 1) % subjects),
                size: 50,
            })
            .collect();
        let mut graph = snapshot(run_id, 0);
        graph.nodes = nodes;
        graph.edges = edges;
        graph.clusters = clusters;
        graph
    }

    #[tokio::test]
    async fn a_full_size_graph_round_trips_in_key_order_within_a_small_payload() {
        let (store, datastore, path) = sqlite_store_and_datastore("full_size").await;
        let graph = synthetic_large_graph("library-graph-full-size", 10_000, 8);
        store.commit_library_graph(&graph).await.expect("commit");

        let mut expected = graph.clone();
        expected
            .nodes
            .sort_by(|left, right| left.subject_key.cmp(&right.subject_key));
        expected.edges.sort_by(|left, right| {
            (&left.from_key, &left.to_key).cmp(&(&right.from_key, &right.to_key))
        });
        assert_eq!(
            store.get_latest_library_graph("movie").await.expect("read"),
            Some(expected)
        );
        let payload_bytes = count(
            &datastore,
            "SELECT LENGTH(payload) AS n FROM discovery_library_graph_payloads",
        )
        .await;
        assert!(
            payload_bytes < 2 * 1024 * 1024,
            "a 10,000-subject graph should store in under 2 MiB, took {payload_bytes} bytes"
        );
        let _ = std::fs::remove_file(path);
    }
}

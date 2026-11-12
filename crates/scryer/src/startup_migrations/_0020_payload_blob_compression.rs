//! Compresses the plans and media analyses that schema migration 0338 copied
//! into their blob tables as plain UTF-8 bytes.
//!
//! It runs in the background after startup, one bounded batch at a time: each
//! batch is one read of candidate sizes, one read of their payloads, the
//! compression off the async runtime, and one compare-and-swap write. The
//! scan pages by primary key from a cursor, so no row is read twice in a pass.
//! A row a normal write replaced in the meantime no longer holds the bytes
//! the pass read, so its swap matches nothing and the newer value stays.
//! Every batch commits on its own, so an interrupted pass loses nothing and
//! the next start continues it. The runner records completion once a whole
//! pass has run; after that the pass is never dispatched again.

use std::future::Future;
use std::ops::ControlFlow;

use scryer_infrastructure_sql::compressed_blob::{
    LOCATION_PLAN_MAX_BYTES, MEDIA_ANALYSIS_MAX_BYTES, ZSTD_FRAME_MAGIC, compress_text,
    is_compressed_text,
};
use scryer_infrastructure_sql::runtime::{SqlArg, SqlRuntime, StoreDatastore};

pub(crate) const ID: &str = "0020_payload_blob_compression";
pub(crate) const DESCRIPTION: &str = "compress plans and media analyses stored as plain bytes";

struct PayloadTable {
    name: &'static str,
    key: &'static str,
    payload: &'static str,
    max_bytes: usize,
}

const TABLES: [PayloadTable; 2] = [
    PayloadTable {
        name: "location_operation_plans",
        key: "operation_id",
        payload: "plan",
        max_bytes: LOCATION_PLAN_MAX_BYTES,
    },
    PayloadTable {
        name: "media_file_analyses",
        key: "media_file_id",
        payload: "analysis",
        max_bytes: MEDIA_ANALYSIS_MAX_BYTES,
    },
];

/// How much one batch may hold. A batch always takes at least one row, so a
/// plan larger than the byte budget goes alone.
#[derive(Clone, Copy, Debug)]
pub(crate) struct BatchLimits {
    pub(crate) rows: i64,
    pub(crate) bytes: i64,
}

pub(crate) const DEFAULT_LIMITS: BatchLimits = BatchLimits {
    rows: 64,
    bytes: 4 * 1024 * 1024,
};

/// Points inside a batch where a caller may observe or stop the pass.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Stage {
    /// The batch is read and compressed, and its write has not started.
    BeforeWrite,
    /// The batch's write has committed.
    AfterBatch,
}

pub(crate) async fn migrate(datastore: &StoreDatastore) -> Result<(), String> {
    run(datastore, DEFAULT_LIMITS, |_| async {
        ControlFlow::Continue(())
    })
    .await
    .map(|_| ())
}

/// One pass over both tables. Returns `true` when the pass reached the end of
/// both, and `false` when `hook` stopped it early.
pub(crate) async fn run<H, F>(
    datastore: &StoreDatastore,
    limits: BatchLimits,
    mut hook: H,
) -> Result<bool, String>
where
    H: FnMut(Stage) -> F,
    F: Future<Output = ControlFlow<()>>,
{
    let magic = SqlArg::OptBytes(Some(ZSTD_FRAME_MAGIC.to_vec()));
    for table in &TABLES {
        let PayloadTable {
            name,
            key,
            payload,
            max_bytes,
        } = *table;
        let candidates_sql = format!(
            "SELECT {key} AS payload_key, length({payload}) AS payload_size FROM {name}
              WHERE {key} > {{}} AND substr({payload}, 1, 4) <> {{}}
              ORDER BY {key} LIMIT {{}}"
        );
        let payloads_sql = format!(
            "SELECT {key} AS payload_key, {payload} AS payload FROM {name}
              WHERE {key} > {{}} AND {key} <= {{}} AND substr({payload}, 1, 4) <> {{}}
              ORDER BY {key}"
        );
        let mut cursor = String::new();
        loop {
            let candidates = SqlRuntime::fetch_all(
                datastore.read_exec(),
                &candidates_sql,
                &[
                    SqlArg::Text(cursor.clone()),
                    magic.clone(),
                    SqlArg::I64(limits.rows.max(1)),
                ],
            )
            .await
            .map_err(|error| error.to_string())?;
            if candidates.is_empty() {
                break;
            }
            let mut last = None;
            let mut batch_bytes = 0_i64;
            for row in &candidates {
                let size = row.i64("payload_size").map_err(|error| error.to_string())?;
                if last.is_some() && batch_bytes.saturating_add(size) > limits.bytes {
                    break;
                }
                batch_bytes = batch_bytes.saturating_add(size);
                last = Some(row.text("payload_key").map_err(|error| error.to_string())?);
            }
            let last = last.expect("a batch takes at least one row");

            let rows = SqlRuntime::fetch_all(
                datastore.read_exec(),
                &payloads_sql,
                &[
                    SqlArg::Text(cursor.clone()),
                    SqlArg::Text(last.clone()),
                    magic.clone(),
                ],
            )
            .await
            .map_err(|error| error.to_string())?
            .into_iter()
            .map(|row| {
                Ok((
                    row.text("payload_key")?,
                    row.opt_bytes("payload")?.unwrap_or_default(),
                ))
            })
            .collect::<scryer_application::AppResult<Vec<(String, Vec<u8>)>>>()
            .map_err(|error| error.to_string())?;
            cursor = last;

            let swaps = tokio::task::spawn_blocking(move || compress_rows(rows, max_bytes))
                .await
                .map_err(|error| error.to_string())?;

            if hook(Stage::BeforeWrite).await.is_break() {
                return Ok(false);
            }
            if !swaps.is_empty() {
                write_swaps(datastore, name, key, payload, swaps).await?;
            }
            if hook(Stage::AfterBatch).await.is_break() {
                return Ok(false);
            }
            // Let the writers queued behind this batch go first.
            tokio::task::yield_now().await;
        }
    }
    Ok(true)
}

/// The rows to swap: each key with the bytes it was read with and their
/// compressed form. A row that is not valid UTF-8, or is larger than its
/// readers decode once compressed, stays as it is; readers accept plain
/// bytes of any size.
fn compress_rows(
    rows: Vec<(String, Vec<u8>)>,
    max_bytes: usize,
) -> Vec<(String, Vec<u8>, Vec<u8>)> {
    rows.into_iter()
        .filter_map(|(key, stored)| {
            if is_compressed_text(&stored) || stored.len() > max_bytes {
                return None;
            }
            let compressed = compress_text(std::str::from_utf8(&stored).ok()?, max_bytes).ok()?;
            Some((key, stored, compressed))
        })
        .collect()
}

/// One statement for the batch, swapping each row only while it still holds
/// the bytes the pass read.
async fn write_swaps(
    datastore: &StoreDatastore,
    name: &str,
    key: &str,
    payload: &str,
    swaps: Vec<(String, Vec<u8>, Vec<u8>)>,
) -> Result<(), String> {
    let mut sql = format!("UPDATE {name} SET {payload} = CASE {key}");
    let mut args = Vec::with_capacity(swaps.len() * 4);
    for (row_key, _, compressed) in &swaps {
        sql.push_str(" WHEN {} THEN {}");
        args.push(SqlArg::Text(row_key.clone()));
        args.push(SqlArg::OptBytes(Some(compressed.clone())));
    }
    sql.push_str(&format!(" ELSE {payload} END WHERE "));
    for (index, (row_key, stored, _)) in swaps.into_iter().enumerate() {
        if index > 0 {
            sql.push_str(" OR ");
        }
        sql.push_str(&format!("({key} = {{}} AND {payload} = {{}})"));
        args.push(SqlArg::Text(row_key));
        args.push(SqlArg::OptBytes(Some(stored)));
    }
    SqlRuntime::execute_write(datastore, "compress_payload_blobs", &sql, args)
        .await
        .map(|_| ())
        .map_err(|error| error.to_string())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use scryer_infrastructure_datastore::{MigrationMode, SqliteServices};
    use scryer_infrastructure_sql::compressed_blob::decompress_text;

    pub(crate) async fn seeded() -> (tempfile::TempDir, SqliteServices, Vec<(String, String)>) {
        let temp = tempfile::tempdir().expect("tempdir");
        let services = SqliteServices::new_with_mode(
            temp.path().join("scryer.db").to_string_lossy().to_string(),
            MigrationMode::Apply,
        )
        .await
        .expect("sqlite services");
        let pool = services.pool().clone();
        let library_id =
            scryer_domain::default_library_id_for_facet(&scryer_domain::MediaFacet::Series);
        let (root_folder_id,): (String,) =
            sqlx::query_as("SELECT id FROM library_roots WHERE library_id = ?1 LIMIT 1")
                .bind(&library_id)
                .fetch_one(&pool)
                .await
                .expect("default series root");
        sqlx::query(
            "INSERT INTO titles (id, name, name_normalized, library_id, root_folder_id, facet, created_at)
             VALUES ('blob-title', 'Blob Fixture', 'blob fixture', ?1, ?2, 'series', '2026-01-01T00:00:00Z')",
        )
        .bind(&library_id)
        .bind(&root_folder_id)
        .execute(&pool)
        .await
        .expect("title fixture");

        let mut expected = Vec::new();
        for index in 0..5 {
            let id = format!("op-{index}");
            let plan = format!(
                r#"{{"titles":["synthetic-{index}"],"pad":"{}"}}"#,
                "x".repeat(index * 100)
            );
            sqlx::query(
                "INSERT INTO location_operations (id, operation_type, execution_mode)
                 VALUES (?1, 'root_move', 'catalog_only')",
            )
            .bind(&id)
            .execute(&pool)
            .await
            .expect("operation fixture");
            // Every other plan is already a frame, as a new write leaves it.
            let stored = if index % 2 == 0 {
                plan.clone().into_bytes()
            } else {
                compress_text(&plan, LOCATION_PLAN_MAX_BYTES).unwrap()
            };
            sqlx::query(
                "INSERT INTO location_operation_plans (operation_id, plan) VALUES (?1, ?2)",
            )
            .bind(&id)
            .bind(stored)
            .execute(&pool)
            .await
            .expect("plan fixture");
            expected.push((format!("plan:{id}"), plan));
        }
        for index in 0..4 {
            let id = format!("media-{index}");
            let analysis = format!(r#"{{"audio_languages":["synthetic-{index}"]}}"#);
            sqlx::query(
                "INSERT INTO media_files (id, title_id, file_path, size_bytes, scan_status, created_at)
                 VALUES (?1, 'blob-title', '/synthetic/' || ?1 || '.mkv', 100, 'complete', '2026-01-01T00:00:00Z')",
            )
            .bind(&id)
            .execute(&pool)
            .await
            .expect("media file fixture");
            let stored = if index % 2 == 1 {
                analysis.clone().into_bytes()
            } else {
                compress_text(&analysis, MEDIA_ANALYSIS_MAX_BYTES).unwrap()
            };
            sqlx::query(
                "INSERT INTO media_file_analyses (media_file_id, analysis) VALUES (?1, ?2)",
            )
            .bind(&id)
            .bind(stored)
            .execute(&pool)
            .await
            .expect("analysis fixture");
            expected.push((format!("analysis:{id}"), analysis));
        }
        expected.sort();
        (temp, services, expected)
    }

    /// Every stored payload, decoded, with whether it is a frame.
    pub(crate) async fn stored(services: &SqliteServices) -> (Vec<(String, String)>, usize) {
        let rows: Vec<(String, Vec<u8>)> = sqlx::query_as(
            "SELECT 'plan:' || operation_id, plan FROM location_operation_plans
             UNION ALL
             SELECT 'analysis:' || media_file_id, analysis FROM media_file_analyses
             ORDER BY 1",
        )
        .fetch_all(services.pool())
        .await
        .expect("read payloads");
        let plain = rows
            .iter()
            .filter(|(_, bytes)| !is_compressed_text(bytes))
            .count();
        let decoded = rows
            .into_iter()
            .map(|(key, bytes)| {
                (
                    key,
                    decompress_text(&bytes, LOCATION_PLAN_MAX_BYTES).expect("decodes"),
                )
            })
            .collect();
        (decoded, plain)
    }

    fn continue_always(_: Stage) -> std::future::Ready<ControlFlow<()>> {
        std::future::ready(ControlFlow::Continue(()))
    }

    #[tokio::test]
    async fn a_pass_compresses_every_plain_row_and_keeps_every_value() {
        let (_temp, services, expected) = seeded().await;
        assert_eq!(stored(&services).await.1, 5);
        let finished = run(
            &services.datastore(),
            BatchLimits {
                rows: 2,
                bytes: 150,
            },
            continue_always,
        )
        .await
        .expect("pass");
        assert!(finished);
        assert_eq!(stored(&services).await, (expected, 0));
    }

    #[tokio::test]
    async fn an_interrupted_pass_keeps_its_batches_and_the_next_pass_finishes() {
        let (_temp, services, expected) = seeded().await;
        let mut batches = 0;
        let finished = run(
            &services.datastore(),
            BatchLimits { rows: 1, bytes: 1 },
            |stage| {
                if stage == Stage::AfterBatch {
                    batches += 1;
                }
                std::future::ready(if batches == 2 {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                })
            },
        )
        .await
        .expect("interrupted pass");
        assert!(!finished);
        let (values, plain) = stored(&services).await;
        assert_eq!(values, expected);
        assert_eq!(plain, 3, "two one-row batches committed");

        let finished = run(&services.datastore(), DEFAULT_LIMITS, continue_always)
            .await
            .expect("resumed pass");
        assert!(finished);
        assert_eq!(stored(&services).await, (expected, 0));
    }

    #[tokio::test]
    async fn a_row_rewritten_during_a_batch_keeps_the_newer_value() {
        let (_temp, services, expected) = seeded().await;
        let pool = services.pool().clone();
        let newer = r#"{"titles":["synthetic-newer"]}"#;
        let mut rewritten = false;
        let finished = run(&services.datastore(), DEFAULT_LIMITS, |stage| {
            let pool = pool.clone();
            let first = stage == Stage::BeforeWrite && !rewritten;
            if first {
                rewritten = true;
            }
            async move {
                if first {
                    // A normal write lands between the batch's read and its swap.
                    sqlx::query(
                        "UPDATE location_operation_plans SET plan = ?1 WHERE operation_id = 'op-0'",
                    )
                    .bind(newer.as_bytes())
                    .execute(&pool)
                    .await
                    .expect("concurrent write");
                }
                ControlFlow::Continue(())
            }
        })
        .await
        .expect("pass");
        assert!(finished);

        let (values, plain) = stored(&services).await;
        let expected: Vec<_> = expected
            .into_iter()
            .map(|(key, value)| {
                if key == "plan:op-0" {
                    (key, newer.to_string())
                } else {
                    (key, value)
                }
            })
            .collect();
        assert_eq!(values, expected);
        assert_eq!(
            plain, 1,
            "only the concurrently written row is left as written"
        );
        let op_0: Vec<u8> = sqlx::query_scalar(
            "SELECT plan FROM location_operation_plans WHERE operation_id = 'op-0'",
        )
        .fetch_one(services.pool())
        .await
        .unwrap();
        assert_eq!(op_0, newer.as_bytes());
    }

    #[test]
    fn rows_over_the_bound_or_not_utf8_stay_plain() {
        let rows = vec![
            ("big".to_string(), b"0123456789".to_vec()),
            ("binary".to_string(), vec![0xFF, 0xFE]),
            ("small".to_string(), b"{}".to_vec()),
        ];
        let swaps = compress_rows(rows, 4);
        assert_eq!(swaps.len(), 1);
        assert_eq!(swaps[0].0, "small");
        assert!(is_compressed_text(&swaps[0].2));
    }
}

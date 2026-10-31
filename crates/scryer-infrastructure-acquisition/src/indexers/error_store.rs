use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use chrono::{DateTime, Utc};
use scryer_application::{
    AppError, AppResult, CapturedIndexerHttpHeader, CapturedIndexerHttpResponse,
    IndexerErrorClassification, IndexerErrorDetail, IndexerErrorOperation, IndexerErrorPage,
    IndexerErrorRecorder, IndexerErrorRepository, IndexerErrorSummary, NewIndexerError,
    redact_indexer_response_headers,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::queries::sql_runtime::{SqlArg, SqlRow, SqlRuntime, StoreDatastore};

const PAYLOAD_FORMAT_VERSION: i32 = 2;
// The original schema requires a status. Zero cannot be an HTTP status and
// represents a transport error with no response.
const TRANSPORT_ERROR_HTTP_STATUS: u16 = 0;

#[derive(Clone)]
pub struct IndexerErrorStore {
    datastore: StoreDatastore,
}

impl IndexerErrorStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }
}

/// Error entries waiting for the persister beyond this many are dropped, and
/// counted, rather than holding up the indexer call that saw them.
const PERSIST_QUEUE_CAPACITY: usize = 256;

/// Records indexer HTTP errors without making the caller wait for storage.
///
/// The plugin host calls `record` synchronously from inside an indexer call,
/// often in a task that also polls other work, such as the other titles of an
/// acquisition walk. Waiting there for the SQLite writer gate would stop that
/// task, including a sibling future that holds the gate or is next in line
/// for it, and so freeze every writer in the process. `record` therefore only
/// counts the error and queues it; one persister task owned by the recorder
/// writes the queue in order.
#[derive(Clone)]
pub struct QueuedIndexerErrorRecorder {
    queue: tokio::sync::mpsc::Sender<NewIndexerError>,
}

impl QueuedIndexerErrorRecorder {
    /// Spawns the persister on the current Tokio runtime, so this must be
    /// called from within one. `record` may then be called from any thread,
    /// inside or outside that runtime. The persister stops once every clone
    /// of the recorder is dropped and the queue is drained.
    pub fn new(repository: Arc<dyn IndexerErrorRepository>) -> Self {
        Self::with_capacity(repository, PERSIST_QUEUE_CAPACITY)
    }

    fn with_capacity(repository: Arc<dyn IndexerErrorRepository>, capacity: usize) -> Self {
        let (queue, mut pending) = tokio::sync::mpsc::channel::<NewIndexerError>(capacity);
        tokio::spawn(async move {
            while let Some(error) = pending.recv().await {
                let indexer_id = error.indexer_id.clone();
                if let Err(error) = repository.record(error).await {
                    tracing::warn!(
                        indexer_id = indexer_id.as_str(),
                        error = %error,
                        "failed to persist indexer HTTP error history"
                    );
                }
            }
        });
        Self { queue }
    }
}

impl IndexerErrorRecorder for QueuedIndexerErrorRecorder {
    fn record(&self, error: NewIndexerError) -> AppResult<()> {
        // Counted before persistence, and regardless of whether persistence
        // succeeds: the classification is the operator-facing signal, and a
        // storage failure must not also erase the evidence that the indexer
        // failed. Every label value is a name or a bounded enum string.
        metrics::counter!(
            "scryer_indexer_errors_total",
            "indexer" => error.indexer_name.clone(),
            "operation" => error.operation.as_str(),
            "class" => error.classification.as_str()
        )
        .increment(1);
        match self.queue.try_send(error) {
            Ok(()) => Ok(()),
            Err(tokio::sync::mpsc::error::TrySendError::Full(error)) => {
                // A burst larger than the queue loses history rows, never the
                // caller's progress: the counter above already holds the
                // signal, and this one shows how much history went missing.
                tracing::warn!(
                    indexer = error.indexer_name.as_str(),
                    operation = error.operation.as_str(),
                    "indexer HTTP error history queue is full; dropping this entry"
                );
                metrics::counter!(
                    "scryer_indexer_errors_dropped_total",
                    "indexer" => error.indexer_name,
                    "operation" => error.operation.as_str(),
                    "class" => error.classification.as_str()
                )
                .increment(1);
                Ok(())
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => Err(AppError::Repository(
                "indexer error history persister has stopped".to_string(),
            )),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredResponseEnvelope {
    version: i32,
    status: Option<u16>,
    headers: Vec<StoredResponseHeader>,
    body_base64: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredResponseHeader {
    name: String,
    value_base64: String,
}

#[async_trait]
impl IndexerErrorRepository for IndexerErrorStore {
    async fn record(&self, error: NewIndexerError) -> AppResult<()> {
        let http_status = error
            .response
            .as_ref()
            .map_or(TRANSPORT_ERROR_HTTP_STATUS, |response| response.status);
        let payload = encode_response(error.response).await?;
        SqlRuntime::execute_write(
            &self.datastore,
            "record_indexer_error",
            "INSERT INTO indexer_errors (
                id, indexer_id, indexer_name, operation, http_status, classification,
                provider_error_code, message, content_type, payload_format_version,
                response_zstd, occurred_at
             ) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
            vec![
                SqlArg::Text(error.id),
                SqlArg::Text(error.indexer_id),
                SqlArg::Text(error.indexer_name),
                SqlArg::Text(error.operation.as_str().to_string()),
                SqlArg::I32(i32::from(http_status)),
                SqlArg::Text(error.classification.as_str().to_string()),
                SqlArg::OptI32(error.provider_error_code.map(i32::from)),
                SqlArg::Text(error.message),
                SqlArg::OptText(error.content_type),
                SqlArg::I32(PAYLOAD_FORMAT_VERSION),
                SqlArg::OptBytes(Some(payload)),
                SqlArg::Timestamp(error.occurred_at),
            ],
        )
        .await?;
        Ok(())
    }

    async fn list(
        &self,
        indexer_id: Option<&str>,
        first: usize,
        after: Option<&str>,
    ) -> AppResult<IndexerErrorPage> {
        let mut clauses = Vec::new();
        let mut args = Vec::new();
        if let Some(indexer_id) = indexer_id {
            clauses.push("indexer_id = {}".to_string());
            args.push(SqlArg::Text(indexer_id.to_string()));
        }
        if let Some(after) = after {
            let cursor = decode_cursor(after)?;
            clauses.push("(occurred_at < {} OR (occurred_at = {} AND id < {}))".to_string());
            args.push(SqlArg::Timestamp(cursor.occurred_at));
            args.push(SqlArg::Timestamp(cursor.occurred_at));
            args.push(SqlArg::Text(cursor.id));
        }
        let where_clause =
            (!clauses.is_empty()).then(|| format!(" WHERE {}", clauses.join(" AND ")));
        args.push(SqlArg::I64((first.saturating_add(1)) as i64));
        let query = format!(
            "SELECT id, indexer_id, indexer_name, operation, http_status, classification,
                    provider_error_code, message, content_type, occurred_at
             FROM indexer_errors{} ORDER BY occurred_at DESC, id DESC LIMIT {{}}",
            where_clause.unwrap_or_default()
        );
        let mut rows = SqlRuntime::fetch_all(self.datastore.read_exec(), &query, &args).await?;
        let has_next = rows.len() > first;
        rows.truncate(first);
        let items = rows
            .iter()
            .map(row_to_summary)
            .collect::<AppResult<Vec<_>>>()?;
        let next_cursor = has_next.then(|| items.last()).flatten().map(encode_cursor);
        Ok(IndexerErrorPage { items, next_cursor })
    }

    async fn get_detail(&self, id: &str) -> AppResult<Option<IndexerErrorDetail>> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT id, indexer_id, indexer_name, operation, http_status, classification,
                    provider_error_code, message, content_type, occurred_at, response_zstd
             FROM indexer_errors WHERE id = {}",
            &[SqlArg::Text(id.to_string())],
        )
        .await?;
        row.map(row_to_detail).transpose()
    }

    async fn delete_older_than(&self, cutoff: DateTime<Utc>) -> AppResult<u32> {
        let deleted = SqlRuntime::execute_write(
            &self.datastore,
            "delete_expired_indexer_errors",
            "DELETE FROM indexer_errors WHERE occurred_at < {}",
            vec![SqlArg::Timestamp(cutoff)],
        )
        .await?;
        u32::try_from(deleted).map_err(|_| {
            AppError::Repository("deleted indexer error count exceeds u32 range".to_string())
        })
    }
}

async fn encode_response(response: Option<CapturedIndexerHttpResponse>) -> AppResult<Vec<u8>> {
    let response = response.map(|mut response| {
        redact_indexer_response_headers(&mut response);
        response
    });
    tokio::task::spawn_blocking(move || {
        let (status, headers, body) = match response {
            Some(response) => (Some(response.status), response.headers, response.body),
            None => (None, Vec::new(), Vec::new()),
        };
        let envelope = StoredResponseEnvelope {
            version: PAYLOAD_FORMAT_VERSION,
            status,
            headers: headers
                .into_iter()
                .map(|header| StoredResponseHeader {
                    name: header.name,
                    value_base64: BASE64.encode(header.value),
                })
                .collect(),
            body_base64: BASE64.encode(body),
        };
        let json = serde_json::to_vec(&envelope).map_err(|error| {
            AppError::Repository(format!("encode indexer error response: {error}"))
        })?;
        zstd::encode_all(json.as_slice(), 3).map_err(|error| {
            AppError::Repository(format!("compress indexer error response: {error}"))
        })
    })
    .await
    .map_err(|error| {
        AppError::Repository(format!("indexer error compression task failed: {error}"))
    })?
}

fn row_to_summary(row: &SqlRow) -> AppResult<IndexerErrorSummary> {
    Ok(IndexerErrorSummary {
        id: row.text("id")?,
        indexer_id: row.text("indexer_id")?,
        indexer_name: row.text("indexer_name")?,
        operation: IndexerErrorOperation::parse(&row.text("operation")?)
            .ok_or_else(|| AppError::Repository("invalid indexer_errors.operation".to_string()))?,
        occurred_at: row.timestamp("occurred_at")?,
        http_status: (row.i32("http_status")? != i32::from(TRANSPORT_ERROR_HTTP_STATUS))
            .then(|| checked_u16(row.i32("http_status")?, "http_status"))
            .transpose()?,
        classification: IndexerErrorClassification::parse(&row.text("classification")?)
            .ok_or_else(|| {
                AppError::Repository("invalid indexer_errors.classification".to_string())
            })?,
        provider_error_code: row
            .opt_i32("provider_error_code")?
            .map(|code| checked_u16(code, "provider_error_code"))
            .transpose()?,
        message: row.text("message")?,
        content_type: row.opt_text("content_type")?,
    })
}

fn row_to_detail(row: SqlRow) -> AppResult<IndexerErrorDetail> {
    let summary = row_to_summary(&row)?;
    let payload = row
        .opt_bytes("response_zstd")?
        .ok_or_else(|| AppError::Repository("indexer_errors.response_zstd is NULL".to_string()))?;
    let response = decode_response(&payload)?;
    Ok(IndexerErrorDetail { summary, response })
}

fn decode_response(payload: &[u8]) -> AppResult<Option<CapturedIndexerHttpResponse>> {
    let json = zstd::decode_all(payload).map_err(|error| {
        AppError::Repository(format!("decompress indexer error response: {error}"))
    })?;
    let envelope: StoredResponseEnvelope = serde_json::from_slice(&json)
        .map_err(|error| AppError::Repository(format!("decode indexer error response: {error}")))?;
    if !(1..=PAYLOAD_FORMAT_VERSION).contains(&envelope.version) {
        return Err(AppError::Repository(format!(
            "unsupported indexer error payload version {}",
            envelope.version
        )));
    }
    let headers = envelope
        .headers
        .into_iter()
        .map(|header| {
            BASE64
                .decode(header.value_base64)
                .map(|value| CapturedIndexerHttpHeader {
                    name: header.name,
                    value,
                })
                .map_err(|error| {
                    AppError::Repository(format!("decode indexer response header: {error}"))
                })
        })
        .collect::<AppResult<Vec<_>>>()?;
    let body = BASE64
        .decode(envelope.body_base64)
        .map_err(|error| AppError::Repository(format!("decode indexer response body: {error}")))?;
    Ok(envelope.status.map(|status| CapturedIndexerHttpResponse {
        status,
        headers,
        body,
    }))
}

#[derive(Clone)]
struct IndexerErrorCursor {
    occurred_at: DateTime<Utc>,
    id: String,
}

fn encode_cursor(summary: &IndexerErrorSummary) -> String {
    BASE64.encode(format!(
        "{}\n{}",
        summary.occurred_at.to_rfc3339(),
        summary.id
    ))
}

fn decode_cursor(cursor: &str) -> AppResult<IndexerErrorCursor> {
    let decoded = BASE64.decode(cursor).map_err(|_| {
        AppError::validation_refused(
            crate::error_reasons::ACQUISITION_INDEXER_ERROR_CURSOR_INVALID,
            "invalid indexer error cursor".to_string(),
        )
    })?;
    let decoded = String::from_utf8(decoded).map_err(|_| {
        AppError::validation_refused(
            crate::error_reasons::ACQUISITION_INDEXER_ERROR_CURSOR_INVALID,
            "invalid indexer error cursor".to_string(),
        )
    })?;
    let (occurred_at, id) = decoded
        .split_once('\n')
        .filter(|(_, id)| !id.is_empty())
        .ok_or_else(|| {
            AppError::validation_refused(
                crate::error_reasons::ACQUISITION_INDEXER_ERROR_CURSOR_INVALID,
                "invalid indexer error cursor".to_string(),
            )
        })?;
    let occurred_at = DateTime::parse_from_rfc3339(occurred_at)
        .map_err(|_| {
            AppError::validation_refused(
                crate::error_reasons::ACQUISITION_INDEXER_ERROR_CURSOR_INVALID,
                "invalid indexer error cursor".to_string(),
            )
        })?
        .with_timezone(&Utc);
    Ok(IndexerErrorCursor {
        occurred_at,
        id: id.to_string(),
    })
}

fn checked_u16(value: i32, column: &str) -> AppResult<u16> {
    u16::try_from(value)
        .map_err(|_| AppError::Repository(format!("indexer_errors.{column} is outside u16 range")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn test_store() -> (IndexerErrorStore, sqlx::SqlitePool) {
        let (store, pool, _writer_gate) = test_store_with_writer_gate().await;
        (store, pool)
    }

    /// The store plus its SQLite writer gate, so a test can stand in for
    /// another writer that holds the gate.
    async fn test_store_with_writer_gate() -> (
        IndexerErrorStore,
        sqlx::SqlitePool,
        Arc<tokio::sync::Mutex<()>>,
    ) {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory SQLite pool");
        sqlx::raw_sql("PRAGMA foreign_keys = ON; CREATE TABLE indexers (id TEXT PRIMARY KEY);")
            .execute(&pool)
            .await
            .expect("indexer parent table");
        sqlx::raw_sql(include_str!(
            "../../../scryer/src/db/migrations/0174_indexer_error_history.sql"
        ))
        .execute(&pool)
        .await
        .expect("indexer error migration");
        let writer_gate = Arc::new(tokio::sync::Mutex::new(()));
        let store = IndexerErrorStore::new(StoreDatastore::Sqlite {
            pool: pool.clone(),
            writer_gate: Arc::clone(&writer_gate),
        });
        (store, pool, writer_gate)
    }

    async fn insert_indexer(pool: &sqlx::SqlitePool, id: &str) {
        sqlx::query("INSERT INTO indexers (id) VALUES (?)")
            .bind(id)
            .execute(pool)
            .await
            .expect("indexer row");
    }

    /// Delegates to the SQLite store and reports every finished write, so a
    /// test waits for the row instead of for elapsed time.
    struct ObservedIndexerErrorStore {
        inner: IndexerErrorStore,
        persisted: std::sync::mpsc::Sender<String>,
    }

    #[async_trait]
    impl IndexerErrorRepository for ObservedIndexerErrorStore {
        async fn record(&self, error: NewIndexerError) -> AppResult<()> {
            let id = error.id.clone();
            let result = self.inner.record(error).await;
            let _ = self.persisted.send(id);
            result
        }

        async fn list(
            &self,
            indexer_id: Option<&str>,
            first: usize,
            after: Option<&str>,
        ) -> AppResult<IndexerErrorPage> {
            self.inner.list(indexer_id, first, after).await
        }

        async fn get_detail(&self, id: &str) -> AppResult<Option<IndexerErrorDetail>> {
            self.inner.get_detail(id).await
        }

        async fn delete_older_than(&self, cutoff: DateTime<Utc>) -> AppResult<u32> {
            self.inner.delete_older_than(cutoff).await
        }
    }

    /// An acquisition walk polls several titles inside one task. If one title
    /// holds (or is next in line for) the SQLite writer gate while a sibling
    /// records an indexer error, recording must not stop the task: the holder
    /// could then never be polled again to release the gate, and every writer
    /// in the process would wait behind it forever.
    #[test]
    fn error_capture_does_not_stall_a_sibling_future_holding_the_writer_gate() {
        let runtime = multi_thread_runtime(2);
        let (store, pool, writer_gate) = runtime.block_on(test_store_with_writer_gate());
        runtime.block_on(insert_indexer(&pool, "indexer-a"));
        let (persisted_tx, persisted_rx) = std::sync::mpsc::channel();
        let recorder = {
            let _guard = runtime.enter();
            QueuedIndexerErrorRecorder::new(Arc::new(ObservedIndexerErrorStore {
                inner: store.clone(),
                persisted: persisted_tx,
            }))
        };

        let (finished_tx, finished_rx) = std::sync::mpsc::channel();
        runtime.spawn(async move {
            let gate_held = tokio::sync::Notify::new();
            let recorded = tokio::sync::Notify::new();
            let holder = async {
                let _write = writer_gate.lock().await;
                gate_held.notify_one();
                recorded.notified().await;
            };
            let capturer = async {
                gate_held.notified().await;
                let result = recorder.record(error_event("sibling", "indexer-a", Utc::now()));
                recorded.notify_one();
                result
            };
            let ((), result) = tokio::join!(holder, capturer);
            let _ = finished_tx.send(result);
        });

        let finished = finished_rx.recv_timeout(BOUND);
        if finished.is_err() {
            runtime.shutdown_background();
            panic!(
                "recording an indexer error blocked the task while a sibling future held the writer gate"
            );
        }
        assert!(matches!(finished, Ok(Ok(()))), "{finished:?}");
        let persisted = persisted_rx.recv_timeout(BOUND);
        assert_eq!(persisted.as_deref(), Ok("sibling"));
        let detail = runtime
            .block_on(store.get_detail("sibling"))
            .expect("detail query");
        runtime.shutdown_background();
        assert!(
            detail.is_some(),
            "the error row must persist once the gate frees"
        );
    }

    fn error_event(id: &str, indexer_id: &str, occurred_at: DateTime<Utc>) -> NewIndexerError {
        NewIndexerError {
            id: id.to_string(),
            indexer_id: indexer_id.to_string(),
            indexer_name: "Example indexer".to_string(),
            operation: IndexerErrorOperation::InteractiveSearch,
            classification: IndexerErrorClassification::HttpRateLimited,
            provider_error_code: None,
            message: "Indexer rate limit reached".to_string(),
            content_type: Some("application/octet-stream".to_string()),
            response: Some(CapturedIndexerHttpResponse {
                status: 429,
                headers: vec![
                    CapturedIndexerHttpHeader {
                        name: "x-duplicate".to_string(),
                        value: vec![0, 255],
                    },
                    CapturedIndexerHttpHeader {
                        name: "x-duplicate".to_string(),
                        value: b"second".to_vec(),
                    },
                    CapturedIndexerHttpHeader {
                        name: "authorization".to_string(),
                        value: b"should-not-persist".to_vec(),
                    },
                ],
                body: vec![0, 1, 255, b'x'],
            }),
            occurred_at,
        }
    }

    #[tokio::test]
    async fn stored_response_round_trips_binary_body_and_duplicate_headers() {
        let encoded = encode_response(Some(CapturedIndexerHttpResponse {
            status: 429,
            headers: vec![
                CapturedIndexerHttpHeader {
                    name: "retry-after".to_string(),
                    value: b"120".to_vec(),
                },
                CapturedIndexerHttpHeader {
                    name: "x-duplicate".to_string(),
                    value: vec![0, 255],
                },
                CapturedIndexerHttpHeader {
                    name: "x-duplicate".to_string(),
                    value: b"second".to_vec(),
                },
                CapturedIndexerHttpHeader {
                    name: "set-cookie".to_string(),
                    value: b"session=secret".to_vec(),
                },
            ],
            body: vec![0, 1, 255, b'x'],
        }))
        .await
        .expect("response encodes as zstd");

        let decoded = decode_response(&encoded).expect("response decodes");
        let decoded = decoded.expect("response is present");
        assert_eq!(decoded.status, 429);
        assert_eq!(decoded.body, vec![0, 1, 255, b'x']);
        assert_eq!(decoded.headers.len(), 4);
        assert_eq!(decoded.headers[0].value, b"120");
        assert_eq!(decoded.headers[1].value, vec![0, 255]);
        assert_eq!(decoded.headers[2].value, b"second");
        assert!(decoded.headers[3].value.is_empty());
    }

    #[test]
    fn cursor_round_trip_and_invalid_input() {
        let summary = IndexerErrorSummary {
            id: "err-2".to_string(),
            indexer_id: "indexer-1".to_string(),
            indexer_name: "Example".to_string(),
            operation: IndexerErrorOperation::InteractiveSearch,
            occurred_at: "2026-08-23T10:00:00Z".parse().expect("timestamp"),
            http_status: Some(500),
            classification: IndexerErrorClassification::HttpServerError,
            provider_error_code: None,
            message: "HTTP server error".to_string(),
            content_type: None,
        };

        let cursor = encode_cursor(&summary);
        let decoded = decode_cursor(&cursor).expect("cursor decodes");
        assert_eq!(decoded.id, summary.id);
        assert_eq!(decoded.occurred_at, summary.occurred_at);
        assert!(decode_cursor("not a cursor").is_err());
    }

    #[tokio::test]
    async fn sqlite_store_persists_transport_error_without_response() {
        let (store, pool) = test_store().await;
        sqlx::query("INSERT INTO indexers (id) VALUES (?)")
            .bind("indexer-a")
            .execute(&pool)
            .await
            .expect("indexer row");

        let mut error = error_event("transport", "indexer-a", Utc::now());
        error.classification = IndexerErrorClassification::HttpRequestTimeout;
        error.message = "Indexer search timed out".to_string();
        error.content_type = None;
        error.response = None;
        store.record(error).await.expect("transport event");

        let page = store.list(Some("indexer-a"), 10, None).await.expect("page");
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].http_status, None);
        assert_eq!(
            page.items[0].classification,
            IndexerErrorClassification::HttpRequestTimeout
        );
        let detail = store
            .get_detail("transport")
            .await
            .expect("detail query")
            .expect("detail");
        assert_eq!(detail.response, None);
    }

    #[tokio::test]
    async fn sqlite_store_persists_pages_expires_and_cascades_indexer_errors() {
        let (store, pool) = test_store().await;
        for id in ["indexer-a", "indexer-b"] {
            sqlx::query("INSERT INTO indexers (id) VALUES (?)")
                .bind(id)
                .execute(&pool)
                .await
                .expect("indexer row");
        }
        let old = DateTime::parse_from_rfc3339("2026-07-20T10:00:00Z")
            .expect("timestamp")
            .with_timezone(&Utc);
        let first = DateTime::parse_from_rfc3339("2026-08-20T10:00:00Z")
            .expect("timestamp")
            .with_timezone(&Utc);
        let second = DateTime::parse_from_rfc3339("2026-08-21T10:00:00Z")
            .expect("timestamp")
            .with_timezone(&Utc);

        store
            .record(error_event("old", "indexer-a", old))
            .await
            .expect("old event");
        store
            .record(error_event("first", "indexer-a", first))
            .await
            .expect("first event");
        store
            .record(error_event("second", "indexer-a", second))
            .await
            .expect("second event");
        store
            .record(error_event("other", "indexer-b", second))
            .await
            .expect("other indexer event");

        let page = store
            .list(Some("indexer-a"), 1, None)
            .await
            .expect("first page");
        assert_eq!(
            page.items
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["second"]
        );
        let next = page.next_cursor.expect("next cursor");
        let page = store
            .list(Some("indexer-a"), 10, Some(&next))
            .await
            .expect("following page");
        assert_eq!(
            page.items
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["first", "old"]
        );

        let detail = store
            .get_detail("second")
            .await
            .expect("detail query")
            .expect("detail");
        let response = detail.response.expect("captured response");
        assert_eq!(response.body, vec![0, 1, 255, b'x']);
        assert_eq!(response.headers.len(), 3);
        assert_eq!(response.headers[0].value, vec![0, 255]);
        assert_eq!(response.headers[1].value, b"second");
        assert!(response.headers[2].value.is_empty());

        assert_eq!(
            store
                .delete_older_than(
                    DateTime::parse_from_rfc3339("2026-07-22T10:00:00Z")
                        .expect("cutoff")
                        .with_timezone(&Utc),
                )
                .await
                .expect("expiry"),
            1
        );
        assert!(store.get_detail("old").await.expect("old detail").is_none());

        sqlx::query("DELETE FROM indexers WHERE id = ?")
            .bind("indexer-a")
            .execute(&pool)
            .await
            .expect("delete indexer");
        assert!(
            store
                .get_detail("second")
                .await
                .expect("cascaded detail")
                .is_none()
        );
        assert!(
            store
                .get_detail("other")
                .await
                .expect("other detail")
                .is_some()
        );
    }

    const BOUND: std::time::Duration = std::time::Duration::from_secs(30);

    fn multi_thread_runtime(workers: usize) -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(workers)
            .enable_all()
            .build()
            .expect("application runtime")
    }

    fn datastore_down<T>() -> AppResult<T> {
        Err(AppError::Repository("datastore is down".to_string()))
    }

    /// Stands in for a datastore that refuses the write, and reports each
    /// attempt so a test can wait for the persister to have tried.
    struct FailingIndexerErrorRepository {
        attempts: tokio::sync::mpsc::UnboundedSender<String>,
    }

    fn failing_repository() -> (
        Arc<FailingIndexerErrorRepository>,
        tokio::sync::mpsc::UnboundedReceiver<String>,
    ) {
        let (attempts, attempted) = tokio::sync::mpsc::unbounded_channel();
        (
            Arc::new(FailingIndexerErrorRepository { attempts }),
            attempted,
        )
    }

    #[async_trait]
    impl IndexerErrorRepository for FailingIndexerErrorRepository {
        async fn record(&self, error: NewIndexerError) -> AppResult<()> {
            let _ = self.attempts.send(error.id);
            datastore_down()
        }

        async fn list(
            &self,
            _indexer_id: Option<&str>,
            _first: usize,
            _after: Option<&str>,
        ) -> AppResult<IndexerErrorPage> {
            datastore_down()
        }

        async fn get_detail(&self, _id: &str) -> AppResult<Option<IndexerErrorDetail>> {
            datastore_down()
        }

        async fn delete_older_than(&self, _cutoff: DateTime<Utc>) -> AppResult<u32> {
            datastore_down()
        }
    }

    /// Models Postgres persistence waiting for socket I/O on the application
    /// runtime: each write completes only once the observer sends a byte on
    /// that error's socket.
    struct RuntimeDrivenIndexerErrorRepository {
        entered: std::sync::mpsc::Sender<String>,
        persisted: std::sync::mpsc::Sender<String>,
        sockets: std::sync::Mutex<std::collections::HashMap<String, tokio::net::TcpStream>>,
    }

    #[async_trait]
    impl IndexerErrorRepository for RuntimeDrivenIndexerErrorRepository {
        async fn record(&self, error: NewIndexerError) -> AppResult<()> {
            use tokio::io::AsyncReadExt;
            let mut socket = self
                .sockets
                .lock()
                .unwrap()
                .remove(&error.id)
                .expect("error socket");
            let _ = self.entered.send(error.id.clone());
            let mut response = [0_u8; 1];
            socket
                .read_exact(&mut response)
                .await
                .map_err(|io_error| AppError::Repository(io_error.to_string()))?;
            let _ = self.persisted.send(error.id);
            Ok(())
        }

        async fn list(
            &self,
            _indexer_id: Option<&str>,
            _first: usize,
            _after: Option<&str>,
        ) -> AppResult<IndexerErrorPage> {
            datastore_down()
        }

        async fn get_detail(&self, _id: &str) -> AppResult<Option<IndexerErrorDetail>> {
            datastore_down()
        }

        async fn delete_older_than(&self, _cutoff: DateTime<Utc>) -> AppResult<u32> {
            datastore_down()
        }
    }

    /// More concurrent callers than runtime workers, all recording while the
    /// first write is still waiting on runtime I/O: every caller returns, the
    /// runtime keeps serving unrelated work, and every write completes once
    /// its response arrives.
    #[test]
    fn concurrent_indexer_error_capture_keeps_runtime_responsive() {
        use std::io::Write;
        const WORKERS: usize = 2;
        const CONCURRENT_ERRORS: usize = WORKERS * 2;
        let runtime = multi_thread_runtime(WORKERS);
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (persisted_tx, persisted_rx) = std::sync::mpsc::channel();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("persistence listener");
        let address = listener.local_addr().expect("persistence address");
        let mut sockets = std::collections::HashMap::new();
        let mut peers = Vec::new();
        for index in 0..CONCURRENT_ERRORS {
            let peer =
                std::net::TcpStream::connect_timeout(&address, BOUND).expect("persistence peer");
            peer.set_write_timeout(Some(BOUND))
                .expect("peer write bound");
            let (socket, _) = listener.accept().expect("persistence connection");
            socket
                .set_nonblocking(true)
                .expect("async persistence socket");
            let socket = {
                let _guard = runtime.enter();
                tokio::net::TcpStream::from_std(socket).expect("register persistence socket")
            };
            sockets.insert(format!("error-{index}"), socket);
            peers.push(peer);
        }
        let probe_listener = std::net::TcpListener::bind("127.0.0.1:0").expect("probe listener");
        let probe_address = probe_listener.local_addr().expect("probe address");
        probe_listener
            .set_nonblocking(true)
            .expect("async probe listener");
        let probe_listener = {
            let _guard = runtime.enter();
            tokio::net::TcpListener::from_std(probe_listener).expect("register probe listener")
        };
        let recorder = {
            let _guard = runtime.enter();
            QueuedIndexerErrorRecorder::new(Arc::new(RuntimeDrivenIndexerErrorRepository {
                entered: entered_tx,
                persisted: persisted_tx,
                sockets: std::sync::Mutex::new(sockets),
            }))
        };
        let (finished_tx, finished_rx) = std::sync::mpsc::channel();
        for index in 0..CONCURRENT_ERRORS {
            let recorder = recorder.clone();
            let finished = finished_tx.clone();
            runtime.spawn(async move {
                let result = recorder.record(error_event(
                    &format!("error-{index}"),
                    &format!("indexer-{index}"),
                    Utc::now(),
                ));
                let _ = finished.send(result);
            });
        }
        drop(finished_tx);

        // No response byte has been sent yet, so no write can have finished:
        // every caller returning here proves none of them waited for storage.
        let finished = (0..CONCURRENT_ERRORS)
            .map(|_| finished_rx.recv_timeout(BOUND))
            .collect::<Vec<_>>();
        // The persister is now parked on runtime I/O for its first write.
        let first_entered = entered_rx.recv_timeout(BOUND);
        let (probe_tx, probe_rx) = std::sync::mpsc::channel();
        runtime.spawn(async move {
            let result = probe_listener.accept().await.map(|_| ());
            let _ = probe_tx.send(result);
        });
        let connected = std::net::TcpStream::connect_timeout(&probe_address, BOUND);
        let responsive = matches!(probe_rx.recv_timeout(BOUND), Ok(Ok(())));

        // These bytes reach the sockets without application-runtime work, so
        // a write that never completes cannot be blamed on a slow database.
        let responses_sent = peers.iter_mut().all(|peer| peer.write_all(&[1]).is_ok());
        let mut persisted = (0..CONCURRENT_ERRORS)
            .map_while(|_| persisted_rx.recv_timeout(BOUND).ok())
            .collect::<Vec<_>>();
        persisted.sort();
        runtime.shutdown_background();

        assert!(
            finished.iter().all(|result| matches!(result, Ok(Ok(())))),
            "every caller must return while persistence is pending: {finished:?}"
        );
        assert!(
            first_entered.is_ok(),
            "the persister never reached the repository"
        );
        assert!(connected.is_ok(), "the probe TCP connection must establish");
        assert!(
            responsive,
            "the runtime must serve unrelated work while persistence is pending"
        );
        assert!(
            responses_sent,
            "persistence responses must reach every socket"
        );
        let mut expected = (0..CONCURRENT_ERRORS)
            .map(|index| format!("error-{index}"))
            .collect::<Vec<_>>();
        expected.sort();
        assert_eq!(persisted, expected, "every queued error must persist");
    }

    #[test]
    fn record_returns_before_persistence_and_the_row_follows() {
        let runtime = multi_thread_runtime(2);
        let (store, pool, writer_gate) = runtime.block_on(test_store_with_writer_gate());
        runtime.block_on(insert_indexer(&pool, "indexer-a"));
        let (persisted_tx, persisted_rx) = std::sync::mpsc::channel();
        let recorder = {
            let _guard = runtime.enter();
            QueuedIndexerErrorRecorder::new(Arc::new(ObservedIndexerErrorStore {
                inner: store.clone(),
                persisted: persisted_tx,
            }))
        };

        // Another writer holds the gate, so the write cannot finish before
        // `record` returns.
        let other_writer = runtime.block_on(Arc::clone(&writer_gate).lock_owned());
        let result = recorder.record(error_event("queued", "indexer-a", Utc::now()));
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(
            persisted_rx.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        );
        assert!(
            runtime
                .block_on(store.get_detail("queued"))
                .expect("detail query")
                .is_none(),
            "the row must not exist while the writer gate is held"
        );

        drop(other_writer);
        assert_eq!(persisted_rx.recv_timeout(BOUND).as_deref(), Ok("queued"));
        let detail = runtime
            .block_on(store.get_detail("queued"))
            .expect("detail query")
            .expect("persisted row");
        runtime.shutdown_background();
        assert_eq!(detail.summary.indexer_id, "indexer-a");
        assert_eq!(
            detail.summary.classification,
            IndexerErrorClassification::HttpRateLimited
        );
        assert_eq!(detail.response.map(|response| response.status), Some(429));
    }

    /// Parks every write until the test releases it, and reports when each
    /// write starts and finishes.
    struct ParkedIndexerErrorRepository {
        entered: tokio::sync::mpsc::UnboundedSender<String>,
        persisted: tokio::sync::mpsc::UnboundedSender<String>,
        release: Arc<tokio::sync::Semaphore>,
    }

    #[async_trait]
    impl IndexerErrorRepository for ParkedIndexerErrorRepository {
        async fn record(&self, error: NewIndexerError) -> AppResult<()> {
            let _ = self.entered.send(error.id.clone());
            self.release
                .acquire()
                .await
                .expect("release semaphore")
                .forget();
            let _ = self.persisted.send(error.id);
            Ok(())
        }

        async fn list(
            &self,
            _indexer_id: Option<&str>,
            _first: usize,
            _after: Option<&str>,
        ) -> AppResult<IndexerErrorPage> {
            datastore_down()
        }

        async fn get_detail(&self, _id: &str) -> AppResult<Option<IndexerErrorDetail>> {
            datastore_down()
        }

        async fn delete_older_than(&self, _cutoff: DateTime<Utc>) -> AppResult<u32> {
            datastore_down()
        }
    }

    type CounterSeries = Vec<(
        std::collections::BTreeMap<String, String>,
        metrics_util::debugging::DebugValue,
    )>;

    /// Every series by metric name, from one snapshot: taking a snapshot
    /// resets the debugging recorder's counters, so it must happen once.
    fn counter_series(
        snapshotter: &metrics_util::debugging::Snapshotter,
    ) -> std::collections::BTreeMap<String, CounterSeries> {
        let mut series = std::collections::BTreeMap::<String, CounterSeries>::new();
        for (key, _, _, value) in snapshotter.snapshot().into_vec() {
            let labels = key
                .key()
                .labels()
                .map(|label| (label.key().to_string(), label.value().to_string()))
                .collect();
            series
                .entry(key.key().name().to_string())
                .or_default()
                .push((labels, value));
        }
        series
    }

    fn assert_labels_match_fixture(series: &CounterSeries) {
        assert_eq!(series.len(), 1, "unexpected series: {series:?}");
        let (labels, value) = &series[0];
        assert_eq!(
            labels.get("indexer").map(String::as_str),
            Some("Example indexer")
        );
        assert_eq!(
            labels.get("operation").map(String::as_str),
            Some("interactive_search")
        );
        assert_eq!(
            labels.get("class").map(String::as_str),
            Some("http_rate_limited")
        );
        assert_eq!(value, &metrics_util::debugging::DebugValue::Counter(1));
    }

    #[test]
    fn a_full_queue_drops_the_entry_without_blocking_and_counts_it() {
        const CAPACITY: usize = 2;
        let runtime = multi_thread_runtime(1);
        let (entered_tx, mut entered_rx) = tokio::sync::mpsc::unbounded_channel();
        let (persisted_tx, mut persisted_rx) = tokio::sync::mpsc::unbounded_channel();
        let release = Arc::new(tokio::sync::Semaphore::new(0));
        let recorder = {
            let _guard = runtime.enter();
            QueuedIndexerErrorRecorder::with_capacity(
                Arc::new(ParkedIndexerErrorRepository {
                    entered: entered_tx,
                    persisted: persisted_tx,
                    release: Arc::clone(&release),
                }),
                CAPACITY,
            )
        };

        // The persister takes the first entry and parks inside the repository,
        // which leaves exactly CAPACITY free slots for the next entries.
        recorder
            .record(error_event("queued-0", "indexer-a", Utc::now()))
            .expect("first entry");
        let entered =
            runtime.block_on(async { tokio::time::timeout(BOUND, entered_rx.recv()).await });
        assert_eq!(entered.ok().flatten().as_deref(), Some("queued-0"));
        for index in 1..=CAPACITY {
            recorder
                .record(error_event(
                    &format!("queued-{index}"),
                    "indexer-a",
                    Utc::now(),
                ))
                .expect("queued entry");
        }

        // A thread of its own bounds the call: a regression that blocks on the
        // full queue fails here instead of hanging the test.
        let (outcome_tx, outcome_rx) = std::sync::mpsc::channel();
        let overflowing = recorder.clone();
        std::thread::spawn(move || {
            let debugging = metrics_util::debugging::DebuggingRecorder::new();
            let snapshotter = debugging.snapshotter();
            let result = metrics::with_local_recorder(&debugging, || {
                overflowing.record(error_event("overflow", "indexer-a", Utc::now()))
            });
            let _ = outcome_tx.send((result, counter_series(&snapshotter)));
        });
        let outcome = outcome_rx.recv_timeout(BOUND);
        let Ok((result, mut series)) = outcome else {
            runtime.shutdown_background();
            panic!("recording into a full queue blocked the caller");
        };
        assert!(result.is_ok(), "{result:?}");
        assert_labels_match_fixture(
            &series
                .remove("scryer_indexer_errors_total")
                .unwrap_or_default(),
        );
        assert_labels_match_fixture(
            &series
                .remove("scryer_indexer_errors_dropped_total")
                .unwrap_or_default(),
        );

        release.add_permits(CAPACITY + 1);
        let persisted = runtime.block_on(async {
            let mut persisted = Vec::new();
            for _ in 0..=CAPACITY {
                match tokio::time::timeout(BOUND, persisted_rx.recv()).await {
                    Ok(Some(id)) => persisted.push(id),
                    _ => break,
                }
            }
            persisted
        });
        assert_eq!(persisted, ["queued-0", "queued-1", "queued-2"]);

        // Dropping the last recorder closes the queue; the persister drains
        // it and exits, dropping the repository. The dropped entry was never
        // queued, so the persisted stream ends without it.
        drop(recorder);
        let after_drain =
            runtime.block_on(async { tokio::time::timeout(BOUND, persisted_rx.recv()).await });
        runtime.shutdown_background();
        assert_eq!(after_drain.ok(), Some(None));
    }

    #[test]
    fn current_thread_error_capture_hands_off_without_blocking() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let (repository, mut attempted) = failing_repository();
            let subject = QueuedIndexerErrorRecorder::new(repository);
            subject
                .record(error_event("current-thread", "fixture", Utc::now()))
                .expect("a current-thread runtime queues the entry");
            let attempt = tokio::time::timeout(BOUND, attempted.recv()).await;
            assert_eq!(attempt.ok().flatten().as_deref(), Some("current-thread"));
        });
    }

    #[test]
    fn error_capture_outside_runtime_hands_off_to_the_persister() {
        let runtime = multi_thread_runtime(1);
        let (repository, mut attempted) = failing_repository();
        let subject = {
            let _guard = runtime.enter();
            QueuedIndexerErrorRecorder::new(repository)
        };
        assert!(tokio::runtime::Handle::try_current().is_err());
        subject
            .record(error_event("outside-runtime", "fixture", Utc::now()))
            .expect("a caller outside the runtime queues the entry");
        let attempt =
            runtime.block_on(async { tokio::time::timeout(BOUND, attempted.recv()).await });
        runtime.shutdown_background();
        assert_eq!(attempt.ok().flatten().as_deref(), Some("outside-runtime"));
    }

    #[test]
    fn classified_errors_are_counted_even_when_persistence_fails() {
        let runtime = multi_thread_runtime(1);
        let (repository, mut attempted) = failing_repository();
        let subject = {
            let _guard = runtime.enter();
            QueuedIndexerErrorRecorder::new(repository)
        };

        let debugging = metrics_util::debugging::DebuggingRecorder::new();
        let snapshotter = debugging.snapshotter();
        let result = metrics::with_local_recorder(&debugging, || {
            subject.record(error_event("err-1", "idx-1", Utc::now()))
        });
        assert!(result.is_ok(), "{result:?}");
        // The stub repository must actually have refused the write, otherwise
        // the test proves nothing.
        let attempt =
            runtime.block_on(async { tokio::time::timeout(BOUND, attempted.recv()).await });
        runtime.shutdown_background();
        assert_eq!(attempt.ok().flatten().as_deref(), Some("err-1"));

        let mut series = counter_series(&snapshotter);
        assert_labels_match_fixture(
            &series
                .remove("scryer_indexer_errors_total")
                .unwrap_or_default(),
        );
        assert!(
            !series.contains_key("scryer_indexer_errors_dropped_total"),
            "a queued entry is not a dropped one"
        );
    }
}

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

#[derive(Clone)]
pub struct BlockingIndexerErrorRecorder {
    repository: Arc<dyn IndexerErrorRepository>,
    runtime: tokio::runtime::Handle,
}

impl BlockingIndexerErrorRecorder {
    pub fn new(repository: Arc<dyn IndexerErrorRepository>) -> Self {
        Self {
            repository,
            runtime: tokio::runtime::Handle::current(),
        }
    }
}

impl IndexerErrorRecorder for BlockingIndexerErrorRecorder {
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
        let repository = Arc::clone(&self.repository);
        let runtime = self.runtime.clone();
        let result = std::thread::scope(|scope| {
            scope
                .spawn(move || runtime.block_on(repository.record(error)))
                .join()
                .map_err(|_| {
                    AppError::Repository("indexer error recorder thread panicked".to_string())
                })?
        });
        if let Err(error) = &result {
            tracing::warn!(error = %error, "failed to persist indexer HTTP error history");
        }
        result
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
    let decoded = BASE64
        .decode(cursor)
        .map_err(|_| AppError::Validation("invalid indexer error cursor".to_string()))?;
    let decoded = String::from_utf8(decoded)
        .map_err(|_| AppError::Validation("invalid indexer error cursor".to_string()))?;
    let (occurred_at, id) = decoded
        .split_once('\n')
        .filter(|(_, id)| !id.is_empty())
        .ok_or_else(|| AppError::Validation("invalid indexer error cursor".to_string()))?;
    let occurred_at = DateTime::parse_from_rfc3339(occurred_at)
        .map_err(|_| AppError::Validation("invalid indexer error cursor".to_string()))?
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
        let store = IndexerErrorStore::new(StoreDatastore::Sqlite {
            pool: pool.clone(),
            writer_gate: Arc::new(tokio::sync::Mutex::new(())),
        });
        (store, pool)
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

    /// Stands in for a datastore that refuses the write, so the test can prove
    /// the counter survives a persistence failure.
    struct FailingIndexerErrorRepository;

    #[async_trait]
    impl IndexerErrorRepository for FailingIndexerErrorRepository {
        async fn record(&self, _error: NewIndexerError) -> AppResult<()> {
            Err(AppError::Repository("datastore is down".to_string()))
        }

        async fn list(
            &self,
            _indexer_id: Option<&str>,
            _first: usize,
            _after: Option<&str>,
        ) -> AppResult<IndexerErrorPage> {
            Err(AppError::Repository("datastore is down".to_string()))
        }

        async fn get_detail(&self, _id: &str) -> AppResult<Option<IndexerErrorDetail>> {
            Err(AppError::Repository("datastore is down".to_string()))
        }

        async fn delete_older_than(&self, _cutoff: DateTime<Utc>) -> AppResult<u32> {
            Err(AppError::Repository("datastore is down".to_string()))
        }
    }

    #[test]
    fn classified_errors_are_counted_even_when_persistence_fails() {
        // A multi-thread runtime: `record` blocks on the repository future from
        // a scoped std thread, which needs a runtime that keeps being driven.
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("multi-thread runtime");
        let _guard = runtime.enter();
        let subject = BlockingIndexerErrorRecorder::new(Arc::new(FailingIndexerErrorRepository));

        let debugging = metrics_util::debugging::DebuggingRecorder::new();
        let snapshotter = debugging.snapshotter();
        let result = metrics::with_local_recorder(&debugging, || {
            subject.record(error_event("err-1", "idx-1", Utc::now()))
        });

        assert!(
            result.is_err(),
            "the stub repository must fail, otherwise the test proves nothing"
        );

        let recorded = snapshotter
            .snapshot()
            .into_vec()
            .into_iter()
            .filter(|(key, _, _, _)| key.key().name() == "scryer_indexer_errors_total")
            .map(|(key, _, _, value)| {
                let labels = key
                    .key()
                    .labels()
                    .map(|label| (label.key().to_string(), label.value().to_string()))
                    .collect::<std::collections::BTreeMap<_, _>>();
                (labels, value)
            })
            .collect::<Vec<_>>();

        assert_eq!(recorded.len(), 1, "unexpected series: {recorded:?}");
        let (labels, value) = &recorded[0];
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
}

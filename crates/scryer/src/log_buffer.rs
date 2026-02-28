use std::collections::VecDeque;
use std::io::Write;
use std::sync::{Arc, Mutex};

use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::Json;
use scryer_application::AppUseCase;
use scryer_domain::Entitlement;
use serde::{Deserialize, Serialize};

use crate::middleware::resolve_actor_with_entitlement;

const DEFAULT_CAPACITY: usize = 1000;
const DEFAULT_LIMIT: usize = 250;
const MAX_LIMIT: usize = 2000;

/// Thread-safe ring buffer that captures log lines.
#[derive(Clone)]
pub(crate) struct LogRingBuffer {
    inner: Arc<Mutex<RingBufferInner>>,
}

struct RingBufferInner {
    lines: VecDeque<String>,
    capacity: usize,
    /// Accumulates partial writes (no trailing newline yet).
    partial: String,
}

impl LogRingBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(RingBufferInner {
                lines: VecDeque::with_capacity(capacity),
                capacity,
                partial: String::new(),
            })),
        }
    }

    pub fn with_default_capacity() -> Self {
        Self::new(DEFAULT_CAPACITY)
    }

    pub fn snapshot(&self, limit: usize) -> Vec<String> {
        let inner = self.inner.lock().unwrap();
        let safe_limit = limit.min(inner.lines.len());
        inner
            .lines
            .iter()
            .skip(inner.lines.len().saturating_sub(safe_limit))
            .cloned()
            .collect()
    }
}

impl Write for LogRingBuffer {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let text = String::from_utf8_lossy(buf);
        let mut inner = self.inner.lock().unwrap();

        for ch in text.chars() {
            if ch == '\n' {
                if !inner.partial.is_empty() {
                    let line = std::mem::take(&mut inner.partial);
                    if inner.lines.len() >= inner.capacity {
                        inner.lines.pop_front();
                    }
                    inner.lines.push_back(line);
                }
            } else {
                inner.partial.push(ch);
            }
        }

        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Adapter that lets `tracing_subscriber` write to our ring buffer.
/// Implements `tracing_subscriber::fmt::MakeWriter` by returning a clone
/// of the buffer (which implements `io::Write`).
#[derive(Clone)]
pub(crate) struct LogBufferWriter {
    buffer: LogRingBuffer,
}

impl LogBufferWriter {
    pub fn new(buffer: LogRingBuffer) -> Self {
        Self { buffer }
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogBufferWriter {
    type Writer = LogRingBuffer;

    fn make_writer(&'a self) -> Self::Writer {
        self.buffer.clone()
    }
}

// --- REST endpoint ---

#[derive(Deserialize)]
pub(crate) struct LogsQuery {
    limit: Option<usize>,
}

#[derive(Serialize)]
struct LogsResponse {
    generated_at: String,
    lines: Vec<String>,
    count: usize,
}

pub(crate) async fn logs_handler(
    State((buffer, app)): State<(LogRingBuffer, AppUseCase)>,
    headers: HeaderMap,
    Query(query): Query<LogsQuery>,
) -> Response {
    if let Err(error) =
        resolve_actor_with_entitlement(&app, &headers, Entitlement::ManageConfig).await
    {
        return crate::middleware::map_app_error(error);
    }

    let limit = query
        .limit
        .unwrap_or(DEFAULT_LIMIT)
        .clamp(1, MAX_LIMIT);
    let lines = buffer.snapshot(limit);
    let count = lines.len();

    Json(LogsResponse {
        generated_at: chrono::Utc::now().to_rfc3339(),
        lines,
        count,
    })
    .into_response()
}

mod nzbget;
mod router;
mod sabnzbd;
pub mod weaver;
pub mod weaver_graphql;
pub mod weaver_subscription;
pub mod weaver_supervisor;

use std::io::{BufRead, BufReader as StdBufReader, Cursor, Read, Write};
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use futures_util::StreamExt;
use quick_xml::Reader;
use quick_xml::events::Event;
use scryer_application::{
    AppError, AppResult, DownloadClientAddRequest, NZB_HEAD_PROBE_BYTES, RateLimitCooldownAction,
    StagedNzbRef, StagedNzbStore, enforce_nzb_category_gate,
};
use scryer_domain::MediaFacet;
use scryer_outbound_http::{OutboundHttpClient, OutboundHttpError, RequestPolicy};
use serde_json::{Value, json};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub use nzbget::NzbgetDownloadClient;
pub use router::{PrioritizedDownloadClientRouter, download_client_feedback_timeout};
pub use sabnzbd::SabnzbdDownloadClient;
pub use weaver::WeaverDownloadClient;
pub use weaver_subscription::{WeaverSubscriptionBridgeClient, start_weaver_subscription_bridge};
pub use weaver_supervisor::start_weaver_bridge_supervisor;

const MAX_NZB_BYTES: u64 = 32 * 1024 * 1024;
const STAGED_NZB_ZSTD_LEVEL: i32 = 3;
const NZB_HEAD_CLOSE_TAG: &[u8] = b"</head>";

#[derive(Clone, Default)]
pub struct BuiltinDownloadClientConnectionTester;

#[async_trait]
impl scryer_application::BuiltinDownloadClientConnectionTester
    for BuiltinDownloadClientConnectionTester
{
    async fn test_connection(&self, client_type: &str, config_json: &str) -> AppResult<()> {
        let client_type = client_type.trim().to_lowercase();
        let config = parse_download_client_config_json(config_json)?;
        let base_url = resolve_download_client_base_url(&config).ok_or_else(|| {
            AppError::Validation("cannot compute base URL from config - host is required".into())
        })?;
        validate_test_flight_url(&base_url)?;

        match client_type.as_str() {
            "nzbget" => {
                let username = read_config_string(&config, &["username"]);
                let password = read_config_string(&config, &["password"]);
                NzbgetDownloadClient::new(base_url, username, password, "SCORE".to_string())
                    .test_connection()
                    .await?;
            }
            "sabnzbd" => {
                let api_key = read_config_string(&config, &["api_key", "apiKey", "apikey"]);
                let username = read_config_string(&config, &["username"]);
                let password = read_config_string(&config, &["password"]);
                if api_key.is_none() && (username.is_none() || password.is_none()) {
                    return Err(AppError::Validation(
                        "sabnzbd requires an API key or username/password".into(),
                    ));
                }
                SabnzbdDownloadClient::with_auth(base_url, api_key, username, password)
                    .test_connection()
                    .await?;
            }
            "weaver" => {
                let api_key = read_config_string(&config, &["api_key", "apiKey", "apikey"]);
                WeaverDownloadClient::new(base_url, api_key)
                    .test_connection()
                    .await?;
            }
            _ => {
                return Err(AppError::Validation(format!(
                    "test connection is not supported for client type '{client_type}'"
                )));
            }
        }

        Ok(())
    }
}

/// Compute a base URL from host/port/use_ssl/url_base in a config_json string.
/// Public for use by the GraphQL mapper layer.
pub fn resolve_base_url_from_config_json(config_json: &str) -> Option<String> {
    let parsed = parse_download_client_config_json(config_json).ok()?;
    resolve_download_client_base_url(&parsed)
}

fn validate_test_flight_url(raw: &str) -> AppResult<()> {
    let url = url::Url::parse(raw)
        .map_err(|error| AppError::Validation(format!("invalid URL: {error}")))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(AppError::Validation("URL must use http or https".into()));
    }
    if url.host_str().is_none() {
        return Err(AppError::Validation("URL must include a host".into()));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(AppError::Validation(
            "URL must not include embedded credentials".into(),
        ));
    }
    Ok(())
}

fn parse_download_client_config_json(raw: &str) -> AppResult<Value> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_str::<Value>(trimmed).map_err(|error| {
        AppError::Validation(format!("invalid download client config JSON: {error}"))
    })
}

fn read_config_string(config: &Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(value) = config.get(*key).and_then(Value::as_str) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn read_config_bool(config: &Value, keys: &[&str], default_value: bool) -> bool {
    for key in keys {
        if let Some(value) = config.get(*key) {
            if let Some(bool_value) = value.as_bool() {
                return bool_value;
            }
            if let Some(raw_string) = value.as_str() {
                let normalized = raw_string.trim().to_ascii_lowercase();
                if normalized == "true" || normalized == "1" || normalized == "yes" {
                    return true;
                }
                if normalized == "false" || normalized == "0" || normalized == "no" {
                    return false;
                }
            }
        }
    }
    default_value
}

/// Build a base URL from config_json component parts (host, port, use_ssl, url_base).
pub fn resolve_download_client_base_url(json_config: &Value) -> Option<String> {
    let host = read_config_string(json_config, &["host"])?;
    let port = read_config_string(json_config, &["port"]);
    let use_ssl = read_config_bool(json_config, &["use_ssl", "useSsl"], false);
    let url_base = read_config_string(json_config, &["url_base", "urlBase"]);

    let mut value = String::new();
    if use_ssl {
        value.push_str("https://");
    } else {
        value.push_str("http://");
    }
    value.push_str(&host);

    if let Some(port_value) = port
        && !port_value.is_empty()
    {
        value.push(':');
        value.push_str(&port_value);
    }

    if let Some(path_value) = url_base {
        let normalized_path = path_value.trim_start_matches('/');
        if !normalized_path.is_empty() {
            value.push('/');
            value.push_str(normalized_path);
        }
    }

    Some(value)
}

// ---------------------------------------------------------------------------
// Shared helpers used by multiple download client implementations
// ---------------------------------------------------------------------------

pub struct StagedNzbLease {
    pub staged_nzb: StagedNzbRef,
    pub self_staged: bool,
    store: Arc<dyn StagedNzbStore>,
    _permit: Option<OwnedSemaphorePermit>,
}

impl Drop for StagedNzbLease {
    fn drop(&mut self) {
        if let Err(error) = self
            .store
            .mark_artifact_inactive(&self.staged_nzb.compressed_path)
        {
            tracing::warn!(
                path = %self.staged_nzb.compressed_path.display(),
                error = %error,
                "failed to mark staged nzb artifact inactive"
            );
        }
    }
}

pub fn extract_i64_value(value: Option<&Value>) -> Option<i64> {
    value.and_then(|value| {
        value.as_i64().or_else(|| {
            value
                .as_str()
                .and_then(|raw| raw.trim().parse::<i64>().ok())
        })
    })
}

pub fn extract_f64_value(value: Option<&Value>) -> Option<f64> {
    value.and_then(|value| {
        value.as_f64().or_else(|| {
            value
                .as_str()
                .and_then(|raw| raw.trim().parse::<f64>().ok())
        })
    })
}

pub fn size_to_bytes(size_mb: f64) -> Option<i64> {
    if !size_mb.is_finite() {
        return None;
    }
    if size_mb <= 0.0 {
        return Some(0);
    }
    let bytes = (size_mb * 1_048_576f64).round() as i64;
    Some(bytes.max(0))
}

pub fn progress_percent_from_sizes(size_mb: f64, remaining_mb: f64) -> u8 {
    if size_mb <= 0.0 || !size_mb.is_finite() || !remaining_mb.is_finite() {
        return 0;
    }

    let completed_mb = (size_mb - remaining_mb).clamp(0.0, size_mb);
    if completed_mb <= 0.0 {
        return 0;
    }

    let percent = ((completed_mb / size_mb) * 100.0).round();
    let clamped = if percent.is_nan() {
        0.0
    } else {
        percent.clamp(0.0, 100.0)
    };
    clamped as u8
}

pub fn parse_duration_seconds(raw: &str) -> Option<i64> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Ok(seconds) = trimmed.parse::<i64>() {
        return Some(seconds.max(0));
    }

    let mut parts = trimmed.split(':');
    let first = parts.next()?;
    let second = parts.next()?;
    let third = parts.next();
    if parts.next().is_some() {
        return None;
    }

    let (hours, minutes, seconds) = if let Some(third_part) = third {
        let hours = first.parse::<i64>().ok()?;
        let minutes = second.parse::<i64>().ok()?;
        let seconds = third_part.parse::<i64>().ok()?;
        (hours, minutes, seconds)
    } else {
        let minutes = first.parse::<i64>().ok()?;
        let seconds = second.parse::<i64>().ok()?;
        (0, minutes, seconds)
    };

    if hours < 0 || minutes < 0 || seconds < 0 || minutes >= 60 || seconds >= 60 {
        return None;
    }

    Some(hours * 3600 + minutes * 60 + seconds)
}

pub fn is_http_url(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("https://")
}

fn is_nzb_root_name(name: &str) -> bool {
    name.rsplit(':')
        .next()
        .is_some_and(|local_name| local_name == "nzb")
}

struct MpscChunkReader {
    receiver: tokio::sync::mpsc::Receiver<Vec<u8>>,
    current: Cursor<Vec<u8>>,
    closed: bool,
}

impl MpscChunkReader {
    fn new(receiver: tokio::sync::mpsc::Receiver<Vec<u8>>) -> Self {
        Self {
            receiver,
            current: Cursor::new(Vec::new()),
            closed: false,
        }
    }
}

impl Read for MpscChunkReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        loop {
            let position = self.current.position() as usize;
            if position < self.current.get_ref().len() {
                return self.current.read(buf);
            }
            if self.closed {
                return Ok(0);
            }
            match self.receiver.blocking_recv() {
                Some(chunk) => self.current = Cursor::new(chunk),
                None => {
                    self.closed = true;
                    return Ok(0);
                }
            }
        }
    }
}

struct TeeZstdReader<R: Read> {
    inner: R,
    encoder: zstd::stream::Encoder<'static, std::io::BufWriter<std::fs::File>>,
}

impl<R: Read> TeeZstdReader<R> {
    fn new(inner: R, output_path: &Path) -> AppResult<Self> {
        let file = std::fs::File::create(output_path).map_err(|error| {
            AppError::Repository(format!(
                "failed to create staged nzb file {}: {error}",
                output_path.display()
            ))
        })?;
        let writer = std::io::BufWriter::new(file);
        let encoder =
            zstd::stream::Encoder::new(writer, STAGED_NZB_ZSTD_LEVEL).map_err(|error| {
                AppError::Repository(format!(
                    "failed to initialize staged nzb zstd stream: {error}"
                ))
            })?;
        Ok(Self { inner, encoder })
    }

    fn finish(mut self) -> AppResult<()> {
        self.encoder.flush().map_err(|error| {
            AppError::Repository(format!("failed to flush staged nzb encoder: {error}"))
        })?;
        let mut writer = self.encoder.finish().map_err(|error| {
            AppError::Repository(format!(
                "failed to finalize staged nzb zstd stream: {error}"
            ))
        })?;
        writer.flush().map_err(|error| {
            AppError::Repository(format!("failed to flush staged nzb artifact: {error}"))
        })?;
        Ok(())
    }
}

impl<R: Read> Read for TeeZstdReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let bytes_read = self.inner.read(buf)?;
        if bytes_read > 0 {
            self.encoder.write_all(&buf[..bytes_read])?;
        }
        Ok(bytes_read)
    }
}

fn validate_nzb_reader<R: BufRead>(mut reader: Reader<R>) -> AppResult<Reader<R>> {
    let mut event_buf = Vec::new();
    let mut saw_root = false;
    let mut depth = 0usize;

    loop {
        match reader.read_event_into(&mut event_buf) {
            Ok(Event::Decl(_))
            | Ok(Event::Comment(_))
            | Ok(Event::PI(_))
            | Ok(Event::DocType(_)) => {}
            Ok(Event::Text(text)) if !saw_root => {
                let text = quick_xml::escape::unescape(text.as_ref()).map_err(|err| {
                    AppError::Validation(format!("nzb XML text decode failed: {err}"))
                })?;
                if !text
                    .trim_matches(|ch: char| ch.is_whitespace() || ch == '\u{feff}')
                    .is_empty()
                {
                    return Err(AppError::Validation(
                        "nzb download payload did not look like xml".into(),
                    ));
                }
            }
            Ok(Event::Start(start)) if !saw_root => {
                if !is_nzb_root_name(start.name().as_ref()) {
                    return Err(AppError::Validation(
                        "nzb download payload root element must be <nzb>".into(),
                    ));
                }
                saw_root = true;
                depth = 1;
            }
            Ok(Event::Empty(start)) if !saw_root => {
                if !is_nzb_root_name(start.name().as_ref()) {
                    return Err(AppError::Validation(
                        "nzb download payload root element must be <nzb>".into(),
                    ));
                }
                saw_root = true;
                depth = 0;
            }
            Ok(Event::Start(_)) if saw_root => {
                depth += 1;
            }
            Ok(Event::End(_)) if saw_root => {
                depth = depth.saturating_sub(1);
            }
            Ok(Event::Eof) => {
                if !saw_root {
                    return Err(AppError::Validation(
                        "nzb download payload root element must be <nzb>".into(),
                    ));
                }
                if depth != 0 {
                    return Err(AppError::Validation(
                        "nzb download payload was not valid xml: unexpected end of file".into(),
                    ));
                }
                return Ok(reader);
            }
            Ok(_) => {}
            Err(error) => {
                return Err(AppError::Validation(format!(
                    "nzb download payload was not valid xml: {error}"
                )));
            }
        }
        event_buf.clear();
    }
}

fn stream_validate_and_compress_nzb(
    receiver: tokio::sync::mpsc::Receiver<Vec<u8>>,
    output_path: &Path,
) -> AppResult<()> {
    let source = MpscChunkReader::new(receiver);
    let tee = TeeZstdReader::new(source, output_path)?;
    let buf_reader = StdBufReader::new(tee);
    let mut reader = Reader::from_reader(buf_reader);
    reader.config_mut().trim_text(false);
    let reader = validate_nzb_reader(reader)?;
    let buf_reader = reader.into_inner();
    let tee = buf_reader.into_inner();
    tee.finish()
}

#[cfg(test)]
fn validate_nzb_xml(bytes: &[u8]) -> AppResult<()> {
    if bytes.is_empty() {
        return Err(AppError::Repository(
            "nzb download response body was empty".into(),
        ));
    }

    if bytes.len() as u64 > MAX_NZB_BYTES {
        return Err(AppError::Repository(format!(
            "nzb download payload exceeded {} bytes",
            MAX_NZB_BYTES
        )));
    }

    let mut reader = Reader::from_reader(Cursor::new(bytes));
    reader.config_mut().trim_text(false);
    validate_nzb_reader(reader).map(|_| ())
}

pub fn request_source_hint_for_nzb(request: &DownloadClientAddRequest) -> AppResult<String> {
    let source_hint = request
        .source_hint
        .clone()
        .and_then(|value| {
            let value = value.trim().to_string();
            (!value.is_empty()).then_some(value)
        })
        .ok_or_else(|| {
            AppError::Validation("source hint is required to queue a download".into())
        })?;

    if !is_http_url(&source_hint) {
        return Err(AppError::Validation(format!(
            "source hint must be an NZB URL; got {source_hint}"
        )));
    }

    Ok(source_hint)
}

/// Accumulates the leading bytes of a streaming NZB download so the
/// indexer-asserted `<meta type="category">` can be enforced against the
/// submitted subject before the payload reaches a download client
/// (plan 136 §6, Pillar D1).
struct NzbCategoryProbe<'a> {
    expected_facet: &'a MediaFacet,
    head_bytes: Vec<u8>,
    scanned_bytes: usize,
    decided: bool,
}

impl<'a> NzbCategoryProbe<'a> {
    fn new(expected_facet: &'a MediaFacet) -> Self {
        Self {
            expected_facet,
            head_bytes: Vec::new(),
            scanned_bytes: 0,
            decided: false,
        }
    }

    /// Feed a downloaded chunk, deciding as soon as the head has closed or the
    /// probe window is full.
    fn observe(&mut self, chunk: &[u8]) -> AppResult<()> {
        if self.decided {
            return Ok(());
        }

        let remaining = NZB_HEAD_PROBE_BYTES.saturating_sub(self.head_bytes.len());
        if remaining > 0 {
            self.head_bytes
                .extend_from_slice(&chunk[..remaining.min(chunk.len())]);
        }

        if self.head_bytes.len() >= NZB_HEAD_PROBE_BYTES || self.head_closed() {
            return self.finish();
        }

        Ok(())
    }

    /// Decide on whatever was collected, for payloads that ended before the
    /// head close tag or the probe window was reached.
    fn finish(&mut self) -> AppResult<()> {
        if self.decided {
            return Ok(());
        }
        self.decided = true;
        enforce_nzb_category_gate(&self.head_bytes, self.expected_facet)
    }

    /// Scan only the bytes added since the last call, keeping a tag-sized
    /// overlap so a close tag split across chunks is still found.
    fn head_closed(&mut self) -> bool {
        let scan_from = self
            .scanned_bytes
            .saturating_sub(NZB_HEAD_CLOSE_TAG.len().saturating_sub(1));
        let closed = self.head_bytes[scan_from..]
            .windows(NZB_HEAD_CLOSE_TAG.len())
            .any(|window| window.eq_ignore_ascii_case(NZB_HEAD_CLOSE_TAG));
        self.scanned_bytes = self.head_bytes.len();
        closed
    }
}

pub async fn stage_nzb_from_url(
    client: &OutboundHttpClient,
    store: &Arc<dyn StagedNzbStore>,
    pipeline_limit: &Arc<Semaphore>,
    url: &str,
    title_id: Option<&str>,
    expected_facet: &MediaFacet,
) -> AppResult<StagedNzbLease> {
    let permit = pipeline_limit
        .clone()
        .acquire_owned()
        .await
        .map_err(|error| {
            AppError::Repository(format!("failed to acquire nzb pipeline permit: {error}"))
        })?;

    let scope = nzb_download_scope(url);
    let response = client
        .send(
            RequestPolicy::safe_read(scope, "nzb_download")
                .with_max_retries(2)
                .with_backoff(
                    std::time::Duration::from_secs(1),
                    std::time::Duration::from_secs(15),
                ),
            || client.client().get(url).header("User-Agent", "scryer/0.1"),
        )
        .await
        .map_err(map_nzb_download_outbound_error)?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.map_err(|err| {
            AppError::Repository(format!("nzb download response read failed: {err}"))
        })?;
        let preview: String = body.chars().take(300).collect();
        return Err(AppError::Repository(format!(
            "nzb download failed with status {status}: {preview}"
        )));
    }

    let pending = store.create_pending_staged_nzb(url, title_id).await?;
    let partial_path = pending.partial_path.clone();
    let stage_result = async {
        let (validator_tx, validator_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(4);
        let validator_path = partial_path.clone();
        let mut validator_task = Some(tokio::task::spawn_blocking(move || {
            stream_validate_and_compress_nzb(validator_rx, &validator_path)
        }));
        let mut raw_size_bytes = 0u64;
        let mut stream = response.bytes_stream();
        let mut stream_result = Ok(());
        let mut category_probe = NzbCategoryProbe::new(expected_facet);

        while let Some(chunk_result) = stream.next().await {
            let chunk = match chunk_result {
                Ok(chunk) => chunk,
                Err(error) => {
                    stream_result = Err(AppError::Repository(format!(
                        "nzb download body read failed: {error}"
                    )));
                    break;
                }
            };
            if chunk.is_empty() {
                continue;
            }

            raw_size_bytes += chunk.len() as u64;
            if raw_size_bytes > MAX_NZB_BYTES {
                stream_result = Err(AppError::Repository(format!(
                    "nzb download payload exceeded {} bytes",
                    MAX_NZB_BYTES
                )));
                break;
            }

            // Pillar D1: the indexer's own category assertion lives in the
            // head, so it is decided as soon as the head has streamed in —
            // before the payload is ever handed to a download client.
            if let Err(error) = category_probe.observe(&chunk) {
                stream_result = Err(error);
                break;
            }

            if validator_tx.send(chunk.to_vec()).await.is_err() {
                let validator_result = validator_task
                    .take()
                    .expect("validator task should exist")
                    .await
                    .map_err(|error| {
                        AppError::Repository(format!("nzb validation task failed to join: {error}"))
                    })?;
                stream_result = match validator_result {
                    Ok(()) => Err(AppError::Repository(
                        "nzb validation task stopped before download completed".into(),
                    )),
                    Err(error) => Err(error),
                };
                break;
            }
        }

        drop(validator_tx);
        let validator_result = match validator_task.take() {
            Some(task) => task.await.map_err(|error| {
                AppError::Repository(format!("nzb validation task failed to join: {error}"))
            })?,
            None => Ok(()),
        };

        stream_result?;
        if raw_size_bytes == 0 {
            return Err(AppError::Repository(
                "nzb download response body was empty".into(),
            ));
        }
        validator_result?;
        // A payload shorter than the probe window never tripped the in-loop
        // check; decide it now that the whole head is known to be present.
        category_probe.finish()?;

        store
            .finalize_pending_staged_nzb(pending, raw_size_bytes)
            .await
    }
    .await;

    if let Err(error) = tokio::fs::remove_file(&partial_path).await
        && error.kind() != std::io::ErrorKind::NotFound
        && stage_result.is_err()
    {
        tracing::warn!(
            path = %partial_path.display(),
            error = %error,
            "failed to remove partial staged nzb artifact"
        );
    }

    let staged_nzb = stage_result?;
    store.mark_artifact_active(&staged_nzb.compressed_path)?;
    Ok(StagedNzbLease {
        staged_nzb,
        self_staged: false,
        store: Arc::clone(store),
        _permit: Some(permit),
    })
}

pub async fn stage_nzb_from_bytes(
    store: &Arc<dyn StagedNzbStore>,
    pipeline_limit: &Arc<Semaphore>,
    source_label: &str,
    title_id: Option<&str>,
    bytes: Vec<u8>,
) -> AppResult<StagedNzbLease> {
    if bytes.is_empty() {
        return Err(AppError::Repository(
            "resolved NZB download artifact was empty".into(),
        ));
    }
    if bytes.len() as u64 > MAX_NZB_BYTES {
        return Err(AppError::Repository(format!(
            "resolved NZB download artifact exceeded {} bytes",
            MAX_NZB_BYTES
        )));
    }

    let permit = pipeline_limit
        .clone()
        .acquire_owned()
        .await
        .map_err(|error| {
            AppError::Repository(format!("failed to acquire nzb pipeline permit: {error}"))
        })?;
    let pending = store
        .create_pending_staged_nzb(source_label, title_id)
        .await?;
    let partial_path = pending.partial_path.clone();
    let raw_size_bytes = bytes.len() as u64;
    let stage_result = async {
        let (validator_tx, validator_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(1);
        let validator_path = partial_path.clone();
        let validator_task = tokio::task::spawn_blocking(move || {
            stream_validate_and_compress_nzb(validator_rx, &validator_path)
        });
        validator_tx.send(bytes).await.map_err(|_| {
            AppError::Repository("nzb validation task stopped before artifact was staged".into())
        })?;
        drop(validator_tx);
        validator_task.await.map_err(|error| {
            AppError::Repository(format!("nzb validation task failed to join: {error}"))
        })??;
        store
            .finalize_pending_staged_nzb(pending, raw_size_bytes)
            .await
    }
    .await;

    if let Err(error) = tokio::fs::remove_file(&partial_path).await
        && error.kind() != std::io::ErrorKind::NotFound
        && stage_result.is_err()
    {
        tracing::warn!(
            path = %partial_path.display(),
            error = %error,
            "failed to remove partial staged nzb artifact"
        );
    }

    let staged_nzb = stage_result?;
    store.mark_artifact_active(&staged_nzb.compressed_path)?;
    Ok(StagedNzbLease {
        staged_nzb,
        self_staged: false,
        store: Arc::clone(store),
        _permit: Some(permit),
    })
}

pub async fn resolve_staged_nzb_for_request(
    client: &OutboundHttpClient,
    store: &Arc<dyn StagedNzbStore>,
    pipeline_limit: &Arc<Semaphore>,
    request: &DownloadClientAddRequest,
) -> AppResult<StagedNzbLease> {
    if let Some(staged_nzb) = request.staged_nzb.clone() {
        store.mark_artifact_active(&staged_nzb.compressed_path)?;
        return Ok(StagedNzbLease {
            staged_nzb,
            self_staged: false,
            store: Arc::clone(store),
            _permit: None,
        });
    }

    let source_hint = request_source_hint_for_nzb(request)?;
    let mut staged = stage_nzb_from_url(
        client,
        store,
        pipeline_limit,
        &source_hint,
        Some(&request.title.id),
        request
            .search_facet
            .as_ref()
            .unwrap_or(&request.title.facet),
    )
    .await?;
    staged.self_staged = true;
    Ok(staged)
}

fn map_nzb_download_outbound_error(error: OutboundHttpError) -> AppError {
    match error {
        OutboundHttpError::RateLimited(rate_limited) => {
            let retry_after = rate_limited.retry_after.filter(|delay| !delay.is_zero());
            AppError::rate_limited_temporary_unavailable(
                match retry_after {
                    Some(delay) => format!(
                        "nzb download request was rate limited; retry after {}s",
                        delay.as_secs()
                    ),
                    None => "nzb download request was rate limited".to_string(),
                },
                retry_after,
                RateLimitCooldownAction::AlreadyRecorded,
            )
        }
        OutboundHttpError::Transport { source, .. } => {
            AppError::Repository(format!("nzb download request failed: {source}"))
        }
    }
}

fn nzb_download_scope(url: &str) -> String {
    match reqwest::Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_string))
    {
        Some(host) => format!("nzb_download:{host}"),
        None => "nzb_download:unknown".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::Duration;

    use scryer_application::{AppError, StagedNzbStore};
    use scryer_outbound_http::OutboundHttpError;
    use tempfile::TempDir;

    use crate::downloads::staged_nzb_store::FileSystemStagedNzbStore;

    use super::{
        MAX_NZB_BYTES, map_nzb_download_outbound_error, stream_validate_and_compress_nzb,
        validate_nzb_xml,
    };

    #[test]
    fn nzb_download_outbound_rate_limit_preserves_retry_after() {
        let error = OutboundHttpError::RateLimited(scryer_outbound_http::RateLimitedError {
            scope: scryer_outbound_http::RateLimitScopeKey::from("nzb-download"),
            retry_after: Some(Duration::from_secs(50)),
            attempts: 1,
            retry_after_source: scryer_outbound_http::RetryAfterSource::Seconds,
            request_label: std::borrow::Cow::Borrowed("nzb download"),
        });
        let error = map_nzb_download_outbound_error(error);

        match error {
            AppError::TemporaryUnavailable {
                message,
                retry_after,
                ..
            } => {
                assert!(message.contains("retry after 50s"));
                assert_eq!(retry_after, Some(Duration::from_secs(50)));
            }
            other => panic!("expected temporary unavailable error, got {other:?}"),
        }
    }

    fn load_real_nzb_fixture_bytes() -> Vec<u8> {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let fixture_path = Path::new(manifest_dir)
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("tests")
            .join("fixtures")
            .join("nzbgeek")
            .join("nzb_content.xml");
        std::fs::read(&fixture_path).unwrap_or_else(|error| {
            panic!("failed to load fixture {}: {error}", fixture_path.display())
        })
    }

    async fn materialize_real_staged_nzb_fixture(
        output_root: &Path,
    ) -> scryer_application::AppResult<scryer_application::StagedNzbRef> {
        let store = FileSystemStagedNzbStore::new(output_root).await?;
        let pending = store
            .create_pending_staged_nzb(
                "https://example.invalid/nzbgeek/nzb_content.xml",
                Some("real-nzb-fixture"),
            )
            .await?;
        let partial_path = pending.partial_path.clone();
        let raw_nzb = load_real_nzb_fixture_bytes();
        let raw_size_bytes = raw_nzb.len() as u64;
        let (tx, rx) = tokio::sync::mpsc::channel::<Vec<u8>>(2);
        let validator = tokio::task::spawn_blocking(move || {
            stream_validate_and_compress_nzb(rx, &partial_path)
        });
        let chunk_sizes = [1usize, 2, 5, 3, 8, 13, 21];
        let mut offset = 0usize;
        let mut chunk_index = 0usize;

        while offset < raw_nzb.len() {
            let chunk_size = chunk_sizes[chunk_index % chunk_sizes.len()];
            let end = (offset + chunk_size).min(raw_nzb.len());
            tx.send(raw_nzb[offset..end].to_vec())
                .await
                .expect("fixture stream receiver should stay alive");
            offset = end;
            chunk_index += 1;
        }
        drop(tx);

        validator
            .await
            .expect("validator task should join successfully")?;
        store
            .finalize_pending_staged_nzb(pending, raw_size_bytes)
            .await
    }

    #[test]
    fn validate_nzb_xml_accepts_well_formed_nzb_root() {
        let bytes = br#"<?xml version="1.0" encoding="utf-8"?>
            <!-- comment -->
            <nzb xmlns="http://www.newzbin.com/DTD/2003/nzb"></nzb>"#;

        validate_nzb_xml(bytes).expect("valid nzb xml should pass");
    }

    #[test]
    fn validate_nzb_xml_rejects_malformed_xml() {
        let error = validate_nzb_xml(br#"<?xml version="1.0"?><nzb>"#)
            .expect_err("malformed xml should fail");

        assert!(error.to_string().contains("unexpected end of file"));
    }

    #[test]
    fn validate_nzb_xml_rejects_wrong_root_element() {
        let error = validate_nzb_xml(br#"<?xml version="1.0"?><rss></rss>"#)
            .expect_err("wrong root should fail");

        assert!(error.to_string().contains("root element must be <nzb>"));
    }

    #[test]
    fn validate_nzb_xml_accepts_utf8_bom_before_root() {
        let bytes = b"\xEF\xBB\xBF<?xml version=\"1.0\"?><nzb></nzb>";

        validate_nzb_xml(bytes).expect("utf-8 bom should be tolerated");
    }

    #[test]
    fn validate_nzb_xml_accepts_prefixed_nzb_root() {
        let bytes = br#"<?xml version="1.0"?><ns:nzb xmlns:ns="urn:test"></ns:nzb>"#;

        validate_nzb_xml(bytes).expect("prefixed nzb root should pass");
    }

    #[test]
    fn validate_nzb_xml_rejects_html_payload() {
        let error = validate_nzb_xml(br#"<!doctype html><html><body>nope</body></html>"#)
            .expect_err("html payload should fail");

        assert!(error.to_string().contains("root element must be <nzb>"));
    }

    #[test]
    fn validate_nzb_xml_rejects_empty_payload() {
        let error = validate_nzb_xml(b"").expect_err("empty payload should fail");

        assert!(error.to_string().contains("body was empty"));
    }

    #[test]
    fn validate_nzb_xml_rejects_oversized_payload() {
        let mut bytes = vec![b' '; MAX_NZB_BYTES as usize + 1];
        bytes[0] = b'<';

        let error = validate_nzb_xml(&bytes).expect_err("oversized payload should fail");

        assert!(error.to_string().contains("payload exceeded"));
    }

    #[tokio::test]
    async fn stage_nzb_from_url_follows_trusted_redirects() {
        use std::sync::Arc;

        use scryer_outbound_http::{
            OutboundHttpClient, RateLimitRegistry, no_redirect_reqwest_client,
        };
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let nzb_bytes = load_real_nzb_fixture_bytes();

        let target_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("target listener should bind");
        let target_addr = target_listener.local_addr().expect("target addr");
        let target_body = nzb_bytes.clone();
        tokio::spawn(async move {
            let (mut socket, _) = target_listener.accept().await.expect("accept target");
            let mut buf = [0u8; 1024];
            let _ = socket.read(&mut buf).await;
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/x-nzb\r\nContent-Length: {}\r\n\r\n",
                target_body.len()
            );
            socket
                .write_all(header.as_bytes())
                .await
                .expect("write target header");
            socket
                .write_all(&target_body)
                .await
                .expect("write target body");
        });

        let origin_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("origin listener should bind");
        let origin_addr = origin_listener.local_addr().expect("origin addr");
        tokio::spawn(async move {
            let (mut socket, _) = origin_listener.accept().await.expect("accept origin");
            let mut buf = [0u8; 1024];
            let _ = socket.read(&mut buf).await;
            let response = format!(
                "HTTP/1.1 301 Moved Permanently\r\nLocation: http://{target_addr}/download.nzb\r\nContent-Length: 0\r\n\r\n"
            );
            socket
                .write_all(response.as_bytes())
                .await
                .expect("write redirect");
        });

        let outbound =
            OutboundHttpClient::new(no_redirect_reqwest_client(), RateLimitRegistry::isolated());
        let tempdir = TempDir::new().expect("tempdir");
        let store: Arc<dyn StagedNzbStore> = Arc::new(
            FileSystemStagedNzbStore::new(tempdir.path())
                .await
                .expect("staged nzb store"),
        );
        let pipeline_limit = Arc::new(tokio::sync::Semaphore::new(1));

        let staged = super::stage_nzb_from_url(
            &outbound,
            &store,
            &pipeline_limit,
            &format!("http://{origin_addr}/getnzb?id=1"),
            Some("redirect-test"),
            &scryer_domain::MediaFacet::Movie,
        )
        .await
        .expect("nzb download should follow the 301 redirect");

        let compressed =
            std::fs::read(&staged.staged_nzb.compressed_path).expect("staged file should exist");
        let decompressed = zstd::stream::decode_all(std::io::Cursor::new(compressed))
            .expect("staged artifact should decode");
        assert_eq!(decompressed, nzb_bytes);
    }

    #[tokio::test]
    async fn real_nzb_fixture_round_trips_through_streaming_staged_zstd_pipeline() {
        let tempdir = TempDir::new().expect("tempdir");
        let staged = materialize_real_staged_nzb_fixture(tempdir.path())
            .await
            .expect("real fixture should stage successfully");
        let compressed = std::fs::read(&staged.compressed_path).expect("staged file should exist");
        let decompressed = zstd::stream::decode_all(std::io::Cursor::new(compressed))
            .expect("staged artifact should decode");

        assert_eq!(decompressed, load_real_nzb_fixture_bytes());
        assert!(
            staged
                .compressed_path
                .extension()
                .is_some_and(|ext| ext == "zst"),
            "staged artifact should use .zst extension"
        );
        assert!(
            staged
                .compressed_path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().ends_with(".nzb.zst")),
            "staged artifact should use the same .nzb.zst suffix as production"
        );
    }

    /// Head shape taken from the incident NZB: the indexer declared the
    /// release as anime, and Scryer submitted it for a live-action series.
    fn anime_categorized_nzb() -> Vec<u8> {
        br#"<?xml version="1.0" encoding="iso-8859-1" ?>
<nzb xmlns="http://www.newzbin.com/DTD/2003/nzb">
<head>
 <meta type="name">Tide.Chart.S02.DANiSH.JAPANESE.1080p.WEB.H264</meta>
 <meta type="title">Tide.Chart.S02.DANiSH.JAPANESE.1080p.WEB.H264</meta>
 <meta type="category">TV &gt; Anime</meta>
</head>
<file poster="poster@example.invalid" date="1700000000" subject="[1/1] - &quot;tide.chart.par2&quot; yEnc (1/1)">
<groups><group>alt.binaries.test</group></groups>
<segments><segment bytes="1024" number="1">segment@example.invalid</segment></segments>
</file>
</nzb>"#
            .to_vec()
    }

    async fn serve_nzb_once(body: Vec<u8>) -> std::net::SocketAddr {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("nzb listener should bind");
        let addr = listener.local_addr().expect("nzb listener addr");
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept nzb request");
            let mut buf = [0u8; 1024];
            let _ = socket.read(&mut buf).await;
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/x-nzb\r\nContent-Length: {}\r\n\r\n",
                body.len()
            );
            socket
                .write_all(header.as_bytes())
                .await
                .expect("write nzb header");
            socket.write_all(&body).await.expect("write nzb body");
        });

        addr
    }

    async fn stage_served_nzb(
        body: Vec<u8>,
        facet: scryer_domain::MediaFacet,
        tempdir: &TempDir,
    ) -> scryer_application::AppResult<super::StagedNzbLease> {
        use std::sync::Arc;

        use scryer_outbound_http::{
            OutboundHttpClient, RateLimitRegistry, no_redirect_reqwest_client,
        };

        let addr = serve_nzb_once(body).await;
        let outbound =
            OutboundHttpClient::new(no_redirect_reqwest_client(), RateLimitRegistry::isolated());
        let store: Arc<dyn StagedNzbStore> = Arc::new(
            FileSystemStagedNzbStore::new(tempdir.path())
                .await
                .expect("staged nzb store"),
        );
        let pipeline_limit = Arc::new(tokio::sync::Semaphore::new(1));

        super::stage_nzb_from_url(
            &outbound,
            &store,
            &pipeline_limit,
            &format!("http://{addr}/getnzb?id=1"),
            Some("category-gate-test"),
            &facet,
        )
        .await
    }

    #[tokio::test]
    async fn stage_nzb_from_url_blocks_an_anime_categorized_nzb_for_a_series_subject() {
        let tempdir = TempDir::new().expect("tempdir");

        let error = match stage_served_nzb(
            anime_categorized_nzb(),
            scryer_domain::MediaFacet::Series,
            &tempdir,
        )
        .await
        {
            Ok(_) => panic!("an anime-categorized nzb must not be staged for a series subject"),
            Err(error) => error,
        };

        assert!(
            error.to_string().contains("category_mismatch"),
            "gate error must name the category_mismatch code: {error}"
        );
        assert!(
            matches!(error, AppError::Validation(_)),
            "a category veto is definitive, never a failover-eligible transport error: {error:?}"
        );
    }

    #[tokio::test]
    async fn stage_nzb_from_url_allows_an_anime_categorized_nzb_for_an_anime_subject() {
        let tempdir = TempDir::new().expect("tempdir");

        let staged = stage_served_nzb(
            anime_categorized_nzb(),
            scryer_domain::MediaFacet::Anime,
            &tempdir,
        )
        .await
        .expect("the same bytes are exactly right for an anime subject");

        let compressed =
            std::fs::read(&staged.staged_nzb.compressed_path).expect("staged file should exist");
        let decompressed = zstd::stream::decode_all(std::io::Cursor::new(compressed))
            .expect("staged artifact should decode");
        assert_eq!(decompressed, anime_categorized_nzb());
    }

    #[tokio::test]
    async fn stage_nzb_from_url_allows_an_nzb_without_a_category_meta() {
        let tempdir = TempDir::new().expect("tempdir");

        stage_served_nzb(
            load_real_nzb_fixture_bytes(),
            scryer_domain::MediaFacet::Series,
            &tempdir,
        )
        .await
        .expect("an nzb that asserts no category stays permissive");
    }
}

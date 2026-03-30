mod multi_indexer;
mod nzbget;
mod router;
mod sabnzbd;
pub(crate) mod weaver;
pub mod weaver_subscription;

use std::io::{BufRead, BufReader as StdBufReader, Cursor, Read, Write};
use std::path::Path;
use std::sync::Arc;

use futures_util::StreamExt;
use quick_xml::Reader;
use quick_xml::events::Event;
use scryer_application::{
    AppError, AppResult, DownloadClientAddRequest, StagedNzbRef, StagedNzbStore,
};
use serde_json::{Value, json};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub use multi_indexer::MultiIndexerSearchClient;
pub use nzbget::NzbgetDownloadClient;
pub use router::PrioritizedDownloadClientRouter;
pub use sabnzbd::SabnzbdDownloadClient;
pub use weaver::WeaverDownloadClient;
pub use weaver_subscription::start_weaver_subscription_bridge;

const MAX_NZB_BYTES: u64 = 32 * 1024 * 1024;
const STAGED_NZB_ZSTD_LEVEL: i32 = 3;

/// Compute a base URL from host/port/use_ssl/url_base in a config_json string.
/// Public for use by the GraphQL mapper layer.
pub fn resolve_base_url_from_config_json(config_json: &str) -> Option<String> {
    let parsed = parse_download_client_config_json(config_json).ok()?;
    resolve_download_client_base_url(&parsed)
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

pub(crate) struct StagedNzbLease {
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

pub(crate) fn extract_i64_value(value: Option<&Value>) -> Option<i64> {
    value.and_then(|value| {
        value.as_i64().or_else(|| {
            value
                .as_str()
                .and_then(|raw| raw.trim().parse::<i64>().ok())
        })
    })
}

pub(crate) fn extract_f64_value(value: Option<&Value>) -> Option<f64> {
    value.and_then(|value| {
        value.as_f64().or_else(|| {
            value
                .as_str()
                .and_then(|raw| raw.trim().parse::<f64>().ok())
        })
    })
}

pub(crate) fn size_to_bytes(size_mb: f64) -> Option<i64> {
    if !size_mb.is_finite() {
        return None;
    }
    if size_mb <= 0.0 {
        return Some(0);
    }
    let bytes = (size_mb * 1_048_576f64).round() as i64;
    Some(bytes.max(0))
}

pub(crate) fn progress_percent_from_sizes(size_mb: f64, remaining_mb: f64) -> u8 {
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

pub(crate) fn parse_duration_seconds(raw: &str) -> Option<i64> {
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

pub(crate) fn is_http_url(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("https://")
}

fn is_nzb_root_name(name: &[u8]) -> bool {
    name.rsplit(|byte| *byte == b':')
        .next()
        .is_some_and(|local_name| local_name == b"nzb")
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
        let encoder = zstd::stream::Encoder::new(writer, STAGED_NZB_ZSTD_LEVEL).map_err(|error| {
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
                let text = text.unescape().map_err(|err| {
                    AppError::Repository(format!("nzb XML text decode failed: {err}"))
                })?;
                if !text
                    .trim_matches(|ch: char| ch.is_whitespace() || ch == '\u{feff}')
                    .is_empty()
                {
                    return Err(AppError::Repository(
                        "nzb download payload did not look like xml".into(),
                    ));
                }
            }
            Ok(Event::Start(start)) if !saw_root => {
                if !is_nzb_root_name(start.name().as_ref()) {
                    return Err(AppError::Repository(
                        "nzb download payload root element must be <nzb>".into(),
                    ));
                }
                saw_root = true;
                depth = 1;
            }
            Ok(Event::Empty(start)) if !saw_root => {
                if !is_nzb_root_name(start.name().as_ref()) {
                    return Err(AppError::Repository(
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
                    return Err(AppError::Repository(
                        "nzb download payload root element must be <nzb>".into(),
                    ));
                }
                if depth != 0 {
                    return Err(AppError::Repository(
                        "nzb download payload was not valid xml: unexpected end of file".into(),
                    ));
                }
                return Ok(reader);
            }
            Ok(_) => {}
            Err(error) => {
                return Err(AppError::Repository(format!(
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

pub(crate) fn request_source_hint_for_nzb(request: &DownloadClientAddRequest) -> AppResult<String> {
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

pub(crate) async fn stage_nzb_from_url(
    client: &reqwest::Client,
    store: &Arc<dyn StagedNzbStore>,
    pipeline_limit: &Arc<Semaphore>,
    url: &str,
    title_id: Option<&str>,
) -> AppResult<StagedNzbLease> {
    let permit = pipeline_limit
        .clone()
        .acquire_owned()
        .await
        .map_err(|error| {
            AppError::Repository(format!("failed to acquire nzb pipeline permit: {error}"))
        })?;

    let response = client
        .get(url)
        .header("User-Agent", "scryer/0.1")
        .send()
        .await
        .map_err(|err| AppError::Repository(format!("nzb download request failed: {err}")))?;

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

            if validator_tx.send(chunk.to_vec()).await.is_err() {
                let validator_result = validator_task
                    .take()
                    .expect("validator task should exist")
                    .await
                    .map_err(|error| {
                        AppError::Repository(format!(
                            "nzb validation task failed to join: {error}"
                        ))
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

pub(crate) async fn resolve_staged_nzb_for_request(
    client: &reqwest::Client,
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
    )
    .await?;
    staged.self_staged = true;
    Ok(staged)
}

#[cfg(test)]
mod tests {
    use super::{MAX_NZB_BYTES, validate_nzb_xml};

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
}

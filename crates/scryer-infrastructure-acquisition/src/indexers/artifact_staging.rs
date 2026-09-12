use std::io::{BufRead, BufReader as StdBufReader, Cursor, Read, Write};
use std::path::Path;
use std::sync::Arc;

use futures_util::{Stream, StreamExt};
use quick_xml::Reader;
use quick_xml::events::Event;
use scryer_application::{
    AppError, AppResult, NZB_HEAD_PROBE_BYTES, StagedNzbStore, enforce_nzb_category_gate,
};
use scryer_domain::MediaFacet;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

use crate::downloads::clients::StagedNzbLease;

pub const MAX_NZB_BYTES: u64 = 32 * 1024 * 1024;
const STAGED_NZB_ZSTD_LEVEL: i32 = 3;
const NZB_HEAD_CLOSE_TAG: &[u8] = b"</head>";

pub enum BufferedOrStagedNzb {
    Buffered(Vec<u8>),
    Staged(StagedNzbLease),
}

struct PartialArtifactCleanup(std::path::PathBuf);

impl Drop for PartialArtifactCleanup {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_file(&self.0)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(path = %self.0.display(), %error, "failed to remove partial staged nzb artifact");
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "shares the streaming staging context, plus optional HTTP request accounting"
)]
pub async fn stage_or_buffer_nzb_response(
    response: reqwest::Response,
    tally: Option<&scryer_application::IndexerRequestTally>,
    store: &Arc<dyn StagedNzbStore>,
    pipeline_limit: &Arc<Semaphore>,
    source_label: &str,
    title_id: Option<&str>,
    expected_facet: Option<&MediaFacet>,
    cancellation: &CancellationToken,
) -> AppResult<BufferedOrStagedNzb> {
    let mut captured = scryer_application::CapturedIndexerHttpResponse {
        status: response.status().as_u16(),
        headers: response
            .headers()
            .iter()
            .map(
                |(key, value)| scryer_application::CapturedIndexerHttpHeader {
                    name: key.to_string(),
                    value: value.as_bytes().to_vec(),
                },
            )
            .collect(),
        body: Vec::new(),
    };
    let mut body_failed = false;
    let result = async {
        let status = response.status();
        if !status.is_success() {
            return Err(AppError::Repository(format!(
                "nzb download failed with status {status}"
            )));
        }
        let mut stream = response.bytes_stream().inspect(|chunk| match chunk {
            Ok(bytes) => {
                let remaining = (64 * 1024usize).saturating_sub(captured.body.len());
                captured
                    .body
                    .extend_from_slice(&bytes[..remaining.min(bytes.len())]);
            }
            Err(_) => body_failed = true,
        });
        let first = read_artifact_prefix(&mut stream, cancellation).await?;
        if first
            .iter()
            .find(|byte| !byte.is_ascii_whitespace())
            .is_some_and(|byte| matches!(*byte, b'd' | b'm' | b'M'))
        {
            let mut bytes = first;
            while let Some(chunk) = tokio::select! {
                _ = cancellation.cancelled() => return cancellation_error(),
                next = stream.next() => next,
            } {
                let chunk = chunk.map_err(|error| {
                    AppError::Repository(format!("nzb download body read failed: {error}"))
                })?;
                if bytes.len().saturating_add(chunk.len()) > MAX_NZB_BYTES as usize {
                    return Err(AppError::Repository(format!(
                        "download artifact payload exceeded {} bytes",
                        MAX_NZB_BYTES
                    )));
                }
                bytes.extend_from_slice(&chunk);
            }
            return Ok(BufferedOrStagedNzb::Buffered(bytes));
        }
        let stream = futures_util::stream::iter(vec![Ok::<_, reqwest::Error>(first)])
            .chain(stream.map(|chunk| chunk.map(|bytes| bytes.to_vec())));
        Ok(BufferedOrStagedNzb::Staged(
            stage_nzb_from_stream(
                stream,
                store,
                pipeline_limit,
                source_label,
                title_id,
                expected_facet,
                cancellation,
            )
            .await?,
        ))
    }
    .await;
    if let Some(tally) = tally {
        if cancellation.is_cancelled() {
            tally.note_result(scryer_application::IndexerRequestResult::Cancelled);
        } else if body_failed {
            tally.note_result(scryer_application::IndexerRequestResult::NoResponse);
        } else {
            tally.note_response(&captured);
        }
    }
    result
}

/// Wait for the first non-whitespace byte without depending on HTTP chunk boundaries.
async fn read_artifact_prefix<S, B, E>(
    stream: &mut S,
    cancellation: &CancellationToken,
) -> AppResult<Vec<u8>>
where
    S: Stream<Item = Result<B, E>> + Unpin,
    B: AsRef<[u8]>,
    E: std::fmt::Display,
{
    let mut prefix = Vec::new();
    while let Some(chunk) = tokio::select! {
        _ = cancellation.cancelled() => return cancellation_error(),
        next = stream.next() => next,
    } {
        let chunk = chunk.map_err(|error| {
            AppError::Repository(format!("nzb download body read failed: {error}"))
        })?;
        let chunk = chunk.as_ref();
        if prefix.len().saturating_add(chunk.len()) > MAX_NZB_BYTES as usize {
            return Err(AppError::Repository(format!(
                "download artifact payload exceeded {} bytes",
                MAX_NZB_BYTES
            )));
        }
        prefix.extend_from_slice(chunk);
        if chunk.iter().any(|byte| !byte.is_ascii_whitespace()) {
            return Ok(prefix);
        }
    }
    if prefix.is_empty() {
        return Err(AppError::Repository(
            "nzb download response body was empty".into(),
        ));
    }
    Ok(prefix)
}

pub async fn stage_nzb_from_bytes(
    store: &Arc<dyn StagedNzbStore>,
    pipeline_limit: &Arc<Semaphore>,
    source_label: &str,
    title_id: Option<&str>,
    expected_facet: Option<&MediaFacet>,
    cancellation: &CancellationToken,
    bytes: Vec<u8>,
) -> AppResult<StagedNzbLease> {
    stage_nzb_from_stream(
        futures_util::stream::iter(vec![Ok::<_, std::io::Error>(bytes)]),
        store,
        pipeline_limit,
        source_label,
        title_id,
        expected_facet,
        cancellation,
    )
    .await
}

async fn stage_nzb_from_stream<S, B, E>(
    stream: S,
    store: &Arc<dyn StagedNzbStore>,
    pipeline_limit: &Arc<Semaphore>,
    source_label: &str,
    title_id: Option<&str>,
    expected_facet: Option<&MediaFacet>,
    cancellation: &CancellationToken,
) -> AppResult<StagedNzbLease>
where
    S: Stream<Item = Result<B, E>> + Unpin,
    B: AsRef<[u8]>,
    E: std::fmt::Display,
{
    let permit = tokio::select! {
        _ = cancellation.cancelled() => return cancellation_error(),
        result = pipeline_limit.clone().acquire_owned() => result.map_err(|error| {
            AppError::Repository(format!("failed to acquire nzb pipeline permit: {error}"))
        })?,
    };
    let pending = store
        .create_pending_staged_nzb(source_label, title_id)
        .await?;
    let partial_path = pending.partial_path.clone();
    let partial_cleanup = Arc::new(PartialArtifactCleanup(partial_path.clone()));
    let stage_result = async {
        let (validator_tx, validator_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(4);
        let validator_path = partial_path.clone();
        let validator_cleanup = Arc::clone(&partial_cleanup);
        let validator_task = tokio::task::spawn_blocking(move || {
            // Keep cleanup alive until the writer closes, even if the async
            // resolution future is dropped while validation is in progress.
            let _cleanup = validator_cleanup;
            stream_validate_and_compress_nzb(validator_rx, &validator_path)
        });
        let mut stream = stream;
        let mut raw_size_bytes = 0u64;
        let mut category_probe = expected_facet.map(NzbCategoryProbe::new);
        let mut error_probe = Vec::new();

        let stream_result: AppResult<()> = loop {
            let next = tokio::select! {
                _ = cancellation.cancelled() => break cancellation_error(),
                next = stream.next() => next,
            };
            let Some(chunk) = next else {
                break Ok(());
            };
            let chunk = match chunk {
                Ok(chunk) => chunk,
                Err(error) => {
                    break Err(AppError::Repository(format!(
                        "nzb download body read failed: {error}"
                    )));
                }
            };
            let bytes = chunk.as_ref();
            if bytes.is_empty() {
                continue;
            }
            raw_size_bytes = raw_size_bytes.saturating_add(bytes.len() as u64);
            if raw_size_bytes > MAX_NZB_BYTES {
                break Err(AppError::Repository(format!(
                    "nzb download payload exceeded {} bytes",
                    MAX_NZB_BYTES
                )));
            }
            let remaining = 4096usize.saturating_sub(error_probe.len());
            error_probe.extend_from_slice(&bytes[..remaining.min(bytes.len())]);
            if let Some(error) = newznab_quota_error(&error_probe) {
                break Err(error);
            }
            if let Some(probe) = category_probe.as_mut()
                && let Err(error) = probe.observe(bytes)
            {
                break Err(error);
            }
            tokio::select! {
                _ = cancellation.cancelled() => break cancellation_error(),
                sent = validator_tx.send(bytes.to_vec()) => match sent {
                    Ok(()) => {},
                    Err(_) => break Err(AppError::Repository(
                        "nzb validation task stopped before download completed".into()
                    )),
                },
            }
        };
        drop(validator_tx);
        let validator_result = validator_task.await.map_err(|error| {
            AppError::Repository(format!("nzb validation task failed to join: {error}"))
        })?;
        stream_result?;
        validator_result?;
        if raw_size_bytes == 0 {
            return Err(AppError::Repository(
                "nzb download response body was empty".into(),
            ));
        }
        if let Some(probe) = category_probe.as_mut() {
            probe.finish()?;
        }
        store
            .finalize_pending_staged_nzb(pending, raw_size_bytes)
            .await
    }
    .await;

    if stage_result.is_err()
        && let Err(error) = tokio::fs::remove_file(&partial_path).await
        && error.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(path = %partial_path.display(), %error, "failed to remove partial staged nzb artifact");
    }

    let staged_nzb = stage_result?;
    store.mark_artifact_active(&staged_nzb.compressed_path)?;
    Ok(StagedNzbLease::new(
        staged_nzb,
        Arc::clone(store),
        Some(permit),
    ))
}

fn cancellation_error<T>() -> AppResult<T> {
    Err(AppError::TemporaryUnavailable {
        message: "nzb artifact resolution was cancelled".into(),
        retry_after: None,
        rate_limit_cooldown: scryer_application::RateLimitCooldownAction::None,
    })
}

fn newznab_quota_error(bytes: &[u8]) -> Option<AppError> {
    let head = std::str::from_utf8(&bytes[..bytes.len().min(4096)]).ok()?;
    let error = head.get(head.find("<error")?..)?;
    let code = attribute(error, "code")?.parse::<u16>().ok()?;
    if !matches!(code, 500 | 501) {
        return None;
    }
    Some(AppError::NewznabQuotaExceeded {
        code,
        message: attribute(error, "description")
            .unwrap_or("Indexer quota has been exceeded")
            .to_string(),
    })
}

fn attribute<'a>(value: &'a str, name: &str) -> Option<&'a str> {
    let start = value.find(name)? + name.len();
    let value = value.get(start..)?.trim_start();
    let value = value.strip_prefix('=')?.trim_start();
    let quote = value.chars().next()?;
    let value = value.strip_prefix(quote)?;
    value.split(quote).next()
}

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

    fn observe(&mut self, chunk: &[u8]) -> AppResult<()> {
        if self.decided {
            return Ok(());
        }
        let remaining = NZB_HEAD_PROBE_BYTES.saturating_sub(self.head_bytes.len());
        self.head_bytes
            .extend_from_slice(&chunk[..remaining.min(chunk.len())]);
        if self.head_bytes.len() >= NZB_HEAD_PROBE_BYTES || self.head_closed() {
            self.finish()?;
        }
        Ok(())
    }

    fn finish(&mut self) -> AppResult<()> {
        if !self.decided {
            self.decided = true;
            enforce_nzb_category_gate(&self.head_bytes, self.expected_facet)?;
        }
        Ok(())
    }

    fn head_closed(&mut self) -> bool {
        let scan_from = self
            .scanned_bytes
            .saturating_sub(NZB_HEAD_CLOSE_TAG.len() - 1);
        let closed = self.head_bytes[scan_from..]
            .windows(NZB_HEAD_CLOSE_TAG.len())
            .any(|window| window.eq_ignore_ascii_case(NZB_HEAD_CLOSE_TAG));
        self.scanned_bytes = self.head_bytes.len();
        closed
    }
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
            if (self.current.position() as usize) < self.current.get_ref().len() {
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
        let encoder =
            zstd::stream::Encoder::new(std::io::BufWriter::new(file), STAGED_NZB_ZSTD_LEVEL)
                .map_err(|error| {
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

fn stream_validate_and_compress_nzb(
    receiver: tokio::sync::mpsc::Receiver<Vec<u8>>,
    output_path: &Path,
) -> AppResult<()> {
    let source = MpscChunkReader::new(receiver);
    let tee = TeeZstdReader::new(source, output_path)?;
    let mut reader = Reader::from_reader(StdBufReader::new(tee));
    reader.config_mut().trim_text(false);
    let reader = validate_nzb_reader(reader)?;
    reader.into_inner().into_inner().finish()
}

fn validate_nzb_reader<R: BufRead>(mut reader: Reader<R>) -> AppResult<Reader<R>> {
    let mut event_buf = Vec::new();
    let mut saw_root = false;
    let mut depth = 0usize;
    loop {
        match reader.read_event_into(&mut event_buf) {
            Ok(Event::Comment(_)) | Ok(Event::PI(_)) => {}
            Ok(Event::Decl(_)) | Ok(Event::DocType(_)) if !saw_root => {}
            Ok(Event::Text(text)) if depth == 0 => {
                let text = quick_xml::escape::unescape(text.as_ref()).map_err(|error| {
                    AppError::Validation(format!("nzb XML text decode failed: {error}"))
                })?;
                if !text
                    .trim_matches(|ch: char| {
                        matches!(ch, ' ' | '\t' | '\r' | '\n') || (!saw_root && ch == '\u{feff}')
                    })
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
            }
            Ok(Event::Start(_)) if depth > 0 => depth += 1,
            Ok(Event::End(_)) if depth > 0 => depth -= 1,
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
            Ok(_) if depth == 0 => {
                return Err(AppError::Validation(
                    "nzb download payload contains content outside its root element".into(),
                ));
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use futures_util::{StreamExt, stream};
    use scryer_application::{AppError, StagedNzbStore};
    use tempfile::TempDir;
    use tokio::sync::Semaphore;
    use tokio_util::sync::CancellationToken;

    use crate::downloads::staged_nzb_store::FileSystemStagedNzbStore;

    use super::{MAX_NZB_BYTES, stage_nzb_from_bytes, stage_nzb_from_stream};

    async fn store(tempdir: &TempDir) -> Arc<dyn StagedNzbStore> {
        Arc::new(
            FileSystemStagedNzbStore::new(tempdir.path())
                .await
                .expect("store"),
        )
    }

    fn valid_nzb() -> Vec<u8> {
        br#"<?xml version="1.0"?><nzb><head><meta type="category">TV</meta></head><file/></nzb>"#
            .to_vec()
    }

    #[tokio::test]
    async fn rejects_content_after_closed_nzb_root_in_streams_and_buffers() {
        for root in ["<nzb/>", "<nzb></nzb>"] {
            for suffix in [
                "garbage",
                "<nzb/>",
                "<extra></extra>",
                "<![CDATA[text]]>",
                "&#65;",
                "<?xml version=\"1.0\"?>",
            ] {
                let tempdir = TempDir::new().unwrap();
                let store = store(&tempdir).await;
                let limit = Arc::new(Semaphore::new(1));
                let cancel = CancellationToken::new();
                let bytes = format!("{root}{suffix}").into_bytes();
                assert!(
                    stage_nzb_from_bytes(&store, &limit, "fixture", None, None, &cancel, bytes)
                        .await
                        .is_err()
                );
                let chunks = stream::iter(vec![
                    Ok::<_, std::io::Error>(root.as_bytes()),
                    Ok(suffix.as_bytes()),
                ]);
                assert!(
                    stage_nzb_from_stream(chunks, &store, &limit, "fixture", None, None, &cancel)
                        .await
                        .is_err()
                );
                assert!(!contains_partial(tempdir.path()));
            }
            let xml = format!("{root} \n<!-- trailing comment --><?done ok?>");
            let reader = quick_xml::Reader::from_str(&xml);
            assert!(super::validate_nzb_reader(reader).is_ok());
        }
    }

    fn contains_partial(path: &std::path::Path) -> bool {
        std::fs::read_dir(path)
            .expect("staging directory")
            .flatten()
            .any(|entry| {
                let path = entry.path();
                path.is_dir() && contains_partial(&path)
                    || path
                        .file_name()
                        .is_some_and(|name| name.to_string_lossy().ends_with(".part"))
            })
    }

    #[tokio::test]
    async fn artifact_prefix_preserves_fragmented_whitespace_and_magnets() {
        for marker in ["magnet:", "MAGNET:", "MaGnEt:"] {
            let body =
                format!(" \r\n\t{marker}?xt=urn:btih:0123456789012345678901234567890123456789");
            for split in 0..body.len() {
                let mut chunks = stream::iter(vec![
                    Ok::<_, std::io::Error>(&body.as_bytes()[..split]),
                    Ok(&body.as_bytes()[split..]),
                ]);
                let mut bytes = super::read_artifact_prefix(&mut chunks, &CancellationToken::new())
                    .await
                    .unwrap();
                assert!(
                    bytes
                        .iter()
                        .find(|b| !b.is_ascii_whitespace())
                        .unwrap()
                        .eq_ignore_ascii_case(&b'm')
                );
                while let Some(chunk) = chunks.next().await {
                    bytes.extend_from_slice(chunk.unwrap());
                }
                let artifact = crate::indexers::artifact_transport::classify(
                    "fixture", None, None, bytes, None,
                )
                .unwrap();
                assert!(matches!(
                    artifact,
                    scryer_application::ResolvedDownloadArtifact::Magnet { .. }
                ));
            }
        }
        let mut oversized = stream::iter(vec![Ok::<_, std::io::Error>(vec![
            b' ';
            super::MAX_NZB_BYTES
                as usize
                + 1
        ])]);
        assert!(
            super::read_artifact_prefix(&mut oversized, &CancellationToken::new())
                .await
                .is_err()
        );
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let mut stalled = stream::pending::<Result<Vec<u8>, std::io::Error>>();
        assert!(
            super::read_artifact_prefix(&mut stalled, &cancellation)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn generic_http_artifacts_preserve_torrents_and_body_magnets() {
        for body in [
            b"d4:infod4:name4:testee".to_vec(),
            b"magnet:?xt=urn:btih:0123456789012345678901234567890123456789".to_vec(),
            b" \r\nMAGNET:?xt=urn:btih:0123456789012345678901234567890123456789".to_vec(),
        ] {
            let server = wiremock::MockServer::start().await;
            wiremock::Mock::given(wiremock::matchers::method("GET"))
                .respond_with(wiremock::ResponseTemplate::new(200).set_body_bytes(body.clone()))
                .expect(1)
                .mount(&server)
                .await;
            let tempdir = TempDir::new().unwrap();
            let store = store(&tempdir).await;
            let response = scryer_outbound_http::generic_reqwest_client()
                .get(server.uri())
                .send()
                .await
                .unwrap();
            let result = super::stage_or_buffer_nzb_response(
                response,
                None,
                &store,
                &Arc::new(Semaphore::new(1)),
                "generic",
                None,
                None,
                &CancellationToken::new(),
            )
            .await
            .unwrap();
            assert!(matches!(result, super::BufferedOrStagedNzb::Buffered(bytes) if bytes == body));
            assert!(!contains_partial(tempdir.path()));
        }
    }

    #[tokio::test]
    async fn dropping_resolution_cleans_partial_after_validator_exits() {
        let tempdir = TempDir::new().unwrap();
        let store = store(&tempdir).await;
        let task = tokio::spawn(async move {
            let chunks = stream::iter(vec![Ok::<_, std::io::Error>(b"<nzb>".to_vec())])
                .chain(stream::pending());
            stage_nzb_from_stream(
                chunks,
                &store,
                &Arc::new(Semaphore::new(1)),
                "aborted",
                None,
                None,
                &CancellationToken::new(),
            )
            .await
        });
        tokio::time::timeout(Duration::from_secs(2), async {
            while !contains_partial(tempdir.path()) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("writer creates partial artifact");
        task.abort();
        assert!(task.await.is_err());
        tokio::time::timeout(Duration::from_secs(2), async {
            while contains_partial(tempdir.path()) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("writer must close and remove partial after future drop");
    }

    #[tokio::test]
    async fn streaming_valid_nzb_is_zstd_staged() {
        let tempdir = TempDir::new().expect("tempdir");
        let store = store(&tempdir).await;
        let bytes = valid_nzb();
        let chunks = stream::iter(vec![
            Ok::<_, std::io::Error>(bytes[..19].to_vec()),
            Ok(bytes[19..].to_vec()),
        ]);
        let lease = stage_nzb_from_stream(
            chunks,
            &store,
            &Arc::new(Semaphore::new(1)),
            "streaming-fixture",
            None,
            None,
            &CancellationToken::new(),
        )
        .await
        .expect("valid stream stages");
        let compressed = std::fs::read(&lease.staged_nzb.compressed_path).expect("staged zstd");
        assert_eq!(
            zstd::stream::decode_all(std::io::Cursor::new(compressed)).unwrap(),
            bytes
        );
    }

    #[tokio::test]
    async fn malformed_and_category_denied_nzbs_do_not_stage() {
        let tempdir = TempDir::new().expect("tempdir");
        let store = store(&tempdir).await;
        let limit = Arc::new(Semaphore::new(1));
        let malformed = match stage_nzb_from_bytes(
            &store,
            &limit,
            "malformed",
            None,
            None,
            &CancellationToken::new(),
            b"<nzb>".to_vec(),
        )
        .await
        {
            Ok(_) => panic!("malformed XML must reject"),
            Err(error) => error,
        };
        assert!(malformed.to_string().contains("unexpected end of file"));

        let denied = match stage_nzb_from_bytes(
            &store,
            &limit,
            "category",
            None,
            Some(&scryer_domain::MediaFacet::Movie),
            &CancellationToken::new(),
            valid_nzb(),
        )
        .await
        {
            Ok(_) => panic!("category assertion must reject"),
            Err(error) => error,
        };
        assert!(matches!(denied, AppError::Validation(_)));
    }

    #[tokio::test]
    async fn oversize_and_cancellation_leave_no_partial_artifact() {
        let tempdir = TempDir::new().expect("tempdir");
        let store = store(&tempdir).await;
        let limit = Arc::new(Semaphore::new(1));
        let oversize = match stage_nzb_from_bytes(
            &store,
            &limit,
            "oversize",
            None,
            None,
            &CancellationToken::new(),
            vec![b'x'; MAX_NZB_BYTES as usize + 1],
        )
        .await
        {
            Ok(_) => panic!("oversize payload must reject"),
            Err(error) => error,
        };
        assert!(oversize.to_string().contains("exceeded"));

        let cancellation = CancellationToken::new();
        let canceller = cancellation.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            canceller.cancel();
        });
        let interrupted = stream::iter(vec![Ok::<_, std::io::Error>(
            b"<?xml version=\"1.0\"?><nzb>".to_vec(),
        )])
        .chain(stream::pending());
        let error = match stage_nzb_from_stream(
            interrupted,
            &store,
            &limit,
            "cancelled",
            None,
            None,
            &cancellation,
        )
        .await
        {
            Ok(_) => panic!("cancellation must interrupt stream"),
            Err(error) => error,
        };
        assert!(matches!(error, AppError::TemporaryUnavailable { .. }));
        assert!(!contains_partial(tempdir.path()));
    }
}

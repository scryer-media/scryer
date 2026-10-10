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
                let chunk = chunk.map_err(body_read_failed)?;
                if bytes.len().saturating_add(chunk.len()) > MAX_NZB_BYTES as usize {
                    return Err(payload_exceeded("download artifact"));
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
        let chunk = chunk.map_err(body_read_failed)?;
        let chunk = chunk.as_ref();
        if prefix.len().saturating_add(chunk.len()) > MAX_NZB_BYTES as usize {
            return Err(payload_exceeded("download artifact"));
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

/// Stage an NZB body, inflating it first when it arrives compressed.
///
/// Some indexers serve stored `.nzb.gz` files as a plain download (no
/// `Content-Encoding`), so the HTTP client never decodes them. The body's
/// leading bytes decide: gzip, zlib and zstd are inflated inline, and the
/// decompressed NZB goes through the same size cap, category gate and XML
/// validation as a plain one.
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
    S: Stream<Item = Result<B, E>> + Unpin + Send,
    B: AsRef<[u8]> + Send,
    E: std::fmt::Display,
{
    let (packaging, head, rest) = sniff_nzb_stream(stream, cancellation).await?;
    let raw = futures_util::stream::iter(head.into_iter().map(Ok))
        .chain(rest.map(|chunk| chunk.map_err(body_read_failed)));
    match packaging {
        NzbPackaging::Plain => {
            stage_validated_nzb_stream(
                raw,
                store,
                pipeline_limit,
                source_label,
                title_id,
                expected_facet,
                cancellation,
            )
            .await
        }
        NzbPackaging::Compressed(compression) => {
            stage_validated_nzb_stream(
                Box::pin(decode_nzb_stream(compression, raw)),
                store,
                pipeline_limit,
                source_label,
                title_id,
                expected_facet,
                cancellation,
            )
            .await
        }
        NzbPackaging::Unsupported(format) => Err(unsupported_packaging(format)),
    }
}

/// How a downloaded NZB body is packaged, judged from its leading bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NzbPackaging {
    /// Not a recognised compression; XML validation decides what it is.
    Plain,
    /// A stream compression inflated inline before validation.
    Compressed(NzbCompression),
    /// A compression or archive format staging does not open.
    Unsupported(&'static str),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NzbCompression {
    Gzip,
    Zlib,
    Zstd,
}

impl NzbCompression {
    fn label(self) -> &'static str {
        match self {
            Self::Gzip => "gzip",
            Self::Zlib => "zlib",
            Self::Zstd => "zstd",
        }
    }
}

/// Leading-byte signatures, longest first where one could prefix another.
/// Archive containers (xz, zip, 7z, RAR) and bzip2 are recognised only to
/// refuse them with a clear message.
const NZB_PACKAGING_SIGNATURES: &[(&[u8], NzbPackaging)] = &[
    (
        &[0x1f, 0x8b],
        NzbPackaging::Compressed(NzbCompression::Gzip),
    ),
    (
        &[0x78, 0x01],
        NzbPackaging::Compressed(NzbCompression::Zlib),
    ),
    (
        &[0x78, 0x5e],
        NzbPackaging::Compressed(NzbCompression::Zlib),
    ),
    (
        &[0x78, 0x9c],
        NzbPackaging::Compressed(NzbCompression::Zlib),
    ),
    (
        &[0x78, 0xda],
        NzbPackaging::Compressed(NzbCompression::Zlib),
    ),
    (
        &[0x28, 0xb5, 0x2f, 0xfd],
        NzbPackaging::Compressed(NzbCompression::Zstd),
    ),
    (b"BZh", NzbPackaging::Unsupported("bzip2")),
    (
        &[0xfd, b'7', b'z', b'X', b'Z', 0x00],
        NzbPackaging::Unsupported("xz"),
    ),
    (b"PK\x03\x04", NzbPackaging::Unsupported("zip")),
    (
        &[b'7', b'z', 0xbc, 0xaf, 0x27, 0x1c],
        NzbPackaging::Unsupported("7z"),
    ),
    (b"Rar!\x1a\x07", NzbPackaging::Unsupported("RAR")),
];
const NZB_PACKAGING_SNIFF_BYTES: usize = 6;

/// `None` while `prefix` is too short to tell and more bytes may follow.
fn sniff_nzb_packaging(prefix: &[u8], complete: bool) -> Option<NzbPackaging> {
    if let Some((_, packaging)) = NZB_PACKAGING_SIGNATURES
        .iter()
        .find(|(signature, _)| prefix.starts_with(signature))
    {
        return Some(*packaging);
    }
    if !complete
        && NZB_PACKAGING_SIGNATURES
            .iter()
            .any(|(signature, _)| signature.starts_with(prefix))
    {
        return None;
    }
    Some(NzbPackaging::Plain)
}

/// Read just enough of `stream` to classify its packaging, keeping the
/// chunks read so the caller can replay them.
async fn sniff_nzb_stream<S, B, E>(
    mut stream: S,
    cancellation: &CancellationToken,
) -> AppResult<(NzbPackaging, Vec<B>, S)>
where
    S: Stream<Item = Result<B, E>> + Unpin,
    B: AsRef<[u8]>,
    E: std::fmt::Display,
{
    let mut head = Vec::new();
    let mut prefix = Vec::with_capacity(NZB_PACKAGING_SNIFF_BYTES);
    loop {
        if let Some(packaging) = sniff_nzb_packaging(&prefix, false) {
            return Ok((packaging, head, stream));
        }
        let next = tokio::select! {
            _ = cancellation.cancelled() => return cancellation_error(),
            next = stream.next() => next,
        };
        match next {
            None => {
                let packaging = sniff_nzb_packaging(&prefix, true).unwrap_or(NzbPackaging::Plain);
                return Ok((packaging, head, stream));
            }
            Some(Err(error)) => return Err(body_read_failed(error)),
            Some(Ok(chunk)) => {
                let bytes = chunk.as_ref();
                let take = NZB_PACKAGING_SNIFF_BYTES
                    .saturating_sub(prefix.len())
                    .min(bytes.len());
                prefix.extend_from_slice(&bytes[..take]);
                head.push(chunk);
            }
        }
    }
}

/// A failure that happened before decompression: carried through the
/// decoder's `io::Error` so it surfaces as itself, not as corrupt input.
#[derive(Debug)]
struct RawNzbBodyError(AppError);

impl std::fmt::Display for RawNzbBodyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::error::Error for RawNzbBodyError {}

/// Inflate `raw` as it streams. The compressed input is capped at
/// [`MAX_NZB_BYTES`] here; the inflated output is capped by the staging loop
/// that consumes it, so a decompression bomb stops at the same limit as a
/// plain oversized NZB.
fn decode_nzb_stream<'a, S, B>(
    compression: NzbCompression,
    raw: S,
) -> impl Stream<Item = AppResult<Vec<u8>>> + Send + 'a
where
    S: Stream<Item = AppResult<B>> + Send + 'a,
    B: AsRef<[u8]> + Send + 'a,
{
    use async_compression::tokio::bufread::{GzipDecoder, ZlibDecoder, ZstdDecoder};

    let mut compressed_bytes = 0u64;
    let input = raw.map(move |chunk| {
        let chunk = chunk.map_err(|error| std::io::Error::other(RawNzbBodyError(error)))?;
        let bytes = chunk.as_ref();
        compressed_bytes = compressed_bytes.saturating_add(bytes.len() as u64);
        if compressed_bytes > MAX_NZB_BYTES {
            return Err(std::io::Error::other(RawNzbBodyError(payload_exceeded(
                "compressed nzb download",
            ))));
        }
        Ok(Cursor::new(bytes.to_vec()))
    });
    let reader = tokio_util::io::StreamReader::new(input);
    let decoder: std::pin::Pin<Box<dyn tokio::io::AsyncRead + Send + 'a>> = match compression {
        NzbCompression::Gzip => {
            let mut decoder = GzipDecoder::new(reader);
            decoder.multiple_members(true);
            Box::pin(decoder)
        }
        NzbCompression::Zlib => Box::pin(ZlibDecoder::new(reader)),
        NzbCompression::Zstd => {
            let mut decoder = ZstdDecoder::new(reader);
            decoder.multiple_members(true);
            Box::pin(decoder)
        }
    };
    tokio_util::io::ReaderStream::new(decoder).map(move |chunk| {
        chunk
            .map(|bytes| bytes.to_vec())
            .map_err(|error| decoded_chunk_error(compression, error))
    })
}

fn decoded_chunk_error(compression: NzbCompression, error: std::io::Error) -> AppError {
    if !error
        .get_ref()
        .is_some_and(|inner| inner.is::<RawNzbBodyError>())
    {
        return AppError::Validation(format!(
            "nzb download payload is {}-compressed but could not be decompressed: {error}",
            compression.label()
        ));
    }
    match error
        .into_inner()
        .map(|inner| inner.downcast::<RawNzbBodyError>())
    {
        Some(Ok(raw)) => raw.0,
        _ => AppError::Repository("nzb download body error was lost while decoding".into()),
    }
}

/// Inflate a gzip-, zlib- or zstd-compressed NZB held in memory, such as one
/// an indexer plugin returned from its own fetch. Any other payload comes back
/// unchanged for the caller to classify.
pub(crate) async fn inflate_compressed_nzb(bytes: Vec<u8>) -> AppResult<Vec<u8>> {
    let Some(NzbPackaging::Compressed(compression)) = sniff_nzb_packaging(&bytes, true) else {
        return Ok(bytes);
    };
    let mut decoded_stream = std::pin::pin!(decode_nzb_stream(
        compression,
        futures_util::stream::iter([Ok::<_, AppError>(bytes)]),
    ));
    let mut decoded = Vec::new();
    while let Some(chunk) = decoded_stream.next().await {
        let chunk = chunk?;
        if decoded.len().saturating_add(chunk.len()) > MAX_NZB_BYTES as usize {
            return Err(payload_exceeded("nzb download"));
        }
        decoded.extend_from_slice(&chunk);
    }
    Ok(decoded)
}

/// Archive containers need the archive-extractor plugin and a workspace on
/// disk, which NZB staging does not have; say so rather than reporting the
/// payload as malformed XML.
fn unsupported_packaging(format: &str) -> AppError {
    AppError::Validation(format!(
        "nzb download payload is a {format} file; only plain, gzip, zlib or zstd NZBs can be staged"
    ))
}

async fn stage_validated_nzb_stream<S, B>(
    stream: S,
    store: &Arc<dyn StagedNzbStore>,
    pipeline_limit: &Arc<Semaphore>,
    source_label: &str,
    title_id: Option<&str>,
    expected_facet: Option<&MediaFacet>,
    cancellation: &CancellationToken,
) -> AppResult<StagedNzbLease>
where
    S: Stream<Item = AppResult<B>> + Unpin,
    B: AsRef<[u8]>,
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
                Err(error) => break Err(error),
            };
            let bytes = chunk.as_ref();
            if bytes.is_empty() {
                continue;
            }
            raw_size_bytes = raw_size_bytes.saturating_add(bytes.len() as u64);
            if raw_size_bytes > MAX_NZB_BYTES {
                break Err(payload_exceeded("nzb download"));
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

/// The indexer's response body broke off mid-stream (a reset connection, a
/// truncated chunked body, a decode failure).
///
/// A transport failure at the indexer, reported the way every other artifact
/// transport failure is — a timeout or a 5xx from the same fetch is already
/// [`AppError::DownloadSubmitUnavailable`] — so it is retryable rather than
/// burning the release, and its text reaches the operator instead of being
/// masked as an internal repository error.
fn body_read_failed(error: impl std::fmt::Display) -> AppError {
    AppError::DownloadSubmitUnavailable(format!("nzb download body read failed: {error}"))
}

/// The indexer served more than [`MAX_NZB_BYTES`]. A property of the artifact
/// itself, so a retry would fetch the same oversized payload: a validation
/// failure the operator can read, not a masked repository error.
fn payload_exceeded(label: &str) -> AppError {
    AppError::Validation(format!("{label} payload exceeded {MAX_NZB_BYTES} bytes"))
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
        tokio::time::timeout(Duration::from_secs(30), async {
            while !contains_partial(tempdir.path()) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("writer creates partial artifact");
        task.abort();
        assert!(task.await.is_err());
        tokio::time::timeout(Duration::from_secs(30), async {
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

    async fn compress(compression: super::NzbCompression, bytes: &[u8]) -> Vec<u8> {
        use async_compression::tokio::bufread::{GzipEncoder, ZlibEncoder, ZstdEncoder};
        use tokio::io::AsyncReadExt;

        let mut output = Vec::new();
        match compression {
            super::NzbCompression::Gzip => GzipEncoder::new(bytes).read_to_end(&mut output).await,
            super::NzbCompression::Zlib => ZlibEncoder::new(bytes).read_to_end(&mut output).await,
            super::NzbCompression::Zstd => ZstdEncoder::new(bytes).read_to_end(&mut output).await,
        }
        .expect("compress fixture");
        output
    }

    const COMPRESSIONS: [super::NzbCompression; 3] = [
        super::NzbCompression::Gzip,
        super::NzbCompression::Zlib,
        super::NzbCompression::Zstd,
    ];

    fn staged_bytes(lease: &crate::downloads::clients::StagedNzbLease) -> Vec<u8> {
        let compressed = std::fs::read(&lease.staged_nzb.compressed_path).expect("staged zstd");
        zstd::stream::decode_all(std::io::Cursor::new(compressed)).unwrap()
    }

    #[tokio::test]
    async fn compressed_nzbs_are_inflated_before_staging() {
        for compression in COMPRESSIONS {
            let compressed = compress(compression, &valid_nzb()).await;
            let tempdir = TempDir::new().unwrap();
            let store = store(&tempdir).await;
            let limit = Arc::new(Semaphore::new(1));

            let lease = stage_nzb_from_bytes(
                &store,
                &limit,
                "compressed-bytes",
                None,
                Some(&scryer_domain::MediaFacet::Series),
                &CancellationToken::new(),
                compressed.clone(),
            )
            .await
            .unwrap_or_else(|error| panic!("{compression:?} bytes must stage: {error}"));
            assert_eq!(staged_bytes(&lease), valid_nzb(), "{compression:?}");
            drop(lease);

            // The signature split across chunk boundaries still sniffs.
            let chunks = stream::iter(vec![
                Ok::<_, std::io::Error>(compressed[..1].to_vec()),
                Ok(compressed[1..2].to_vec()),
                Ok(compressed[2..].to_vec()),
            ]);
            let lease = stage_nzb_from_stream(
                chunks,
                &store,
                &limit,
                "compressed-stream",
                None,
                None,
                &CancellationToken::new(),
            )
            .await
            .unwrap_or_else(|error| panic!("{compression:?} stream must stage: {error}"));
            assert_eq!(staged_bytes(&lease), valid_nzb(), "{compression:?}");
        }
    }

    #[tokio::test]
    async fn concatenated_gzip_members_inflate_as_one_nzb() {
        let nzb = valid_nzb();
        let (first, second) = nzb.split_at(nzb.len() / 2);
        let mut members = compress(super::NzbCompression::Gzip, first).await;
        members.extend(compress(super::NzbCompression::Gzip, second).await);
        assert_eq!(super::inflate_compressed_nzb(members).await.unwrap(), nzb);
    }

    #[tokio::test]
    async fn compressed_nzbs_still_face_the_category_gate_and_xml_validation() {
        let tempdir = TempDir::new().unwrap();
        let store = store(&tempdir).await;
        let limit = Arc::new(Semaphore::new(1));
        let denied = stage_nzb_from_bytes(
            &store,
            &limit,
            "compressed-category",
            None,
            Some(&scryer_domain::MediaFacet::Movie),
            &CancellationToken::new(),
            compress(super::NzbCompression::Gzip, &valid_nzb()).await,
        )
        .await
        .err()
        .expect("category gate must reject the inflated NZB");
        assert!(matches!(denied, AppError::Validation(_)), "{denied:?}");

        let not_nzb = stage_nzb_from_bytes(
            &store,
            &limit,
            "compressed-html",
            None,
            None,
            &CancellationToken::new(),
            compress(
                super::NzbCompression::Zlib,
                b"<html><body>login</body></html>",
            )
            .await,
        )
        .await
        .err()
        .expect("an inflated non-NZB must reject");
        assert!(not_nzb.to_string().contains("root element must be <nzb>"));
        assert!(!contains_partial(tempdir.path()));
    }

    #[tokio::test]
    async fn decompression_past_the_nzb_limit_is_refused() {
        let mut bomb = b"<nzb>".to_vec();
        bomb.resize(MAX_NZB_BYTES as usize + 1, b' ');
        bomb.extend_from_slice(b"</nzb>");
        let compressed = compress(super::NzbCompression::Zstd, &bomb).await;
        assert!((compressed.len() as u64) < MAX_NZB_BYTES / 100);

        let tempdir = TempDir::new().unwrap();
        let store = store(&tempdir).await;
        let error = stage_nzb_from_bytes(
            &store,
            &Arc::new(Semaphore::new(1)),
            "bomb",
            None,
            None,
            &CancellationToken::new(),
            compressed.clone(),
        )
        .await
        .err()
        .expect("an NZB inflating past the limit must reject");
        assert!(
            matches!(&error, AppError::Validation(message) if message.contains("payload exceeded")),
            "{error:?}"
        );
        assert!(!contains_partial(tempdir.path()));

        let error = super::inflate_compressed_nzb(compressed)
            .await
            .expect_err("in-memory inflation is capped too");
        assert!(matches!(error, AppError::Validation(_)), "{error:?}");
    }

    #[tokio::test]
    async fn corrupt_compression_and_archives_are_reported_clearly() {
        let tempdir = TempDir::new().unwrap();
        let store = store(&tempdir).await;
        let limit = Arc::new(Semaphore::new(1));

        let mut truncated = compress(super::NzbCompression::Gzip, &valid_nzb()).await;
        truncated.truncate(truncated.len() / 2);
        let mut garbled = compress(super::NzbCompression::Zstd, &valid_nzb()).await;
        let last = garbled.len() - 1;
        garbled[4..last].iter_mut().for_each(|byte| *byte ^= 0x5a);
        for (label, payload) in [("gzip", truncated), ("zstd", garbled)] {
            let error = stage_nzb_from_bytes(
                &store,
                &limit,
                "corrupt",
                None,
                None,
                &CancellationToken::new(),
                payload,
            )
            .await
            .err()
            .expect("corrupt compressed payload must reject");
            assert!(
                matches!(&error, AppError::Validation(message)
                    if message.contains(&format!("{label}-compressed"))),
                "{label}: {error:?}"
            );
        }

        for (label, payload) in [
            ("zip", b"PK\x03\x04synthetic".to_vec()),
            ("xz", b"\xfd7zXZ\x00synthetic".to_vec()),
            ("7z", b"7z\xbc\xaf\x27\x1csynthetic".to_vec()),
            ("RAR", b"Rar!\x1a\x07\x00synthetic".to_vec()),
            ("bzip2", b"BZh91AY&SYsynthetic".to_vec()),
        ] {
            let error = stage_nzb_from_bytes(
                &store,
                &limit,
                "archive",
                None,
                None,
                &CancellationToken::new(),
                payload,
            )
            .await
            .err()
            .expect("archive payload must reject");
            assert!(
                matches!(&error, AppError::Validation(message)
                    if message.contains(&format!("a {label} file"))),
                "{label}: {error:?}"
            );
        }
        assert!(!contains_partial(tempdir.path()));
    }

    #[tokio::test]
    async fn a_broken_compressed_body_stays_a_retryable_transport_failure() {
        let tempdir = TempDir::new().unwrap();
        let store = store(&tempdir).await;
        let compressed = compress(super::NzbCompression::Gzip, &valid_nzb()).await;
        let chunks = stream::iter(vec![
            Ok::<_, std::io::Error>(compressed[..8].to_vec()),
            Err(std::io::Error::new(
                std::io::ErrorKind::ConnectionReset,
                "connection reset",
            )),
        ]);
        let error = stage_nzb_from_stream(
            chunks,
            &store,
            &Arc::new(Semaphore::new(1)),
            "broken",
            None,
            None,
            &CancellationToken::new(),
        )
        .await
        .err()
        .expect("a broken body must reject");
        assert!(
            matches!(&error, AppError::DownloadSubmitUnavailable(message)
                if message.contains("body read failed")),
            "{error:?}"
        );
        assert!(!contains_partial(tempdir.path()));
    }

    #[tokio::test]
    async fn plain_and_non_nzb_payloads_pass_through_inflation_unchanged() {
        for payload in [
            valid_nzb(),
            b"d4:infod4:name4:testee".to_vec(),
            b"x".to_vec(),
            Vec::new(),
        ] {
            assert_eq!(
                super::inflate_compressed_nzb(payload.clone())
                    .await
                    .unwrap(),
                payload
            );
        }
        let inflated = super::inflate_compressed_nzb(
            compress(super::NzbCompression::Gzip, &valid_nzb()).await,
        )
        .await
        .unwrap();
        assert!(matches!(
            crate::indexers::artifact_transport::classify("fixture", None, None, inflated, None)
                .unwrap(),
            scryer_application::ResolvedDownloadArtifact::Nzb { .. }
        ));
    }

    #[tokio::test]
    async fn gzip_file_served_without_content_encoding_is_staged() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .insert_header("content-type", "application/gzip")
                    .set_body_bytes(compress(super::NzbCompression::Gzip, &valid_nzb()).await),
            )
            .expect(1)
            .mount(&server)
            .await;
        let tempdir = TempDir::new().unwrap();
        let store = store(&tempdir).await;
        let response = scryer_outbound_http::generic_reqwest_client()
            .get(format!("{}/nzbs/1/synthetic.nzb.gz", server.uri()))
            .send()
            .await
            .unwrap();
        let result = super::stage_or_buffer_nzb_response(
            response,
            None,
            &store,
            &Arc::new(Semaphore::new(1)),
            "gzip-file",
            None,
            None,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        let super::BufferedOrStagedNzb::Staged(lease) = result else {
            panic!("a gzip-served NZB must stage");
        };
        assert_eq!(staged_bytes(&lease), valid_nzb());
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

    /// An oversized or truncated artifact is the indexer's fault, and the
    /// operator who clicked grab sees the message. `AppError::Repository` is
    /// masked as "Internal server error" at the API, so neither may be one: an
    /// oversized payload is a validation failure (a retry fetches the same
    /// bytes), a body that broke off mid-stream is a retryable transport
    /// failure like the artifact fetch's own timeouts.
    #[tokio::test]
    async fn oversized_and_truncated_artifacts_are_reported_rather_than_masked() {
        let tempdir = TempDir::new().expect("tempdir");
        let store = store(&tempdir).await;
        let limit = Arc::new(Semaphore::new(1));

        let oversized_stream = stream::iter(vec![
            Ok::<_, std::io::Error>(b"<?xml version=\"1.0\"?><nzb>".to_vec()),
            Ok(vec![b' '; MAX_NZB_BYTES as usize]),
        ]);
        let error = match stage_nzb_from_stream(
            oversized_stream,
            &store,
            &limit,
            "oversized",
            None,
            None,
            &CancellationToken::new(),
        )
        .await
        {
            Ok(_) => panic!("an oversized stream must reject"),
            Err(error) => error,
        };
        assert!(
            matches!(&error, AppError::Validation(message) if message.contains("payload exceeded")),
            "{error:?}"
        );
        assert!(!contains_partial(tempdir.path()));

        let oversized_chunk = vec![b' '; MAX_NZB_BYTES as usize + 1];
        let mut oversized_prefix = stream::iter(vec![Ok::<_, std::io::Error>(oversized_chunk)]);
        let error = super::read_artifact_prefix(&mut oversized_prefix, &CancellationToken::new())
            .await
            .expect_err("an oversized prefix must reject");
        assert!(matches!(error, AppError::Validation(_)), "{error:?}");

        let truncated = stream::iter(vec![
            Ok::<_, std::io::Error>(b"<?xml version=\"1.0\"?><nzb>".to_vec()),
            Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "error decoding response body",
            )),
        ]);
        let error = match stage_nzb_from_stream(
            truncated,
            &store,
            &limit,
            "truncated",
            None,
            None,
            &CancellationToken::new(),
        )
        .await
        {
            Ok(_) => panic!("a truncated stream must reject"),
            Err(error) => error,
        };
        assert!(
            matches!(
                &error,
                AppError::DownloadSubmitUnavailable(message)
                    if message.contains("body read failed") && message.contains("decoding")
            ),
            "{error:?}"
        );
        assert!(error.is_retryable_download_submit_failure());
        assert!(!contains_partial(tempdir.path()));

        let mut broken_prefix = stream::iter(vec![Err::<Vec<u8>, _>(std::io::Error::new(
            std::io::ErrorKind::ConnectionReset,
            "connection reset",
        ))]);
        let error = super::read_artifact_prefix(&mut broken_prefix, &CancellationToken::new())
            .await
            .expect_err("a broken prefix must reject");
        assert!(
            matches!(error, AppError::DownloadSubmitUnavailable(_)),
            "{error:?}"
        );
    }
}

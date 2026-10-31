//! Shared compatibility endpoints; the client key alone selects source grants.

mod feed;
mod request;
mod response;
mod stream;

use axum::{
    Router,
    body::Body,
    extract::{RawQuery, State},
    http::{HeaderMap, header},
    response::{IntoResponse, Response},
    routing::get,
};
use request::{Format, Operation};
use scryer_application::{
    AppError, AppResult, AppUseCase, ResolvedDownloadArtifact,
    error_reasons::indexer_clients as reasons,
    indexer_clients::{
        IndexerClientArtifactReference, IndexerClientProtocol, IndexerClientUseOutcome,
    },
};
use scryer_infrastructure_acquisition::indexers::nab_caps::{write_json_caps, write_xml_caps};
use scryer_infrastructure_crypto::artifact_reference::ArtifactReferenceCipher;
use std::sync::Arc;

pub(crate) const NEWZNAB_PATH: &str = "/newznab/api";
pub(crate) const TORZNAB_PATH: &str = "/torznab/api";

pub(crate) fn is_endpoint(path: &str) -> bool {
    matches!(path, NEWZNAB_PATH | TORZNAB_PATH)
}

#[derive(Clone)]
pub(crate) struct NabState {
    pub app: AppUseCase,
    pub cipher: Arc<ArtifactReferenceCipher>,
    pub base_path: String,
    pub tls: bool,
}

pub(crate) fn router(state: NabState) -> Router {
    Router::new()
        .route(NEWZNAB_PATH, get(newznab))
        .route(TORZNAB_PATH, get(torznab))
        .with_state(state)
}

async fn newznab(
    State(state): State<NabState>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
) -> Response {
    handle(
        state,
        headers,
        query.unwrap_or_default(),
        IndexerClientProtocol::Newznab,
    )
    .await
}

async fn torznab(
    State(state): State<NabState>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
) -> Response {
    handle(
        state,
        headers,
        query.unwrap_or_default(),
        IndexerClientProtocol::Torznab,
    )
    .await
}

fn invalid_key() -> AppError {
    AppError::unauthorized_reason(reasons::KEY_INVALID, "Invalid indexer client key")
}

fn invalid_reference() -> AppError {
    AppError::unauthorized_reason(reasons::REFERENCE_INVALID, "Invalid artifact reference")
}

fn unavailable() -> AppError {
    AppError::temporary_unavailable_reason(
        reasons::UNAVAILABLE,
        "Compatibility response unavailable",
        None,
    )
}

async fn handle(
    state: NabState,
    headers: HeaderMap,
    query: String,
    protocol: IndexerClientProtocol,
) -> Response {
    let mut format = Format::Xml;
    // Also honor JSON when authentication or another parameter is invalid.
    if url::form_urlencoded::parse(query.as_bytes())
        .any(|(key, value)| key == "o" && value == "json")
    {
        format = Format::Json;
    }
    let reporting = state.app.clone();
    let mut usage = None;
    let mut success_outcome = IndexerClientUseOutcome::CapsReady;
    let result = async {
        let request = request::parse(&headers, &query, protocol)?;
        format = request.format;
        if let Operation::Get(reference) = request.operation {
            let plaintext = state.cipher.open(&reference)?;
            let reference: IndexerClientArtifactReference =
                serde_json::from_slice(&plaintext).map_err(|_| invalid_reference())?;
            if let Some(key) = request.key {
                let authenticated = state.app.authenticate_indexer_client(&key).await?;
                reference.require_authenticated_client(&authenticated, protocol)?;
            }
            return artifact_response(
                state
                    .app
                    .fetch_indexer_client_artifact(&reference, protocol)
                    .await?,
                protocol,
            );
        }
        let authenticated = Arc::new(
            state
                .app
                .authenticate_indexer_client(request.key.as_deref().ok_or_else(invalid_key)?)
                .await?,
        );
        usage = Some(authenticated.clone());
        match request.operation {
            Operation::Caps => {
                let caps = state
                    .app
                    .indexer_client_caps(&authenticated, protocol)
                    .await?;
                let mut count = ByteCount(0);
                serde_json::to_writer(&mut count, &caps).map_err(|_| unavailable())?;
                let scratch = count.0.saturating_mul(32).saturating_add(16 * 1024);
                state.app.revalidate_indexer_client(&authenticated).await?;
                let body =
                    stream::serialize_owned(
                        state.app.clone(),
                        scratch,
                        move |output| match format {
                            Format::Xml => write_xml_caps(output, &caps),
                            Format::Json => write_json_caps(output, &caps),
                        },
                    )
                    .await?;
                Ok(document(body, format))
            }
            Operation::Search(search) => {
                let endpoint = endpoint_url(&state, &headers, protocol)?;
                let page = state
                    .app
                    .search_indexer_client(&authenticated, search)
                    .await?;
                success_outcome = if page
                    .sources
                    .iter()
                    .any(|source| source.pending || source.stopped)
                {
                    IndexerClientUseOutcome::SearchPartial
                } else {
                    IndexerClientUseOutcome::SearchReady
                };
                // The shared core's deadline-short page is a successful partial
                // feed. Pending sources continue without moving emitted rows.
                let body = feed::body(
                    page,
                    authenticated.clone(),
                    protocol,
                    format,
                    endpoint,
                    state.cipher,
                )
                .await?;
                state.app.revalidate_indexer_client(&authenticated).await?;
                Ok(document(body, format))
            }
            Operation::Get(_) => unreachable!("retrieval handled before full-key authentication"),
        }
    }
    .await;
    if let Some(authenticated) = usage {
        reporting
            .record_indexer_client_use(
                &authenticated,
                match &result {
                    Ok(_) => success_outcome,
                    Err(error) => IndexerClientUseOutcome::from_error(error),
                },
            )
            .await;
    }
    result.unwrap_or_else(|error| response::error_response(&error, format))
}

fn document(body: Body, format: Format) -> Response {
    let content_type = match format {
        Format::Xml => "application/xml; charset=utf-8",
        Format::Json => "application/json",
    };
    (
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "no-store"),
        ],
        body,
    )
        .into_response()
}

struct ByteCount(usize);
impl std::io::Write for ByteCount {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 = self
            .0
            .checked_add(bytes.len())
            .ok_or_else(|| std::io::Error::other("Response too large"))?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn request_scheme(headers: &HeaderMap, tls: bool) -> &str {
    headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .filter(|value| matches!(*value, "http" | "https"))
        .unwrap_or(if tls { "https" } else { "http" })
}

#[test]
fn forwarded_scheme_is_validated_before_the_tls_fallback() {
    let mut headers = HeaderMap::new();
    assert_eq!(request_scheme(&headers, false), "http");
    assert_eq!(request_scheme(&headers, true), "https");
    for (value, expected) in [
        ("https", "https"),
        ("https, http", "https"),
        ("http", "http"),
        ("javascript", "http"),
    ] {
        headers.insert("x-forwarded-proto", value.parse().unwrap());
        assert_eq!(request_scheme(&headers, false), expected);
    }
}

fn endpoint_url(
    state: &NabState,
    headers: &HeaderMap,
    protocol: IndexerClientProtocol,
) -> AppResult<String> {
    let policy = state.app.public_url_runtime().snapshot();
    let origin = if let Some(url) = policy.resolve().map_err(|_| unavailable())? {
        url.origin().ascii_serialization()
    } else {
        // Host identifies the address this authenticated caller used. Forwarded
        // hosts cannot rewrite references; proxies can use the public URL.
        let host = headers
            .get(header::HOST)
            .and_then(|value| value.to_str().ok())
            .ok_or_else(unavailable)?;
        let authority: axum::http::uri::Authority = host.parse().map_err(|_| unavailable())?;
        if authority.as_str().contains('@') {
            return Err(unavailable());
        }
        let scheme = request_scheme(headers, state.tls);
        format!("{scheme}://{authority}")
    };
    let path = match protocol {
        IndexerClientProtocol::Newznab => NEWZNAB_PATH,
        IndexerClientProtocol::Torznab => TORZNAB_PATH,
    };
    Ok(format!("{origin}{}{path}", state.base_path))
}

fn artifact_response(
    artifact: ResolvedDownloadArtifact,
    protocol: IndexerClientProtocol,
) -> AppResult<Response> {
    let mut response = match (protocol, artifact) {
        (IndexerClientProtocol::Newznab, ResolvedDownloadArtifact::Nzb { bytes, .. }) => {
            ([(header::CONTENT_TYPE, "application/x-nzb")], bytes).into_response()
        }
        (IndexerClientProtocol::Torznab, ResolvedDownloadArtifact::TorrentFile { bytes, .. }) => {
            ([(header::CONTENT_TYPE, "application/x-bittorrent")], bytes).into_response()
        }
        (IndexerClientProtocol::Torznab, ResolvedDownloadArtifact::Magnet { uri, .. }) => {
            if !uri.starts_with("magnet:?") {
                return Err(invalid_reference());
            }
            let location = header::HeaderValue::from_str(&uri).map_err(|_| invalid_reference())?;
            (
                axum::http::StatusCode::TEMPORARY_REDIRECT,
                [(header::LOCATION, location)],
            )
                .into_response()
        }
        _ => return Err(invalid_reference()),
    };
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    Ok(response)
}

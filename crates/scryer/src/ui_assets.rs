use std::collections::HashMap;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, OnceLock, RwLock};

use axum::body::Body;
use axum::http::{HeaderMap, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use brotli::Decompressor;
use tokio::fs;

use crate::base_path::BasePath;
use crate::middleware::index_page;

mod embedded_ui_assets {
    pub struct EmbeddedWebAsset {
        pub path: &'static str,
        pub offset: usize,
        pub length: usize,
    }

    include!(concat!(env!("OUT_DIR"), "/embedded_ui_assets.rs"));
}

pub(crate) static UI_ASSET_MODE: OnceLock<UiAssetMode> = OnceLock::new();
const BASE_PATH_PLACEHOLDER: &str = "__SCRYER_BASE_PATH__";
const GRAPHQL_URL_PLACEHOLDER: &str = "__SCRYER_GRAPHQL_URL__";
static GZIP_UI_ASSET_CACHE: LazyLock<RwLock<HashMap<String, Vec<u8>>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));
const BROTLI_BUFFER_SIZE: usize = 4096;

#[derive(Clone, Debug)]
pub(crate) enum UiAssetMode {
    Filesystem(PathBuf),
    Embedded,
    Fallback,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(crate) enum UiContentEncoding {
    Brotli,
    Gzip,
    Identity,
}

pub(crate) async fn ui_fallback(method: Method, uri: Uri, headers: HeaderMap) -> Response {
    if method != Method::GET && method != Method::HEAD {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }

    let request_path = uri.path();
    let head_only = method == Method::HEAD;
    let preferred_encoding = preferred_content_encoding(&headers);
    let mut response = match ui_asset_mode() {
        UiAssetMode::Filesystem(dist_dir) => {
            serve_ui_path(dist_dir, request_path, head_only, preferred_encoding).await
        }
        UiAssetMode::Embedded => {
            serve_embedded_ui(request_path, head_only, preferred_encoding).await
        }
        UiAssetMode::Fallback => serve_fallback_ui(request_path).await,
    };
    if request_path.ends_with("/lists/oauth/callback")
        || request_path.ends_with("/lists/oauth/return")
    {
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            axum::http::HeaderValue::from_static("no-store"),
        );
        response.headers_mut().insert(
            header::REFERRER_POLICY,
            axum::http::HeaderValue::from_static("no-referrer"),
        );
    }
    response
}

fn preferred_content_encoding(headers: &HeaderMap) -> UiContentEncoding {
    let Some(value) = headers
        .get(header::ACCEPT_ENCODING)
        .and_then(|v| v.to_str().ok())
    else {
        return UiContentEncoding::Identity;
    };

    let brotli_quality = negotiated_quality(value, "br");
    let gzip_quality = negotiated_quality(value, "gzip");

    if brotli_quality <= 0.0 && gzip_quality <= 0.0 {
        return UiContentEncoding::Identity;
    }

    if brotli_quality >= gzip_quality {
        UiContentEncoding::Brotli
    } else {
        UiContentEncoding::Gzip
    }
}

fn negotiated_quality(value: &str, encoding: &str) -> f32 {
    let mut wildcard_quality = None;
    let mut specific_quality = None;

    for entry in value.split(',') {
        let mut parts = entry.trim().split(';');
        let token = parts.next().unwrap_or("").trim();
        if token.is_empty() {
            continue;
        }

        let mut quality = 1.0_f32;
        for parameter in parts {
            let parameter = parameter.trim();
            if let Some(raw_q) = parameter.strip_prefix("q=") {
                quality = raw_q
                    .parse::<f32>()
                    .ok()
                    .filter(|q| (0.0..=1.0).contains(q))
                    .unwrap_or(0.0);
            }
        }

        if token.eq_ignore_ascii_case(encoding) {
            specific_quality = Some(quality);
        } else if token == "*" {
            wildcard_quality = Some(quality);
        }
    }

    specific_quality.or(wildcard_quality).unwrap_or(0.0)
}

pub(crate) fn ui_asset_mode() -> &'static UiAssetMode {
    UI_ASSET_MODE.get_or_init(resolve_ui_asset_mode)
}

pub(crate) fn resolve_ui_asset_mode() -> UiAssetMode {
    // Debug builds are API-only; the UI is served by Vite via the dev proxy.
    if cfg!(debug_assertions) {
        return UiAssetMode::Fallback;
    }

    if let Ok(path) = std::env::var("SCRYER_WEB_DIST_DIR")
        && !path.trim().is_empty()
    {
        return UiAssetMode::Filesystem(PathBuf::from(path));
    }

    if embedded_ui_assets::HAS_EMBEDDED_WEB_UI {
        return UiAssetMode::Embedded;
    }

    let default_dist_dir = PathBuf::from("./crates/scryer/ui");
    if default_dist_dir.exists() {
        return UiAssetMode::Filesystem(default_dist_dir);
    }

    UiAssetMode::Fallback
}

pub(crate) async fn serve_embedded_ui(
    request_path: &str,
    head_only: bool,
    preferred_encoding: UiContentEncoding,
) -> Response {
    if should_serve_spa_index(request_path) {
        return serve_embedded_index(head_only).await;
    }

    let decoded = percent_encoding::percent_decode_str(request_path).decode_utf8_lossy();
    let relative_path = decoded.trim_start_matches('/');
    if relative_path.is_empty()
        || relative_path.ends_with('/')
        || contains_unsafe_path_segments(relative_path)
    {
        return StatusCode::NOT_FOUND.into_response();
    }

    // Don't serve pre-compressed files directly — they're only used as negotiated variants.
    if relative_path.ends_with(".gz") || relative_path.ends_with(".br") {
        return StatusCode::NOT_FOUND.into_response();
    }

    let content_type = infer_content_type(Path::new(relative_path));
    let cache_control = cache_control_for_asset(relative_path);
    let brotli_path = format!("{relative_path}.br");

    if let Some(brotli_bytes) = embedded_ui_asset(&brotli_path) {
        match preferred_encoding {
            UiContentEncoding::Brotli => {
                return negotiated_asset_response(
                    brotli_bytes.len(),
                    content_type,
                    cache_control,
                    Some("br"),
                    head_only,
                    if head_only {
                        Body::empty()
                    } else {
                        Body::from(brotli_bytes)
                    },
                );
            }
            UiContentEncoding::Gzip => match gzip_from_brotli_cached(relative_path, brotli_bytes) {
                Ok(gzip_bytes) => {
                    return negotiated_asset_response(
                        gzip_bytes.len(),
                        content_type,
                        cache_control,
                        Some("gzip"),
                        head_only,
                        if head_only {
                            Body::empty()
                        } else {
                            Body::from(gzip_bytes)
                        },
                    );
                }
                Err(error) => {
                    tracing::warn!(error = %error, path = relative_path, "failed to build cached gzip asset from Brotli bytes");
                    return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                }
            },
            UiContentEncoding::Identity => match decompress_brotli_bytes(brotli_bytes) {
                Ok(raw_bytes) => {
                    return negotiated_asset_response(
                        raw_bytes.len(),
                        content_type,
                        cache_control,
                        None,
                        head_only,
                        if head_only {
                            Body::empty()
                        } else {
                            Body::from(raw_bytes)
                        },
                    );
                }
                Err(error) => {
                    tracing::warn!(error = %error, path = relative_path, "failed to decompress Brotli asset to raw bytes");
                    return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                }
            },
        }
    }

    // Fallback: serve uncompressed asset directly (images, fonts, etc.).
    match embedded_ui_asset(relative_path) {
        Some(bytes) => {
            let content_len = bytes.len().to_string();
            let response = Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, content_type)
                .header(header::CONTENT_LENGTH, &content_len)
                .header(header::CACHE_CONTROL, cache_control)
                .body(if head_only {
                    Body::empty()
                } else {
                    Body::from(bytes)
                });
            response.unwrap_or_else(|error| {
                tracing::warn!(error = %error, path = relative_path, "failed to build embedded asset response");
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            })
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

pub(crate) fn embedded_ui_asset(path: &str) -> Option<&'static [u8]> {
    static INDEX: LazyLock<HashMap<&'static str, &'static [u8]>> = LazyLock::new(|| {
        embedded_ui_assets::EMBEDDED_WEB_FILES
            .iter()
            .map(|asset| {
                let end = asset
                    .offset
                    .checked_add(asset.length)
                    .expect("embedded UI descriptor range overflow");
                let bytes = embedded_ui_assets::EMBEDDED_WEB_BLOB
                    .get(asset.offset..end)
                    .expect("embedded UI descriptor outside packed blob");
                (asset.path, bytes)
            })
            .collect()
    });
    let normalized_path = path.trim_start_matches('/');
    INDEX.get(normalized_path).copied()
}

pub(crate) async fn serve_embedded_index(head_only: bool) -> Response {
    match embedded_ui_asset("index.html") {
        Some(index_html) => {
            let index_html = render_index_html(index_html);
            let content_len = index_html.len().to_string();
            let response = Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                .header(header::CONTENT_LENGTH, &content_len)
                .header(header::CACHE_CONTROL, "no-cache")
                .body(if head_only {
                    Body::empty()
                } else {
                    Body::from(index_html)
                });
            response.unwrap_or_else(|error| {
                tracing::warn!(error = %error, "failed to build embedded index response");
                Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .body(Body::empty())
                    .expect("response build")
            })
        }
        None => index_page().await.into_response(),
    }
}

pub(crate) async fn serve_ui_path(
    dist_dir: &Path,
    request_path: &str,
    head_only: bool,
    preferred_encoding: UiContentEncoding,
) -> Response {
    if !dist_dir.exists() {
        return serve_fallback_ui(request_path).await;
    }

    if should_serve_spa_index(request_path) {
        return serve_index_html(dist_dir, head_only).await;
    }

    let decoded = percent_encoding::percent_decode_str(request_path).decode_utf8_lossy();
    let relative_path = decoded.trim_start_matches('/');
    if contains_unsafe_path_segments(relative_path) {
        return StatusCode::NOT_FOUND.into_response();
    }

    // Don't serve negotiated pre-compressed files directly.
    if relative_path.ends_with(".gz") || relative_path.ends_with(".br") {
        return StatusCode::NOT_FOUND.into_response();
    }

    let candidate = dist_dir.join(relative_path);
    let canonical = match candidate.canonicalize() {
        Ok(path) => path,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    let canonical_root = match dist_dir.canonicalize() {
        Ok(path) => path,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    if !canonical.starts_with(&canonical_root) {
        return StatusCode::NOT_FOUND.into_response();
    }
    match fs::metadata(&canonical).await {
        Ok(metadata) if metadata.is_file() => {
            let brotli_candidate = dist_dir.join(format!("{relative_path}.br"));
            if let Ok(brotli_canonical) = brotli_candidate.canonicalize()
                && brotli_canonical.starts_with(&canonical_root)
                && let Ok(brotli_meta) = fs::metadata(&brotli_canonical).await
                && brotli_meta.is_file()
            {
                match preferred_encoding {
                    UiContentEncoding::Brotli => {
                        return serve_file_precompressed(
                            brotli_canonical,
                            &canonical,
                            "br",
                            head_only,
                        )
                        .await;
                    }
                    UiContentEncoding::Gzip => {
                        return serve_file_gzip_from_brotli(
                            brotli_canonical,
                            &canonical,
                            relative_path,
                            head_only,
                        )
                        .await;
                    }
                    UiContentEncoding::Identity => {}
                }
            }
            serve_file(canonical, head_only).await
        }
        Ok(metadata) if metadata.is_dir() => StatusCode::NOT_FOUND.into_response(),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

pub(crate) async fn serve_fallback_ui(request_path: &str) -> Response {
    if should_serve_spa_index(request_path) {
        index_page().await.into_response()
    } else {
        StatusCode::NOT_FOUND.into_response()
    }
}

pub(crate) fn should_serve_spa_index(request_path: &str) -> bool {
    let normalized = request_path.trim();
    if normalized.is_empty() || normalized == "/" {
        return true;
    }

    !is_reserved_non_spa_path(normalized) && !looks_like_static_asset_request(normalized)
}

pub(crate) fn is_reserved_non_spa_path(request_path: &str) -> bool {
    let first_segment = request_path
        .trim_matches('/')
        .split('/')
        .find(|segment| !segment.is_empty());

    matches!(
        first_segment,
        Some("graphql" | "health" | "metrics" | "admin" | "images")
    )
}

pub(crate) fn looks_like_static_asset_request(request_path: &str) -> bool {
    let last_segment = request_path
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default();
    Path::new(last_segment).extension().is_some()
}

pub(crate) fn contains_unsafe_path_segments(path: &str) -> bool {
    let decoded = percent_encoding::percent_decode_str(path).decode_utf8_lossy();
    decoded
        .split('/')
        .any(|segment| segment == ".." || segment == "." || segment.contains('\\'))
}

pub(crate) fn cache_control_for_asset(path: &str) -> &'static str {
    if path.starts_with("assets/") || path.starts_with("_next/static/") {
        "public, max-age=31536000, immutable"
    } else if path == "index.html" || path == "manifest.json" || path == "service-worker.js" {
        "no-cache"
    } else {
        "public, max-age=3600"
    }
}

pub(crate) fn infer_content_type(path: &Path) -> &'static str {
    if path.file_name().and_then(|name| name.to_str()) == Some("manifest.json") {
        return "application/manifest+json; charset=utf-8";
    }

    match path.extension().and_then(|ext| ext.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js") => "application/javascript; charset=utf-8",
        Some("mjs") => "application/javascript; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        Some("svg") => "image/svg+xml",
        Some("ico") => "image/x-icon",
        Some("txt") => "text/plain; charset=utf-8",
        Some("xml") => "application/xml; charset=utf-8",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        Some("ttf") => "font/ttf",
        Some("wasm") => "application/wasm",
        Some("map") => "application/json; charset=utf-8",
        _ => "application/octet-stream",
    }
}

pub(crate) async fn serve_file(path: PathBuf, head_only: bool) -> Response {
    match fs::read(&path).await {
        Ok(bytes) => {
            let asset_path = path.to_string_lossy();
            let relative_key = relative_asset_key(&asset_path);
            let content_len = bytes.len().to_string();
            let response = Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, infer_content_type(&path))
                .header(header::CONTENT_LENGTH, &content_len)
                .header(header::CACHE_CONTROL, cache_control_for_asset(relative_key))
                .body(if head_only {
                    Body::empty()
                } else {
                    Body::from(bytes)
                });
            response
                .unwrap_or_else(|error| {
                    tracing::warn!(error = %error, path = %path.display(), "failed to build file response");
                    Response::builder()
                        .status(StatusCode::INTERNAL_SERVER_ERROR)
                        .body(Body::empty())
                        .expect("response build")
                })
        }
        Err(error) => {
            tracing::warn!(error = %error, path = %path.display(), "failed to read ui asset file");
            Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::empty())
                .expect("response build")
        }
    }
}

/// Serve a pre-compressed asset with the content type of the original path.
async fn serve_file_precompressed(
    compressed_path: PathBuf,
    original_path: &Path,
    content_encoding: &'static str,
    head_only: bool,
) -> Response {
    match fs::read(&compressed_path).await {
        Ok(bytes) => {
            let asset_path = original_path.to_string_lossy();
            let relative_key = relative_asset_key(&asset_path);
            negotiated_asset_response(
                bytes.len(),
                infer_content_type(original_path),
                cache_control_for_asset(relative_key),
                Some(content_encoding),
                head_only,
                if head_only {
                    Body::empty()
                } else {
                    Body::from(bytes)
                },
            )
        }
        Err(_) => serve_file(original_path.to_path_buf(), head_only).await,
    }
}

async fn serve_file_gzip_from_brotli(
    brotli_path: PathBuf,
    original_path: &Path,
    cache_key: &str,
    head_only: bool,
) -> Response {
    match fs::read(&brotli_path).await {
        Ok(bytes) => match gzip_from_brotli_cached(cache_key, &bytes) {
            Ok(gzip_bytes) => {
                let asset_path = original_path.to_string_lossy();
                let relative_key = relative_asset_key(&asset_path);
                negotiated_asset_response(
                    gzip_bytes.len(),
                    infer_content_type(original_path),
                    cache_control_for_asset(relative_key),
                    Some("gzip"),
                    head_only,
                    if head_only {
                        Body::empty()
                    } else {
                        Body::from(gzip_bytes)
                    },
                )
            }
            Err(error) => {
                tracing::warn!(error = %error, path = %brotli_path.display(), "failed to build cached gzip asset from Brotli file");
                serve_file(original_path.to_path_buf(), head_only).await
            }
        },
        Err(_) => serve_file(original_path.to_path_buf(), head_only).await,
    }
}

fn negotiated_asset_response(
    bytes_len: usize,
    content_type: &'static str,
    cache_control: &'static str,
    content_encoding: Option<&'static str>,
    head_only: bool,
    body: Body,
) -> Response {
    let content_len = bytes_len.to_string();
    let mut builder = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CONTENT_LENGTH, &content_len)
        .header(header::CACHE_CONTROL, cache_control)
        .header(header::VARY, "Accept-Encoding");

    if let Some(content_encoding) = content_encoding {
        builder = builder.header(header::CONTENT_ENCODING, content_encoding);
    }

    builder
        .body(if head_only { Body::empty() } else { body })
        .unwrap_or_else(|error| {
            tracing::warn!(error = %error, "failed to build negotiated asset response");
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::empty())
                .expect("response build")
        })
}

fn relative_asset_key(asset_path: &str) -> &str {
    asset_path
        .rsplit_once("/dist/")
        .map(|(_, rest)| rest)
        .or_else(|| asset_path.rsplit_once("/out/").map(|(_, rest)| rest))
        .or_else(|| asset_path.rsplit_once("/ui/").map(|(_, rest)| rest))
        .unwrap_or(asset_path)
}

fn gzip_from_brotli_cached(cache_key: &str, brotli_bytes: &[u8]) -> std::io::Result<Vec<u8>> {
    if let Ok(cache) = GZIP_UI_ASSET_CACHE.read()
        && let Some(bytes) = cache.get(cache_key)
    {
        return Ok(bytes.clone());
    }

    let raw_bytes = decompress_brotli_bytes(brotli_bytes)?;
    let gzip_bytes = gzip_compress_bytes(&raw_bytes)?;

    if let Ok(mut cache) = GZIP_UI_ASSET_CACHE.write() {
        let bytes = cache
            .entry(cache_key.to_string())
            .or_insert_with(|| gzip_bytes.clone())
            .clone();
        return Ok(bytes);
    }

    Ok(gzip_bytes)
}

fn decompress_brotli_bytes(brotli_bytes: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut decoder = Decompressor::new(Cursor::new(brotli_bytes), BROTLI_BUFFER_SIZE);
    let mut raw_bytes = Vec::new();
    decoder.read_to_end(&mut raw_bytes)?;
    Ok(raw_bytes)
}

fn gzip_compress_bytes(raw_bytes: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    encoder.write_all(raw_bytes)?;
    encoder.finish()
}

pub(crate) async fn serve_index_html(dist_dir: &Path, head_only: bool) -> Response {
    let index = dist_dir.join("index.html");
    match fs::read(&index).await {
        Ok(index_html) => {
            let index_html = render_index_html(&index_html);
            let content_len = index_html.len().to_string();
            let response = Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                .header(header::CONTENT_LENGTH, &content_len)
                .header(header::CACHE_CONTROL, "no-cache")
                .body(if head_only {
                    Body::empty()
                } else {
                    Body::from(index_html)
                });
            response.unwrap_or_else(|error| {
                tracing::warn!(error = %error, "failed to build index response");
                Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .body(Body::empty())
                    .expect("response build")
            })
        }
        Err(error) => {
            tracing::warn!(
                error = %error,
                dist_dir = %dist_dir.display(),
                "index.html missing from ui dist directory"
            );
            index_page().await.into_response()
        }
    }
}

fn render_index_html(index_html: &[u8]) -> Vec<u8> {
    let base_path = BasePath::from_env();
    let graphql_url = base_path.join("/graphql");
    let ui_root = base_path.ui_root();

    // Inject a <base> tag so the browser resolves relative asset URLs (./assets/app.js,
    // ./manifest.json, etc.) against the UI root rather than the current page path.
    // Without this, deep SPA routes like /scryer/series/123 cause the browser to resolve
    // ./assets/app.js as /scryer/series/assets/app.js → 404.
    let base_tag = format!(r#"<base href="{ui_root}" />"#);

    String::from_utf8_lossy(index_html)
        .replace(BASE_PATH_PLACEHOLDER, base_path.basename())
        .replace(GRAPHQL_URL_PLACEHOLDER, &graphql_url)
        .replacen(
            r#"<meta charset="UTF-8" />"#,
            &format!("<meta charset=\"UTF-8\" />\n    {base_tag}"),
            1,
        )
        .into_bytes()
}

#[cfg(test)]
mod tests {
    use super::{
        UiContentEncoding, cache_control_for_asset, embedded_ui_asset, embedded_ui_assets,
        infer_content_type, looks_like_static_asset_request, negotiated_quality,
        preferred_content_encoding, serve_fallback_ui, should_serve_spa_index,
    };
    use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
    use std::path::Path;

    #[test]
    fn embedded_descriptors_are_sorted_packed_and_resolvable() {
        let mut expected_offset = 0;
        let mut previous_path: Option<&str> = None;
        for asset in embedded_ui_assets::EMBEDDED_WEB_FILES {
            if let Some(previous_path) = previous_path {
                assert!(
                    previous_path < asset.path,
                    "descriptors must be path-sorted"
                );
            }
            assert_eq!(asset.offset, expected_offset, "descriptors must be packed");
            let end = asset.offset + asset.length;
            assert_eq!(
                embedded_ui_asset(asset.path),
                Some(&embedded_ui_assets::EMBEDDED_WEB_BLOB[asset.offset..end])
            );
            expected_offset = end;
            previous_path = Some(asset.path);
        }
        assert_eq!(expected_offset, embedded_ui_assets::EMBEDDED_WEB_BLOB.len());
    }

    #[test]
    fn spa_index_is_served_for_catalog_routes() {
        assert!(should_serve_spa_index("/"));
        assert!(should_serve_spa_index("/anime"));
        assert!(should_serve_spa_index("/titles/stoneguard"));
    }

    #[test]
    fn spa_index_is_not_served_for_reserved_or_asset_like_paths() {
        assert!(!should_serve_spa_index("/images/titles/abc/poster/w500"));
        assert!(!should_serve_spa_index("/graphql"));
        assert!(!should_serve_spa_index("/health"));
        assert!(!should_serve_spa_index("/assets/app.js"));
        assert!(looks_like_static_asset_request("/assets/app.js"));
    }

    #[test]
    fn negotiated_quality_honors_specific_and_wildcard_values() {
        assert_eq!(negotiated_quality("br, gzip;q=0.5", "br"), 1.0);
        assert_eq!(negotiated_quality("gzip;q=0.8, *;q=0.3", "br"), 0.3);
        assert_eq!(negotiated_quality("br;q=0, gzip;q=1", "br"), 0.0);
    }

    #[test]
    fn preferred_content_encoding_prefers_brotli_then_gzip_then_identity() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::ACCEPT_ENCODING,
            HeaderValue::from_static("br;q=1.0, gzip;q=0.8"),
        );
        assert_eq!(
            preferred_content_encoding(&headers),
            UiContentEncoding::Brotli
        );

        headers.insert(
            header::ACCEPT_ENCODING,
            HeaderValue::from_static("gzip;q=1.0, br;q=0.2"),
        );
        assert_eq!(
            preferred_content_encoding(&headers),
            UiContentEncoding::Gzip
        );

        headers.insert(
            header::ACCEPT_ENCODING,
            HeaderValue::from_static("identity"),
        );
        assert_eq!(
            preferred_content_encoding(&headers),
            UiContentEncoding::Identity
        );

        let headers = HeaderMap::new();
        assert_eq!(
            preferred_content_encoding(&headers),
            UiContentEncoding::Identity
        );
    }

    #[test]
    fn svg_content_type_omits_charset_for_compression_compatibility() {
        assert_eq!(infer_content_type(Path::new("logo.svg")), "image/svg+xml");
    }

    #[test]
    fn manifest_and_service_worker_headers_are_pwa_safe() {
        assert_eq!(
            infer_content_type(Path::new("manifest.json")),
            "application/manifest+json; charset=utf-8"
        );
        assert_eq!(cache_control_for_asset("manifest.json"), "no-cache");
        assert_eq!(cache_control_for_asset("service-worker.js"), "no-cache");
    }

    #[tokio::test]
    async fn fallback_mode_returns_not_found_for_reserved_image_paths() {
        let response = serve_fallback_ui("/images/titles/missing/poster/w500").await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn fallback_mode_serves_index_for_spa_routes() {
        let response = serve_fallback_ui("/anime").await;
        assert_eq!(response.status(), StatusCode::OK);
    }
}

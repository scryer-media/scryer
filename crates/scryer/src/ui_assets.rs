use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use axum::body::Body;
use axum::http::{header, HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use tokio::fs;

use crate::base_path::BasePath;
use crate::middleware::index_page;

mod embedded_ui_assets {
    include!(concat!(env!("OUT_DIR"), "/embedded_ui_assets.rs"));
}

pub(crate) static UI_ASSET_MODE: OnceLock<UiAssetMode> = OnceLock::new();
const BASE_PATH_PLACEHOLDER: &str = "__SCRYER_BASE_PATH__";
const GRAPHQL_URL_PLACEHOLDER: &str = "__SCRYER_GRAPHQL_URL__";

#[derive(Clone, Debug)]
pub(crate) enum UiAssetMode {
    Filesystem(PathBuf),
    Embedded,
    Fallback,
}

pub(crate) async fn ui_fallback(method: Method, uri: Uri, headers: HeaderMap) -> Response {
    if method != Method::GET && method != Method::HEAD {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }

    let request_path = uri.path();
    let head_only = method == Method::HEAD;
    let accept_gzip = accepts_gzip(&headers);
    match ui_asset_mode() {
        UiAssetMode::Filesystem(dist_dir) => {
            serve_ui_path(dist_dir, request_path, head_only, accept_gzip).await
        }
        UiAssetMode::Embedded => serve_embedded_ui(request_path, head_only, accept_gzip).await,
        UiAssetMode::Fallback => serve_fallback_ui(request_path).await,
    }
}

fn accepts_gzip(headers: &HeaderMap) -> bool {
    headers
        .get(header::ACCEPT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("gzip"))
}

pub(crate) fn ui_asset_mode() -> &'static UiAssetMode {
    UI_ASSET_MODE.get_or_init(resolve_ui_asset_mode)
}

pub(crate) fn resolve_ui_asset_mode() -> UiAssetMode {
    // Debug builds are API-only; the UI is served by Vite via the dev proxy.
    if cfg!(debug_assertions) {
        return UiAssetMode::Fallback;
    }

    if let Ok(path) = std::env::var("SCRYER_WEB_DIST_DIR") {
        if !path.trim().is_empty() {
            return UiAssetMode::Filesystem(PathBuf::from(path));
        }
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
    accept_gzip: bool,
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

    // Don't serve .gz files directly — they're only used as pre-compressed variants.
    if relative_path.ends_with(".gz") {
        return StatusCode::NOT_FOUND.into_response();
    }

    match embedded_ui_asset(relative_path) {
        Some(bytes) => {
            let content_type = infer_content_type(Path::new(relative_path));
            let cache_control = cache_control_for_asset(relative_path);

            // Serve pre-compressed .gz variant if client accepts gzip.
            if accept_gzip {
                let gz_path = format!("{relative_path}.gz");
                if let Some(gz_bytes) = embedded_ui_asset(&gz_path) {
                    let content_len = gz_bytes.len().to_string();
                    let response = Response::builder()
                        .status(StatusCode::OK)
                        .header(header::CONTENT_TYPE, content_type)
                        .header(header::CONTENT_ENCODING, "gzip")
                        .header(header::CONTENT_LENGTH, &content_len)
                        .header(header::CACHE_CONTROL, cache_control)
                        .body(if head_only {
                            Body::empty()
                        } else {
                            Body::from(gz_bytes)
                        });
                    return response.unwrap_or_else(|error| {
                        tracing::warn!(error = %error, path = relative_path, "failed to build compressed asset response");
                        Response::builder()
                            .status(StatusCode::INTERNAL_SERVER_ERROR)
                            .body(Body::empty())
                            .expect("response build")
                    });
                }
            }

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
                Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .body(Body::empty())
                    .expect("response build")
            })
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

pub(crate) fn embedded_ui_asset(path: &str) -> Option<&'static [u8]> {
    static INDEX: OnceLock<HashMap<&'static str, &'static [u8]>> = OnceLock::new();
    let map = INDEX.get_or_init(|| {
        embedded_ui_assets::EMBEDDED_WEB_FILES
            .iter()
            .copied()
            .collect()
    });
    let normalized_path = path.trim_start_matches('/');
    map.get(normalized_path).copied()
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
    accept_gzip: bool,
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

    // Don't serve .gz files directly.
    if relative_path.ends_with(".gz") {
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
            // Try pre-compressed variant for filesystem mode too.
            if accept_gzip {
                let gz_candidate = dist_dir.join(format!("{relative_path}.gz"));
                if let Ok(gz_canonical) = gz_candidate.canonicalize() {
                    if gz_canonical.starts_with(&canonical_root) {
                        if let Ok(gz_meta) = fs::metadata(&gz_canonical).await {
                            if gz_meta.is_file() {
                                return serve_file_gzipped(
                                    gz_canonical,
                                    &canonical,
                                    head_only,
                                )
                                .await;
                            }
                        }
                    }
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
        Some("graphql" | "graphiql" | "health" | "metrics" | "admin" | "images")
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
            let relative_key = asset_path
                .rsplit_once("/dist/")
                .map(|(_, rest)| rest)
                .or_else(|| asset_path.rsplit_once("/out/").map(|(_, rest)| rest))
                .or_else(|| asset_path.rsplit_once("/ui/").map(|(_, rest)| rest))
                .unwrap_or(&asset_path);
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

/// Serve a pre-compressed `.gz` file with the content type of the original path.
async fn serve_file_gzipped(
    gz_path: PathBuf,
    original_path: &Path,
    head_only: bool,
) -> Response {
    match fs::read(&gz_path).await {
        Ok(bytes) => {
            let asset_path = original_path.to_string_lossy();
            let relative_key = asset_path
                .rsplit_once("/dist/")
                .map(|(_, rest)| rest)
                .or_else(|| asset_path.rsplit_once("/out/").map(|(_, rest)| rest))
                .or_else(|| asset_path.rsplit_once("/ui/").map(|(_, rest)| rest))
                .unwrap_or(&asset_path);
            let content_len = bytes.len().to_string();
            let response = Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, infer_content_type(original_path))
                .header(header::CONTENT_ENCODING, "gzip")
                .header(header::CONTENT_LENGTH, &content_len)
                .header(
                    header::CACHE_CONTROL,
                    cache_control_for_asset(relative_key),
                )
                .body(if head_only {
                    Body::empty()
                } else {
                    Body::from(bytes)
                });
            response.unwrap_or_else(|error| {
                tracing::warn!(error = %error, path = %gz_path.display(), "failed to build gzipped file response");
                Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .body(Body::empty())
                    .expect("response build")
            })
        }
        Err(_) => serve_file(original_path.to_path_buf(), head_only).await,
    }
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
    String::from_utf8_lossy(index_html)
        .replace(BASE_PATH_PLACEHOLDER, base_path.basename())
        .replace(GRAPHQL_URL_PLACEHOLDER, &graphql_url)
        .into_bytes()
}

#[cfg(test)]
mod tests {
    use super::{
        cache_control_for_asset, infer_content_type, looks_like_static_asset_request,
        serve_fallback_ui, should_serve_spa_index,
    };
    use axum::http::StatusCode;
    use std::path::Path;

    #[test]
    fn spa_index_is_served_for_catalog_routes() {
        assert!(should_serve_spa_index("/"));
        assert!(should_serve_spa_index("/anime"));
        assert!(should_serve_spa_index("/titles/attack-on-titan"));
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

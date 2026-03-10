use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use axum::body::Body;
use axum::http::{header, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use tokio::fs;

use crate::middleware::index_page;

mod embedded_ui_assets {
    include!(concat!(env!("OUT_DIR"), "/embedded_ui_assets.rs"));
}

pub(crate) static UI_ASSET_MODE: OnceLock<UiAssetMode> = OnceLock::new();

#[derive(Clone, Debug)]
pub(crate) enum UiAssetMode {
    Filesystem(PathBuf),
    Embedded,
    Fallback,
}

pub(crate) async fn ui_fallback(method: Method, uri: Uri) -> Response {
    if method != Method::GET && method != Method::HEAD {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }

    let request_path = uri.path();
    let head_only = method == Method::HEAD;
    match ui_asset_mode() {
        UiAssetMode::Filesystem(dist_dir) => serve_ui_path(dist_dir, request_path, head_only).await,
        UiAssetMode::Embedded => serve_embedded_ui(request_path, head_only).await,
        UiAssetMode::Fallback => index_page().await.into_response(),
    }
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

pub(crate) async fn serve_embedded_ui(request_path: &str, head_only: bool) -> Response {
    if should_fallback_to_index(request_path) {
        return serve_embedded_index(head_only).await;
    }

    let decoded = percent_encoding::percent_decode_str(request_path).decode_utf8_lossy();
    let relative_path = decoded.trim_start_matches('/');
    if relative_path.is_empty()
        || relative_path.ends_with('/')
        || contains_unsafe_path_segments(relative_path)
    {
        return serve_embedded_index(head_only).await;
    }

    match embedded_ui_asset(relative_path) {
        Some(bytes) => {
            let content_len = bytes.len().to_string();
            let response = Response::builder()
                .status(StatusCode::OK)
                .header(
                    header::CONTENT_TYPE,
                    infer_content_type(Path::new(relative_path)),
                )
                .header(header::CONTENT_LENGTH, &content_len)
                .header(
                    header::CACHE_CONTROL,
                    cache_control_for_asset(relative_path),
                )
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
        None => serve_embedded_index(head_only).await,
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
) -> Response {
    if !dist_dir.exists() {
        return index_page().await.into_response();
    }

    if should_fallback_to_index(request_path) {
        return serve_index_html(dist_dir, head_only).await;
    }

    let decoded = percent_encoding::percent_decode_str(request_path).decode_utf8_lossy();
    let relative_path = decoded.trim_start_matches('/');
    if contains_unsafe_path_segments(relative_path) {
        return serve_index_html(dist_dir, head_only).await;
    }

    let candidate = dist_dir.join(relative_path);
    let canonical = match candidate.canonicalize() {
        Ok(path) => path,
        Err(_) => return serve_index_html(dist_dir, head_only).await,
    };
    let canonical_root = match dist_dir.canonicalize() {
        Ok(path) => path,
        Err(_) => return serve_index_html(dist_dir, head_only).await,
    };
    if !canonical.starts_with(&canonical_root) {
        return serve_index_html(dist_dir, head_only).await;
    }
    match fs::metadata(&canonical).await {
        Ok(metadata) if metadata.is_file() => serve_file(canonical, head_only).await,
        Ok(metadata) if metadata.is_dir() => serve_index_html(dist_dir, head_only).await,
        _ => serve_index_html(dist_dir, head_only).await,
    }
}

pub(crate) fn should_fallback_to_index(request_path: &str) -> bool {
    request_path == "/" || request_path.trim().is_empty()
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
    } else if path == "index.html" {
        "no-cache"
    } else {
        "public, max-age=3600"
    }
}

pub(crate) fn infer_content_type(path: &Path) -> &'static str {
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
        Some("svg") => "image/svg+xml; charset=utf-8",
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

pub(crate) async fn serve_index_html(dist_dir: &Path, head_only: bool) -> Response {
    let index = dist_dir.join("index.html");
    match fs::read(&index).await {
        Ok(index_html) => {
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

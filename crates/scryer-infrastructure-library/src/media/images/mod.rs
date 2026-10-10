mod image_proxy_runtime;
pub mod image_proxy_store;
#[cfg(feature = "image-processing")]
pub mod processor;
pub mod title_image_store;
pub use image_proxy_runtime::{ImageProxyBlob, ImageProxyRuntime};
pub use image_proxy_store::ImageProxyStore;

use scryer_application::{AppError, AppResult, TitleImageKind};

pub fn normalize_title_image_source_url(source_url: &str) -> AppResult<String> {
    let trimmed = source_url.trim();
    if trimmed.is_empty() {
        return Err(AppError::Validation(
            "title image URL must not be empty".into(),
        ));
    }

    let mut parsed = match url::Url::parse(trimmed) {
        Ok(parsed) => parsed,
        Err(_) => {
            let relative = trimmed.trim_start_matches('/');
            url::Url::parse(&format!("https://artworks.thetvdb.com/{relative}")).map_err(
                |error| AppError::Validation(format!("invalid title image URL: {error}")),
            )?
        }
    };
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(AppError::Validation(
            "title image URL must use http or https".into(),
        ));
    }
    if parsed.host_str().is_none() {
        return Err(AppError::Validation(
            "title image URL must include a host".into(),
        ));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(AppError::Validation(
            "title image URL must not include credentials".into(),
        ));
    }

    let normalized_path = parsed
        .path()
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("/");
    parsed.set_path(&format!("/{normalized_path}"));
    image_proxy_store::approved_upstream_url(parsed.as_str()).ok_or_else(|| {
        AppError::Validation(
            "title image URL host must be image.tmdb.org, artworks.thetvdb.com, \
             cdn.myanimelist.net, or an AniList CDN shard"
                .into(),
        )
    })
}

static BASE_PATH: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Install the base path the server resolved at startup. Local image URLs are
/// synthesized under it. The first installation wins; the composition root
/// installs it once before any store is built.
pub fn install_base_path(normalized: &str) {
    let _ = BASE_PATH.set(normalized.trim_end_matches('/').to_string());
}

/// The installed base path, empty at the root or before installation.
pub fn normalized_base_path() -> String {
    BASE_PATH.get().cloned().unwrap_or_default()
}

pub fn synthesize_local_title_image_url(
    base_path: &str,
    title_id: &str,
    kind: TitleImageKind,
    variant_key: &str,
    version_hash: &str,
) -> String {
    let version_value = version_hash
        .split_once(':')
        .map(|(_, digest)| digest)
        .unwrap_or(version_hash);
    let version = version_value.chars().take(16).collect::<String>();
    format!(
        "{base_path}/images/titles/{title_id}/{}/{variant_key}?v={version}",
        kind.as_str()
    )
}

pub fn title_image_blob_digest(bytes: &[u8]) -> String {
    format!("blake3:{}", blake3::hash(bytes).to_hex())
}

pub fn content_type_for_format(format: String) -> String {
    match format.trim().to_ascii_lowercase().as_str() {
        "jpeg" | "jpg" => "image/jpeg".to_string(),
        "png" => "image/png".to_string(),
        "webp" => "image/webp".to_string(),
        "avif" => "image/avif".to_string(),
        other => format!("image/{other}"),
    }
}

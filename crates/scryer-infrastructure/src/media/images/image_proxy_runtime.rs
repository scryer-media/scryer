use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use reqwest::header::{
    ACCEPT, CONTENT_LENGTH, CONTENT_TYPE, ETAG, IF_MODIFIED_SINCE, IF_NONE_MATCH, LAST_MODIFIED,
};
use scryer_application::{
    AppError, AppResult, ImageProxyCacheControl, ImageProxyCacheEntryRecord, ImageProxyRepository,
    ImageProxySourceRecord, TitleImageKind, TitleImageRepository,
};
use scryer_outbound_http::{
    HostRpsProfile, OutboundHttpClient, OutboundHttpError, RateLimitRegistry, RequestPolicy,
    no_redirect_reqwest_client,
};
use tokio::sync::{Mutex, OnceCell, OwnedRwLockReadGuard, RwLock};

use super::image_proxy_store::approved_upstream_url;

const MAX_SOURCE_BYTES: usize = 20 * 1024 * 1024;
const DEFAULT_CACHE_BYTES: u64 = 256 * 1024 * 1024;
const FRESH_DAYS: i64 = 7;
const STALE_DAYS: i64 = 30;
const ACCESS_TOUCH_MINUTES: i64 = 60;
const IMAGE_PROXY_HOST_RPS: f64 = 100.0;
const IMAGE_PROXY_HOST_RPS_BURST: u32 = 100;
const IMAGE_PROXY_HOST_RPS_LANE: &str = "image_proxy";
const IMAGE_PROXY_ACCEPT: &str = "image/webp,image/jpeg;q=0.9,image/png;q=0.8";

const PORTRAIT_FALLBACK: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 2 3"><rect width="2" height="3" fill="#20242b"/><circle cx="1" cy="1" r=".42" fill="#667080"/><path d="M.3 2.7c.08-.72.33-1.08.7-1.08s.62.36.7 1.08" fill="#667080"/></svg>"##;
const LANDSCAPE_FALLBACK: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 9"><rect width="16" height="9" fill="#20242b"/><path d="m1 8 4-4 3 3 2-2 5 3" fill="#667080"/><circle cx="12" cy="2.5" r="1" fill="#667080"/></svg>"##;

#[derive(Clone, Debug)]
pub struct ImageProxyBlob {
    pub content_type: String,
    pub etag: String,
    pub bytes: Vec<u8>,
    pub fallback: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CacheFreshness {
    Fresh,
    Stale,
    Expired,
}

type SharedFetchResult = Result<Option<ImageProxyBlob>, String>;
type InflightFetch = OnceCell<SharedFetchResult>;

#[derive(Clone)]
pub struct ImageProxyRuntime {
    repository: Arc<dyn ImageProxyRepository>,
    title_images: Arc<dyn TitleImageRepository>,
    cache_dir: PathBuf,
    outbound_http: OutboundHttpClient,
    configured_cache_bytes: Arc<AtomicU64>,
    environment_override_bytes: Option<u64>,
    inflight: Arc<Mutex<HashMap<String, Weak<InflightFetch>>>>,
    source_flush: Arc<Mutex<()>>,
    cache_lifecycle: Arc<RwLock<()>>,
}

impl ImageProxyRuntime {
    pub fn new(
        repository: Arc<dyn ImageProxyRepository>,
        title_images: Arc<dyn TitleImageRepository>,
        data_dir: impl AsRef<Path>,
    ) -> Self {
        let environment_override_bytes = std::env::var("SCRYER_IMAGE_CACHE_MAX_BYTES")
            .ok()
            .and_then(|value| value.parse::<u64>().ok());
        Self {
            repository,
            title_images,
            cache_dir: data_dir.as_ref().join("cache").join("images"),
            outbound_http: image_outbound_http_client(),
            configured_cache_bytes: Arc::new(AtomicU64::new(DEFAULT_CACHE_BYTES)),
            environment_override_bytes,
            inflight: Arc::new(Mutex::new(HashMap::new())),
            source_flush: Arc::new(Mutex::new(())),
            cache_lifecycle: Arc::new(RwLock::new(())),
        }
    }

    pub fn configured_max_bytes(&self) -> u64 {
        self.configured_cache_bytes.load(Ordering::Relaxed)
    }

    pub fn effective_max_bytes(&self) -> u64 {
        self.environment_override_bytes
            .unwrap_or_else(|| self.configured_max_bytes())
    }

    pub async fn resolve(self: &Arc<Self>, token: &str, variant: &str) -> ImageProxyBlob {
        if !valid_token(token) {
            return fallback_blob("landscape");
        }
        let source = match self.repository.get_image_proxy_source(token).await {
            Ok(Some(source)) => source,
            Ok(None) => return fallback_blob("landscape"),
            Err(error) => {
                tracing::warn!(error = %error, token, "failed to load image proxy source");
                return fallback_blob("landscape");
            }
        };
        self.flush_source_touches_in_background(token.to_string());

        if !variant_allowed(&source.image_kind, variant) {
            return fallback_blob(&source.fallback_class);
        }

        if let Some(blob) = self.local_blob(&source, variant).await {
            return blob;
        }

        if let Some((entry, bytes, freshness)) = self.read_cached(token, variant).await {
            if freshness == CacheFreshness::Fresh {
                return cached_blob(&entry, bytes);
            }
            if freshness == CacheFreshness::Stale {
                let runtime = Arc::clone(self);
                let source_for_refresh = source.clone();
                let token = token.to_string();
                let variant = variant.to_string();
                let observed_fetched_at = entry.fetched_at;
                tokio::spawn(async move {
                    let _ = runtime
                        .refresh_stale_singleflight(
                            &source_for_refresh,
                            &token,
                            &variant,
                            observed_fetched_at,
                        )
                        .await;
                });
                return cached_blob(&entry, bytes);
            }
        }

        match self.fetch_singleflight(&source, token, variant).await {
            Ok(Some(blob)) => blob,
            Ok(None) => fallback_blob(&source.fallback_class),
            Err(error) => {
                tracing::debug!(
                    error = %error,
                    token,
                    kind = %source.image_kind,
                    variant,
                    "image proxy fetch failed; serving fallback"
                );
                fallback_blob(&source.fallback_class)
            }
        }
    }

    pub async fn clear_cache(&self) -> AppResult<()> {
        let _lifecycle_guard = self.cache_lifecycle.write().await;
        if tokio::fs::try_exists(&self.cache_dir)
            .await
            .unwrap_or(false)
        {
            tokio::fs::remove_dir_all(&self.cache_dir)
                .await
                .map_err(|error| {
                    AppError::Repository(format!("failed to clear image cache: {error}"))
                })?;
        }
        tokio::fs::create_dir_all(&self.cache_dir)
            .await
            .map_err(|error| {
                AppError::Repository(format!("failed to recreate image cache: {error}"))
            })?;
        self.repository.clear_image_proxy_cache_entries().await?;
        self.repository.clear_image_proxy_memory();
        self.inflight.lock().await.clear();
        Ok(())
    }

    pub async fn prune(&self) -> AppResult<()> {
        let _lifecycle_guard = self.cache_lifecycle.write().await;
        let cutoff = Utc::now() - chrono::Duration::days(STALE_DAYS);
        self.repository
            .prune_image_proxy_sources_before(cutoff)
            .await?;
        self.reconcile_cache_state().await?;
        self.enforce_budget().await?;
        Ok(())
    }

    async fn local_blob(
        &self,
        source: &ImageProxySourceRecord,
        variant: &str,
    ) -> Option<ImageProxyBlob> {
        let title_id = source
            .owner_type
            .as_deref()
            .filter(|owner_type| *owner_type == "title")
            .and(source.owner_id.as_deref())?;
        let kind = match source.image_kind.as_str() {
            "poster" if matches!(variant, "w70" | "w250") => TitleImageKind::Poster,
            "fanart" if variant == "w1280" => TitleImageKind::Fanart,
            _ => return None,
        };
        self.title_images
            .get_title_image_blob(title_id, kind, variant)
            .await
            .ok()
            .flatten()
            .map(|blob| ImageProxyBlob {
                content_type: blob.content_type,
                etag: blob.etag,
                bytes: blob.bytes,
                fallback: false,
            })
    }

    async fn read_cached(
        self: &Arc<Self>,
        token: &str,
        variant: &str,
    ) -> Option<(ImageProxyCacheEntryRecord, Vec<u8>, CacheFreshness)> {
        let mut entry = self
            .repository
            .get_image_proxy_cache_entry(token, variant)
            .await
            .ok()??;
        let path = self.cache_path(token, variant);
        let bytes = match tokio::fs::read(&path).await {
            Ok(bytes) if bytes.len() as i64 == entry.byte_size => bytes,
            _ => {
                let _ = self
                    .repository
                    .delete_image_proxy_cache_entry(token, variant)
                    .await;
                return None;
            }
        };
        let age = Utc::now().signed_duration_since(entry.fetched_at);
        let freshness = if age <= chrono::Duration::days(FRESH_DAYS) {
            CacheFreshness::Fresh
        } else if age <= chrono::Duration::days(STALE_DAYS) {
            CacheFreshness::Stale
        } else {
            CacheFreshness::Expired
        };
        let now = Utc::now();
        if now.signed_duration_since(entry.last_accessed_at)
            >= chrono::Duration::minutes(ACCESS_TOUCH_MINUTES)
        {
            let observed_fetched_at = entry.fetched_at;
            entry.last_accessed_at = now;
            self.touch_cache_entry_in_background(
                token.to_string(),
                variant.to_string(),
                observed_fetched_at,
                now,
            );
        }
        Some((entry, bytes, freshness))
    }

    async fn fetch_singleflight(
        self: &Arc<Self>,
        source: &ImageProxySourceRecord,
        token: &str,
        variant: &str,
    ) -> AppResult<Option<ImageProxyBlob>> {
        let lifecycle_guard = self.cache_lifecycle.clone().read_owned().await;
        let fetch = self.inflight_fetch(format!("{token}:{variant}")).await;
        let result = fetch
            .get_or_init(move || async move {
                if let Some((entry, bytes, freshness)) = self.read_cached(token, variant).await
                    && freshness != CacheFreshness::Expired
                {
                    return Ok(Some(cached_blob(&entry, bytes)));
                }
                let existing = self
                    .repository
                    .get_image_proxy_cache_entry(token, variant)
                    .await
                    .map_err(|error| error.to_string())?;
                self.fetch_and_cache(source, token, variant, existing, lifecycle_guard)
                    .await
                    .map_err(|error| error.to_string())
            })
            .await;
        clone_shared_fetch_result(result)
    }

    async fn refresh_stale_singleflight(
        self: &Arc<Self>,
        source: &ImageProxySourceRecord,
        token: &str,
        variant: &str,
        observed_fetched_at: DateTime<Utc>,
    ) -> AppResult<()> {
        let lifecycle_guard = self.cache_lifecycle.clone().read_owned().await;
        let fetch = self.inflight_fetch(format!("{token}:{variant}")).await;
        let result = fetch
            .get_or_init(move || async move {
                let existing = self
                    .repository
                    .get_image_proxy_cache_entry(token, variant)
                    .await
                    .map_err(|error| error.to_string())?;
                if existing
                    .as_ref()
                    .is_some_and(|entry| entry.fetched_at > observed_fetched_at)
                {
                    return Ok(None);
                }
                self.fetch_and_cache(source, token, variant, existing, lifecycle_guard)
                    .await
                    .map_err(|error| error.to_string())
            })
            .await;
        clone_shared_fetch_result(result).map(|_| ())
    }

    async fn inflight_fetch(&self, key: String) -> Arc<InflightFetch> {
        let mut inflight = self.inflight.lock().await;
        inflight.retain(|_, fetch| fetch.strong_count() > 0);
        if let Some(existing) = inflight.get(&key).and_then(Weak::upgrade) {
            existing
        } else {
            let created = Arc::new(OnceCell::new());
            inflight.insert(key, Arc::downgrade(&created));
            created
        }
    }

    async fn fetch_and_cache(
        self: &Arc<Self>,
        source: &ImageProxySourceRecord,
        token: &str,
        variant: &str,
        existing: Option<ImageProxyCacheEntryRecord>,
        lifecycle_guard: OwnedRwLockReadGuard<()>,
    ) -> AppResult<Option<ImageProxyBlob>> {
        let Some(source_url) = source.upstream_url.as_deref() else {
            return Ok(None);
        };
        let upstream_url = upstream_variant_url(source_url, &source.image_kind, variant)
            .and_then(|url| approved_upstream_url(&url))
            .ok_or_else(|| AppError::Validation("unapproved image proxy source".to_string()))?;
        let mut response = self
            .outbound_http
            .send(image_fetch_policy(), || {
                let mut request = self
                    .outbound_http
                    .client()
                    .get(&upstream_url)
                    .header(ACCEPT, IMAGE_PROXY_ACCEPT);
                if let Some(entry) = existing.as_ref() {
                    if let Some(etag) = entry.upstream_etag.as_deref() {
                        request = request.header(IF_NONE_MATCH, etag);
                    }
                    if let Some(last_modified) = entry.upstream_last_modified.as_deref() {
                        request = request.header(IF_MODIFIED_SINCE, last_modified);
                    }
                }
                request
            })
            .await
            .map_err(outbound_error)?;

        if response.status() == reqwest::StatusCode::NOT_MODIFIED {
            let Some(mut entry) = existing else {
                return Ok(None);
            };
            let bytes = tokio::fs::read(self.cache_path(token, variant))
                .await
                .map_err(|error| {
                    AppError::Repository(format!("failed to read image cache: {error}"))
                })?;
            entry.fetched_at = Utc::now();
            entry.last_accessed_at = entry.fetched_at;
            self.update_cache_metadata_in_background(entry.clone(), lifecycle_guard);
            return Ok(Some(cached_blob(&entry, bytes)));
        }
        if response.status().is_redirection() || !response.status().is_success() {
            return Err(AppError::Repository(format!(
                "image proxy fetch failed with status {}",
                response.status()
            )));
        }
        if let Some(length) = response
            .headers()
            .get(CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<usize>().ok())
            && length > MAX_SOURCE_BYTES
        {
            return Err(AppError::Validation(
                "image proxy response is too large".to_string(),
            ));
        }
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .and_then(approved_content_type)
            .ok_or_else(|| {
                AppError::Validation("unsupported image proxy content type".to_string())
            })?
            .to_string();
        let etag = response
            .headers()
            .get(ETAG)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let last_modified = response
            .headers()
            .get(LAST_MODIFIED)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let mut bytes = Vec::with_capacity(
            response
                .content_length()
                .unwrap_or_default()
                .min(MAX_SOURCE_BYTES as u64) as usize,
        );
        while let Some(chunk) = response.chunk().await.map_err(|error| {
            AppError::Repository(format!("failed to read image proxy response: {error}"))
        })? {
            if bytes.len().saturating_add(chunk.len()) > MAX_SOURCE_BYTES {
                return Err(AppError::Validation(
                    "image proxy response is too large".to_string(),
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        if !valid_raster_bytes(&content_type, &bytes) {
            return Err(AppError::Validation(
                "image proxy response bytes do not match its image content type".to_string(),
            ));
        }

        let now = Utc::now();
        let entry = ImageProxyCacheEntryRecord {
            token: token.to_string(),
            variant: variant.to_string(),
            content_type: content_type.clone(),
            byte_size: bytes.len() as i64,
            upstream_etag: etag,
            upstream_last_modified: last_modified,
            fetched_at: now,
            last_accessed_at: now,
        };
        if self.effective_max_bytes() > 0 {
            self.persist_cache_entry_in_background(
                token.to_string(),
                variant.to_string(),
                bytes.clone(),
                entry,
                lifecycle_guard,
            );
        }
        Ok(Some(ImageProxyBlob {
            content_type,
            etag: content_etag(&bytes),
            bytes,
            fallback: false,
        }))
    }

    fn flush_source_touches_in_background(self: &Arc<Self>, token: String) {
        let Ok(flush_guard) = self.source_flush.clone().try_lock_owned() else {
            return;
        };
        let runtime = Arc::clone(self);
        tokio::spawn(async move {
            let _flush_guard = flush_guard;
            if let Err(error) = runtime.repository.flush_image_proxy_sources().await {
                tracing::warn!(
                    error = %error,
                    token,
                    "failed to persist image proxy source access"
                );
            }
        });
    }

    fn touch_cache_entry_in_background(
        self: &Arc<Self>,
        token: String,
        variant: String,
        observed_fetched_at: DateTime<Utc>,
        last_accessed_at: DateTime<Utc>,
    ) {
        let runtime = Arc::clone(self);
        tokio::spawn(async move {
            if let Err(error) = runtime
                .repository
                .touch_image_proxy_cache_entry(
                    &token,
                    &variant,
                    observed_fetched_at,
                    last_accessed_at,
                )
                .await
            {
                tracing::warn!(
                    error = %error,
                    token,
                    variant,
                    "failed to update image proxy cache access time"
                );
            }
        });
    }

    fn update_cache_metadata_in_background(
        self: &Arc<Self>,
        entry: ImageProxyCacheEntryRecord,
        lifecycle_guard: OwnedRwLockReadGuard<()>,
    ) {
        let runtime = Arc::clone(self);
        tokio::spawn(async move {
            let _lifecycle_guard = lifecycle_guard;
            if let Err(error) = runtime
                .repository
                .upsert_image_proxy_cache_entry(&entry)
                .await
            {
                tracing::warn!(
                    error = %error,
                    token = %entry.token,
                    variant = %entry.variant,
                    "failed to update image proxy cache metadata"
                );
            }
        });
    }

    fn persist_cache_entry_in_background(
        self: &Arc<Self>,
        token: String,
        variant: String,
        bytes: Vec<u8>,
        entry: ImageProxyCacheEntryRecord,
        lifecycle_guard: OwnedRwLockReadGuard<()>,
    ) {
        let runtime = Arc::clone(self);
        tokio::spawn(async move {
            let _lifecycle_guard = lifecycle_guard;
            let result = async {
                runtime.write_cache_file(&token, &variant, &bytes).await?;
                runtime
                    .repository
                    .upsert_image_proxy_cache_entry(&entry)
                    .await?;
                runtime.enforce_budget().await
            }
            .await;
            if let Err(error) = result {
                tracing::warn!(
                    error = %error,
                    token,
                    variant,
                    "failed to persist image proxy cache entry"
                );
            }
        });
    }

    async fn write_cache_file(&self, token: &str, variant: &str, bytes: &[u8]) -> AppResult<()> {
        tokio::fs::create_dir_all(&self.cache_dir)
            .await
            .map_err(|error| {
                AppError::Repository(format!("failed to create image cache: {error}"))
            })?;
        let path = self.cache_path(token, variant);
        let temp = path.with_extension(format!("{}.part", Utc::now().timestamp_micros()));
        tokio::fs::write(&temp, bytes).await.map_err(|error| {
            AppError::Repository(format!("failed to write image cache: {error}"))
        })?;
        if let Err(error) = tokio::fs::rename(&temp, &path).await {
            let _ = tokio::fs::remove_file(&temp).await;
            return Err(AppError::Repository(format!(
                "failed to atomically commit image cache: {error}"
            )));
        }
        Ok(())
    }

    async fn reconcile_cache_state(&self) -> AppResult<()> {
        tokio::fs::create_dir_all(&self.cache_dir)
            .await
            .map_err(|error| {
                AppError::Repository(format!("failed to create image cache: {error}"))
            })?;
        let entries = self.repository.list_image_proxy_cache_entries_lru().await?;
        let mut expected = entries
            .into_iter()
            .map(|entry| (self.cache_path(&entry.token, &entry.variant), entry))
            .collect::<HashMap<_, _>>();
        let mut files = tokio::fs::read_dir(&self.cache_dir)
            .await
            .map_err(|error| {
                AppError::Repository(format!("failed to inspect image cache: {error}"))
            })?;
        while let Some(file) = files.next_entry().await.map_err(|error| {
            AppError::Repository(format!("failed to inspect image cache: {error}"))
        })? {
            let path = file.path();
            let Some(entry) = expected.remove(&path) else {
                self.remove_cache_file(&path).await?;
                continue;
            };
            let valid_size = file
                .metadata()
                .await
                .ok()
                .is_some_and(|metadata| metadata.len() == entry.byte_size.max(0) as u64);
            if !valid_size {
                self.remove_cache_file(&path).await?;
                self.repository
                    .delete_image_proxy_cache_entry(&entry.token, &entry.variant)
                    .await?;
            }
        }
        for entry in expected.into_values() {
            self.repository
                .delete_image_proxy_cache_entry(&entry.token, &entry.variant)
                .await?;
        }
        Ok(())
    }

    async fn enforce_budget(&self) -> AppResult<()> {
        let max = self.effective_max_bytes();
        let entries = self.repository.list_image_proxy_cache_entries_lru().await?;
        let mut total = entries
            .iter()
            .map(|entry| entry.byte_size.max(0) as u64)
            .sum::<u64>();
        for entry in entries {
            if total <= max {
                break;
            }
            self.remove_cache_file(&self.cache_path(&entry.token, &entry.variant))
                .await?;
            self.repository
                .delete_image_proxy_cache_entry(&entry.token, &entry.variant)
                .await?;
            total = total.saturating_sub(entry.byte_size.max(0) as u64);
        }
        Ok(())
    }

    async fn remove_cache_file(&self, path: &Path) -> AppResult<()> {
        match tokio::fs::remove_file(path).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(AppError::Repository(format!(
                "failed to remove image cache file {}: {error}",
                path.display()
            ))),
        }
    }

    fn cache_path(&self, token: &str, variant: &str) -> PathBuf {
        let key = blake3::hash(format!("{token}\0{variant}").as_bytes())
            .to_hex()
            .to_string();
        self.cache_dir.join(format!("{key}.image"))
    }
}

#[async_trait]
impl ImageProxyCacheControl for ImageProxyRuntime {
    async fn clear_cache(&self) -> AppResult<()> {
        ImageProxyRuntime::clear_cache(self).await
    }

    async fn set_configured_max_bytes(&self, value: u64) -> AppResult<()> {
        let _lifecycle_guard = self.cache_lifecycle.write().await;
        self.configured_cache_bytes.store(value, Ordering::Relaxed);
        self.enforce_budget().await
    }
}

fn image_outbound_http_client() -> OutboundHttpClient {
    OutboundHttpClient::new(no_redirect_reqwest_client(), RateLimitRegistry::isolated())
}

fn image_fetch_policy() -> RequestPolicy {
    RequestPolicy::safe_read("image_proxy", "image_proxy_fetch")
        .with_max_retries(1)
        .with_backoff(Duration::from_millis(250), Duration::from_secs(3))
        .with_host_rps_limit(
            IMAGE_PROXY_HOST_RPS_LANE,
            HostRpsProfile::limited(IMAGE_PROXY_HOST_RPS, IMAGE_PROXY_HOST_RPS_BURST),
        )
}

fn variant_allowed(kind: &str, variant: &str) -> bool {
    match kind {
        "poster" => matches!(variant, "original" | "w250" | "w70"),
        "fanart" => matches!(variant, "original" | "w1280"),
        "episode_still" => variant == "original",
        // Reserved now so a future cast mapper can reuse this route and storage schema.
        "person" => matches!(variant, "original" | "w185"),
        _ => false,
    }
}

fn valid_token(token: &str) -> bool {
    token.len() == 64 && token.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn upstream_variant_url(source_url: &str, kind: &str, variant: &str) -> Option<String> {
    let mut parsed = url::Url::parse(source_url).ok()?;
    if parsed.host_str()?.eq_ignore_ascii_case("image.tmdb.org") {
        let size = match (kind, variant) {
            ("poster", "w70") => "w92",
            ("poster", "w250") => "w300",
            ("poster", "original") | ("fanart", "original") => "original",
            ("fanart", "w1280") => "w1280",
            ("person", "w185") => "w185",
            ("person", "original") => "original",
            ("episode_still", "original") => return Some(parsed.to_string()),
            _ => return None,
        };
        let path = parsed.path();
        let prefix = "/t/p/";
        let rest = path.strip_prefix(prefix)?;
        let (_, asset) = rest.split_once('/')?;
        parsed.set_path(&format!("{prefix}{size}/{asset}"));
    }
    Some(parsed.to_string())
}

fn approved_content_type(value: &str) -> Option<&'static str> {
    match value
        .split(';')
        .next()?
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "image/jpeg" => Some("image/jpeg"),
        "image/png" => Some("image/png"),
        "image/webp" => Some("image/webp"),
        "image/avif" => Some("image/avif"),
        _ => None,
    }
}

#[cfg(feature = "image-processing")]
fn valid_raster_bytes(content_type: &str, bytes: &[u8]) -> bool {
    let format = match content_type {
        "image/jpeg" => image::ImageFormat::Jpeg,
        "image/png" => image::ImageFormat::Png,
        "image/webp" => image::ImageFormat::WebP,
        "image/avif" => image::ImageFormat::Avif,
        _ => return false,
    };
    image::load_from_memory_with_format(bytes, format).is_ok()
}

#[cfg(not(feature = "image-processing"))]
fn valid_raster_bytes(content_type: &str, bytes: &[u8]) -> bool {
    match content_type {
        "image/jpeg" => bytes.starts_with(&[0xff, 0xd8, 0xff]) && bytes.ends_with(&[0xff, 0xd9]),
        "image/png" => {
            bytes.starts_with(b"\x89PNG\r\n\x1a\n")
                && bytes.len() >= 20
                && bytes.ends_with(b"\x00\x00\x00\x00IEND\xaeB`\x82")
        }
        "image/webp" => {
            bytes.len() >= 12
                && bytes.starts_with(b"RIFF")
                && &bytes[8..12] == b"WEBP"
                && u32::from_le_bytes(bytes[4..8].try_into().unwrap_or_default()) as usize + 8
                    == bytes.len()
        }
        "image/avif" => {
            bytes.len() >= 12
                && &bytes[4..8] == b"ftyp"
                && bytes[8..bytes.len().min(64)]
                    .windows(4)
                    .any(|brand| matches!(brand, b"avif" | b"avis"))
        }
        _ => false,
    }
}

fn clone_shared_fetch_result(result: &SharedFetchResult) -> AppResult<Option<ImageProxyBlob>> {
    result.clone().map_err(AppError::Repository)
}

fn cached_blob(entry: &ImageProxyCacheEntryRecord, bytes: Vec<u8>) -> ImageProxyBlob {
    ImageProxyBlob {
        content_type: entry.content_type.clone(),
        etag: content_etag(&bytes),
        bytes,
        fallback: false,
    }
}

fn fallback_blob(class: &str) -> ImageProxyBlob {
    let bytes = if class == "portrait" {
        PORTRAIT_FALLBACK.to_vec()
    } else {
        LANDSCAPE_FALLBACK.to_vec()
    };
    ImageProxyBlob {
        content_type: "image/svg+xml".to_string(),
        etag: content_etag(&bytes),
        bytes,
        fallback: true,
    }
}

fn content_etag(bytes: &[u8]) -> String {
    format!("\"blake3:{}\"", blake3::hash(bytes).to_hex())
}

fn outbound_error(error: OutboundHttpError) -> AppError {
    match error {
        OutboundHttpError::RateLimited(rate_limited) => AppError::Repository(format!(
            "image proxy fetch was rate limited{}",
            rate_limited
                .retry_after
                .map(|delay| format!(" for {}s", delay.as_secs()))
                .unwrap_or_default()
        )),
        OutboundHttpError::Transport { source, .. } => {
            AppError::Repository(format!("image proxy fetch failed: {source}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use super::{
        IMAGE_PROXY_HOST_RPS, IMAGE_PROXY_HOST_RPS_BURST, IMAGE_PROXY_HOST_RPS_LANE,
        ImageProxyRuntime, image_fetch_policy, image_outbound_http_client, upstream_variant_url,
        valid_raster_bytes, valid_token, variant_allowed,
    };
    use async_trait::async_trait;
    use chrono::Utc;
    use scryer_application::{
        AppResult, ImageProxyCacheControl, ImageProxyCacheEntryRecord, ImageProxyRegistration,
        ImageProxyRepository, ImageProxySourceRecord, TitleImageBlob, TitleImageKind,
        TitleImageRepository, TitleImageSourceResult, TitleImageSyncTask,
    };
    use scryer_domain::{DomainEvent, NewDomainEvent};
    use scryer_outbound_http::{HostKey, HostRpsProfile, HostRpsProfileSource, RateLimitRegistry};
    use tokio::sync::Notify;

    struct TestImageRepository {
        source: ImageProxySourceRecord,
        cache_entries: Mutex<Vec<ImageProxyCacheEntryRecord>>,
        cache_reads: AtomicUsize,
        cache_read_delay: Option<std::time::Duration>,
        cache_write_started: Option<Arc<Notify>>,
        cache_write_release: Option<Arc<Notify>>,
        cache_deletes: AtomicUsize,
        cache_clears: AtomicUsize,
        memory_clears: AtomicUsize,
    }

    #[async_trait]
    impl ImageProxyRepository for TestImageRepository {
        fn register_image_source(&self, _registration: ImageProxyRegistration) -> String {
            unreachable!("test repository does not register sources")
        }

        async fn flush_image_proxy_sources(&self) -> AppResult<()> {
            Ok(())
        }

        fn clear_image_proxy_memory(&self) {
            self.memory_clears.fetch_add(1, Ordering::Relaxed);
        }

        async fn get_image_proxy_source(
            &self,
            token: &str,
        ) -> AppResult<Option<ImageProxySourceRecord>> {
            Ok((token == self.source.token).then(|| self.source.clone()))
        }

        async fn get_image_proxy_cache_entry(
            &self,
            token: &str,
            variant: &str,
        ) -> AppResult<Option<ImageProxyCacheEntryRecord>> {
            self.cache_reads.fetch_add(1, Ordering::Relaxed);
            if let Some(delay) = self.cache_read_delay {
                tokio::time::sleep(delay).await;
            }
            Ok(self
                .cache_entries
                .lock()
                .expect("cache entries lock")
                .iter()
                .find(|entry| entry.token == token && entry.variant == variant)
                .cloned())
        }

        async fn upsert_image_proxy_cache_entry(
            &self,
            entry: &ImageProxyCacheEntryRecord,
        ) -> AppResult<()> {
            if let Some(started) = self.cache_write_started.as_ref() {
                started.notify_one();
            }
            if let Some(release) = self.cache_write_release.as_ref() {
                release.notified().await;
            }
            let mut entries = self.cache_entries.lock().expect("cache entries lock");
            if let Some(existing) = entries
                .iter_mut()
                .find(|existing| existing.token == entry.token && existing.variant == entry.variant)
            {
                *existing = entry.clone();
            } else {
                entries.push(entry.clone());
            }
            Ok(())
        }

        async fn touch_image_proxy_cache_entry(
            &self,
            token: &str,
            variant: &str,
            observed_fetched_at: chrono::DateTime<Utc>,
            last_accessed_at: chrono::DateTime<Utc>,
        ) -> AppResult<()> {
            let mut entries = self.cache_entries.lock().expect("cache entries lock");
            if let Some(entry) = entries.iter_mut().find(|entry| {
                entry.token == token
                    && entry.variant == variant
                    && entry.fetched_at == observed_fetched_at
            }) {
                entry.last_accessed_at = last_accessed_at;
            }
            Ok(())
        }

        async fn delete_image_proxy_cache_entry(
            &self,
            token: &str,
            variant: &str,
        ) -> AppResult<()> {
            let mut entries = self.cache_entries.lock().expect("cache entries lock");
            let original_len = entries.len();
            entries.retain(|entry| entry.token != token || entry.variant != variant);
            if entries.len() != original_len {
                self.cache_deletes.fetch_add(1, Ordering::Relaxed);
            }
            Ok(())
        }

        async fn list_image_proxy_cache_entries_lru(
            &self,
        ) -> AppResult<Vec<ImageProxyCacheEntryRecord>> {
            let mut entries = self
                .cache_entries
                .lock()
                .expect("cache entries lock")
                .clone();
            entries.sort_by_key(|entry| entry.last_accessed_at);
            Ok(entries)
        }

        async fn clear_image_proxy_cache_entries(&self) -> AppResult<()> {
            self.cache_entries
                .lock()
                .expect("cache entries lock")
                .clear();
            self.cache_clears.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }

        async fn prune_image_proxy_sources_before(
            &self,
            _cutoff: chrono::DateTime<Utc>,
        ) -> AppResult<u64> {
            Ok(0)
        }
    }

    struct TestTitleImageRepository {
        blob: TitleImageBlob,
        reads: AtomicUsize,
    }

    #[async_trait]
    impl TitleImageRepository for TestTitleImageRepository {
        async fn list_title_image_refresh_work(
            &self,
            _limit: usize,
            _skipped: &[TitleImageSyncTask],
        ) -> AppResult<Vec<TitleImageSyncTask>> {
            Ok(Vec::new())
        }

        async fn clear_title_image_cache(&self) -> AppResult<()> {
            Ok(())
        }

        async fn upsert_title_image_source_result(
            &self,
            _title_id: &str,
            _result: TitleImageSourceResult,
            _event: Option<NewDomainEvent>,
        ) -> AppResult<Option<DomainEvent>> {
            Ok(None)
        }

        async fn get_title_image_blob(
            &self,
            _title_id: &str,
            _kind: TitleImageKind,
            _variant_key: &str,
        ) -> AppResult<Option<TitleImageBlob>> {
            self.reads.fetch_add(1, Ordering::Relaxed);
            Ok(Some(TitleImageBlob {
                content_type: self.blob.content_type.clone(),
                etag: self.blob.etag.clone(),
                bytes: self.blob.bytes.clone(),
            }))
        }
    }

    #[test]
    fn image_http_client_has_an_isolated_governor_and_dedicated_lane() {
        let host = HostKey::from("image-proxy-isolation.example.test");
        let shared_registry = RateLimitRegistry::new();
        shared_registry.register_host_profile(
            host.clone(),
            HostRpsProfile::limited(1.0, 1),
            HostRpsProfileSource::ExplicitRegistration,
        );

        let image_http = image_outbound_http_client();
        assert_ne!(
            image_http.registry().profile_for_host(&host).source,
            HostRpsProfileSource::ExplicitRegistration
        );

        let policy = image_fetch_policy();
        let image_limit = policy
            .host_rps_override
            .expect("image fetches use a dedicated governor lane");
        assert_eq!(image_limit.lane.as_ref(), IMAGE_PROXY_HOST_RPS_LANE);
        assert_eq!(
            image_limit.profile.requests_per_second,
            IMAGE_PROXY_HOST_RPS
        );
        assert_eq!(image_limit.profile.burst, IMAGE_PROXY_HOST_RPS_BURST);
    }

    #[tokio::test]
    async fn clear_waits_for_background_cache_persistence_then_removes_it() {
        let token = "e".repeat(64);
        let write_started = Arc::new(Notify::new());
        let write_release = Arc::new(Notify::new());
        let image_repository = Arc::new(TestImageRepository {
            source: ImageProxySourceRecord {
                token: token.clone(),
                upstream_url: None,
                owner_type: Some("episode".to_string()),
                owner_id: Some("episode-3".to_string()),
                image_kind: "episode_still".to_string(),
                fallback_class: "landscape".to_string(),
                last_seen_at: Utc::now(),
            },
            cache_entries: Mutex::new(Vec::new()),
            cache_reads: AtomicUsize::new(0),
            cache_read_delay: None,
            cache_write_started: Some(write_started.clone()),
            cache_write_release: Some(write_release.clone()),
            cache_deletes: AtomicUsize::new(0),
            cache_clears: AtomicUsize::new(0),
            memory_clears: AtomicUsize::new(0),
        });
        let title_images = Arc::new(TestTitleImageRepository {
            blob: TitleImageBlob {
                content_type: "image/avif".to_string(),
                etag: "\"unused\"".to_string(),
                bytes: Vec::new(),
            },
            reads: AtomicUsize::new(0),
        });
        let temp = tempfile::tempdir().expect("temporary image cache");
        let runtime = Arc::new(ImageProxyRuntime::new(
            image_repository.clone(),
            title_images,
            temp.path(),
        ));
        let bytes = b"background-cache-write".to_vec();
        let now = Utc::now();
        let entry = ImageProxyCacheEntryRecord {
            token: token.clone(),
            variant: "original".to_string(),
            content_type: "image/png".to_string(),
            byte_size: bytes.len() as i64,
            upstream_etag: None,
            upstream_last_modified: None,
            fetched_at: now,
            last_accessed_at: now,
        };
        let lifecycle_guard = runtime.cache_lifecycle.clone().read_owned().await;

        runtime.persist_cache_entry_in_background(
            token.clone(),
            "original".to_string(),
            bytes.clone(),
            entry,
            lifecycle_guard,
        );

        tokio::time::timeout(std::time::Duration::from_secs(1), write_started.notified())
            .await
            .expect("background cache write should reach the repository");
        assert!(
            image_repository
                .cache_entries
                .lock()
                .expect("cache entries lock")
                .is_empty(),
            "the caller returned while cache persistence remained blocked"
        );

        let clear_runtime = Arc::clone(&runtime);
        let clear_task = tokio::spawn(async move { clear_runtime.clear_cache().await });
        tokio::task::yield_now().await;
        assert!(
            !clear_task.is_finished(),
            "clear must wait until the in-flight cache write releases its lifecycle guard"
        );

        write_release.notify_one();
        clear_task
            .await
            .expect("clear task")
            .expect("clear image proxy cache");
        assert!(
            image_repository
                .cache_entries
                .lock()
                .expect("cache entries lock")
                .is_empty(),
            "clear removes metadata written by the completed background task"
        );
        assert!(
            !tokio::fs::try_exists(runtime.cache_path(&token, "original"))
                .await
                .expect("inspect cleared cache file"),
            "clear removes bytes written by the completed background task"
        );
    }

    #[test]
    fn policies_are_kind_specific_and_person_extensible() {
        assert!(variant_allowed("poster", "w250"));
        assert!(!variant_allowed("poster", "w1280"));
        assert!(variant_allowed("episode_still", "original"));
        assert!(variant_allowed("person", "w185"));
    }

    #[test]
    fn tmdb_variant_mapping_preserves_asset_identity() {
        assert_eq!(
            upstream_variant_url(
                "https://image.tmdb.org/t/p/w500/poster.jpg",
                "poster",
                "w70"
            )
            .as_deref(),
            Some("https://image.tmdb.org/t/p/w92/poster.jpg")
        );
        assert_eq!(
            upstream_variant_url(
                "https://image.tmdb.org/t/p/original/poster.jpg",
                "poster",
                "w250"
            )
            .as_deref(),
            Some("https://image.tmdb.org/t/p/w300/poster.jpg")
        );
        assert_eq!(
            upstream_variant_url(
                "https://image.tmdb.org/t/p/original/background.jpg",
                "fanart",
                "w1280"
            )
            .as_deref(),
            Some("https://image.tmdb.org/t/p/w1280/background.jpg")
        );
        assert_eq!(
            upstream_variant_url(
                "https://artworks.thetvdb.com/banners/poster.jpg",
                "poster",
                "w250"
            )
            .as_deref(),
            Some("https://artworks.thetvdb.com/banners/poster.jpg")
        );
    }

    #[test]
    fn tokens_are_fixed_length_blake3_hex() {
        assert!(valid_token(&"a".repeat(64)));
        assert!(!valid_token("abc"));
        assert!(!valid_token(&format!("{}z", "a".repeat(63))));
    }

    #[tokio::test]
    async fn local_title_avif_wins_and_clear_resets_all_proxy_cache_state() {
        let token = "a".repeat(64);
        let image_repository = Arc::new(TestImageRepository {
            source: ImageProxySourceRecord {
                token: token.clone(),
                upstream_url: Some("https://image.tmdb.org/t/p/original/poster.jpg".to_string()),
                owner_type: Some("title".to_string()),
                owner_id: Some("title-1".to_string()),
                image_kind: "poster".to_string(),
                fallback_class: "portrait".to_string(),
                last_seen_at: Utc::now(),
            },
            cache_entries: Mutex::new(Vec::new()),
            cache_reads: AtomicUsize::new(0),
            cache_read_delay: None,
            cache_write_started: None,
            cache_write_release: None,
            cache_deletes: AtomicUsize::new(0),
            cache_clears: AtomicUsize::new(0),
            memory_clears: AtomicUsize::new(0),
        });
        let title_images = Arc::new(TestTitleImageRepository {
            blob: TitleImageBlob {
                content_type: "image/avif".to_string(),
                etag: "\"local\"".to_string(),
                bytes: vec![1, 2, 3],
            },
            reads: AtomicUsize::new(0),
        });
        let temp = tempfile::tempdir().expect("temporary image cache");
        let runtime = Arc::new(ImageProxyRuntime::new(
            image_repository.clone(),
            title_images.clone(),
            temp.path(),
        ));

        let blob = runtime.resolve(&token, "w250").await;

        assert_eq!(blob.content_type, "image/avif");
        assert_eq!(blob.bytes, vec![1, 2, 3]);
        assert_eq!(title_images.reads.load(Ordering::Relaxed), 1);
        assert_eq!(image_repository.cache_reads.load(Ordering::Relaxed), 0);

        let unknown = runtime.resolve(&"b".repeat(64), "original").await;
        assert!(unknown.fallback);
        assert_eq!(title_images.reads.load(Ordering::Relaxed), 1);
        assert_eq!(image_repository.cache_reads.load(Ordering::Relaxed), 0);

        tokio::fs::create_dir_all(&runtime.cache_dir)
            .await
            .expect("create proxy cache directory");
        tokio::fs::write(runtime.cache_dir.join("orphan.image"), b"cached")
            .await
            .expect("seed proxy cache file");
        runtime.clear_cache().await.expect("clear proxy cache");
        assert!(
            tokio::fs::read_dir(&runtime.cache_dir)
                .await
                .expect("read cleared proxy cache")
                .next_entry()
                .await
                .expect("read next cache entry")
                .is_none()
        );
        assert_eq!(image_repository.cache_clears.load(Ordering::Relaxed), 1);
        assert_eq!(image_repository.memory_clears.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn disk_cache_obeys_fresh_stale_expired_and_budget_boundaries() {
        let token = "c".repeat(64);
        let bytes = b"disk-cache".to_vec();
        let now = Utc::now();
        let image_repository = Arc::new(TestImageRepository {
            source: ImageProxySourceRecord {
                token: token.clone(),
                upstream_url: None,
                owner_type: Some("episode".to_string()),
                owner_id: Some("episode-1".to_string()),
                image_kind: "episode_still".to_string(),
                fallback_class: "landscape".to_string(),
                last_seen_at: now,
            },
            cache_entries: Mutex::new(vec![ImageProxyCacheEntryRecord {
                token: token.clone(),
                variant: "original".to_string(),
                content_type: "image/png".to_string(),
                byte_size: bytes.len() as i64,
                upstream_etag: Some("upstream-etag".to_string()),
                upstream_last_modified: None,
                fetched_at: now,
                last_accessed_at: now,
            }]),
            cache_reads: AtomicUsize::new(0),
            cache_read_delay: None,
            cache_write_started: None,
            cache_write_release: None,
            cache_deletes: AtomicUsize::new(0),
            cache_clears: AtomicUsize::new(0),
            memory_clears: AtomicUsize::new(0),
        });
        let title_images = Arc::new(TestTitleImageRepository {
            blob: TitleImageBlob {
                content_type: "image/avif".to_string(),
                etag: "\"unused\"".to_string(),
                bytes: Vec::new(),
            },
            reads: AtomicUsize::new(0),
        });
        let temp = tempfile::tempdir().expect("temporary image cache");
        let mut runtime =
            ImageProxyRuntime::new(image_repository.clone(), title_images.clone(), temp.path());
        runtime.environment_override_bytes = None;
        let runtime = Arc::new(runtime);
        tokio::fs::create_dir_all(&runtime.cache_dir)
            .await
            .expect("create image cache");
        let cache_path = runtime.cache_path(&token, "original");
        tokio::fs::write(&cache_path, &bytes)
            .await
            .expect("write cached image");

        let fresh = runtime.resolve(&token, "original").await;
        assert_eq!(fresh.bytes, bytes);
        assert!(!fresh.fallback);

        image_repository
            .cache_entries
            .lock()
            .expect("cache entries lock")[0]
            .fetched_at = now - chrono::Duration::days(8);
        let stale = runtime.resolve(&token, "original").await;
        assert_eq!(stale.bytes, bytes);
        assert!(!stale.fallback);

        image_repository
            .cache_entries
            .lock()
            .expect("cache entries lock")[0]
            .fetched_at = now - chrono::Duration::days(31);
        let expired = runtime.resolve(&token, "original").await;
        assert!(expired.fallback);

        image_repository
            .cache_entries
            .lock()
            .expect("cache entries lock")[0]
            .fetched_at = now;
        ImageProxyCacheControl::set_configured_max_bytes(runtime.as_ref(), 0)
            .await
            .expect("reduce cache budget");
        assert!(!tokio::fs::try_exists(cache_path).await.expect("cache path"));
        assert!(
            image_repository
                .cache_entries
                .lock()
                .expect("cache entries lock")
                .is_empty()
        );
        assert_eq!(title_images.reads.load(Ordering::Relaxed), 0);
        assert!(image_repository.cache_deletes.load(Ordering::Relaxed) >= 1);
    }

    #[tokio::test]
    async fn maintenance_reconciles_files_and_evicts_the_least_recently_used_entries() {
        let tokens = ["e".repeat(64), "f".repeat(64), "1".repeat(64)];
        let now = Utc::now();
        let entry = |token: &str, age_minutes: i64| ImageProxyCacheEntryRecord {
            token: token.to_string(),
            variant: "original".to_string(),
            content_type: "image/png".to_string(),
            byte_size: 4,
            upstream_etag: None,
            upstream_last_modified: None,
            fetched_at: now,
            last_accessed_at: now - chrono::Duration::minutes(age_minutes),
        };
        let image_repository = Arc::new(TestImageRepository {
            source: ImageProxySourceRecord {
                token: tokens[0].clone(),
                upstream_url: None,
                owner_type: Some("episode".to_string()),
                owner_id: Some("episode-lru".to_string()),
                image_kind: "episode_still".to_string(),
                fallback_class: "landscape".to_string(),
                last_seen_at: now,
            },
            cache_entries: Mutex::new(vec![
                entry(&tokens[0], 30),
                entry(&tokens[1], 20),
                entry(&tokens[2], 10),
            ]),
            cache_reads: AtomicUsize::new(0),
            cache_read_delay: None,
            cache_write_started: None,
            cache_write_release: None,
            cache_deletes: AtomicUsize::new(0),
            cache_clears: AtomicUsize::new(0),
            memory_clears: AtomicUsize::new(0),
        });
        let title_images = Arc::new(TestTitleImageRepository {
            blob: TitleImageBlob {
                content_type: "image/avif".to_string(),
                etag: "\"unused\"".to_string(),
                bytes: Vec::new(),
            },
            reads: AtomicUsize::new(0),
        });
        let temp = tempfile::tempdir().expect("temporary image cache");
        let mut runtime =
            ImageProxyRuntime::new(image_repository.clone(), title_images, temp.path());
        runtime.environment_override_bytes = None;
        let runtime = Arc::new(runtime);
        tokio::fs::create_dir_all(&runtime.cache_dir)
            .await
            .expect("create image cache directory");
        for token in &tokens {
            tokio::fs::write(runtime.cache_path(token, "original"), [1, 2, 3, 4])
                .await
                .expect("seed cached image");
        }

        ImageProxyCacheControl::set_configured_max_bytes(runtime.as_ref(), 8)
            .await
            .expect("enforce image cache budget");
        assert!(
            !tokio::fs::try_exists(runtime.cache_path(&tokens[0], "original"))
                .await
                .expect("oldest cache path")
        );
        assert!(
            tokio::fs::try_exists(runtime.cache_path(&tokens[1], "original"))
                .await
                .expect("second cache path")
        );
        assert!(
            tokio::fs::try_exists(runtime.cache_path(&tokens[2], "original"))
                .await
                .expect("newest cache path")
        );

        let missing_token = "2".repeat(64);
        image_repository
            .upsert_image_proxy_cache_entry(&entry(&missing_token, 5))
            .await
            .expect("seed metadata without bytes");
        let orphan = runtime.cache_dir.join("orphan.image");
        tokio::fs::write(&orphan, b"orphan")
            .await
            .expect("seed bytes without metadata");
        runtime
            .prune()
            .await
            .expect("run startup-style maintenance");

        assert!(!tokio::fs::try_exists(orphan).await.expect("orphan path"));
        assert!(
            image_repository
                .get_image_proxy_cache_entry(&missing_token, "original")
                .await
                .expect("read reconciled metadata")
                .is_none()
        );
        let remaining_tokens = image_repository
            .cache_entries
            .lock()
            .expect("cache entries lock")
            .iter()
            .map(|entry| entry.token.clone())
            .collect::<Vec<_>>();
        assert_eq!(remaining_tokens, vec![tokens[1].clone(), tokens[2].clone()]);

        let undeletable_path = runtime.cache_path(&tokens[1], "original");
        tokio::fs::remove_file(&undeletable_path)
            .await
            .expect("replace cached file with a directory");
        tokio::fs::create_dir(&undeletable_path)
            .await
            .expect("create undeletable cache-path fixture");
        ImageProxyCacheControl::set_configured_max_bytes(runtime.as_ref(), 0)
            .await
            .expect_err("eviction must report a filesystem deletion failure");
        assert!(
            image_repository
                .get_image_proxy_cache_entry(&tokens[1], "original")
                .await
                .expect("read retained metadata")
                .is_some(),
            "failed file deletion must retain accounting metadata"
        );
    }

    #[tokio::test]
    async fn concurrent_resolve_calls_share_one_cache_miss_initializer() {
        const REQUEST_COUNT: usize = 8;
        let token = "d".repeat(64);
        let image_repository = Arc::new(TestImageRepository {
            source: ImageProxySourceRecord {
                token: token.clone(),
                upstream_url: None,
                owner_type: Some("episode".to_string()),
                owner_id: Some("episode-2".to_string()),
                image_kind: "episode_still".to_string(),
                fallback_class: "landscape".to_string(),
                last_seen_at: Utc::now(),
            },
            cache_entries: Mutex::new(Vec::new()),
            cache_reads: AtomicUsize::new(0),
            cache_read_delay: Some(std::time::Duration::from_millis(10)),
            cache_write_started: None,
            cache_write_release: None,
            cache_deletes: AtomicUsize::new(0),
            cache_clears: AtomicUsize::new(0),
            memory_clears: AtomicUsize::new(0),
        });
        let title_images = Arc::new(TestTitleImageRepository {
            blob: TitleImageBlob {
                content_type: "image/avif".to_string(),
                etag: "\"unused\"".to_string(),
                bytes: Vec::new(),
            },
            reads: AtomicUsize::new(0),
        });
        let temp = tempfile::tempdir().expect("temporary image cache");
        let runtime = Arc::new(ImageProxyRuntime::new(
            image_repository.clone(),
            title_images,
            temp.path(),
        ));

        let mut tasks = Vec::new();
        for _ in 0..REQUEST_COUNT {
            let runtime = Arc::clone(&runtime);
            let token = token.clone();
            tasks.push(tokio::spawn(async move {
                runtime.resolve(&token, "original").await
            }));
        }
        for task in tasks {
            assert!(task.await.expect("concurrent image resolve").fallback);
        }

        assert_eq!(
            image_repository.cache_reads.load(Ordering::Relaxed),
            REQUEST_COUNT + 2,
            "each request performs its initial cache lookup, while one shared initializer performs the two miss-path lookups"
        );
    }

    #[cfg(feature = "image-processing")]
    #[test]
    fn raster_validation_decodes_the_complete_image() {
        assert!(!valid_raster_bytes("image/jpeg", &[0xff, 0xd8, 0xff]));
        assert!(!valid_raster_bytes(
            "image/png",
            b"\x89PNG\r\n\x1a\ntruncated"
        ));

        let image = image::DynamicImage::new_rgba8(1, 1);
        let mut encoded = std::io::Cursor::new(Vec::new());
        image
            .write_to(&mut encoded, image::ImageFormat::Png)
            .expect("encode valid test image");
        assert!(valid_raster_bytes("image/png", encoded.get_ref()));
    }
}

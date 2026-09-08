use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use reqwest::header::{
    ACCEPT, CONTENT_LENGTH, CONTENT_TYPE, ETAG, IF_MODIFIED_SINCE, IF_NONE_MATCH, LAST_MODIFIED,
};
use scryer_application::{
    AppError, AppResult, ImageProxyCacheControl, ImageProxyCacheEntryRecord, ImageProxyCacheUsage,
    ImageProxyRepository, ImageProxySourceRecord, TitleImageKind, TitleImageProcessor,
    TitleImageRepository,
};
use scryer_outbound_http::{
    HostRpsProfile, OutboundHttpClient, OutboundHttpError, RateLimitRegistry, RequestPolicy,
    no_redirect_reqwest_client,
};
use tokio::sync::{Mutex, Notify, OnceCell, OwnedRwLockReadGuard, RwLock};

use super::image_proxy_store::approved_upstream_url;

const MAX_SOURCE_BYTES: usize = 20 * 1024 * 1024;
const DEFAULT_CACHE_BYTES: u64 = 256 * 1024 * 1024;
const FRESH_DAYS: i64 = 7;
const STALE_DAYS: i64 = 30;
const ACCESS_TOUCH_MINUTES: i64 = 60;
/// Least-recently-used rows fetched per eviction round while the cache is
/// over budget; keeps enforcement bounded instead of loading the whole table.
const EVICTION_BATCH: u32 = 64;
const IMAGE_PROXY_HOST_RPS: f64 = 100.0;
const IMAGE_PROXY_HOST_RPS_BURST: u32 = 100;
const IMAGE_PROXY_HOST_RPS_LANE: &str = "image_proxy";
const IMAGE_PROXY_ACCEPT: &str = "image/webp,image/jpeg;q=0.9,image/png;q=0.8";

#[derive(Clone, Debug)]
pub struct ImageProxyBlob {
    pub content_type: String,
    pub etag: String,
    pub bytes: Vec<u8>,
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
    /// Readers must see bytes and MIME metadata from the same publication.
    cache_publication: Arc<RwLock<()>>,
    /// Coalesces budget enforcement across concurrent background persists:
    /// one pass runs at a time and overlapping writers request another pass.
    budget_gate: Arc<Mutex<()>>,
    budget_dirty: Arc<AtomicBool>,
    jpeg_wake: Arc<Notify>,
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
            cache_publication: Arc::new(RwLock::new(())),
            budget_gate: Arc::new(Mutex::new(())),
            budget_dirty: Arc::new(AtomicBool::new(false)),
            jpeg_wake: Arc::new(Notify::new()),
        }
    }

    pub fn configured_max_bytes(&self) -> u64 {
        self.configured_cache_bytes.load(Ordering::Relaxed)
    }

    pub fn effective_max_bytes(&self) -> u64 {
        self.environment_override_bytes
            .unwrap_or_else(|| self.configured_max_bytes())
    }

    pub async fn resolve(self: &Arc<Self>, token: &str, variant: &str) -> Option<ImageProxyBlob> {
        if !valid_token(token) {
            return None;
        }
        let source = match self.repository.get_image_proxy_source(token).await {
            Ok(Some(source)) => source,
            Ok(None) => return None,
            Err(error) => {
                tracing::warn!(error = %error, token, "failed to load image proxy source");
                return None;
            }
        };
        self.flush_source_touches_in_background(token.to_string());

        if !variant_allowed(&source.image_kind, variant) {
            return None;
        }

        if let Some(blob) = self.local_blob(&source, variant).await {
            return Some(blob);
        }

        let cached = {
            let _guard = self.cache_lifecycle.read().await;
            self.read_cached(token, variant).await
        };
        if let Some((entry, bytes, freshness)) = cached {
            if freshness == CacheFreshness::Fresh {
                return Some(cached_blob(&entry, bytes));
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
                return Some(cached_blob(&entry, bytes));
            }
        }

        match self.fetch_singleflight(&source, token, variant).await {
            Ok(blob) => blob,
            Err(error) => {
                tracing::debug!(
                    error = %error,
                    token,
                    kind = %source.image_kind,
                    variant,
                    "image proxy fetch failed; image unavailable"
                );
                None
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
        let orphaned = self
            .repository
            .prune_orphaned_discovery_image_proxy_sources()
            .await?;
        if orphaned > 0 {
            tracing::info!(
                removed = orphaned,
                "pruned image proxy sources whose discovery items no longer exist"
            );
        }
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
            })
    }

    async fn read_cached(
        self: &Arc<Self>,
        token: &str,
        variant: &str,
    ) -> Option<(ImageProxyCacheEntryRecord, Vec<u8>, CacheFreshness)> {
        let _publication = self.cache_publication.read().await;
        let mut entry = self
            .repository
            .get_image_proxy_cache_entry(token, variant)
            .await
            .ok()??;
        let path = self.existing_cache_path(&entry).await;
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
            let bytes = tokio::fs::read(self.existing_cache_path(&entry).await)
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
                let _publication = runtime.cache_publication.write().await;
                runtime
                    .write_cache_file_at(&runtime.entry_cache_path(&entry), &bytes)
                    .await?;
                runtime
                    .repository
                    .upsert_image_proxy_cache_entry(&entry)
                    .await?;
                runtime.remove_obsolete_cache_file(&entry).await?;
                if entry.content_type == "image/jpeg" {
                    runtime.jpeg_wake.notify_one();
                }
                runtime.enforce_budget_after_write().await
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

    async fn enforce_budget_after_write(&self) -> AppResult<()> {
        self.budget_dirty.store(true, Ordering::Release);
        let mut result = Ok(());
        loop {
            let Ok(gate) = self.budget_gate.try_lock() else {
                return result;
            };
            if self.budget_dirty.swap(false, Ordering::AcqRel)
                && let Err(error) = self.enforce_budget().await
            {
                result = Err(error);
            }
            // Release before checking for another write. A writer arriving
            // after this check can then acquire the gate itself; one that
            // skipped while we held it leaves dirty set for this loop.
            drop(gate);
            if !self.budget_dirty.load(Ordering::Acquire) {
                return result;
            }
        }
    }

    #[cfg(test)]
    async fn write_cache_file(&self, token: &str, variant: &str, bytes: &[u8]) -> AppResult<()> {
        self.write_cache_file_at(&self.cache_path(token, variant), bytes)
            .await
    }

    async fn write_cache_file_at(&self, path: &Path, bytes: &[u8]) -> AppResult<()> {
        tokio::fs::create_dir_all(&self.cache_dir)
            .await
            .map_err(|error| {
                AppError::Repository(format!("failed to create image cache: {error}"))
            })?;
        let temp = path.with_extension(format!("{}.part", uuid::Uuid::new_v4()));
        tokio::fs::write(&temp, bytes).await.map_err(|error| {
            AppError::Repository(format!("failed to write image cache: {error}"))
        })?;
        if let Err(error) = tokio::fs::rename(&temp, path).await {
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
        let mut expected = HashMap::new();
        for entry in entries {
            expected.insert(self.existing_cache_path(&entry).await, entry);
        }
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
        let ImageProxyCacheUsage {
            mut total_bytes,
            mut entry_count,
        } = self.repository.image_proxy_cache_usage().await?;
        while total_bytes > max && entry_count > 0 {
            let batch = self
                .repository
                .list_image_proxy_cache_entries_lru_oldest(EVICTION_BATCH)
                .await?;
            if batch.is_empty() {
                break;
            }
            for entry in batch {
                if total_bytes <= max {
                    return Ok(());
                }
                self.remove_cache_file(&self.existing_cache_path(&entry).await)
                    .await?;
                self.remove_obsolete_cache_file(&entry).await?;
                self.repository
                    .delete_image_proxy_cache_entry(&entry.token, &entry.variant)
                    .await?;
                total_bytes = total_bytes.saturating_sub(entry.byte_size.max(0) as u64);
                entry_count = entry_count.saturating_sub(1);
            }
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

    fn entry_cache_path(&self, entry: &ImageProxyCacheEntryRecord) -> PathBuf {
        let path = self.cache_path(&entry.token, &entry.variant);
        if entry.content_type == "image/avif" {
            path.with_extension("avif")
        } else {
            path
        }
    }

    async fn existing_cache_path(&self, entry: &ImageProxyCacheEntryRecord) -> PathBuf {
        let path = self.entry_cache_path(entry);
        // Older upstream AVIF responses used the format-independent filename.
        if entry.content_type == "image/avif"
            && !tokio::fs::try_exists(&path).await.unwrap_or(false)
        {
            self.cache_path(&entry.token, &entry.variant)
        } else {
            path
        }
    }

    async fn remove_obsolete_cache_file(
        &self,
        entry: &ImageProxyCacheEntryRecord,
    ) -> AppResult<()> {
        let legacy = self.cache_path(&entry.token, &entry.variant);
        let obsolete = if entry.content_type == "image/avif" {
            legacy
        } else {
            legacy.with_extension("avif")
        };
        self.remove_cache_file(&obsolete).await
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
    async fn list_cached_jpegs(
        &self,
        limit: usize,
        after: Option<(&str, &str)>,
    ) -> AppResult<Vec<ImageProxyCacheEntryRecord>> {
        if !cfg!(feature = "image-processing") {
            return Ok(Vec::new());
        }
        self.repository.list_cached_jpegs(limit, after).await
    }

    async fn wait_for_cached_jpeg(&self) {
        self.jpeg_wake.notified().await;
    }

    async fn optimize_cached_jpeg(
        &self,
        entry: ImageProxyCacheEntryRecord,
        processor: Arc<dyn TitleImageProcessor>,
    ) -> AppResult<bool> {
        let (bytes, width) = {
            let _guard = self.cache_lifecycle.read().await;
            let Some(current) = self
                .repository
                .get_image_proxy_cache_entry(&entry.token, &entry.variant)
                .await?
            else {
                return Ok(false);
            };
            if entry.content_type != "image/jpeg" || !same_cached_image(&entry, &current) {
                return Ok(false);
            }
            if entry.byte_size <= 0 || entry.byte_size as u64 > MAX_SOURCE_BYTES as u64 {
                return Err(AppError::Validation(
                    "cached JPEG exceeds source size limit".into(),
                ));
            }
            let Some(source) = self.repository.get_image_proxy_source(&entry.token).await? else {
                return Ok(false);
            };
            let width = match (source.image_kind.as_str(), entry.variant.as_str()) {
                ("poster", "w70") => 70,
                ("poster", _) => 300,
                ("fanart", _) => 1280,
                ("episode_still", _) => 300,
                ("person", _) => 185,
                _ => return Err(AppError::Validation("unsupported cached JPEG kind".into())),
            };
            let bytes = match tokio::fs::read(self.cache_path(&entry.token, &entry.variant)).await {
                Ok(bytes) => bytes,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
                Err(error) => {
                    return Err(AppError::Repository(format!(
                        "failed to read cached JPEG: {error}"
                    )));
                }
            };
            if bytes.len() as i64 != entry.byte_size {
                return Ok(false);
            }
            (bytes, width)
        };
        let original_digest = blake3::hash(&bytes);
        let encoded = processor.encode_cached_jpeg(bytes, width).await?;
        if !valid_raster_bytes("image/avif", &encoded) {
            return Err(AppError::Validation(
                "cached JPEG encoder returned invalid AVIF".into(),
            ));
        }

        // Encoding holds no cache lock. Recheck after refresh/eviction/reset, then publish
        // the separate AVIF file before switching metadata and deleting the JPEG.
        let _guard = self.cache_lifecycle.write().await;
        let Some(mut current) = self
            .repository
            .get_image_proxy_cache_entry(&entry.token, &entry.variant)
            .await?
        else {
            return Ok(false);
        };
        if !same_cached_image(&entry, &current) {
            return Ok(false);
        }
        let legacy = self.cache_path(&entry.token, &entry.variant);
        let Ok(original) = tokio::fs::read(&legacy).await else {
            return Ok(false);
        };
        if blake3::hash(&original) != original_digest {
            return Ok(false);
        }
        current.content_type = "image/avif".into();
        current.byte_size = encoded.len() as i64;
        self.write_cache_file_at(&self.entry_cache_path(&current), &encoded)
            .await?;
        self.repository
            .upsert_image_proxy_cache_entry(&current)
            .await?;
        self.remove_cache_file(&legacy).await?;
        self.enforce_budget().await?;
        tracing::debug!(token = %entry.token, variant = %entry.variant, width, source_bytes = entry.byte_size, avif_bytes = encoded.len(), "optimized cached JPEG");
        Ok(true)
    }

    async fn clear_cache(&self) -> AppResult<()> {
        ImageProxyRuntime::clear_cache(self).await
    }

    async fn set_configured_max_bytes(&self, value: u64) -> AppResult<()> {
        let _lifecycle_guard = self.cache_lifecycle.write().await;
        self.configured_cache_bytes.store(value, Ordering::Relaxed);
        self.enforce_budget().await
    }
}

fn same_cached_image(a: &ImageProxyCacheEntryRecord, b: &ImageProxyCacheEntryRecord) -> bool {
    a.content_type == b.content_type
        && a.byte_size == b.byte_size
        && a.fetched_at == b.fetched_at
        && a.upstream_etag == b.upstream_etag
        && a.upstream_last_modified == b.upstream_last_modified
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
        "episode_still" => matches!(variant, "original" | "w300"),
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
            ("episode_still", "w300") => "w300",
            ("episode_still", "original") => "original",
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

/// Structural container check only: the bytes are served verbatim to the
/// browser, so a full decode buys nothing and costs a CPU-bound pass per
/// fetch. Walk container boundaries so a complete image may have trailing
/// bytes without mistaking a marker inside metadata for its actual trailer.
fn valid_raster_bytes(content_type: &str, bytes: &[u8]) -> bool {
    match content_type {
        "image/jpeg" => complete_jpeg_container(bytes),
        "image/png" => complete_png_container(bytes),
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

fn complete_jpeg_container(bytes: &[u8]) -> bool {
    if !bytes.starts_with(&[0xff, 0xd8]) {
        return false;
    }
    let mut offset = 2;
    let mut in_scan = false;
    let mut saw_scan = false;
    while offset < bytes.len() {
        if in_scan {
            let Some(next_marker) = bytes[offset..].iter().position(|byte| *byte == 0xff) else {
                return false;
            };
            offset += next_marker;
        }
        if bytes.get(offset) != Some(&0xff) {
            return false;
        }
        while bytes.get(offset) == Some(&0xff) {
            offset += 1;
        }
        let Some(&marker) = bytes.get(offset) else {
            return false;
        };
        offset += 1;
        match marker {
            0x00 | 0xd0..=0xd7 if in_scan => continue,
            0xd9 => return saw_scan,
            0x01 => continue,
            0x00 | 0xd0..=0xd8 => return false,
            _ => {}
        }
        let Some(length_bytes) = bytes.get(offset..offset + 2) else {
            return false;
        };
        let length = usize::from(u16::from_be_bytes([length_bytes[0], length_bytes[1]]));
        if length < 2 || length > bytes.len() - offset {
            return false;
        }
        offset += length;
        // DNL can interrupt entropy data without ending the current scan.
        in_scan = marker == 0xda || (in_scan && marker == 0xdc);
        saw_scan |= in_scan;
    }
    false
}

fn complete_png_container(bytes: &[u8]) -> bool {
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return false;
    }
    let mut offset = 8;
    while let Some(header) = bytes.get(offset..offset + 8) {
        let length = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
        let Some(end) = offset
            .checked_add(12)
            .and_then(|start| start.checked_add(length))
        else {
            return false;
        };
        if end > bytes.len() {
            return false;
        }
        if &header[4..] == b"IEND" {
            return length == 0 && &bytes[offset + 8..end] == b"\xaeB`\x82";
        }
        offset = end;
    }
    false
}

fn clone_shared_fetch_result(result: &SharedFetchResult) -> AppResult<Option<ImageProxyBlob>> {
    result.clone().map_err(AppError::Repository)
}

fn cached_blob(entry: &ImageProxyCacheEntryRecord, bytes: Vec<u8>) -> ImageProxyBlob {
    ImageProxyBlob {
        content_type: entry.content_type.clone(),
        etag: content_etag(&bytes),
        bytes,
    }
}

fn content_etag(bytes: &[u8]) -> String {
    format!("\"blake3:{}\"", blake3::hash(bytes).to_hex())
}

fn outbound_error(error: OutboundHttpError) -> AppError {
    match error {
        OutboundHttpError::DispatchRejected => AppError::canceled("outbound dispatch is closed"),
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
        AppResult, ImageProxyCacheControl, ImageProxyCacheEntryRecord, ImageProxyCacheUsage,
        ImageProxyRegistration, ImageProxyRepository, ImageProxySourceRecord, TitleImageBlob,
        TitleImageKind, TitleImageRepository, TitleImageSourceResult, TitleImageSyncTask,
    };
    use scryer_domain::{DomainEvent, NewDomainEvent};
    use scryer_outbound_http::{HostKey, HostRpsProfile, HostRpsProfileSource, RateLimitRegistry};
    use tokio::sync::Notify;

    struct GatedJpegProcessor {
        started: Notify,
        release: tokio::sync::Semaphore,
    }

    #[async_trait]
    impl scryer_application::TitleImageProcessor for GatedJpegProcessor {
        async fn encode_cached_jpeg(&self, _bytes: Vec<u8>, width: u32) -> AppResult<Vec<u8>> {
            assert_eq!(width, 300);
            self.started.notify_one();
            self.release.acquire().await.unwrap().forget();
            Ok([&[0, 0, 0, 0x1c][..], b"ftypavif", &[0u8; 16]].concat())
        }

        async fn fetch_and_process_image(
            &self,
            _: TitleImageKind,
            _: &str,
            _: Vec<scryer_application::TitleImageVariantSpec>,
        ) -> AppResult<TitleImageSourceResult> {
            unreachable!("disk conversion must never fetch a library master")
        }
    }

    async fn jpeg_fixture(
        block_metadata: bool,
    ) -> (
        tempfile::TempDir,
        Arc<ImageProxyRuntime>,
        Arc<TestImageRepository>,
        ImageProxyCacheEntryRecord,
        Arc<GatedJpegProcessor>,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let entry = ImageProxyCacheEntryRecord {
            token: "a".repeat(64),
            variant: "w250".into(),
            content_type: "image/jpeg".into(),
            byte_size: 4,
            upstream_etag: Some("original-etag".into()),
            upstream_last_modified: None,
            fetched_at: Utc::now(),
            last_accessed_at: Utc::now(),
        };
        let repo = Arc::new(TestImageRepository {
            source: ImageProxySourceRecord {
                token: entry.token.clone(),
                upstream_url: None,
                owner_type: Some("discovery".into()),
                owner_id: Some("discovery-1".into()),
                image_kind: "poster".into(),
                fallback_class: "portrait".into(),
                last_seen_at: Utc::now(),
            },
            cache_entries: Mutex::new(vec![entry.clone()]),
            cache_reads: AtomicUsize::new(0),
            cache_read_delay: None,
            cache_write_started: block_metadata.then(|| Arc::new(Notify::new())),
            cache_write_release: block_metadata.then(|| Arc::new(Notify::new())),
            cache_deletes: AtomicUsize::new(0),
            cache_clears: AtomicUsize::new(0),
            memory_clears: AtomicUsize::new(0),
            full_lru_lists: AtomicUsize::new(0),
            usage_reads: AtomicUsize::new(0),
            usage_pause: Mutex::new(None),
            orphan_sweeps: AtomicUsize::new(0),
        });
        let titles = Arc::new(TestTitleImageRepository {
            blob: TitleImageBlob {
                content_type: "image/avif".into(),
                etag: "unused".into(),
                bytes: vec![],
            },
            reads: AtomicUsize::new(0),
        });
        let mut runtime = ImageProxyRuntime::new(repo.clone(), titles, temp.path());
        runtime.environment_override_bytes = None;
        let runtime = Arc::new(runtime);
        runtime
            .write_cache_file(&entry.token, &entry.variant, b"jpeg")
            .await
            .unwrap();
        let processor = Arc::new(GatedJpegProcessor {
            started: Notify::new(),
            release: tokio::sync::Semaphore::new(0),
        });
        (temp, runtime, repo, entry, processor)
    }

    #[tokio::test]
    async fn jpeg_conversion_survives_restart_and_obeys_cache_budget() {
        let (temp, runtime, repo, entry, processor) = jpeg_fixture(false).await;
        processor.release.add_permits(1);
        assert!(
            runtime
                .optimize_cached_jpeg(entry.clone(), processor.clone())
                .await
                .unwrap()
        );
        assert!(
            !tokio::fs::try_exists(runtime.cache_path(&entry.token, &entry.variant))
                .await
                .unwrap()
        );
        let converted = repo
            .get_image_proxy_cache_entry(&entry.token, &entry.variant)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(converted.content_type, "image/avif");
        assert_eq!(converted.upstream_etag, entry.upstream_etag);
        assert_eq!(converted.fetched_at, entry.fetched_at);
        assert!(
            !runtime
                .optimize_cached_jpeg(entry.clone(), processor)
                .await
                .unwrap()
        );
        let mut restarted = ImageProxyRuntime::new(repo, runtime.title_images.clone(), temp.path());
        restarted.environment_override_bytes = None;
        let restarted = Arc::new(restarted);
        restarted.prune().await.unwrap();
        let blob = restarted
            .resolve(&entry.token, &entry.variant)
            .await
            .unwrap();
        assert_eq!(blob.content_type, "image/avif");
        assert_eq!(blob.bytes.len() as i64, converted.byte_size);
        restarted.set_configured_max_bytes(0).await.unwrap();
        assert!(
            !tokio::fs::try_exists(restarted.entry_cache_path(&converted))
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn jpeg_refresh_publishes_format_and_bytes_together_and_wakes_encoding() {
        let (_temp, runtime, repo, jpeg, _processor) = jpeg_fixture(true).await;
        let mut avif = jpeg.clone();
        avif.content_type = "image/avif".into();
        avif.byte_size = 3;
        repo.cache_entries.lock().unwrap()[0] = avif.clone();
        runtime
            .write_cache_file_at(&runtime.entry_cache_path(&avif), b"old")
            .await
            .unwrap();
        let guard = runtime.cache_lifecycle.clone().read_owned().await;
        runtime.persist_cache_entry_in_background(
            jpeg.token.clone(),
            jpeg.variant.clone(),
            b"jpeg".to_vec(),
            jpeg.clone(),
            guard,
        );
        repo.cache_write_started.as_ref().unwrap().notified().await;
        let reader_runtime = runtime.clone();
        let reader_entry = jpeg.clone();
        let reader = tokio::spawn(async move {
            reader_runtime
                .resolve(&reader_entry.token, &reader_entry.variant)
                .await
                .unwrap()
        });
        tokio::task::yield_now().await;
        assert!(!reader.is_finished());
        repo.cache_write_release.as_ref().unwrap().notify_one();
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            runtime.wait_for_cached_jpeg(),
        )
        .await
        .unwrap();
        let blob = reader.await.unwrap();
        assert_eq!(blob.content_type, "image/jpeg");
        assert_eq!(blob.bytes, b"jpeg");
        assert!(
            !tokio::fs::try_exists(runtime.entry_cache_path(&avif))
                .await
                .unwrap()
        );
        assert_eq!(repo.cache_deletes.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn jpeg_conversion_does_not_resurrect_a_cleared_cache() {
        let (_temp, runtime, _repo, entry, processor) = jpeg_fixture(false).await;
        let task_runtime = runtime.clone();
        let task_processor = processor.clone();
        let task_entry = entry.clone();
        let task = tokio::spawn(async move {
            task_runtime
                .optimize_cached_jpeg(task_entry, task_processor)
                .await
        });
        processor.started.notified().await;
        runtime.clear_cache().await.unwrap();
        processor.release.add_permits(1);
        assert!(!task.await.unwrap().unwrap());
        assert!(
            runtime
                .resolve(&entry.token, &entry.variant)
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn jpeg_conversion_abandoned_before_metadata_commit_preserves_original() {
        let (_temp, runtime, repo, entry, processor) = jpeg_fixture(true).await;
        processor.release.add_permits(1);
        let task_runtime = runtime.clone();
        let task_entry = entry.clone();
        let task = tokio::spawn(async move {
            task_runtime
                .optimize_cached_jpeg(task_entry, processor)
                .await
        });
        repo.cache_write_started.as_ref().unwrap().notified().await;
        let avif_path = runtime
            .cache_path(&entry.token, &entry.variant)
            .with_extension("avif");
        assert!(tokio::fs::try_exists(&avif_path).await.unwrap());
        assert_eq!(
            tokio::fs::read(runtime.cache_path(&entry.token, &entry.variant))
                .await
                .unwrap(),
            b"jpeg"
        );
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        let blob = runtime.resolve(&entry.token, &entry.variant).await.unwrap();
        assert_eq!(blob.content_type, "image/jpeg");
        assert_eq!(blob.bytes, b"jpeg");
        runtime.prune().await.unwrap();
        assert!(!tokio::fs::try_exists(avif_path).await.unwrap());
        assert!(
            tokio::fs::try_exists(runtime.cache_path(&entry.token, &entry.variant))
                .await
                .unwrap()
        );
    }

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
        full_lru_lists: AtomicUsize,
        usage_reads: AtomicUsize,
        usage_pause: Mutex<Option<(Arc<Notify>, Arc<Notify>)>>,
        orphan_sweeps: AtomicUsize,
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
            self.full_lru_lists.fetch_add(1, Ordering::Relaxed);
            let mut entries = self
                .cache_entries
                .lock()
                .expect("cache entries lock")
                .clone();
            entries.sort_by_key(|entry| entry.last_accessed_at);
            Ok(entries)
        }

        async fn list_image_proxy_cache_entries_lru_oldest(
            &self,
            limit: u32,
        ) -> AppResult<Vec<ImageProxyCacheEntryRecord>> {
            let mut entries = self
                .cache_entries
                .lock()
                .expect("cache entries lock")
                .clone();
            entries.sort_by_key(|entry| entry.last_accessed_at);
            entries.truncate(limit as usize);
            Ok(entries)
        }

        async fn image_proxy_cache_usage(&self) -> AppResult<ImageProxyCacheUsage> {
            self.usage_reads.fetch_add(1, Ordering::Relaxed);
            let usage = {
                let entries = self.cache_entries.lock().expect("cache entries lock");
                ImageProxyCacheUsage {
                    total_bytes: entries
                        .iter()
                        .map(|entry| entry.byte_size.max(0) as u64)
                        .sum(),
                    entry_count: entries.len() as u64,
                }
            };
            let pause = self.usage_pause.lock().expect("usage pause lock").take();
            if let Some((started, release)) = pause {
                started.notify_one();
                release.notified().await;
            }
            Ok(usage)
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

        async fn prune_orphaned_discovery_image_proxy_sources(&self) -> AppResult<u64> {
            self.orphan_sweeps.fetch_add(1, Ordering::Relaxed);
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
            full_lru_lists: AtomicUsize::new(0),
            usage_reads: AtomicUsize::new(0),
            usage_pause: Mutex::new(None),
            orphan_sweeps: AtomicUsize::new(0),
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
        assert!(variant_allowed("episode_still", "w300"));
        assert!(!variant_allowed("episode_still", "w1280"));
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
                "https://image.tmdb.org/t/p/original/episode.jpg",
                "episode_still",
                "w300"
            )
            .as_deref(),
            Some("https://image.tmdb.org/t/p/w300/episode.jpg")
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
            full_lru_lists: AtomicUsize::new(0),
            usage_reads: AtomicUsize::new(0),
            usage_pause: Mutex::new(None),
            orphan_sweeps: AtomicUsize::new(0),
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

        let blob = runtime
            .resolve(&token, "w250")
            .await
            .expect("local image should resolve");

        assert_eq!(blob.content_type, "image/avif");
        assert_eq!(blob.bytes, vec![1, 2, 3]);
        assert_eq!(title_images.reads.load(Ordering::Relaxed), 1);
        assert_eq!(image_repository.cache_reads.load(Ordering::Relaxed), 0);

        let unknown = runtime.resolve(&"b".repeat(64), "original").await;
        assert!(unknown.is_none());
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
            full_lru_lists: AtomicUsize::new(0),
            usage_reads: AtomicUsize::new(0),
            usage_pause: Mutex::new(None),
            orphan_sweeps: AtomicUsize::new(0),
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

        let fresh = runtime
            .resolve(&token, "original")
            .await
            .expect("fresh cached image should resolve");
        assert_eq!(fresh.bytes, bytes);

        image_repository
            .cache_entries
            .lock()
            .expect("cache entries lock")[0]
            .fetched_at = now - chrono::Duration::days(8);
        let stale = runtime
            .resolve(&token, "original")
            .await
            .expect("stale cached image should remain usable");
        assert_eq!(stale.bytes, bytes);

        image_repository
            .cache_entries
            .lock()
            .expect("cache entries lock")[0]
            .fetched_at = now - chrono::Duration::days(31);
        let expired = runtime.resolve(&token, "original").await;
        assert!(expired.is_none());

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
            full_lru_lists: AtomicUsize::new(0),
            usage_reads: AtomicUsize::new(0),
            usage_pause: Mutex::new(None),
            orphan_sweeps: AtomicUsize::new(0),
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
            full_lru_lists: AtomicUsize::new(0),
            usage_reads: AtomicUsize::new(0),
            usage_pause: Mutex::new(None),
            orphan_sweeps: AtomicUsize::new(0),
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
            assert!(task.await.expect("concurrent image resolve").is_none());
        }

        assert_eq!(
            image_repository.cache_reads.load(Ordering::Relaxed),
            REQUEST_COUNT + 2,
            "each request performs its initial cache lookup, while one shared initializer performs the two miss-path lookups"
        );
    }

    #[test]
    fn raster_validation_checks_container_structure_without_decoding() {
        // Truncated payloads fail on their trailers.
        assert!(!valid_raster_bytes("image/jpeg", &[0xff, 0xd8, 0xff]));
        assert!(!valid_raster_bytes(
            "image/png",
            b"\x89PNG\r\n\x1a\ntruncated"
        ));
        assert!(!valid_raster_bytes(
            "image/webp",
            b"RIFF\x10\x00\x00\x00WEBPVP8 "
        ));
        assert!(!valid_raster_bytes("image/gif", b"GIF89a"));

        // Well-formed containers pass without the pixel data being decodable:
        // the bytes are relayed verbatim, never rasterised on the server.
        let jpeg = [
            &[0xff, 0xd8, 0xff, 0xda, 0x00, 0x02][..],
            &[0x01, 0x02, 0x03],
            &[0xff, 0xd9],
        ]
        .concat();
        assert!(valid_raster_bytes("image/jpeg", &jpeg));
        let mut jpeg_with_trailer = jpeg.clone();
        jpeg_with_trailer.extend_from_slice(b"trailer");
        assert!(valid_raster_bytes("image/jpeg", &jpeg_with_trailer));
        assert!(!valid_raster_bytes("image/jpeg", &jpeg[..jpeg.len() - 2]));
        let jpeg_with_embedded_eoi = [
            &[0xff, 0xd8, 0xff, 0xe0, 0x00, 0x06][..],
            &[0xff, 0xd9, 0x00, 0x00],
            &[0xff, 0xda, 0x00, 0x02, 0x01, 0xff, 0xd9],
        ]
        .concat();
        assert!(valid_raster_bytes("image/jpeg", &jpeg_with_embedded_eoi));
        let png = [
            &b"\x89PNG\r\n\x1a\n"[..],
            &[0u8; 12],
            b"\x00\x00\x00\x00IEND\xaeB`\x82",
        ]
        .concat();
        assert!(valid_raster_bytes("image/png", &png));
        let mut png_with_trailer = png.clone();
        png_with_trailer.extend_from_slice(b"trailer");
        assert!(valid_raster_bytes("image/png", &png_with_trailer));
        assert!(!valid_raster_bytes("image/png", &png[..png.len() - 1]));
        let png_with_embedded_iend = [
            &b"\x89PNG\r\n\x1a\n"[..],
            &4u32.to_be_bytes(),
            b"IDAT",
            b"IEND",
            &[0u8; 4],
        ]
        .concat();
        assert!(!valid_raster_bytes("image/png", &png_with_embedded_iend));
        let webp = [&b"RIFF"[..], &12u32.to_le_bytes(), b"WEBPVP8 ", &[0u8; 4]].concat();
        assert!(valid_raster_bytes("image/webp", &webp));
        let avif = [&[0, 0, 0, 0x1c][..], b"ftypavif", &[0u8; 16]].concat();
        assert!(valid_raster_bytes("image/avif", &avif));
    }

    #[cfg(feature = "image-processing")]
    #[test]
    fn raster_validation_accepts_decodable_images_with_trailing_bytes() {
        let image = image::DynamicImage::new_rgb8(1, 1);
        for (content_type, format) in [
            ("image/jpeg", image::ImageFormat::Jpeg),
            ("image/png", image::ImageFormat::Png),
        ] {
            let mut encoded = std::io::Cursor::new(Vec::new());
            image
                .write_to(&mut encoded, format)
                .expect("encode fixture image");
            let mut bytes = encoded.into_inner();
            assert!(valid_raster_bytes(content_type, &bytes));
            bytes.extend_from_slice(b"\ntrailer");
            assert!(valid_raster_bytes(content_type, &bytes));
        }
    }

    #[tokio::test]
    async fn budget_enforcement_reads_the_usage_scalar_and_evicts_in_bounded_batches() {
        let tokens = ["2".repeat(64), "3".repeat(64), "4".repeat(64)];
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
                owner_id: Some("episode-budget".to_string()),
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
            full_lru_lists: AtomicUsize::new(0),
            usage_reads: AtomicUsize::new(0),
            usage_pause: Mutex::new(None),
            orphan_sweeps: AtomicUsize::new(0),
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

        // 12 bytes cached against an 8 byte budget: exactly the oldest entry goes.
        ImageProxyCacheControl::set_configured_max_bytes(runtime.as_ref(), 8)
            .await
            .expect("apply cache budget");

        let remaining = image_repository
            .cache_entries
            .lock()
            .expect("cache entries lock")
            .iter()
            .map(|entry| entry.token.clone())
            .collect::<Vec<_>>();
        assert_eq!(remaining, vec![tokens[1].clone(), tokens[2].clone()]);
        assert!(
            !tokio::fs::try_exists(runtime.cache_path(&tokens[0], "original"))
                .await
                .expect("evicted path")
        );
        assert_eq!(image_repository.cache_deletes.load(Ordering::Relaxed), 1);
        assert_eq!(
            image_repository.full_lru_lists.load(Ordering::Relaxed),
            0,
            "budget enforcement must never list the whole cache table"
        );
        assert_eq!(image_repository.usage_reads.load(Ordering::Relaxed), 1);

        // Under budget: one scalar read, no listing, no deletes.
        ImageProxyCacheControl::set_configured_max_bytes(runtime.as_ref(), 1024)
            .await
            .expect("raise cache budget");
        assert_eq!(image_repository.cache_deletes.load(Ordering::Relaxed), 1);
        assert_eq!(image_repository.full_lru_lists.load(Ordering::Relaxed), 0);
        assert_eq!(image_repository.usage_reads.load(Ordering::Relaxed), 2);

        // Maintenance still reconciles with the full listing and sweeps
        // orphaned discovery sources exactly once per pass.
        runtime.prune().await.expect("prune image cache");
        assert_eq!(image_repository.orphan_sweeps.load(Ordering::Relaxed), 1);
        assert_eq!(image_repository.full_lru_lists.load(Ordering::Relaxed), 1);

        // A writer that arrives after the active pass read an under-budget
        // snapshot leaves dirty set; the active owner must take a second pass.
        ImageProxyCacheControl::set_configured_max_bytes(runtime.as_ref(), 8)
            .await
            .expect("restore constrained cache budget");
        let usage_started = Arc::new(Notify::new());
        let usage_release = Arc::new(Notify::new());
        *image_repository
            .usage_pause
            .lock()
            .expect("usage pause lock") = Some((usage_started.clone(), usage_release.clone()));
        let reads_before = image_repository.usage_reads.load(Ordering::Relaxed);
        let active_runtime = runtime.clone();
        let active = tokio::spawn(async move { active_runtime.enforce_budget_after_write().await });
        usage_started.notified().await;
        image_repository
            .cache_entries
            .lock()
            .expect("cache entries lock")
            .push(entry(&tokens[0], 5));
        tokio::fs::write(runtime.cache_path(&tokens[0], "original"), [1, 2, 3, 4])
            .await
            .expect("seed concurrently persisted cache image");
        runtime
            .enforce_budget_after_write()
            .await
            .expect("request a second budget pass");
        usage_release.notify_one();
        active
            .await
            .expect("active budget task")
            .expect("active budget enforcement");
        assert!(
            image_repository
                .image_proxy_cache_usage()
                .await
                .expect("read budget after coalesced enforcement")
                .total_bytes
                <= 8
        );
        assert_eq!(
            image_repository.usage_reads.load(Ordering::Relaxed),
            reads_before + 3,
            "the active owner performs the initial and dirty rerun; the final assertion reads once"
        );
    }
}

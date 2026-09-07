use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use scryer_application::{
    AppError, AppResult, ImageProxyCacheEntryRecord, ImageProxyCacheUsage, ImageProxyRegistration,
    ImageProxyRepository, ImageProxySourceRecord, image_proxy_source_token,
};

use crate::storage::sql::runtime::{SqlArg, SqlRuntime, StoreDatastore};

use super::normalized_base_path_from_env;

const MEMORY_SOURCE_LIMIT: usize = 10_000;
const SOURCE_TOUCH_INTERVAL_HOURS: i64 = 24;

#[derive(Default)]
struct MemorySources {
    records: HashMap<String, ImageProxySourceRecord>,
    insertion_order: VecDeque<String>,
}

#[derive(Clone)]
pub struct ImageProxyStore {
    datastore: StoreDatastore,
    base_path: String,
    memory: Arc<Mutex<MemorySources>>,
    pending: Arc<Mutex<HashMap<String, ImageProxySourceRecord>>>,
}

impl ImageProxyStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self {
            datastore,
            base_path: normalized_base_path_from_env(),
            memory: Arc::new(Mutex::new(MemorySources::default())),
            pending: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn remember(&self, record: ImageProxySourceRecord) {
        let Ok(mut memory) = self.memory.lock() else {
            return;
        };
        if !memory.records.contains_key(&record.token) {
            memory.insertion_order.push_back(record.token.clone());
        }
        memory.records.insert(record.token.clone(), record);
        while memory.records.len() > MEMORY_SOURCE_LIMIT {
            let Some(oldest) = memory.insertion_order.pop_front() else {
                break;
            };
            memory.records.remove(&oldest);
        }
    }

    fn memory_source(&self, token: &str) -> Option<ImageProxySourceRecord> {
        self.memory
            .lock()
            .ok()
            .and_then(|memory| memory.records.get(token).cloned())
    }

    fn queue_source(&self, record: ImageProxySourceRecord) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.insert(record.token.clone(), record);
        }
    }

    fn touch_source_if_stale(&self, mut record: ImageProxySourceRecord) -> ImageProxySourceRecord {
        let now = Utc::now();
        if now.signed_duration_since(record.last_seen_at)
            >= chrono::Duration::hours(SOURCE_TOUCH_INTERVAL_HOURS)
        {
            record.last_seen_at = now;
            self.remember(record.clone());
            self.queue_source(record.clone());
        }
        record
    }
}

#[async_trait]
impl ImageProxyRepository for ImageProxyStore {
    fn register_image_source(&self, registration: ImageProxyRegistration) -> String {
        let upstream_url = registration
            .upstream_url
            .as_deref()
            .and_then(approved_upstream_url);
        if registration.upstream_url.is_some() && upstream_url.is_none() {
            tracing::debug!(
                kind = registration.image_kind.as_str(),
                "rejected unapproved image proxy source"
            );
        }

        let token = image_proxy_source_token(
            upstream_url.as_deref(),
            registration.owner_type.as_deref(),
            registration.owner_id.as_deref(),
            registration.image_kind,
        );
        let record = ImageProxySourceRecord {
            token: token.clone(),
            upstream_url,
            owner_type: registration.owner_type,
            owner_id: registration.owner_id,
            image_kind: registration.image_kind.as_str().to_string(),
            fallback_class: registration.fallback_class,
            last_seen_at: Utc::now(),
        };
        let already_registered = self.memory_source(&token).filter(|existing| {
            existing.upstream_url == record.upstream_url
                && existing.owner_type == record.owner_type
                && existing.owner_id == record.owner_id
                && existing.image_kind == record.image_kind
                && existing.fallback_class == record.fallback_class
        });
        if let Some(existing) = already_registered {
            self.touch_source_if_stale(existing);
        } else {
            self.remember(record.clone());
            self.queue_source(record);
        }

        format!(
            "{}/images/media/{token}/{}",
            self.base_path, registration.default_variant
        )
    }

    async fn flush_image_proxy_sources(&self) -> AppResult<()> {
        let records = self
            .pending
            .lock()
            .map(|pending| pending.values().cloned().collect::<Vec<_>>())
            .map_err(|_| AppError::Repository("image proxy source queue is unavailable".into()))?;
        persist_queued_sources(&self.datastore, &self.pending, records).await
    }

    fn clear_image_proxy_memory(&self) {
        if let Ok(mut memory) = self.memory.lock() {
            *memory = MemorySources::default();
        }
    }

    async fn get_image_proxy_source(
        &self,
        token: &str,
    ) -> AppResult<Option<ImageProxySourceRecord>> {
        if let Some(record) = self.memory_source(token) {
            return Ok(Some(self.touch_source_if_stale(record)));
        }
        let record = fetch_source(&self.datastore, token)
            .await?
            .map(|record| self.touch_source_if_stale(record));
        if let Some(record) = &record {
            self.remember(record.clone());
        }
        Ok(record)
    }

    async fn get_image_proxy_cache_entry(
        &self,
        token: &str,
        variant: &str,
    ) -> AppResult<Option<ImageProxyCacheEntryRecord>> {
        SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT token, variant, content_type, byte_size, upstream_etag, upstream_last_modified, fetched_at, last_accessed_at
               FROM image_proxy_cache_entries
              WHERE token = {} AND variant = {}",
            &[SqlArg::Text(token.to_string()), SqlArg::Text(variant.to_string())],
        )
        .await?
        .map(cache_entry_from_row)
        .transpose()
    }

    async fn upsert_image_proxy_cache_entry(
        &self,
        entry: &ImageProxyCacheEntryRecord,
    ) -> AppResult<()> {
        SqlRuntime::execute_write(
            &self.datastore,
            "upsert_image_proxy_cache_entry",
            "INSERT INTO image_proxy_cache_entries
                (token, variant, content_type, byte_size, upstream_etag, upstream_last_modified, fetched_at, last_accessed_at)
             VALUES ({}, {}, {}, {}, {}, {}, {}, {})
             ON CONFLICT(token, variant) DO UPDATE SET
                content_type = excluded.content_type,
                byte_size = excluded.byte_size,
                upstream_etag = excluded.upstream_etag,
                upstream_last_modified = excluded.upstream_last_modified,
                fetched_at = excluded.fetched_at,
                last_accessed_at = excluded.last_accessed_at",
            vec![
                SqlArg::Text(entry.token.clone()),
                SqlArg::Text(entry.variant.clone()),
                SqlArg::Text(entry.content_type.clone()),
                SqlArg::I64(entry.byte_size),
                SqlArg::OptText(entry.upstream_etag.clone()),
                SqlArg::OptText(entry.upstream_last_modified.clone()),
                SqlArg::Timestamp(entry.fetched_at),
                SqlArg::Timestamp(entry.last_accessed_at),
            ],
        )
        .await?;
        Ok(())
    }

    async fn touch_image_proxy_cache_entry(
        &self,
        token: &str,
        variant: &str,
        observed_fetched_at: DateTime<Utc>,
        last_accessed_at: DateTime<Utc>,
    ) -> AppResult<()> {
        SqlRuntime::execute_write(
            &self.datastore,
            "touch_image_proxy_cache_entry",
            "UPDATE image_proxy_cache_entries
                SET last_accessed_at = {}
              WHERE token = {} AND variant = {} AND fetched_at = {}",
            vec![
                SqlArg::Timestamp(last_accessed_at),
                SqlArg::Text(token.to_string()),
                SqlArg::Text(variant.to_string()),
                SqlArg::Timestamp(observed_fetched_at),
            ],
        )
        .await?;
        Ok(())
    }

    async fn delete_image_proxy_cache_entry(&self, token: &str, variant: &str) -> AppResult<()> {
        SqlRuntime::execute_write(
            &self.datastore,
            "delete_image_proxy_cache_entry",
            "DELETE FROM image_proxy_cache_entries WHERE token = {} AND variant = {}",
            vec![
                SqlArg::Text(token.to_string()),
                SqlArg::Text(variant.to_string()),
            ],
        )
        .await?;
        Ok(())
    }

    async fn list_image_proxy_cache_entries_lru(
        &self,
    ) -> AppResult<Vec<ImageProxyCacheEntryRecord>> {
        SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT token, variant, content_type, byte_size, upstream_etag, upstream_last_modified, fetched_at, last_accessed_at
               FROM image_proxy_cache_entries
              ORDER BY last_accessed_at ASC",
            &[],
        )
        .await?
        .into_iter()
        .map(cache_entry_from_row)
        .collect()
    }

    async fn list_image_proxy_cache_entries_lru_oldest(
        &self,
        limit: u32,
    ) -> AppResult<Vec<ImageProxyCacheEntryRecord>> {
        SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT token, variant, content_type, byte_size, upstream_etag, upstream_last_modified, fetched_at, last_accessed_at
               FROM image_proxy_cache_entries
              ORDER BY last_accessed_at ASC
              LIMIT {}",
            &[SqlArg::I64(i64::from(limit))],
        )
        .await?
        .into_iter()
        .map(cache_entry_from_row)
        .collect()
    }

    async fn image_proxy_cache_usage(&self) -> AppResult<ImageProxyCacheUsage> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT COALESCE(SUM(byte_size), 0) AS total_bytes, COUNT(*) AS entry_count
               FROM image_proxy_cache_entries",
            &[],
        )
        .await?;
        let Some(row) = row else {
            return Ok(ImageProxyCacheUsage::default());
        };
        Ok(ImageProxyCacheUsage {
            total_bytes: row.i64("total_bytes")?.max(0) as u64,
            entry_count: row.i64("entry_count")?.max(0) as u64,
        })
    }

    async fn clear_image_proxy_cache_entries(&self) -> AppResult<()> {
        SqlRuntime::execute_write(
            &self.datastore,
            "clear_image_proxy_cache_entries",
            "DELETE FROM image_proxy_cache_entries",
            Vec::new(),
        )
        .await?;
        Ok(())
    }

    async fn prune_image_proxy_sources_before(&self, cutoff: DateTime<Utc>) -> AppResult<u64> {
        let removed = SqlRuntime::execute_write(
            &self.datastore,
            "prune_image_proxy_sources",
            "DELETE FROM image_proxy_sources WHERE last_seen_at < {}",
            vec![SqlArg::Timestamp(cutoff)],
        )
        .await?;
        if let Ok(mut memory) = self.memory.lock() {
            memory
                .records
                .retain(|_, record| record.last_seen_at >= cutoff);
            let retained = memory
                .records
                .keys()
                .cloned()
                .collect::<std::collections::HashSet<_>>();
            memory
                .insertion_order
                .retain(|token| retained.contains(token));
        }
        Ok(removed)
    }

    async fn prune_orphaned_discovery_image_proxy_sources(&self) -> AppResult<u64> {
        let removed = SqlRuntime::execute_write(
            &self.datastore,
            "prune_orphaned_discovery_image_proxy_sources",
            "DELETE FROM image_proxy_sources
              WHERE owner_type = 'discovery'
                AND owner_id IS NOT NULL
                AND NOT EXISTS (
                    SELECT 1 FROM discovery_items i WHERE i.id = image_proxy_sources.owner_id
                )",
            Vec::new(),
        )
        .await?;
        if removed > 0 {
            // The in-memory registry has no view of discovery_items; drop the
            // discovery slice so it cannot resurrect a pruned row on the next
            // touch and re-learns whatever is still live from the database.
            if let Ok(mut memory) = self.memory.lock() {
                memory
                    .records
                    .retain(|_, record| record.owner_type.as_deref() != Some("discovery"));
                let retained = memory
                    .records
                    .keys()
                    .cloned()
                    .collect::<std::collections::HashSet<_>>();
                memory
                    .insertion_order
                    .retain(|token| retained.contains(token));
            }
        }
        Ok(removed)
    }
}

pub(super) fn approved_upstream_url(raw: &str) -> Option<String> {
    let mut parsed = url::Url::parse(raw.trim()).ok()?;
    if !matches!(parsed.scheme(), "http" | "https")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.port().is_some()
    {
        return None;
    }
    let host = parsed.host_str()?.to_ascii_lowercase();
    match host.as_str() {
        "image.tmdb.org" | "artworks.thetvdb.com" | "cdn.myanimelist.net" => {}
        _ if is_anilist_cdn_host(&host) => {}
        _ => return None,
    }
    parsed.set_scheme("https").ok()?;
    parsed.set_fragment(None);
    Some(parsed.to_string())
}

/// AniList serves portraits from numbered CDN shards (`s4.anilist.co` today,
/// `s1`–`s3` on older records). Matching the shard pattern keeps a future `s5`
/// working without admitting arbitrary `*.anilist.co` subdomains, which this
/// allowlist exists to keep out.
fn is_anilist_cdn_host(host: &str) -> bool {
    let Some(shard) = host.strip_suffix(".anilist.co") else {
        return false;
    };
    let Some(index) = shard.strip_prefix('s') else {
        return false;
    };
    !index.is_empty() && index.bytes().all(|byte| byte.is_ascii_digit())
}

async fn persist_queued_sources(
    datastore: &StoreDatastore,
    pending: &Arc<Mutex<HashMap<String, ImageProxySourceRecord>>>,
    records: Vec<ImageProxySourceRecord>,
) -> AppResult<()> {
    if records.is_empty() {
        return Ok(());
    }
    let transaction_records = records.clone();
    SqlRuntime::run_in_transaction(datastore, "flush_image_proxy_sources", move |tx| {
        let records = transaction_records.clone();
        Box::pin(async move {
            for record in records {
                tx.execute(UPSERT_SOURCE_SQL, &source_upsert_args(&record))
                    .await?;
            }
            Ok(())
        })
    })
    .await?;
    for record in &records {
        acknowledge_queued_source(pending, record);
    }
    Ok(())
}

fn acknowledge_queued_source(
    pending: &Arc<Mutex<HashMap<String, ImageProxySourceRecord>>>,
    record: &ImageProxySourceRecord,
) {
    if let Ok(mut queued) = pending.lock()
        && queued
            .get(&record.token)
            .is_some_and(|current| current.last_seen_at <= record.last_seen_at)
    {
        queued.remove(&record.token);
    }
}

const UPSERT_SOURCE_SQL: &str = "INSERT INTO image_proxy_sources
    (token, upstream_url, owner_type, owner_id, image_kind, fallback_class, last_seen_at)
 VALUES ({}, {}, {}, {}, {}, {}, {})
 ON CONFLICT(token) DO UPDATE SET
    upstream_url = excluded.upstream_url,
    owner_type = excluded.owner_type,
    owner_id = excluded.owner_id,
    image_kind = excluded.image_kind,
    fallback_class = excluded.fallback_class,
    last_seen_at = excluded.last_seen_at";

fn source_upsert_args(record: &ImageProxySourceRecord) -> Vec<SqlArg> {
    vec![
        SqlArg::Text(record.token.clone()),
        SqlArg::OptText(record.upstream_url.clone()),
        SqlArg::OptText(record.owner_type.clone()),
        SqlArg::OptText(record.owner_id.clone()),
        SqlArg::Text(record.image_kind.clone()),
        SqlArg::Text(record.fallback_class.clone()),
        SqlArg::Timestamp(record.last_seen_at),
    ]
}

async fn fetch_source(
    datastore: &StoreDatastore,
    token: &str,
) -> AppResult<Option<ImageProxySourceRecord>> {
    SqlRuntime::fetch_optional(
        datastore.read_exec(),
        "SELECT token, upstream_url, owner_type, owner_id, image_kind, fallback_class, last_seen_at
           FROM image_proxy_sources
          WHERE token = {}",
        &[SqlArg::Text(token.to_string())],
    )
    .await?
    .map(|row| {
        Ok(ImageProxySourceRecord {
            token: row.text("token")?,
            upstream_url: row.opt_text("upstream_url")?,
            owner_type: row.opt_text("owner_type")?,
            owner_id: row.opt_text("owner_id")?,
            image_kind: row.text("image_kind")?,
            fallback_class: row.text("fallback_class")?,
            last_seen_at: row.timestamp("last_seen_at")?,
        })
    })
    .transpose()
}

fn cache_entry_from_row(
    row: crate::storage::sql::runtime::SqlRow,
) -> AppResult<ImageProxyCacheEntryRecord> {
    Ok(ImageProxyCacheEntryRecord {
        token: row.text("token")?,
        variant: row.text("variant")?,
        content_type: row.text("content_type")?,
        byte_size: row.i64("byte_size")?,
        upstream_etag: row.opt_text("upstream_etag")?,
        upstream_last_modified: row.opt_text("upstream_last_modified")?,
        fetched_at: row.timestamp("fetched_at")?,
        last_accessed_at: row.timestamp("last_accessed_at")?,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chrono::Utc;
    use scryer_application::{
        ImageProxyCacheEntryRecord, ImageProxyKind, ImageProxyRegistration, ImageProxyRepository,
        ImageProxySourceRecord, image_proxy_source_token,
    };
    use sqlx::sqlite::SqlitePoolOptions;

    use super::{
        ImageProxyStore, MEMORY_SOURCE_LIMIT, acknowledge_queued_source, approved_upstream_url,
        fetch_source,
    };
    use crate::storage::sql::runtime::StoreDatastore;

    #[test]
    fn source_allowlist_uses_exact_hosts_and_upgrades_http() {
        assert_eq!(
            approved_upstream_url("http://image.tmdb.org/t/p/w300/poster.jpg").as_deref(),
            Some("https://image.tmdb.org/t/p/w300/poster.jpg")
        );
        assert!(approved_upstream_url("https://image.tmdb.org.evil.test/a.jpg").is_none());
        assert!(approved_upstream_url("https://evil-image.tmdb.org/a.jpg").is_none());
        assert!(approved_upstream_url("https://user@artworks.thetvdb.com/banners/a.jpg").is_none());
        assert!(approved_upstream_url("https://image.tmdb.org:444/t/p/a.jpg").is_none());
        assert!(approved_upstream_url("ftp://image.tmdb.org/t/p/a.jpg").is_none());
        assert!(approved_upstream_url("https://127.0.0.1/a.jpg").is_none());
    }

    #[test]
    fn source_allowlist_admits_anime_portrait_cdns() {
        // Voice-actor portraits come from AniList's numbered shards and, on the
        // Jikan backfill path, from MAL's CDN.
        for host in ["s4.anilist.co", "s1.anilist.co", "s12.anilist.co"] {
            assert_eq!(
                approved_upstream_url(&format!("https://{host}/file/anilistcdn/a.jpg")).as_deref(),
                Some(format!("https://{host}/file/anilistcdn/a.jpg").as_str()),
                "{host} is an AniList CDN shard"
            );
        }
        assert_eq!(
            approved_upstream_url("https://cdn.myanimelist.net/images/voiceactors/a.jpg")
                .as_deref(),
            Some("https://cdn.myanimelist.net/images/voiceactors/a.jpg")
        );

        // The shard pattern must not widen into arbitrary AniList subdomains,
        // nor into lookalike hosts that merely end with the CDN suffix.
        assert!(approved_upstream_url("https://anilist.co/a.jpg").is_none());
        assert!(approved_upstream_url("https://api.anilist.co/a.jpg").is_none());
        assert!(approved_upstream_url("https://s.anilist.co/a.jpg").is_none());
        assert!(approved_upstream_url("https://s4a.anilist.co/a.jpg").is_none());
        assert!(approved_upstream_url("https://s4.anilist.co.evil.test/a.jpg").is_none());
        assert!(approved_upstream_url("https://evil-s4.anilist.co/a.jpg").is_none());
        assert!(approved_upstream_url("https://cdn.myanimelist.net.evil.test/a.jpg").is_none());
    }

    #[tokio::test]
    async fn registry_persists_rehydrates_prunes_and_bounds_memory() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("sqlite image proxy test pool");
        sqlx::query(
            "CREATE TABLE image_proxy_sources (
                token TEXT PRIMARY KEY,
                upstream_url TEXT,
                owner_type TEXT,
                owner_id TEXT,
                image_kind TEXT NOT NULL,
                fallback_class TEXT NOT NULL,
                last_seen_at TEXT NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .expect("create image proxy sources table");
        sqlx::query(
            "CREATE TABLE image_proxy_cache_entries (
                token TEXT NOT NULL,
                variant TEXT NOT NULL,
                content_type TEXT NOT NULL,
                byte_size INTEGER NOT NULL,
                upstream_etag TEXT,
                upstream_last_modified TEXT,
                fetched_at TEXT NOT NULL,
                last_accessed_at TEXT NOT NULL,
                PRIMARY KEY (token, variant)
            )",
        )
        .execute(&pool)
        .await
        .expect("create image proxy cache entries table");
        let datastore = StoreDatastore::Sqlite {
            pool,
            writer_gate: Arc::new(tokio::sync::Mutex::new(())),
        };
        let store = ImageProxyStore::new(datastore.clone());
        let route = store.register_image_source(ImageProxyRegistration {
            upstream_url: Some("http://image.tmdb.org/t/p/w500/poster.jpg".to_string()),
            owner_type: Some("title".to_string()),
            owner_id: Some("title-1".to_string()),
            image_kind: ImageProxyKind::Poster,
            fallback_class: "portrait".to_string(),
            default_variant: "w250".to_string(),
        });
        let route_parts = route.split('/').collect::<Vec<_>>();
        let token = route_parts
            .get(route_parts.len().saturating_sub(2))
            .expect("token route segment")
            .to_string();

        assert!(
            fetch_source(&datastore, &token)
                .await
                .expect("check staged source")
                .is_none(),
            "registration must not perform an individual database write"
        );
        assert_eq!(store.pending.lock().expect("pending source lock").len(), 1);

        store
            .flush_image_proxy_sources()
            .await
            .expect("durably flush registered source");
        assert!(
            fetch_source(&datastore, &token)
                .await
                .expect("load registered source")
                .is_some()
        );

        store.clear_image_proxy_memory();
        let rehydrated = store
            .get_image_proxy_source(&token)
            .await
            .expect("rehydrate source")
            .expect("persisted source");
        assert_eq!(
            rehydrated.upstream_url.as_deref(),
            Some("https://image.tmdb.org/t/p/w500/poster.jpg")
        );
        assert_eq!(rehydrated.owner_type.as_deref(), Some("title"));
        assert_eq!(rehydrated.image_kind, "poster");

        let fetched_at = chrono::DateTime::<Utc>::from_timestamp(1_800_000_000, 0)
            .expect("fixed fetched timestamp");
        let original_accessed_at = fetched_at + chrono::Duration::seconds(10);
        store
            .upsert_image_proxy_cache_entry(&ImageProxyCacheEntryRecord {
                token: token.clone(),
                variant: "w250".to_string(),
                content_type: "image/jpeg".to_string(),
                byte_size: 10,
                upstream_etag: Some("etag".to_string()),
                upstream_last_modified: None,
                fetched_at,
                last_accessed_at: original_accessed_at,
            })
            .await
            .expect("persist cache metadata");
        store
            .touch_image_proxy_cache_entry(
                &token,
                "w250",
                fetched_at + chrono::Duration::seconds(1),
                fetched_at + chrono::Duration::seconds(20),
            )
            .await
            .expect("ignore a stale cache touch");
        assert_eq!(
            store
                .get_image_proxy_cache_entry(&token, "w250")
                .await
                .expect("read cache metadata")
                .expect("cache metadata")
                .last_accessed_at,
            original_accessed_at
        );
        let latest_accessed_at = fetched_at + chrono::Duration::seconds(30);
        store
            .touch_image_proxy_cache_entry(&token, "w250", fetched_at, latest_accessed_at)
            .await
            .expect("touch current cache metadata");
        let touched = store
            .get_image_proxy_cache_entry(&token, "w250")
            .await
            .expect("read touched cache metadata")
            .expect("touched cache metadata");
        assert_eq!(touched.fetched_at, fetched_at);
        assert_eq!(touched.last_accessed_at, latest_accessed_at);

        let rejected_route = store.register_image_source(ImageProxyRegistration {
            upstream_url: Some("https://image.tmdb.org.evil.test/poster.jpg".to_string()),
            owner_type: Some("title".to_string()),
            owner_id: Some("title-rejected".to_string()),
            image_kind: ImageProxyKind::Poster,
            fallback_class: "portrait".to_string(),
            default_variant: "w250".to_string(),
        });
        let rejected_parts = rejected_route.split('/').collect::<Vec<_>>();
        let rejected_token = rejected_parts[rejected_parts.len() - 2].to_string();
        store
            .flush_image_proxy_sources()
            .await
            .expect("durably flush fallback-only source");
        store.clear_image_proxy_memory();
        assert!(
            store
                .get_image_proxy_source(&rejected_token)
                .await
                .expect("rehydrate fallback source")
                .expect("persisted fallback source record")
                .upstream_url
                .is_none()
        );

        let old_token = image_proxy_source_token(
            Some("https://artworks.thetvdb.com/banners/old.jpg"),
            Some("episode"),
            Some("old-episode"),
            ImageProxyKind::EpisodeStill,
        );
        let old_record = ImageProxySourceRecord {
            token: old_token.clone(),
            upstream_url: Some("https://artworks.thetvdb.com/banners/old.jpg".to_string()),
            owner_type: Some("episode".to_string()),
            owner_id: Some("old-episode".to_string()),
            image_kind: "episode_still".to_string(),
            fallback_class: "landscape".to_string(),
            last_seen_at: Utc::now() - chrono::Duration::days(31),
        };
        store.remember(old_record.clone());
        store.queue_source(old_record);
        store
            .flush_image_proxy_sources()
            .await
            .expect("persist old source");
        assert_eq!(
            store
                .prune_image_proxy_sources_before(Utc::now() - chrono::Duration::days(30))
                .await
                .expect("prune old sources"),
            1
        );
        assert!(
            store
                .get_image_proxy_source(&old_token)
                .await
                .expect("load pruned source")
                .is_none()
        );

        for index in 0..=MEMORY_SOURCE_LIMIT {
            store.remember(ImageProxySourceRecord {
                token: format!("{index:064x}"),
                upstream_url: None,
                owner_type: Some("title".to_string()),
                owner_id: Some(index.to_string()),
                image_kind: "poster".to_string(),
                fallback_class: "portrait".to_string(),
                last_seen_at: Utc::now(),
            });
        }
        let memory = store.memory.lock().expect("memory registry lock");
        assert_eq!(memory.records.len(), MEMORY_SOURCE_LIMIT);
        assert!(!memory.records.contains_key(&format!("{:064x}", 0)));
    }

    #[tokio::test]
    async fn orphaned_discovery_sources_are_swept_and_budget_reads_are_scalar() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("sqlite image proxy test pool");
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .expect("enable foreign keys");
        sqlx::query("CREATE TABLE discovery_items (id TEXT PRIMARY KEY NOT NULL)")
            .execute(&pool)
            .await
            .expect("create discovery items table");
        sqlx::query(
            "CREATE TABLE image_proxy_sources (
                token TEXT PRIMARY KEY,
                upstream_url TEXT,
                owner_type TEXT,
                owner_id TEXT,
                image_kind TEXT NOT NULL,
                fallback_class TEXT NOT NULL,
                last_seen_at TEXT NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .expect("create image proxy sources table");
        sqlx::query(
            "CREATE TABLE image_proxy_cache_entries (
                token TEXT NOT NULL REFERENCES image_proxy_sources(token) ON DELETE CASCADE,
                variant TEXT NOT NULL,
                content_type TEXT NOT NULL,
                byte_size INTEGER NOT NULL,
                upstream_etag TEXT,
                upstream_last_modified TEXT,
                fetched_at TEXT NOT NULL,
                last_accessed_at TEXT NOT NULL,
                PRIMARY KEY (token, variant)
            )",
        )
        .execute(&pool)
        .await
        .expect("create image proxy cache entries table");
        sqlx::query("INSERT INTO discovery_items (id) VALUES ('live-item')")
            .execute(&pool)
            .await
            .expect("seed live discovery item");
        let datastore = StoreDatastore::Sqlite {
            pool,
            writer_gate: Arc::new(tokio::sync::Mutex::new(())),
        };
        let store = ImageProxyStore::new(datastore.clone());

        let now = Utc::now();
        let source = |token: &str, owner_type: &str, owner_id: &str| ImageProxySourceRecord {
            token: token.to_string(),
            upstream_url: Some(format!("https://image.tmdb.org/t/p/w500/{token}.jpg")),
            owner_type: Some(owner_type.to_string()),
            owner_id: Some(owner_id.to_string()),
            image_kind: "poster".to_string(),
            fallback_class: "portrait".to_string(),
            last_seen_at: now,
        };
        let live = source(&"a".repeat(64), "discovery", "live-item");
        let orphan = source(&"b".repeat(64), "discovery", "gone-item");
        let title = source(&"c".repeat(64), "title", "gone-item");
        for record in [&live, &orphan, &title] {
            store.remember(record.clone());
            store.queue_source(record.clone());
        }
        store
            .flush_image_proxy_sources()
            .await
            .expect("persist sources");
        for (record, age_minutes, byte_size) in
            [(&live, 1, 100), (&orphan, 3, 250), (&title, 2, 75)]
        {
            store
                .upsert_image_proxy_cache_entry(&ImageProxyCacheEntryRecord {
                    token: record.token.clone(),
                    variant: "w250".to_string(),
                    content_type: "image/jpeg".to_string(),
                    byte_size,
                    upstream_etag: None,
                    upstream_last_modified: None,
                    fetched_at: now,
                    last_accessed_at: now - chrono::Duration::minutes(age_minutes),
                })
                .await
                .expect("persist cache entry");
        }

        let usage = store
            .image_proxy_cache_usage()
            .await
            .expect("read cache usage");
        assert_eq!(usage.total_bytes, 425);
        assert_eq!(usage.entry_count, 3);
        let oldest = store
            .list_image_proxy_cache_entries_lru_oldest(2)
            .await
            .expect("list oldest entries")
            .into_iter()
            .map(|entry| entry.token)
            .collect::<Vec<_>>();
        assert_eq!(oldest, vec![orphan.token.clone(), title.token.clone()]);

        // Only the discovery-owned source whose item vanished is swept; a
        // title-owned source with the same owner id is untouched, and the
        // orphan's cache entry follows through the cascade.
        assert_eq!(
            store
                .prune_orphaned_discovery_image_proxy_sources()
                .await
                .expect("sweep orphaned discovery sources"),
            1
        );
        assert!(
            fetch_source(&datastore, &orphan.token)
                .await
                .expect("load swept source")
                .is_none()
        );
        assert!(
            fetch_source(&datastore, &live.token)
                .await
                .expect("load live source")
                .is_some()
        );
        assert!(
            fetch_source(&datastore, &title.token)
                .await
                .expect("load title source")
                .is_some()
        );
        assert!(store.memory_source(&orphan.token).is_none());
        assert!(store.memory_source(&title.token).is_some());
        let usage = store
            .image_proxy_cache_usage()
            .await
            .expect("read cache usage after sweep");
        assert_eq!(usage.total_bytes, 175);
        assert_eq!(usage.entry_count, 2);
        assert_eq!(
            store
                .prune_orphaned_discovery_image_proxy_sources()
                .await
                .expect("second sweep is a no-op"),
            0
        );
    }

    #[test]
    fn queued_source_acknowledgement_preserves_a_newer_record() {
        let token = "a".repeat(64);
        let older = ImageProxySourceRecord {
            token: token.clone(),
            upstream_url: Some("https://image.tmdb.org/t/p/original/poster.jpg".to_string()),
            owner_type: Some("title".to_string()),
            owner_id: Some("title-1".to_string()),
            image_kind: "poster".to_string(),
            fallback_class: "portrait".to_string(),
            last_seen_at: Utc::now(),
        };
        let newer = ImageProxySourceRecord {
            last_seen_at: older.last_seen_at + chrono::Duration::seconds(1),
            ..older.clone()
        };
        let pending = Arc::new(std::sync::Mutex::new(std::collections::HashMap::from([(
            token.clone(),
            newer.clone(),
        )])));

        acknowledge_queued_source(&pending, &older);
        assert_eq!(
            pending.lock().expect("pending source lock").get(&token),
            Some(&newer)
        );

        acknowledge_queued_source(&pending, &newer);
        assert!(pending.lock().expect("pending source lock").is_empty());
    }

    #[tokio::test]
    async fn failed_batch_flush_retains_all_sources_for_retry() {
        const SOURCE_COUNT: usize = 32;
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("sqlite image proxy retry pool");
        let datastore = StoreDatastore::Sqlite {
            pool: pool.clone(),
            writer_gate: Arc::new(tokio::sync::Mutex::new(())),
        };
        let store = ImageProxyStore::new(datastore.clone());

        let mut tokens = Vec::new();
        for index in 0..SOURCE_COUNT {
            let route = store.register_image_source(ImageProxyRegistration {
                upstream_url: Some(format!(
                    "https://image.tmdb.org/t/p/original/poster-{index}.jpg"
                )),
                owner_type: Some("title".to_string()),
                owner_id: Some(format!("title-{index}")),
                image_kind: ImageProxyKind::Poster,
                fallback_class: "portrait".to_string(),
                default_variant: "w250".to_string(),
            });
            tokens.push(
                route
                    .split('/')
                    .nth_back(1)
                    .expect("token route segment")
                    .to_string(),
            );
        }

        store
            .flush_image_proxy_sources()
            .await
            .expect_err("flush should fail before the source table exists");
        assert_eq!(
            store.pending.lock().expect("pending source lock").len(),
            SOURCE_COUNT
        );

        sqlx::query(
            "CREATE TABLE image_proxy_sources (
                token TEXT PRIMARY KEY,
                upstream_url TEXT,
                owner_type TEXT,
                owner_id TEXT,
                image_kind TEXT NOT NULL,
                fallback_class TEXT NOT NULL,
                last_seen_at TEXT NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .expect("create image proxy sources table after failed flush");

        store
            .flush_image_proxy_sources()
            .await
            .expect("retry queued source flush");
        assert!(
            store
                .pending
                .lock()
                .expect("pending source lock")
                .is_empty()
        );
        for token in tokens {
            assert!(
                fetch_source(&datastore, &token)
                    .await
                    .expect("load retried source")
                    .is_some()
            );
        }
    }
}

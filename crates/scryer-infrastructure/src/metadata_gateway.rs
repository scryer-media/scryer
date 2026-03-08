use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt::Write as _;
use std::sync::RwLock;
use std::time::Duration;

use async_trait::async_trait;
use ring::digest;
use scryer_application::{
    AnimeEpisodeMapping, AnimeMapping, AppError, AppResult, EpisodeMetadata, MetadataGateway,
    MetadataSearchItem, MovieMetadata, MultiMetadataSearchResult, RichMetadataSearchItem,
    SeasonMetadata, SeriesMetadata,
};
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;
use tracing::{debug, info, warn};

struct ApqCacheEntry {
    etag: String,
    body: String,
}

struct ApqCache {
    map: HashMap<String, ApqCacheEntry>,
    order: VecDeque<String>,
}

impl ApqCache {
    fn new() -> Self {
        Self {
            map: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    fn get(&self, key: &str) -> Option<&ApqCacheEntry> {
        self.map.get(key)
    }

    #[allow(clippy::map_entry)] // entry API borrows map, conflicting with eviction logic
    fn insert(&mut self, key: String, entry: ApqCacheEntry) {
        if self.map.contains_key(&key) {
            self.map.insert(key, entry);
            return;
        }
        if self.map.len() >= 1000 {
            if let Some(oldest) = self.order.pop_front() {
                self.map.remove(&oldest);
            }
        }
        self.order.push_back(key.clone());
        self.map.insert(key, entry);
    }
}

use crate::smg_enrollment;

const SEARCH_TVDB_QUERY: &str = r#"
  query SearchTvdb($query: String!, $type: String, $limit: Int) {
    searchTvdb(query: $query, type: $type, limit: $limit) {
      results {
        tvdb_id
        name
        year
      }
    }
  }
"#;

const SEARCH_TVDB_RICH_QUERY: &str = r#"
  query SearchTvdbRich($query: String!, $type: String, $limit: Int, $language: String) {
    searchTvdb(query: $query, type: $type, limit: $limit, language: $language) {
      results {
        tvdb_id
        name
        imdb_id
        slug
        type
        year
        status
        overview
        popularity
        poster_url
        language
        runtime_minutes
        sort_title
      }
    }
  }
"#;

const SEARCH_TVDB_MULTI_QUERY: &str = r#"
  query SearchTvdbMulti($query: String!, $limit: Int, $language: String) {
    searchTvdbMulti(query: $query, limit: $limit, language: $language) {
      movies {
        tvdb_id name imdb_id slug type year status overview
        popularity poster_url language runtime_minutes sort_title
      }
      series {
        tvdb_id name imdb_id slug type year status overview
        popularity poster_url language runtime_minutes sort_title
      }
      anime {
        tvdb_id name imdb_id slug type year status overview
        popularity poster_url language runtime_minutes sort_title
      }
    }
  }
"#;

const GET_MOVIE_QUERY: &str = r#"
  query GetMovie($tvdbId: Int!, $language: String!) {
    movie(tvdbId: $tvdbId, language: $language) {
      movie {
        tvdb_id
        name
        slug
        year
        status
        overview
        poster_url
        language
        runtime_minutes
        sort_title
        imdb_id
        genres
        studio
        tmdb_release_date
      }
    }
  }
"#;

const GET_SERIES_QUERY: &str = r#"
  query GetSeries($id: String!, $includeEpisodes: Boolean!, $language: String!) {
    series(id: $id, includeEpisodes: $includeEpisodes, language: $language) {
      series {
        tvdb_id
        name
        sort_name
        slug
        status
        year
        first_aired
        overview
        network
        runtime_minutes
        poster_url
        country
        genres
        aliases
        seasons {
          tvdb_id
          number
          label
          episode_type
        }
        episodes {
          tvdb_id
          episode_number
          season_number
          name
          aired
          runtime_minutes
          is_filler
          is_recap
          overview
          absolute_number
        }
        anime_mappings {
          mal_id
          anilist_id
          anidb_id
          kitsu_id
          thetvdb_season
          score
          anime_media_type
          global_media_type
          status
          episode_mappings {
            tvdb_season
            episode_start
            episode_end
          }
        }
      }
    }
  }
"#;

fn sha256_hex(input: &str) -> String {
    let hash = digest::digest(&digest::SHA256, input.as_bytes());
    hash.as_ref()
        .iter()
        .fold(String::with_capacity(64), |mut acc, byte| {
            use std::fmt::Write;
            let _ = write!(acc, "{byte:02x}");
            acc
        })
}

/// Precompute the SHA-256 hash for a static query string (APQ registration).
fn apq_hash(query: &str) -> String {
    sha256_hex(query)
}

/// Configuration for SMG enrollment (mTLS client certificates).
pub struct SmgEnrollmentConfig {
    pub registration_secret: Option<String>,
    pub ca_cert: Option<String>,
}

pub struct MetadataGatewayClient {
    http: Client,
    endpoint: String,
    registration_url: String,
    enrollment_config: SmgEnrollmentConfig,
    db: crate::SqliteServices,
    mtls_client: tokio::sync::RwLock<Option<Client>>,
    search_hash: String,
    search_rich_hash: String,
    search_multi_hash: String,
    movie_hash: String,
    series_hash: String,
    apq_cache: RwLock<ApqCache>,
}

impl MetadataGatewayClient {
    pub fn new(
        endpoint: String,
        accept_invalid_certs: bool,
        db: crate::SqliteServices,
        enrollment_config: SmgEnrollmentConfig,
    ) -> Self {
        if accept_invalid_certs {
            warn!("metadata gateway client: TLS certificate verification DISABLED");
        }

        let search_hash = apq_hash(SEARCH_TVDB_QUERY);
        let search_rich_hash = apq_hash(SEARCH_TVDB_RICH_QUERY);
        let search_multi_hash = apq_hash(SEARCH_TVDB_MULTI_QUERY);
        let movie_hash = apq_hash(GET_MOVIE_QUERY);
        let series_hash = apq_hash(GET_SERIES_QUERY);

        // Derive registration URL from GraphQL endpoint
        let registration_url = if endpoint.ends_with("/graphql") {
            format!("{}/api/register", &endpoint[..endpoint.len() - "/graphql".len()])
        } else {
            format!("{}/api/register", endpoint.trim_end_matches('/'))
        };

        debug!(
            endpoint = %endpoint,
            accept_invalid_certs,
            has_registration_secret = enrollment_config.registration_secret.is_some(),
            %search_hash,
            %search_rich_hash,
            %search_multi_hash,
            %movie_hash,
            %series_hash,
            "metadata gateway client initialized (APQ enabled)"
        );

        Self {
            http: Client::builder()
                .timeout(Duration::from_secs(100))
                .danger_accept_invalid_certs(accept_invalid_certs)
                .build()
                .expect("failed to build HTTP client"),
            endpoint,
            registration_url,
            enrollment_config,
            db,
            mtls_client: tokio::sync::RwLock::new(None),
            search_hash,
            search_rich_hash,
            search_multi_hash,
            movie_hash,
            series_hash,
            apq_cache: RwLock::new(ApqCache::new()),
        }
    }

    /// Get the best available HTTP client (mTLS if enrolled, plain otherwise).
    ///
    /// Enrollment happens lazily on first call when a registration secret is configured.
    /// On enrollment failure, falls back to the plain HTTP client.
    async fn get_http_client(&self) -> Client {
        let secret = match &self.enrollment_config.registration_secret {
            Some(s) => s,
            None => return self.http.clone(),
        };

        // Fast path: already have an mTLS client cached
        {
            let guard = self.mtls_client.read().await;
            if let Some(client) = guard.as_ref() {
                return client.clone();
            }
        }

        // Slow path: need to enroll and build mTLS client
        let mut guard = self.mtls_client.write().await;
        // Double-check after acquiring write lock
        if let Some(client) = guard.as_ref() {
            return client.clone();
        }

        match self.try_build_mtls_client(secret).await {
            Ok(client) => {
                info!("SMG mTLS enrollment successful, using mutual TLS for metadata requests");
                let result = client.clone();
                *guard = Some(client);
                result
            }
            Err(e) => {
                warn!(error = %e, "SMG enrollment failed, using plain HTTP");
                self.http.clone()
            }
        }
    }

    async fn try_build_mtls_client(&self, registration_secret: &str) -> Result<Client, String> {
        let state = smg_enrollment::ensure_enrolled(
            &self.db,
            &self.registration_url,
            registration_secret,
            self.enrollment_config.ca_cert.as_deref(),
        )
        .await?;

        let identity = smg_enrollment::build_mtls_identity(&state)?;
        let ca_cert = smg_enrollment::build_ca_certificate(&state)?;

        Client::builder()
            .timeout(Duration::from_secs(100))
            .identity(identity)
            .add_root_certificate(ca_cert)
            .build()
            .map_err(|e| format!("failed to build mTLS client: {e}"))
    }

    /// Eagerly trigger enrollment in a background task so the mTLS client is ready before
    /// the first real metadata query arrives. Call this once after construction; it is
    /// safe to call concurrently with any other method.
    pub async fn warm_enrollment(&self) {
        self.get_http_client().await;
    }

    /// Execute a GraphQL query using APQ (Automatic Persisted Queries).
    ///
    /// 1. Send GET with hash only (no query body) — cache-friendly.
    ///    Sends `If-None-Match` if we have a cached ETag; on 304 returns cached body.
    /// 2. If the server returns `PersistedQueryNotFound`, POST with full query + hash to register.
    /// 3. Subsequent GETs for the same hash will hit Cloudflare edge cache.
    async fn execute_graphql_apq<T: serde::de::DeserializeOwned>(
        &self,
        query: &str,
        hash: &str,
        variables: serde_json::Value,
    ) -> AppResult<T> {
        let extensions = json!({
            "persistedQuery": {
                "version": 1,
                "sha256Hash": hash
            }
        });

        let variables_str = serde_json::to_string(&variables)
            .map_err(|e| AppError::Repository(format!("failed to serialize variables: {e}")))?;
        let extensions_str = serde_json::to_string(&extensions)
            .map_err(|e| AppError::Repository(format!("failed to serialize extensions: {e}")))?;

        let cache_key = format!("{hash}:{variables_str}");

        // Check for a cached ETag to send If-None-Match
        let cached_etag = self.apq_cache.read().unwrap()
            .get(&cache_key)
            .map(|e| e.etag.clone());

        debug!(endpoint = %self.endpoint, hash, has_etag = cached_etag.is_some(), "APQ GET request");

        let client = self.get_http_client().await;
        let mut req = client
            .get(&self.endpoint)
            .query(&[("extensions", &extensions_str), ("variables", &variables_str)]);
        if let Some(ref etag) = cached_etag {
            req = req.header(reqwest::header::IF_NONE_MATCH, etag);
        }
        let get_result = req.send().await;

        match get_result {
            Ok(resp) if resp.status() == reqwest::StatusCode::NOT_MODIFIED => {
                // 304: serve from our local cache
                let body = self.apq_cache.read().unwrap()
                    .get(&cache_key)
                    .map(|e| e.body.clone())
                    .ok_or_else(|| AppError::Repository("APQ 304 but no cached body".into()))?;
                debug!(hash, "APQ 304 — serving from ETag cache");
                let parsed: GraphqlResponse<T> = serde_json::from_str(&body)
                    .map_err(|e| AppError::Repository(format!("APQ cache: invalid JSON: {e}")))?;
                parsed.data
                    .ok_or_else(|| AppError::Repository("APQ cache: empty data".into()))
            }
            Ok(resp) if resp.status().is_success() => {
                let etag = resp.headers()
                    .get(reqwest::header::ETAG)
                    .and_then(|v| v.to_str().ok())
                    .map(|s| s.to_string());
                let raw = resp.text().await
                    .map_err(|e| AppError::Repository(e.to_string()))?;

                let parsed: GraphqlResponse<T> = serde_json::from_str(&raw).map_err(|e| {
                    AppError::Repository(format!("APQ GET: invalid JSON: {e}"))
                })?;

                // Check for PersistedQueryNotFound before caching
                if let Some(ref errors) = parsed.errors {
                    let is_not_found = errors.iter().any(|e|
                        e.message.contains("PersistedQueryNotFound")
                    );
                    if is_not_found {
                        debug!(hash, "APQ cache miss, registering via POST");
                        return self.execute_graphql_apq_register(query, &extensions, &variables).await;
                    }
                    let msg = errors.first()
                        .map(|e| e.message.as_str())
                        .unwrap_or("metadata gateway returned errors");
                    return Err(AppError::Repository(msg.to_string()));
                }

                // Store ETag + body for future conditional requests (evicts oldest beyond 1000)
                if let Some(etag) = etag {
                    self.apq_cache.write().unwrap()
                        .insert(cache_key, ApqCacheEntry { etag, body: raw });
                }

                parsed.data
                    .ok_or_else(|| AppError::Repository("APQ GET: empty data".into()))
            }
            Ok(resp) => {
                let status = resp.status();
                debug!(status = %status, hash, "APQ GET failed, falling back to POST");
                self.execute_graphql_apq_register(query, &extensions, &variables).await
            }
            Err(err) => {
                debug!(error = %err, hash, "APQ GET network error, falling back to POST");
                self.execute_graphql_apq_register(query, &extensions, &variables).await
            }
        }
    }

    /// POST with full query + extensions to register the hash, then return the result.
    async fn execute_graphql_apq_register<T: serde::de::DeserializeOwned>(
        &self,
        query: &str,
        extensions: &serde_json::Value,
        variables: &serde_json::Value,
    ) -> AppResult<T> {
        let payload = json!({
            "query": query,
            "variables": variables,
            "extensions": extensions,
        });

        self.execute_graphql(payload).await
    }

    async fn execute_graphql<T: serde::de::DeserializeOwned>(
        &self,
        payload: serde_json::Value,
    ) -> AppResult<T> {
        debug!(endpoint = %self.endpoint, "sending metadata gateway request");
        let response = self.send_with_retry(&payload).await?;

        let status = response.status();
        let raw_text = response
            .text()
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        debug!(status = %status, body_len = raw_text.len(), "metadata gateway response");

        if !status.is_success() {
            warn!(status = %status, body = %raw_text, "metadata gateway request failed");
            return Err(AppError::Repository(format!(
                "metadata gateway request failed ({status}): {raw_text}"
            )));
        }

        let parsed: GraphqlResponse<T> = serde_json::from_str(&raw_text).map_err(|err| {
            warn!(body = %raw_text, error = %err, "metadata gateway returned invalid JSON");
            AppError::Repository(format!("metadata gateway returned invalid JSON: {err}"))
        })?;

        if let Some(errors) = parsed.errors {
            let message = errors
                .first()
                .map(|error| error.message.as_str())
                .unwrap_or("metadata gateway returned errors");
            warn!(error = %message, "metadata gateway returned GraphQL errors");
            return Err(AppError::Repository(message.to_string()));
        }

        if parsed.data.is_none() {
            warn!(body = %raw_text, "metadata gateway returned empty data");
        }

        parsed
            .data
            .ok_or_else(|| AppError::Repository("metadata gateway returned empty data".into()))
    }

    async fn send_with_retry(
        &self,
        payload: &serde_json::Value,
    ) -> AppResult<reqwest::Response> {
        let client = self.get_http_client().await;
        let result = client
            .post(&self.endpoint)
            .json(payload)
            .send()
            .await;

        match result {
            Ok(resp) if !resp.status().is_server_error() => Ok(resp),
            Ok(resp) => {
                let status = resp.status();
                tracing::warn!(
                    status = %status,
                    "metadata gateway returned server error, retrying in 1s"
                );
                tokio::time::sleep(Duration::from_secs(1)).await;
                client
                    .post(&self.endpoint)
                    .json(payload)
                    .send()
                    .await
                    .map_err(|err| AppError::Repository(format!("metadata gateway retry failed: {err}")))
            }
            Err(err) if err.is_timeout() || err.is_connect() => {
                tracing::warn!(
                    error = %err,
                    "metadata gateway request failed (transient), retrying in 1s"
                );
                tokio::time::sleep(Duration::from_secs(1)).await;
                client
                    .post(&self.endpoint)
                    .json(payload)
                    .send()
                    .await
                    .map_err(|err| AppError::Repository(format!("metadata gateway retry failed: {err}")))
            }
            Err(err) => Err(AppError::Repository(err.to_string())),
        }
    }

    /// POST a dynamic GraphQL query and return the `data` field as raw JSON.
    /// Tolerates partial errors (some aliases may resolve while others fail).
    async fn post_graphql_partial(
        &self,
        query: &str,
    ) -> AppResult<serde_json::Value> {
        let payload = json!({ "query": query });
        let client = self.get_http_client().await;
        let resp = client
            .post(&self.endpoint)
            .json(&payload)
            .send()
            .await
            .map_err(|e| AppError::Repository(format!("bulk metadata request failed: {e}")))?;

        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| AppError::Repository(format!("bulk metadata read body: {e}")))?;

        if !status.is_success() {
            return Err(AppError::Repository(format!(
                "bulk metadata request failed ({status}): {body}"
            )));
        }

        let parsed: serde_json::Value = serde_json::from_str(&body)
            .map_err(|e| AppError::Repository(format!("bulk metadata invalid JSON: {e}")))?;

        if let Some(errors) = parsed.get("errors") {
            if let Some(arr) = errors.as_array() {
                for err in arr {
                    let msg = err.get("message").and_then(|v| v.as_str()).unwrap_or("unknown");
                    debug!("bulk metadata partial error: {msg}");
                }
            }
        }

        parsed
            .get("data")
            .cloned()
            .ok_or_else(|| AppError::Repository("bulk metadata: no data in response".into()))
    }
}

// ---------------------------------------------------------------------------
// Bulk query builders (GraphQL aliases)
// ---------------------------------------------------------------------------

const MOVIE_FIELD_SELECTION: &str = "\
    tvdb_id name slug year status overview poster_url language \
    runtime_minutes sort_title imdb_id genres studio tmdb_release_date";

const SERIES_FIELD_SELECTION: &str = "\
    tvdb_id name sort_name slug status year first_aired overview network \
    runtime_minutes poster_url country genres aliases \
    seasons { tvdb_id number label episode_type } \
    episodes { tvdb_id episode_number season_number name aired runtime_minutes \
               is_filler is_recap overview absolute_number } \
    anime_mappings { mal_id anilist_id anidb_id kitsu_id thetvdb_season score \
                     anime_media_type global_media_type status \
                     episode_mappings { tvdb_season episode_start episode_end } }";

fn build_bulk_movie_query(tvdb_ids: &[i64], language: &str) -> String {
    let mut q = String::from("query {\n");
    for (i, &id) in tvdb_ids.iter().enumerate() {
        let _ = write!(
            q,
            "  m{i}: movie(tvdbId: {id}, language: \"{language}\") {{ movie {{ {MOVIE_FIELD_SELECTION} }} }}\n"
        );
    }
    q.push_str("}\n");
    q
}

fn build_bulk_series_query(tvdb_ids: &[i64], language: &str) -> String {
    let mut q = String::from("query {\n");
    for (i, &id) in tvdb_ids.iter().enumerate() {
        let _ = write!(
            q,
            "  s{i}: series(id: \"{id}\", includeEpisodes: true, language: \"{language}\") {{ series {{ {SERIES_FIELD_SELECTION} }} }}\n"
        );
    }
    q.push_str("}\n");
    q
}

#[derive(Deserialize)]
struct GraphqlResponse<T> {
    data: Option<T>,
    errors: Option<Vec<GraphqlError>>,
}

#[derive(Deserialize)]
struct GraphqlError {
    message: String,
}

// --- Search types ---

#[derive(Deserialize)]
struct SearchTvdbResponse {
    #[serde(rename = "searchTvdb")]
    search_tvdb: SearchTvdbResult,
}

#[derive(Deserialize)]
struct SearchTvdbResult {
    results: Vec<SearchTvdbItem>,
}

#[derive(Deserialize)]
struct SearchTvdbItem {
    #[serde(rename = "tvdb_id")]
    tvdb_id: i64,
    name: String,
    year: Option<i32>,
}

#[derive(Deserialize)]
struct SearchTvdbRichItem {
    tvdb_id: i64,
    name: String,
    imdb_id: Option<String>,
    slug: Option<String>,
    #[serde(rename = "type")]
    type_hint: Option<String>,
    year: Option<i32>,
    status: Option<String>,
    overview: Option<String>,
    popularity: Option<f64>,
    poster_url: Option<String>,
    language: Option<String>,
    runtime_minutes: Option<i32>,
    sort_title: Option<String>,
}

#[derive(Deserialize)]
struct SearchTvdbRichResponse {
    #[serde(rename = "searchTvdb")]
    search_tvdb: SearchTvdbRichResult,
}

#[derive(Deserialize)]
struct SearchTvdbRichResult {
    results: Vec<SearchTvdbRichItem>,
}

// --- Multi-search types ---

#[derive(Deserialize)]
struct SearchTvdbMultiResponse {
    #[serde(rename = "searchTvdbMulti")]
    search_tvdb_multi: SearchTvdbMultiResult,
}

#[derive(Deserialize)]
struct SearchTvdbMultiResult {
    movies: Vec<SearchTvdbRichItem>,
    series: Vec<SearchTvdbRichItem>,
    anime: Vec<SearchTvdbRichItem>,
}

// --- Movie types ---

#[derive(Deserialize)]
struct MovieResponse {
    movie: MovieResult,
}

#[derive(Deserialize)]
struct MovieResult {
    movie: MovieItem,
}

#[derive(Deserialize)]
struct MovieItem {
    tvdb_id: i64,
    name: String,
    slug: String,
    year: Option<i32>,
    status: String,
    overview: String,
    poster_url: String,
    language: String,
    runtime_minutes: i32,
    sort_title: String,
    imdb_id: String,
    genres: Vec<String>,
    studio: String,
    tmdb_release_date: Option<String>,
}

// --- Series types ---

#[derive(Deserialize)]
struct SeriesResponse {
    series: SeriesResult,
}

#[derive(Deserialize)]
struct SeriesResult {
    series: SeriesItem,
}

#[derive(Deserialize)]
struct SeriesItem {
    tvdb_id: i64,
    name: String,
    sort_name: String,
    slug: String,
    status: String,
    year: Option<i32>,
    first_aired: String,
    overview: String,
    network: String,
    runtime_minutes: i32,
    poster_url: String,
    country: String,
    genres: Vec<String>,
    aliases: Vec<String>,
    seasons: Vec<SeriesSeasonItem>,
    episodes: Vec<SeriesEpisodeItem>,
    #[serde(default)]
    anime_mappings: Vec<AnimeMappingItem>,
}

#[derive(Deserialize)]
struct SeriesSeasonItem {
    tvdb_id: i64,
    number: i32,
    label: String,
    episode_type: String,
}

#[derive(Deserialize)]
struct SeriesEpisodeItem {
    tvdb_id: i64,
    episode_number: i32,
    season_number: i32,
    name: String,
    aired: String,
    runtime_minutes: i32,
    is_filler: bool,
    is_recap: bool,
    overview: String,
    absolute_number: String,
}

#[derive(Deserialize)]
struct AnimeMappingItem {
    mal_id: Option<i64>,
    anilist_id: Option<i64>,
    anidb_id: Option<i64>,
    kitsu_id: Option<i64>,
    thetvdb_season: Option<i32>,
    score: Option<f64>,
    anime_media_type: Option<String>,
    global_media_type: Option<String>,
    status: Option<String>,
    #[serde(default)]
    episode_mappings: Vec<AnimeEpisodeMappingItem>,
}

#[derive(Deserialize)]
struct AnimeEpisodeMappingItem {
    tvdb_season: i32,
    episode_start: i32,
    episode_end: i32,
}

#[async_trait]
impl MetadataGateway for MetadataGatewayClient {
    async fn search_tvdb(
        &self,
        query: &str,
        type_hint: &str,
    ) -> AppResult<Vec<MetadataSearchItem>> {
        let variables = json!({
            "query": query,
            "type": type_hint,
            "limit": 10,
        });

        let data: SearchTvdbResponse = self
            .execute_graphql_apq(SEARCH_TVDB_QUERY, &self.search_hash, variables)
            .await?;

        Ok(data
            .search_tvdb
            .results
            .into_iter()
            .map(|item| MetadataSearchItem {
                tvdb_id: item.tvdb_id.to_string(),
                name: item.name,
                year: item.year,
            })
            .collect())
    }

    async fn search_tvdb_rich(
        &self,
        query: &str,
        type_hint: &str,
        limit: i32,
        language: &str,
    ) -> AppResult<Vec<RichMetadataSearchItem>> {
        let variables = json!({
            "query": query,
            "type": type_hint,
            "limit": limit,
            "language": language,
        });

        let data: SearchTvdbRichResponse = self
            .execute_graphql_apq(SEARCH_TVDB_RICH_QUERY, &self.search_rich_hash, variables)
            .await?;

        Ok(data
            .search_tvdb
            .results
            .into_iter()
            .map(|item| RichMetadataSearchItem {
                tvdb_id: item.tvdb_id.to_string(),
                name: item.name,
                imdb_id: item.imdb_id,
                slug: item.slug,
                type_hint: item.type_hint,
                year: item.year,
                status: item.status,
                overview: item.overview,
                popularity: item.popularity,
                poster_url: item.poster_url,
                language: item.language,
                runtime_minutes: item.runtime_minutes,
                sort_title: item.sort_title,
            })
            .collect())
    }

    async fn search_tvdb_multi(
        &self,
        query: &str,
        limit: i32,
        language: &str,
    ) -> AppResult<MultiMetadataSearchResult> {
        let variables = json!({
            "query": query,
            "limit": limit,
            "language": language,
        });

        let data: SearchTvdbMultiResponse = self
            .execute_graphql_apq(SEARCH_TVDB_MULTI_QUERY, &self.search_multi_hash, variables)
            .await?;

        let convert = |items: Vec<SearchTvdbRichItem>| -> Vec<RichMetadataSearchItem> {
            items
                .into_iter()
                .map(|item| RichMetadataSearchItem {
                    tvdb_id: item.tvdb_id.to_string(),
                    name: item.name,
                    imdb_id: item.imdb_id,
                    slug: item.slug,
                    type_hint: item.type_hint,
                    year: item.year,
                    status: item.status,
                    overview: item.overview,
                    popularity: item.popularity,
                    poster_url: item.poster_url,
                    language: item.language,
                    runtime_minutes: item.runtime_minutes,
                    sort_title: item.sort_title,
                })
                .collect()
        };

        Ok(MultiMetadataSearchResult {
            movies: convert(data.search_tvdb_multi.movies),
            series: convert(data.search_tvdb_multi.series),
            anime: convert(data.search_tvdb_multi.anime),
        })
    }

    async fn get_movie(&self, tvdb_id: i64, language: &str) -> AppResult<MovieMetadata> {
        let variables = json!({
            "tvdbId": tvdb_id,
            "language": language,
        });

        let data: MovieResponse = self
            .execute_graphql_apq(GET_MOVIE_QUERY, &self.movie_hash, variables)
            .await?;
        let m = data.movie.movie;

        Ok(MovieMetadata {
            tvdb_id: m.tvdb_id,
            name: m.name,
            slug: m.slug,
            year: m.year,
            content_status: m.status,
            overview: m.overview,
            poster_url: m.poster_url,
            language: m.language,
            runtime_minutes: m.runtime_minutes,
            sort_title: m.sort_title,
            imdb_id: m.imdb_id,
            genres: m.genres,
            studio: m.studio,
            tmdb_release_date: m.tmdb_release_date,
        })
    }

    async fn get_series(&self, tvdb_id: i64, language: &str) -> AppResult<SeriesMetadata> {
        let variables = json!({
            "id": tvdb_id.to_string(),
            "includeEpisodes": true,
            "language": language,
        });

        let data: SeriesResponse = self
            .execute_graphql_apq(GET_SERIES_QUERY, &self.series_hash, variables)
            .await?;
        let s = data.series.series;

        Ok(SeriesMetadata {
            tvdb_id: s.tvdb_id,
            name: s.name,
            sort_name: s.sort_name,
            slug: s.slug,
            year: s.year,
            content_status: s.status,
            first_aired: s.first_aired,
            overview: s.overview,
            network: s.network,
            runtime_minutes: s.runtime_minutes,
            poster_url: s.poster_url,
            country: s.country,
            genres: s.genres,
            aliases: s.aliases,
            seasons: s
                .seasons
                .into_iter()
                .map(|season| SeasonMetadata {
                    tvdb_id: season.tvdb_id,
                    number: season.number,
                    label: season.label,
                    episode_type: season.episode_type,
                })
                .collect(),
            episodes: s
                .episodes
                .into_iter()
                .map(|ep| EpisodeMetadata {
                    tvdb_id: ep.tvdb_id,
                    episode_number: ep.episode_number,
                    name: ep.name,
                    aired: ep.aired,
                    runtime_minutes: ep.runtime_minutes,
                    is_filler: ep.is_filler,
                    is_recap: ep.is_recap,
                    overview: ep.overview,
                    absolute_number: ep.absolute_number,
                    season_number: ep.season_number,
                })
                .collect(),
            anime_mappings: s
                .anime_mappings
                .into_iter()
                .map(|m| AnimeMapping {
                    mal_id: m.mal_id,
                    anilist_id: m.anilist_id,
                    anidb_id: m.anidb_id,
                    kitsu_id: m.kitsu_id,
                    thetvdb_season: m.thetvdb_season,
                    score: m.score,
                    anime_media_type: m.anime_media_type.unwrap_or_default(),
                    global_media_type: m.global_media_type.unwrap_or_default(),
                    status: m.status.unwrap_or_default(),
                    episode_mappings: m
                        .episode_mappings
                        .into_iter()
                        .map(|e| AnimeEpisodeMapping {
                            tvdb_season: e.tvdb_season,
                            episode_start: e.episode_start,
                            episode_end: e.episode_end,
                        })
                        .collect(),
                })
                .collect(),
        })
    }

    async fn get_movies_bulk(
        &self,
        tvdb_ids: &[i64],
        language: &str,
    ) -> AppResult<HashMap<i64, MovieMetadata>> {
        if tvdb_ids.is_empty() {
            return Ok(HashMap::new());
        }

        // Deduplicate IDs
        let unique: Vec<i64> = tvdb_ids.iter().copied().collect::<HashSet<_>>().into_iter().collect();
        let query = build_bulk_movie_query(&unique, language);

        info!(count = unique.len(), "bulk movie metadata request");
        let data = self.post_graphql_partial(&query).await?;

        let mut results = HashMap::new();
        if let Some(obj) = data.as_object() {
            for (_alias, value) in obj {
                if value.is_null() {
                    continue;
                }
                if let Ok(movie_result) = serde_json::from_value::<MovieResult>(value.clone()) {
                    let m = movie_result.movie;
                    results.insert(m.tvdb_id, MovieMetadata {
                        tvdb_id: m.tvdb_id,
                        name: m.name,
                        slug: m.slug,
                        year: m.year,
                        content_status: m.status,
                        overview: m.overview,
                        poster_url: m.poster_url,
                        language: m.language,
                        runtime_minutes: m.runtime_minutes,
                        sort_title: m.sort_title,
                        imdb_id: m.imdb_id,
                        genres: m.genres,
                        studio: m.studio,
                        tmdb_release_date: m.tmdb_release_date,
                    });
                }
            }
        }

        info!(requested = unique.len(), resolved = results.len(), "bulk movie metadata complete");
        Ok(results)
    }

    async fn get_series_bulk(
        &self,
        tvdb_ids: &[i64],
        language: &str,
    ) -> AppResult<HashMap<i64, SeriesMetadata>> {
        if tvdb_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let unique: Vec<i64> = tvdb_ids.iter().copied().collect::<HashSet<_>>().into_iter().collect();
        let query = build_bulk_series_query(&unique, language);

        info!(count = unique.len(), "bulk series metadata request");
        let data = self.post_graphql_partial(&query).await?;

        let mut results = HashMap::new();
        if let Some(obj) = data.as_object() {
            for (_alias, value) in obj {
                if value.is_null() {
                    continue;
                }
                if let Ok(series_result) = serde_json::from_value::<SeriesResult>(value.clone()) {
                    let s = series_result.series;
                    results.insert(s.tvdb_id, SeriesMetadata {
                        tvdb_id: s.tvdb_id,
                        name: s.name,
                        sort_name: s.sort_name,
                        slug: s.slug,
                        year: s.year,
                        content_status: s.status,
                        first_aired: s.first_aired,
                        overview: s.overview,
                        network: s.network,
                        runtime_minutes: s.runtime_minutes,
                        poster_url: s.poster_url,
                        country: s.country,
                        genres: s.genres,
                        aliases: s.aliases,
                        seasons: s.seasons.into_iter().map(|season| SeasonMetadata {
                            tvdb_id: season.tvdb_id,
                            number: season.number,
                            label: season.label,
                            episode_type: season.episode_type,
                        }).collect(),
                        episodes: s.episodes.into_iter().map(|ep| EpisodeMetadata {
                            tvdb_id: ep.tvdb_id,
                            episode_number: ep.episode_number,
                            name: ep.name,
                            aired: ep.aired,
                            runtime_minutes: ep.runtime_minutes,
                            is_filler: ep.is_filler,
                            is_recap: ep.is_recap,
                            overview: ep.overview,
                            absolute_number: ep.absolute_number,
                            season_number: ep.season_number,
                        }).collect(),
                        anime_mappings: s.anime_mappings.into_iter().map(|m| AnimeMapping {
                            mal_id: m.mal_id,
                            anilist_id: m.anilist_id,
                            anidb_id: m.anidb_id,
                            kitsu_id: m.kitsu_id,
                            thetvdb_season: m.thetvdb_season,
                            score: m.score,
                            anime_media_type: m.anime_media_type.unwrap_or_default(),
                            global_media_type: m.global_media_type.unwrap_or_default(),
                            status: m.status.unwrap_or_default(),
                            episode_mappings: m.episode_mappings.into_iter().map(|e| AnimeEpisodeMapping {
                                tvdb_season: e.tvdb_season,
                                episode_start: e.episode_start,
                                episode_end: e.episode_end,
                            }).collect(),
                        }).collect(),
                    });
                }
            }
        }

        info!(requested = unique.len(), resolved = results.len(), "bulk series metadata complete");
        Ok(results)
    }
}

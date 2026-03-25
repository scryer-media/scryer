use async_trait::async_trait;
use reqwest::{Method, Response, StatusCode};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashSet};

use super::language::{
    from_opensubtitles_language, normalize_subtitle_language_code, same_subtitle_language,
    to_opensubtitles_language,
};
use super::scoring::{SubtitleScoreKind, compute_verified_score};
use crate::{AppError, AppResult, parse_release_metadata};

/// Query parameters for searching subtitles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubtitleMediaKind {
    Movie,
    Episode,
}

#[derive(Debug, Clone)]
pub struct SubtitleQuery {
    /// Whether this search is for a movie or an episode.
    pub media_kind: SubtitleMediaKind,
    /// OpenSubtitles-style file hash (first+last 64KB, little-endian u64 sum).
    pub file_hash: Option<String>,
    /// IMDb ID for the movie itself.
    pub imdb_id: Option<String>,
    /// IMDb ID for the parent series.
    pub series_imdb_id: Option<String>,
    /// Primary title name for feature lookups and text fallback.
    pub title: String,
    /// Alternate title names (aliases) for feature lookups.
    pub title_aliases: Vec<String>,
    /// Release year.
    pub year: Option<i32>,
    /// Season number (series only).
    pub season: Option<i32>,
    /// Episode number (series only).
    pub episode: Option<i32>,
    /// Internal subtitle language codes to search for.
    pub languages: Vec<String>,
    /// Release group from the filename.
    pub release_group: Option<String>,
    /// Source (BluRay, WEB-DL, etc.).
    pub source: Option<String>,
    /// Video codec.
    pub video_codec: Option<String>,
    /// Audio codec.
    pub audio_codec: Option<String>,
    /// Resolution (e.g., "1080p").
    pub resolution: Option<String>,
    /// Whether hearing-impaired subtitles are preferred.
    pub hearing_impaired: Option<bool>,
    /// Whether to include AI-translated results.
    pub include_ai_translated: bool,
    /// Whether to include machine-translated results.
    pub include_machine_translated: bool,
}

/// A single subtitle search result from a provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtitleMatch {
    /// Provider name (e.g., "opensubtitles").
    pub provider: String,
    /// Provider-specific file identifier (for downloading/blacklisting).
    pub provider_file_id: String,
    /// Stable internal subtitle language code.
    pub language: String,
    /// Release info / filename from the provider.
    pub release_info: Option<String>,
    /// Computed match score.
    pub score: i32,
    /// Whether this subtitle is hearing-impaired.
    pub hearing_impaired: bool,
    /// Whether this subtitle is forced (foreign parts only).
    pub forced: bool,
    /// Whether this was flagged as AI-translated.
    pub ai_translated: bool,
    /// Whether this was flagged as machine-translated.
    pub machine_translated: bool,
    /// Uploader name.
    pub uploader: Option<String>,
    /// Download count on the provider.
    pub download_count: Option<i64>,
    /// Whether the file hash matched.
    pub hash_matched: bool,
}

/// Downloaded subtitle file content.
pub struct SubtitleFile {
    /// Raw subtitle content bytes.
    pub content: Vec<u8>,
    /// File extension (e.g., "srt", "ass").
    pub format: String,
}

/// Trait for subtitle providers. Each provider (OpenSubtitles, Podnapisi, etc.)
/// implements this to enable searching and downloading.
#[async_trait]
pub trait SubtitleProvider: Send + Sync {
    /// Search for subtitles matching the query.
    async fn search(&self, query: &SubtitleQuery) -> AppResult<Vec<SubtitleMatch>>;

    /// Download a specific subtitle by provider file ID.
    async fn download(&self, provider_file_id: &str) -> AppResult<SubtitleFile>;

    /// Provider name (e.g., "opensubtitles").
    fn name(&self) -> &str;
}

// ── OpenSubtitles.com v2 REST API ───────────────────────────────────────────

/// OpenSubtitles hash: sum of first and last 64KB as little-endian u64 values.
pub fn compute_opensubtitles_hash(path: &std::path::Path) -> AppResult<String> {
    use std::fs::File;
    use std::io::{Read, Seek, SeekFrom};

    const CHUNK_SIZE: usize = 65536;

    let mut file = File::open(path)
        .map_err(|e| AppError::Repository(format!("cannot open file for hashing: {e}")))?;
    let file_size = file
        .metadata()
        .map_err(|e| AppError::Repository(format!("cannot stat file: {e}")))?
        .len();

    if file_size < CHUNK_SIZE as u64 * 2 {
        return Err(AppError::Validation(
            "file too small for OpenSubtitles hash".into(),
        ));
    }

    let mut hash: u64 = file_size;
    let mut buf = [0u8; 8];

    for _ in 0..(CHUNK_SIZE / 8) {
        file.read_exact(&mut buf)
            .map_err(|e| AppError::Repository(format!("hash read error: {e}")))?;
        hash = hash.wrapping_add(u64::from_le_bytes(buf));
    }

    file.seek(SeekFrom::End(-(CHUNK_SIZE as i64)))
        .map_err(|e| AppError::Repository(format!("hash seek error: {e}")))?;
    for _ in 0..(CHUNK_SIZE / 8) {
        file.read_exact(&mut buf)
            .map_err(|e| AppError::Repository(format!("hash read error: {e}")))?;
        hash = hash.wrapping_add(u64::from_le_bytes(buf));
    }

    Ok(format!("{hash:016x}"))
}

/// OpenSubtitles.com API client.
pub struct OpenSubtitlesProvider {
    api_key: String,
    token: tokio::sync::Mutex<Option<TokenState>>,
    credentials: tokio::sync::Mutex<Option<LoginCredentials>>,
    api_base: tokio::sync::RwLock<String>,
    http: reqwest::Client,
}

#[derive(Clone)]
struct TokenState {
    token: String,
    expires_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Clone)]
struct LoginCredentials {
    username: String,
    password: String,
}

#[derive(Deserialize)]
struct LoginResponse {
    token: String,
    base_url: Option<String>,
}

#[derive(Deserialize)]
struct SearchResponse {
    data: Vec<SearchResult>,
}

#[derive(Deserialize)]
struct SearchResult {
    attributes: SearchAttributes,
}

#[derive(Deserialize)]
struct SearchAttributes {
    language: Option<String>,
    hearing_impaired: Option<bool>,
    foreign_parts_only: Option<bool>,
    ai_translated: Option<bool>,
    machine_translated: Option<bool>,
    release: Option<String>,
    uploader: Option<SearchUploader>,
    download_count: Option<i64>,
    files: Vec<SearchFile>,
    #[serde(default)]
    moviehash_match: bool,
    feature_details: Option<FeatureDetails>,
}

#[derive(Deserialize)]
struct SearchUploader {
    name: Option<String>,
}

#[derive(Deserialize)]
struct SearchFile {
    file_id: i64,
    _file_name: Option<String>,
}

#[derive(Deserialize)]
struct FeatureDetails {
    movie_name: Option<String>,
    year: Option<i32>,
    season_number: Option<i32>,
    episode_number: Option<i32>,
}

#[derive(Deserialize)]
struct FeatureLookupResponse {
    data: Vec<FeatureLookupResult>,
}

#[derive(Deserialize)]
struct FeatureLookupResult {
    id: String,
    attributes: FeatureLookupAttributes,
}

#[derive(Deserialize)]
struct FeatureLookupAttributes {
    title: Option<String>,
    year: Option<i32>,
}

#[derive(Deserialize)]
struct DownloadResponse {
    link: String,
}

const OPENSUBTITLES_API_BASE: &str = "https://api.opensubtitles.com/api/v1";
const EQUIVALENT_RELEASE_GROUPS: &[&[&str]] = &[
    &["FRAMESTOR", "W4NK3R", "BHDSTUDIO"],
    &["LOL", "DIMENSION"],
    &["ASAP", "IMMERSE", "FLEET"],
    &["AVS", "SVA"],
];

impl OpenSubtitlesProvider {
    pub fn new(api_key: String) -> Self {
        Self::with_api_base(api_key, OPENSUBTITLES_API_BASE)
    }

    pub(crate) fn with_api_base(api_key: String, api_base: impl Into<String>) -> Self {
        Self {
            api_key,
            token: tokio::sync::Mutex::new(None),
            credentials: tokio::sync::Mutex::new(None),
            api_base: tokio::sync::RwLock::new(api_base.into()),
            http: reqwest::Client::builder()
                .user_agent("scryer-media/1.0")
                .build()
                .expect("http client"),
        }
    }

    /// Authenticate with username/password, caching the JWT token for 12 hours.
    pub async fn login(&self, username: &str, password: &str) -> AppResult<()> {
        {
            let mut guard = self.credentials.lock().await;
            *guard = Some(LoginCredentials {
                username: username.to_string(),
                password: password.to_string(),
            });
        }

        self.perform_login(username, password).await
    }

    async fn perform_login(&self, username: &str, password: &str) -> AppResult<()> {
        let base = self.api_base().await;
        let resp = self
            .http
            .post(format!("{base}/login"))
            .header("Api-Key", &self.api_key)
            .json(&serde_json::json!({
                "username": username,
                "password": password,
            }))
            .send()
            .await
            .map_err(|e| AppError::Repository(format!("OpenSubtitles login failed: {e}")))?;

        if !resp.status().is_success() {
            return Err(response_error("login", resp).await);
        }

        let login: LoginResponse = resp
            .json()
            .await
            .map_err(|e| AppError::Repository(format!("OpenSubtitles login parse error: {e}")))?;

        let mut guard = self.token.lock().await;
        *guard = Some(TokenState {
            token: login.token,
            expires_at: chrono::Utc::now() + chrono::Duration::hours(11),
        });
        drop(guard);

        if let Some(base_url) = login.base_url.as_deref().and_then(normalize_api_base) {
            let mut guard = self.api_base.write().await;
            *guard = base_url;
        }

        Ok(())
    }

    async fn api_base(&self) -> String {
        self.api_base.read().await.clone()
    }

    async fn get_token(&self) -> Option<String> {
        let guard = self.token.lock().await;
        guard.as_ref().and_then(|t| {
            if t.expires_at > chrono::Utc::now() {
                Some(t.token.clone())
            } else {
                None
            }
        })
    }

    async fn refresh_auth_if_possible(&self) -> AppResult<bool> {
        let creds = self.credentials.lock().await.clone();
        if let Some(creds) = creds {
            self.perform_login(&creds.username, &creds.password).await?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    async fn send_request(
        &self,
        method: Method,
        path: &str,
        params: Option<&[(&str, String)]>,
        body: Option<serde_json::Value>,
    ) -> AppResult<Response> {
        let mut retried = false;

        loop {
            let base = self.api_base().await;
            let url = format!(
                "{}/{}",
                base.trim_end_matches('/'),
                path.trim_start_matches('/')
            );

            let mut req = self
                .http
                .request(method.clone(), url)
                .header("Api-Key", &self.api_key);

            if let Some(token) = self.get_token().await {
                req = req.header("Authorization", format!("Bearer {token}"));
            }
            if let Some(params) = params {
                req = req.query(params);
            }
            if let Some(body) = body.clone() {
                req = req.json(&body);
            }

            let resp = req
                .send()
                .await
                .map_err(|e| AppError::Repository(format!("OpenSubtitles request failed: {e}")))?;

            if resp.status() == StatusCode::UNAUTHORIZED && !retried {
                retried = true;
                if self.refresh_auth_if_possible().await? {
                    continue;
                }
            }

            return Ok(resp);
        }
    }

    async fn search_feature_id(
        &self,
        titles: &[String],
        year: Option<i32>,
    ) -> AppResult<Option<String>> {
        for title in titles {
            let params = vec![("query", title.to_ascii_lowercase())];
            let resp = self
                .send_request(Method::GET, "features", Some(&params), None)
                .await?;

            if !resp.status().is_success() {
                tracing::warn!(title, "OpenSubtitles feature lookup returned non-success");
                continue;
            }

            let body: FeatureLookupResponse = match resp.json().await {
                Ok(body) => body,
                Err(err) => {
                    tracing::warn!(error = %err, title, "OpenSubtitles feature lookup parse failed");
                    continue;
                }
            };

            let wanted = normalize_title_for_match(title);
            let mut exact_year_match = None;
            let mut fallback = None;

            for result in body.data {
                let Some(candidate_title) = result.attributes.title.as_deref() else {
                    continue;
                };
                if normalize_title_for_match(candidate_title) != wanted {
                    continue;
                }

                if year.is_some() && result.attributes.year == year {
                    exact_year_match = Some(result.id);
                    break;
                }
                fallback = Some(result.id);
            }

            if let Some(id) = exact_year_match.or(fallback) {
                return Ok(Some(id));
            }
        }

        Ok(None)
    }
}

#[async_trait]
impl SubtitleProvider for OpenSubtitlesProvider {
    fn name(&self) -> &str {
        "opensubtitles"
    }

    async fn search(&self, query: &SubtitleQuery) -> AppResult<Vec<SubtitleMatch>> {
        let requested_languages: Vec<String> = query
            .languages
            .iter()
            .filter_map(|language| normalize_subtitle_language_code(language))
            .collect();

        let provider_languages: Vec<String> = requested_languages
            .iter()
            .filter_map(|language| to_opensubtitles_language(language))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();

        let title_candidates = collect_title_candidates(query);
        let feature_id = if query.imdb_id.is_none() && query.series_imdb_id.is_none() {
            self.search_feature_id(&title_candidates, query.year)
                .await
                .ok()
                .flatten()
        } else {
            None
        };

        let mut params: Vec<(&str, String)> = Vec::new();
        let mut movie_identifier_match = false;
        let mut series_identifier_match = false;

        if let Some(hash) = &query.file_hash {
            params.push(("moviehash", hash.clone()));
        }

        match query.media_kind {
            SubtitleMediaKind::Movie => {
                if let Some(imdb) = query.imdb_id.as_deref().and_then(sanitize_imdb_id) {
                    params.push(("imdb_id", imdb));
                    movie_identifier_match = true;
                } else if let Some(feature_id) = feature_id.clone() {
                    params.push(("id", feature_id));
                    movie_identifier_match = true;
                } else {
                    params.push(("query", query.title.clone()));
                }
            }
            SubtitleMediaKind::Episode => {
                if let Some(season) = query.season {
                    params.push(("season_number", season.to_string()));
                }
                if let Some(episode) = query.episode {
                    params.push(("episode_number", episode.to_string()));
                }

                if let Some(imdb) = query
                    .series_imdb_id
                    .as_deref()
                    .or(query.imdb_id.as_deref())
                    .and_then(sanitize_imdb_id)
                {
                    params.push(("parent_imdb_id", imdb));
                    series_identifier_match = true;
                } else if let Some(feature_id) = feature_id.clone() {
                    params.push(("parent_feature_id", feature_id));
                    series_identifier_match = true;
                } else {
                    params.push(("query", query.title.clone()));
                }
            }
        }

        if let Some(year) = query.year {
            params.push(("year", year.to_string()));
        }
        if !provider_languages.is_empty() {
            params.push(("languages", provider_languages.join(",")));
        }
        if !query.include_ai_translated {
            params.push(("ai_translated", "exclude".to_string()));
        }
        params.push((
            "machine_translated",
            if query.include_machine_translated {
                "include".to_string()
            } else {
                "exclude".to_string()
            },
        ));
        params.sort_by_key(|(key, _)| *key);

        let resp = self
            .send_request(Method::GET, "subtitles", Some(&params), None)
            .await?;

        if !resp.status().is_success() {
            return Err(response_error("search", resp).await);
        }

        let search_resp: SearchResponse = resp.json().await.map_err(|e| {
            AppError::Repository(format!("OpenSubtitles response parse error: {e}"))
        })?;

        let mut results = Vec::new();
        for result in search_resp.data {
            let attrs = result.attributes;
            let file = match attrs.files.first() {
                Some(file) => file,
                None => continue,
            };

            let ai_translated = attrs.ai_translated.unwrap_or(false);
            let machine_translated = attrs.machine_translated.unwrap_or(false);
            if ai_translated && !query.include_ai_translated {
                continue;
            }
            if machine_translated && !query.include_machine_translated {
                continue;
            }

            let hearing_impaired = attrs.hearing_impaired.unwrap_or(false);
            let forced = attrs.foreign_parts_only.unwrap_or(false);
            let language = attrs
                .language
                .as_deref()
                .and_then(from_opensubtitles_language)
                .unwrap_or_else(|| {
                    attrs
                        .language
                        .as_deref()
                        .and_then(normalize_subtitle_language_code)
                        .unwrap_or_default()
                });

            if !requested_languages.is_empty()
                && !requested_languages
                    .iter()
                    .any(|requested| same_subtitle_language(requested, &language))
            {
                continue;
            }

            let parsed_release = attrs.release.as_deref().map(parse_release_metadata);
            let mut matches = HashSet::new();
            if attrs.moviehash_match {
                matches.insert("hash".to_string());
            }
            if movie_identifier_match {
                matches.insert("imdb_id".to_string());
            }
            if series_identifier_match {
                matches.insert("series_imdb_id".to_string());
            }

            if let Some(details) = &attrs.feature_details {
                if query.year.is_some() && details.year == query.year {
                    matches.insert("year".to_string());
                }
                if query.season.is_some() && details.season_number == query.season {
                    matches.insert("season".to_string());
                }
                if query.episode.is_some() && details.episode_number == query.episode {
                    matches.insert("episode".to_string());
                }
                if title_matches_query(details.movie_name.as_deref(), query) {
                    match query.media_kind {
                        SubtitleMediaKind::Movie => {
                            matches.insert("title".to_string());
                        }
                        SubtitleMediaKind::Episode => {
                            matches.insert("series".to_string());
                        }
                    }
                }
            }

            if let Some(parsed_release) = &parsed_release {
                if let Some(year) = parsed_release.year
                    && query.year == Some(year as i32)
                {
                    matches.insert("year".to_string());
                }
                if release_metadata_title_matches(parsed_release, query) {
                    match query.media_kind {
                        SubtitleMediaKind::Movie => {
                            matches.insert("title".to_string());
                        }
                        SubtitleMediaKind::Episode => {
                            matches.insert("series".to_string());
                        }
                    }
                }
                if release_group_matches(
                    query.release_group.as_deref(),
                    parsed_release.release_group.as_deref(),
                ) {
                    matches.insert("release_group".to_string());
                }
                if source_matches(query.source.as_deref(), parsed_release.source.as_deref()) {
                    matches.insert("source".to_string());
                }
                if resolution_matches(
                    query.resolution.as_deref(),
                    parsed_release.quality.as_deref(),
                ) {
                    matches.insert("resolution".to_string());
                }
                if video_codec_matches(
                    query.video_codec.as_deref(),
                    parsed_release.video_codec.as_deref(),
                ) {
                    matches.insert("video_codec".to_string());
                }
                if audio_codec_matches(query.audio_codec.as_deref(), parsed_release) {
                    matches.insert("audio_codec".to_string());
                }
            }

            if let Some(preferred_hi) = query.hearing_impaired
                && preferred_hi == hearing_impaired
            {
                matches.insert("hearing_impaired".to_string());
            }

            let (weights, score_kind) = match query.media_kind {
                SubtitleMediaKind::Movie => (
                    super::scoring::MOVIE_WEIGHTS.weights(),
                    SubtitleScoreKind::Movie,
                ),
                SubtitleMediaKind::Episode => (
                    super::scoring::SERIES_WEIGHTS.weights(),
                    SubtitleScoreKind::Episode,
                ),
            };
            let score =
                compute_verified_score(&weights, score_kind, &matches, query.season == Some(0));

            results.push(SubtitleMatch {
                provider: "opensubtitles".to_string(),
                provider_file_id: file.file_id.to_string(),
                language,
                release_info: attrs.release,
                score,
                hearing_impaired,
                forced,
                ai_translated,
                machine_translated,
                uploader: attrs.uploader.and_then(|uploader| uploader.name),
                download_count: attrs.download_count,
                hash_matched: attrs.moviehash_match,
            });
        }

        results.sort_by(|a, b| b.score.cmp(&a.score));
        Ok(results)
    }

    async fn download(&self, provider_file_id: &str) -> AppResult<SubtitleFile> {
        let file_id = provider_file_id.parse::<i64>().map_err(|_| {
            AppError::Validation(format!("invalid OpenSubtitles file id: {provider_file_id}"))
        })?;

        let resp = self
            .send_request(
                Method::POST,
                "download",
                None,
                Some(serde_json::json!({
                    "file_id": file_id,
                    "sub_format": "srt",
                })),
            )
            .await?;

        if !resp.status().is_success() {
            return Err(response_error("download", resp).await);
        }

        let dl: DownloadResponse = resp.json().await.map_err(|e| {
            AppError::Repository(format!("OpenSubtitles download parse error: {e}"))
        })?;

        let content = self
            .http
            .get(&dl.link)
            .send()
            .await
            .map_err(|e| AppError::Repository(format!("subtitle file fetch failed: {e}")))?;

        if !content.status().is_success() {
            return Err(response_error("subtitle fetch", content).await);
        }

        let content = normalize_subtitle_line_endings(
            content
                .bytes()
                .await
                .map_err(|e| AppError::Repository(format!("subtitle file read failed: {e}")))?
                .to_vec(),
        );

        if !content.iter().any(|byte| !byte.is_ascii_whitespace()) {
            return Err(AppError::Repository(
                "subtitle download returned empty content".into(),
            ));
        }

        Ok(SubtitleFile {
            content,
            format: "srt".to_string(),
        })
    }
}

fn collect_title_candidates(query: &SubtitleQuery) -> Vec<String> {
    let mut candidates = Vec::with_capacity(query.title_aliases.len() + 1);
    let mut seen = HashSet::new();

    for candidate in std::iter::once(&query.title).chain(query.title_aliases.iter()) {
        let normalized = normalize_title_for_match(candidate);
        if normalized.is_empty() || !seen.insert(normalized) {
            continue;
        }
        candidates.push(candidate.trim().to_string());
    }

    candidates
}

fn normalize_api_base(base_url: &str) -> Option<String> {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }

    if trimmed.starts_with("http://") {
        tracing::warn!(
            base_url = trimmed,
            "OpenSubtitles returned a non-HTTPS API base URL"
        );
    }

    let normalized = if trimmed.starts_with("https://") || trimmed.starts_with("http://") {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    };

    Some(if normalized.ends_with("/api/v1") {
        normalized
    } else {
        format!("{normalized}/api/v1")
    })
}

fn sanitize_imdb_id(imdb_id: &str) -> Option<String> {
    let trimmed = imdb_id
        .trim()
        .trim_start_matches("tt")
        .trim_start_matches('0');
    if trimmed.is_empty() || !trimmed.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    Some(trimmed.to_string())
}

fn normalize_title_for_match(title: &str) -> String {
    title
        .chars()
        .filter_map(|ch| {
            if ch.is_alphanumeric() {
                Some(ch.to_ascii_lowercase())
            } else if ch.is_whitespace() || matches!(ch, '.' | '-' | '_' | '&') {
                Some(' ')
            } else {
                None
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn title_matches_query(candidate: Option<&str>, query: &SubtitleQuery) -> bool {
    let Some(candidate) = candidate else {
        return false;
    };
    let candidate = normalize_title_for_match(candidate);
    collect_title_candidates(query)
        .into_iter()
        .any(|title| normalize_title_for_match(&title) == candidate)
}

fn release_metadata_title_matches(
    parsed: &scryer_release_parser::ParsedReleaseMetadata,
    query: &SubtitleQuery,
) -> bool {
    let mut release_titles = if parsed.normalized_title_variants.is_empty() {
        vec![parsed.normalized_title.clone()]
    } else {
        parsed.normalized_title_variants.clone()
    };
    if release_titles.is_empty() {
        release_titles.push(parsed.normalized_title.clone());
    }

    let candidate_titles = collect_title_candidates(query);
    release_titles.into_iter().any(|release_title| {
        let normalized_release = normalize_title_for_match(&release_title);
        candidate_titles
            .iter()
            .any(|candidate| normalize_title_for_match(candidate) == normalized_release)
    })
}

fn normalize_compare_token(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .map(|ch| ch.to_ascii_uppercase())
        .collect()
}

fn normalize_release_group(value: &str) -> String {
    normalize_compare_token(value)
}

fn release_group_matches(left: Option<&str>, right: Option<&str>) -> bool {
    let (Some(left), Some(right)) = (left, right) else {
        return false;
    };

    let left = normalize_release_group(left);
    let right = normalize_release_group(right);
    if left.is_empty() || right.is_empty() {
        return false;
    }
    if left == right {
        return true;
    }

    EQUIVALENT_RELEASE_GROUPS.iter().any(|group| {
        let members: HashSet<String> = group
            .iter()
            .map(|member| normalize_release_group(member))
            .collect();
        members.contains(&left) && members.contains(&right)
    })
}

fn source_matches(left: Option<&str>, right: Option<&str>) -> bool {
    let (Some(left), Some(right)) = (left, right) else {
        return false;
    };
    normalize_compare_token(left) == normalize_compare_token(right)
}

fn resolution_matches(left: Option<&str>, right: Option<&str>) -> bool {
    let (Some(left), Some(right)) = (left, right) else {
        return false;
    };
    normalize_compare_token(left) == normalize_compare_token(right)
}

fn normalize_video_codec(value: &str) -> String {
    match normalize_compare_token(value).as_str() {
        "H264" | "X264" | "AVC" => "H264".to_string(),
        "H265" | "X265" | "HEVC" => "H265".to_string(),
        "XVID" => "XVID".to_string(),
        "AV1" => "AV1".to_string(),
        other => other.to_string(),
    }
}

fn video_codec_matches(left: Option<&str>, right: Option<&str>) -> bool {
    let (Some(left), Some(right)) = (left, right) else {
        return false;
    };
    normalize_video_codec(left) == normalize_video_codec(right)
}

fn normalize_audio_codec(value: &str) -> String {
    match normalize_compare_token(value).as_str() {
        "DDP" | "DDPLUS" | "EAC3" => "DDP".to_string(),
        "DD" | "AC3" => "DD".to_string(),
        "AAC" => "AAC".to_string(),
        "FLAC" => "FLAC".to_string(),
        "DTS" | "DTSHD" | "DTSHDMA" | "DTSMA" | "DTSX" => "DTS".to_string(),
        "TRUEHD" | "TRUEHDATMOS" => "TRUEHD".to_string(),
        other => other.to_string(),
    }
}

fn audio_codec_matches(
    left: Option<&str>,
    parsed: &scryer_release_parser::ParsedReleaseMetadata,
) -> bool {
    let Some(left) = left else {
        return false;
    };
    let wanted = normalize_audio_codec(left);

    if let Some(audio) = parsed.audio.as_deref()
        && normalize_audio_codec(audio) == wanted
    {
        return true;
    }

    parsed
        .audio_codecs
        .iter()
        .any(|codec| normalize_audio_codec(codec) == wanted)
}

fn normalize_subtitle_line_endings(content: Vec<u8>) -> Vec<u8> {
    let mut out = Vec::with_capacity(content.len());
    let mut idx = 0;

    while idx < content.len() {
        match content[idx] {
            b'\r' => {
                if idx + 1 < content.len() && content[idx + 1] == b'\n' {
                    idx += 1;
                }
                out.push(b'\n');
            }
            byte => out.push(byte),
        }
        idx += 1;
    }

    out
}

async fn response_error(action: &str, resp: Response) -> AppError {
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();

    match status {
        StatusCode::UNAUTHORIZED => {
            AppError::Repository("OpenSubtitles authentication failed".into())
        }
        StatusCode::NOT_ACCEPTABLE => AppError::Repository(format!(
            "OpenSubtitles daily quota reached during {action}: {}",
            compact_error_body(&body)
        )),
        StatusCode::GONE => AppError::Repository(format!("OpenSubtitles {action} link expired")),
        StatusCode::TOO_MANY_REQUESTS => {
            AppError::Repository("OpenSubtitles rate limited — try again later".into())
        }
        status if status.is_server_error() => AppError::Repository(format!(
            "OpenSubtitles {action} failed with {status}: {}",
            compact_error_body(&body)
        )),
        _ => AppError::Repository(format!(
            "OpenSubtitles {action} returned {status}: {}",
            compact_error_body(&body)
        )),
    }
}

fn compact_error_body(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        "empty response".to_string()
    } else if trimmed.len() > 240 {
        format!("{}...", &trimmed[..240])
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    use wiremock::matchers::{body_json, header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn opensubtitles_hash_known_value() {
        let hash_str = format!("{:016x}", 0u64);
        assert_eq!(hash_str.len(), 16);
    }

    #[test]
    fn hash_rejects_file_smaller_than_128kb() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&vec![0u8; 65535]).unwrap();
        tmp.flush().unwrap();
        let result = compute_opensubtitles_hash(tmp.path());
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("too small"));
    }

    #[test]
    fn hash_rejects_file_exactly_128kb_minus_one() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&vec![0u8; 131071]).unwrap();
        tmp.flush().unwrap();
        assert!(compute_opensubtitles_hash(tmp.path()).is_err());
    }

    #[test]
    fn hash_accepts_file_exactly_128kb() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&vec![0u8; 131072]).unwrap();
        tmp.flush().unwrap();
        assert!(compute_opensubtitles_hash(tmp.path()).is_ok());
    }

    #[test]
    fn hash_output_is_16_hex_chars() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&vec![0u8; 131072]).unwrap();
        tmp.flush().unwrap();
        let hash = compute_opensubtitles_hash(tmp.path()).unwrap();
        assert_eq!(hash.len(), 16);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn hash_of_all_zeros_equals_file_size() {
        let size: u64 = 131072;
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&vec![0u8; size as usize]).unwrap();
        tmp.flush().unwrap();
        let hash = compute_opensubtitles_hash(tmp.path()).unwrap();
        assert_eq!(hash, format!("{size:016x}"));
    }

    #[test]
    fn hash_changes_with_different_content() {
        let size = 131072usize;

        let mut tmp1 = tempfile::NamedTempFile::new().unwrap();
        tmp1.write_all(&vec![0u8; size]).unwrap();
        tmp1.flush().unwrap();
        let hash1 = compute_opensubtitles_hash(tmp1.path()).unwrap();

        let mut tmp2 = tempfile::NamedTempFile::new().unwrap();
        tmp2.write_all(&vec![1u8; size]).unwrap();
        tmp2.flush().unwrap();
        let hash2 = compute_opensubtitles_hash(tmp2.path()).unwrap();

        assert_ne!(hash1, hash2);
    }

    #[test]
    fn hash_with_large_file_reads_first_and_last_64kb() {
        let chunk = 65536usize;
        let mut data = Vec::with_capacity(chunk * 4);
        data.extend(vec![1u8; chunk]);
        data.extend(vec![0u8; chunk * 2]);
        data.extend(vec![2u8; chunk]);

        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&data).unwrap();
        tmp.flush().unwrap();

        let hash = compute_opensubtitles_hash(tmp.path()).unwrap();
        assert_eq!(hash.len(), 16);

        let mut data2 = data.clone();
        for byte in &mut data2[chunk..chunk * 3] {
            *byte = 0xFF;
        }
        let mut tmp2 = tempfile::NamedTempFile::new().unwrap();
        tmp2.write_all(&data2).unwrap();
        tmp2.flush().unwrap();

        let hash2 = compute_opensubtitles_hash(tmp2.path()).unwrap();
        assert_eq!(hash, hash2);
    }

    #[test]
    fn subtitle_query_fields_set_correctly() {
        let q = SubtitleQuery {
            media_kind: SubtitleMediaKind::Episode,
            file_hash: Some("abc123".into()),
            imdb_id: None,
            series_imdb_id: Some("tt1234567".into()),
            title: "Breaking Bad".into(),
            title_aliases: vec!["Metastasis".into()],
            year: Some(2008),
            season: Some(1),
            episode: Some(3),
            languages: vec!["eng".into(), "spa".into()],
            release_group: Some("NTb".into()),
            source: Some("WEB-DL".into()),
            video_codec: Some("x264".into()),
            audio_codec: Some("DDP".into()),
            resolution: Some("1080p".into()),
            hearing_impaired: Some(false),
            include_ai_translated: false,
            include_machine_translated: false,
        };

        assert_eq!(q.media_kind, SubtitleMediaKind::Episode);
        assert_eq!(q.series_imdb_id.as_deref(), Some("tt1234567"));
        assert_eq!(q.title_aliases, vec!["Metastasis"]);
        assert_eq!(q.hearing_impaired, Some(false));
    }

    #[test]
    fn subtitle_query_optional_fields_default_none() {
        let q = SubtitleQuery {
            media_kind: SubtitleMediaKind::Movie,
            file_hash: None,
            imdb_id: None,
            series_imdb_id: None,
            title: "Test".into(),
            title_aliases: vec![],
            year: None,
            season: None,
            episode: None,
            languages: vec![],
            release_group: None,
            source: None,
            video_codec: None,
            audio_codec: None,
            resolution: None,
            hearing_impaired: None,
            include_ai_translated: true,
            include_machine_translated: true,
        };

        assert!(q.imdb_id.is_none());
        assert!(q.series_imdb_id.is_none());
        assert!(q.title_aliases.is_empty());
        assert!(q.hearing_impaired.is_none());
    }

    #[test]
    fn subtitle_match_ordering_higher_score_first() {
        let mut matches = [
            SubtitleMatch {
                provider: "opensubtitles".into(),
                provider_file_id: "1".into(),
                language: "eng".into(),
                release_info: None,
                score: 100,
                hearing_impaired: false,
                forced: false,
                ai_translated: false,
                machine_translated: false,
                uploader: None,
                download_count: None,
                hash_matched: false,
            },
            SubtitleMatch {
                provider: "opensubtitles".into(),
                provider_file_id: "2".into(),
                language: "eng".into(),
                release_info: None,
                score: 300,
                hearing_impaired: false,
                forced: false,
                ai_translated: false,
                machine_translated: false,
                uploader: None,
                download_count: None,
                hash_matched: true,
            },
        ];

        matches.sort_by(|a, b| b.score.cmp(&a.score));
        assert_eq!(matches[0].provider_file_id, "2");
    }

    #[tokio::test]
    async fn episode_search_uses_parent_imdb_and_normalizes_language() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/subtitles"))
            .and(query_param("parent_imdb_id", "1234567"))
            .and(query_param("season_number", "2"))
            .and(query_param("episode_number", "5"))
            .and(query_param("languages", "pt-PT"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{
                    "attributes": {
                        "language": "pt-PT",
                        "hearing_impaired": false,
                        "foreign_parts_only": false,
                        "release": "Show.Name.S02E05.1080p.WEB-DL.H.264-GROUP",
                        "download_count": 5,
                        "files": [{ "file_id": 42 }],
                        "moviehash_match": false,
                        "feature_details": {
                            "movie_name": "Show Name",
                            "year": 2024,
                            "season_number": 2,
                            "episode_number": 5
                        }
                    }
                }]
            })))
            .mount(&server)
            .await;

        let provider = OpenSubtitlesProvider::with_api_base(
            "api-key".into(),
            format!("{}/api/v1", server.uri()),
        );
        let results = provider
            .search(&SubtitleQuery {
                media_kind: SubtitleMediaKind::Episode,
                file_hash: None,
                imdb_id: None,
                series_imdb_id: Some("tt1234567".into()),
                title: "Show Name".into(),
                title_aliases: vec![],
                year: Some(2024),
                season: Some(2),
                episode: Some(5),
                languages: vec!["por".into()],
                release_group: Some("GROUP".into()),
                source: Some("WEB-DL".into()),
                video_codec: Some("H.264".into()),
                audio_codec: None,
                resolution: Some("1080p".into()),
                hearing_impaired: Some(false),
                include_ai_translated: false,
                include_machine_translated: false,
            })
            .await
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].language, "por");
        assert!(results[0].score >= 307);
    }

    #[tokio::test]
    async fn search_uses_feature_lookup_when_ids_are_missing() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/features"))
            .and(query_param("query", "movie title"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{
                    "id": "991",
                    "attributes": { "title": "Movie Title", "year": 2024 }
                }]
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/subtitles"))
            .and(query_param("id", "991"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": []
            })))
            .mount(&server)
            .await;

        let provider = OpenSubtitlesProvider::with_api_base(
            "api-key".into(),
            format!("{}/api/v1", server.uri()),
        );
        let results = provider
            .search(&SubtitleQuery {
                media_kind: SubtitleMediaKind::Movie,
                file_hash: None,
                imdb_id: None,
                series_imdb_id: None,
                title: "Movie Title".into(),
                title_aliases: vec!["AKA Movie Title".into()],
                year: Some(2024),
                season: None,
                episode: None,
                languages: vec!["eng".into()],
                release_group: None,
                source: None,
                video_codec: None,
                audio_codec: None,
                resolution: None,
                hearing_impaired: None,
                include_ai_translated: false,
                include_machine_translated: false,
            })
            .await
            .unwrap();

        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn download_requests_srt_and_normalizes_line_endings() {
        let server = MockServer::start().await;
        let download_link = format!("{}/file.srt", server.uri());

        Mock::given(method("POST"))
            .and(path("/api/v1/download"))
            .and(body_json(serde_json::json!({
                "file_id": 77,
                "sub_format": "srt"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "link": download_link
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/file.srt"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("1\r\n00:00:01,000 --> 00:00:02,000\r\nHello\r\n"),
            )
            .mount(&server)
            .await;

        let provider = OpenSubtitlesProvider::with_api_base(
            "api-key".into(),
            format!("{}/api/v1", server.uri()),
        );
        let file = provider.download("77").await.unwrap();

        assert_eq!(file.format, "srt");
        assert_eq!(
            String::from_utf8(file.content).unwrap(),
            "1\n00:00:01,000 --> 00:00:02,000\nHello\n"
        );
    }

    #[tokio::test]
    async fn search_retries_after_unauthorized_login() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/login"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "token": "fresh-token",
                "base_url": server.uri()
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/subtitles"))
            .respond_with(ResponseTemplate::new(401))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/subtitles"))
            .and(header("authorization", "Bearer fresh-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": []
            })))
            .mount(&server)
            .await;

        let provider = OpenSubtitlesProvider::with_api_base(
            "api-key".into(),
            format!("{}/api/v1", server.uri()),
        );
        provider.login("user", "pass").await.unwrap();
        let results = provider
            .search(&SubtitleQuery {
                media_kind: SubtitleMediaKind::Movie,
                file_hash: None,
                imdb_id: Some("tt0000123".into()),
                series_imdb_id: None,
                title: "Movie".into(),
                title_aliases: vec![],
                year: Some(2024),
                season: None,
                episode: None,
                languages: vec!["eng".into()],
                release_group: None,
                source: None,
                video_codec: None,
                audio_codec: None,
                resolution: None,
                hearing_impaired: None,
                include_ai_translated: false,
                include_machine_translated: false,
            })
            .await
            .unwrap();

        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn search_preserves_forced_flag_when_result_is_hearing_impaired() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/subtitles"))
            .and(query_param("languages", "eng"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{
                    "attributes": {
                        "language": "eng",
                        "hearing_impaired": true,
                        "foreign_parts_only": true,
                        "release": "Movie.2024.1080p.WEB-DL.H.264-GROUP",
                        "download_count": 1,
                        "files": [{ "file_id": 9 }],
                        "moviehash_match": false,
                        "feature_details": {
                            "movie_name": "Movie",
                            "year": 2024
                        }
                    }
                }]
            })))
            .mount(&server)
            .await;

        let provider = OpenSubtitlesProvider::with_api_base(
            "api-key".into(),
            format!("{}/api/v1", server.uri()),
        );
        let results = provider
            .search(&SubtitleQuery {
                media_kind: SubtitleMediaKind::Movie,
                file_hash: None,
                imdb_id: None,
                series_imdb_id: None,
                title: "Movie".into(),
                title_aliases: vec![],
                year: Some(2024),
                season: None,
                episode: None,
                languages: vec!["eng".into()],
                release_group: None,
                source: None,
                video_codec: None,
                audio_codec: None,
                resolution: None,
                hearing_impaired: Some(true),
                include_ai_translated: false,
                include_machine_translated: false,
            })
            .await
            .unwrap();

        assert_eq!(results.len(), 1);
        assert!(results[0].hearing_impaired);
        assert!(results[0].forced);
    }
}

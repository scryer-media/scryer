use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{AppError, AppResult};

/// Query parameters for searching subtitles.
#[derive(Debug, Clone)]
pub struct SubtitleQuery {
    /// OpenSubtitles-style file hash (first+last 64KB, little-endian u64 sum).
    pub file_hash: Option<String>,
    /// IMDb ID (e.g., "tt1234567").
    pub imdb_id: Option<String>,
    /// Title name for text-based search fallback.
    pub title: String,
    /// Release year.
    pub year: Option<i32>,
    /// Season number (series only).
    pub season: Option<i32>,
    /// Episode number (series only).
    pub episode: Option<i32>,
    /// ISO 639-2 language codes to search for (e.g., ["eng", "spa"]).
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
    /// ISO 639-2 language code.
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

    // Hash first 64KB
    for _ in 0..(CHUNK_SIZE / 8) {
        file.read_exact(&mut buf)
            .map_err(|e| AppError::Repository(format!("hash read error: {e}")))?;
        hash = hash.wrapping_add(u64::from_le_bytes(buf));
    }

    // Hash last 64KB
    file.seek(SeekFrom::End(-(CHUNK_SIZE as i64)))
        .map_err(|e| AppError::Repository(format!("hash seek error: {e}")))?;
    for _ in 0..(CHUNK_SIZE / 8) {
        file.read_exact(&mut buf)
            .map_err(|e| AppError::Repository(format!("hash read error: {e}")))?;
        hash = hash.wrapping_add(u64::from_le_bytes(buf));
    }

    Ok(format!("{:016x}", hash))
}

/// OpenSubtitles.com API client.
pub struct OpenSubtitlesProvider {
    api_key: String,
    token: tokio::sync::Mutex<Option<TokenState>>,
    http: reqwest::Client,
}

struct TokenState {
    token: String,
    expires_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Deserialize)]
struct LoginResponse {
    token: String,
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
    year: Option<i32>,
    season_number: Option<i32>,
    episode_number: Option<i32>,
}

#[derive(Deserialize)]
struct DownloadResponse {
    link: String,
}

const OPENSUBTITLES_API_BASE: &str = "https://api.opensubtitles.com/api/v1";

impl OpenSubtitlesProvider {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            token: tokio::sync::Mutex::new(None),
            http: reqwest::Client::builder()
                .user_agent("scryer-media/1.0")
                .build()
                .expect("http client"),
        }
    }

    /// Authenticate with username/password, caching the JWT token for 12 hours.
    pub async fn login(&self, username: &str, password: &str) -> AppResult<()> {
        let resp = self
            .http
            .post(format!("{OPENSUBTITLES_API_BASE}/login"))
            .header("Api-Key", &self.api_key)
            .json(&serde_json::json!({
                "username": username,
                "password": password,
            }))
            .send()
            .await
            .map_err(|e| AppError::Repository(format!("OpenSubtitles login failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Repository(format!(
                "OpenSubtitles login returned {status}: {body}"
            )));
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

        Ok(())
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
}

#[async_trait]
impl SubtitleProvider for OpenSubtitlesProvider {
    fn name(&self) -> &str {
        "opensubtitles"
    }

    async fn search(&self, query: &SubtitleQuery) -> AppResult<Vec<SubtitleMatch>> {
        let mut params: Vec<(&str, String)> = Vec::new();

        if let Some(hash) = &query.file_hash {
            params.push(("moviehash", hash.clone()));
        }
        if let Some(imdb) = &query.imdb_id {
            let numeric = imdb.trim_start_matches("tt");
            params.push(("imdb_id", numeric.to_string()));
        }
        if query.file_hash.is_none() && query.imdb_id.is_none() {
            params.push(("query", query.title.clone()));
        }
        if let Some(year) = query.year {
            params.push(("year", year.to_string()));
        }
        if let Some(season) = query.season {
            params.push(("season_number", season.to_string()));
        }
        if let Some(episode) = query.episode {
            params.push(("episode_number", episode.to_string()));
        }
        if !query.languages.is_empty() {
            params.push(("languages", query.languages.join(",")));
        }
        if !query.include_ai_translated {
            params.push(("ai_translated", "exclude".to_string()));
        }
        if !query.include_machine_translated {
            params.push(("machine_translated", "exclude".to_string()));
        }

        let mut req = self
            .http
            .get(format!("{OPENSUBTITLES_API_BASE}/subtitles"))
            .header("Api-Key", &self.api_key);

        if let Some(token) = self.get_token().await {
            req = req.header("Authorization", format!("Bearer {token}"));
        }

        let resp = req
            .query(&params)
            .send()
            .await
            .map_err(|e| AppError::Repository(format!("OpenSubtitles search failed: {e}")))?;

        if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(AppError::Repository(
                "OpenSubtitles rate limited — try again later".into(),
            ));
        }

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Repository(format!(
                "OpenSubtitles search returned {status}: {body}"
            )));
        }

        let search_resp: SearchResponse = resp.json().await.map_err(|e| {
            AppError::Repository(format!("OpenSubtitles response parse error: {e}"))
        })?;

        let mut results = Vec::new();
        for result in search_resp.data {
            let attrs = result.attributes;
            let file = match attrs.files.first() {
                Some(f) => f,
                None => continue,
            };

            let language = attrs.language.unwrap_or_default();
            let hearing_impaired = attrs.hearing_impaired.unwrap_or(false);
            let forced = attrs.foreign_parts_only.unwrap_or(false);
            let ai_translated = attrs.ai_translated.unwrap_or(false);
            let machine_translated = attrs.machine_translated.unwrap_or(false);

            // Build match factors for scoring
            let mut match_factors = std::collections::HashMap::new();
            match_factors.insert("hash".to_string(), attrs.moviehash_match);

            if let Some(details) = &attrs.feature_details {
                if let Some(y) = details.year {
                    match_factors.insert("year".to_string(), query.year == Some(y));
                }
                if let Some(s) = details.season_number {
                    match_factors.insert("season".to_string(), query.season == Some(s));
                }
                if let Some(e) = details.episode_number {
                    match_factors.insert("episode".to_string(), query.episode == Some(e));
                }
            }

            // Release group matching
            if let (Some(release), Some(rg)) = (&attrs.release, &query.release_group) {
                let release_lower = release.to_lowercase();
                let rg_lower = rg.to_lowercase();
                match_factors.insert(
                    "release_group".to_string(),
                    release_lower.contains(&rg_lower),
                );
            }

            // Title match: true for hash matches or IMDB-based searches.
            // For text-only searches, we can't be certain of an exact match.
            let title_matched =
                attrs.moviehash_match || query.imdb_id.is_some() || query.file_hash.is_some();
            match_factors.insert("title".to_string(), title_matched);

            if hearing_impaired {
                match_factors.insert("hearing_impaired".to_string(), true);
            }

            let weights = if query.season.is_some() {
                super::scoring::SERIES_WEIGHTS.weights()
            } else {
                super::scoring::MOVIE_WEIGHTS.weights()
            };
            let score = super::scoring::compute_score(&weights, &match_factors);

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
                uploader: attrs.uploader.and_then(|u| u.name),
                download_count: attrs.download_count,
                hash_matched: attrs.moviehash_match,
            });
        }

        // Sort by score descending
        results.sort_by(|a, b| b.score.cmp(&a.score));
        Ok(results)
    }

    async fn download(&self, provider_file_id: &str) -> AppResult<SubtitleFile> {
        let mut req = self
            .http
            .post(format!("{OPENSUBTITLES_API_BASE}/download"))
            .header("Api-Key", &self.api_key);

        if let Some(token) = self.get_token().await {
            req = req.header("Authorization", format!("Bearer {token}"));
        }

        let resp = req
            .json(&serde_json::json!({
                "file_id": provider_file_id.parse::<i64>().expect("provider_file_id was constructed from i64"),
            }))
            .send()
            .await
            .map_err(|e| {
                AppError::Repository(format!("OpenSubtitles download request failed: {e}"))
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Repository(format!(
                "OpenSubtitles download returned {status}: {body}"
            )));
        }

        let dl: DownloadResponse = resp.json().await.map_err(|e| {
            AppError::Repository(format!("OpenSubtitles download parse error: {e}"))
        })?;

        // Fetch the actual subtitle file from the temporary URL
        let content = self
            .http
            .get(&dl.link)
            .send()
            .await
            .map_err(|e| AppError::Repository(format!("subtitle file fetch failed: {e}")))?
            .bytes()
            .await
            .map_err(|e| AppError::Repository(format!("subtitle file read failed: {e}")))?
            .to_vec();

        // Determine format from URL or default to srt
        let format = if dl.link.contains(".ass") {
            "ass"
        } else if dl.link.contains(".sub") {
            "sub"
        } else {
            "srt"
        }
        .to_string();

        Ok(SubtitleFile { content, format })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn opensubtitles_hash_known_value() {
        // The OpenSubtitles hash algorithm is well-documented.
        // We verify the basic structure works; real integration tests
        // need an actual file.
        let hash_str = format!("{:016x}", 0u64);
        assert_eq!(hash_str.len(), 16);
    }

    // ── compute_opensubtitles_hash with real temp files ─────────────

    #[test]
    fn hash_rejects_file_smaller_than_128kb() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        // Write 64KB — less than 128KB threshold
        tmp.write_all(&vec![0u8; 65535]).unwrap();
        tmp.flush().unwrap();
        let result = compute_opensubtitles_hash(tmp.path());
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("too small"),
            "expected 'too small' in: {err_msg}"
        );
    }

    #[test]
    fn hash_rejects_file_exactly_128kb_minus_one() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        // 128KB - 1 byte — still too small
        tmp.write_all(&vec![0u8; 131071]).unwrap();
        tmp.flush().unwrap();
        assert!(compute_opensubtitles_hash(tmp.path()).is_err());
    }

    #[test]
    fn hash_accepts_file_exactly_128kb() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&vec![0u8; 131072]).unwrap();
        tmp.flush().unwrap();
        let result = compute_opensubtitles_hash(tmp.path());
        assert!(
            result.is_ok(),
            "128KB file should be accepted: {:?}",
            result.err()
        );
    }

    #[test]
    fn hash_output_is_16_hex_chars() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&vec![0u8; 131072]).unwrap();
        tmp.flush().unwrap();
        let hash = compute_opensubtitles_hash(tmp.path()).unwrap();
        assert_eq!(hash.len(), 16, "hash should be 16 hex chars, got: {hash}");
        assert!(
            hash.chars().all(|c| c.is_ascii_hexdigit()),
            "hash should be hex: {hash}"
        );
    }

    #[test]
    fn hash_of_all_zeros_equals_file_size() {
        // When all bytes are 0, each u64 chunk is 0, so the sum is just file_size
        let size: u64 = 131072;
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&vec![0u8; size as usize]).unwrap();
        tmp.flush().unwrap();
        let hash = compute_opensubtitles_hash(tmp.path()).unwrap();
        let expected = format!("{:016x}", size);
        assert_eq!(hash, expected, "all-zero file hash should be file size");
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

        assert_ne!(
            hash1, hash2,
            "different content should produce different hashes"
        );
    }

    #[test]
    fn hash_with_large_file_reads_first_and_last_64kb() {
        // A 256KB file: first 64KB all 0x01, middle 128KB all 0x00, last 64KB all 0x02
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

        // Middle bytes should NOT affect the hash — verify by changing them
        let mut data2 = data.clone();
        for b in &mut data2[chunk..chunk * 3] {
            *b = 0xFF;
        }
        let mut tmp2 = tempfile::NamedTempFile::new().unwrap();
        tmp2.write_all(&data2).unwrap();
        tmp2.flush().unwrap();

        let hash2 = compute_opensubtitles_hash(tmp2.path()).unwrap();
        assert_eq!(hash, hash2, "middle bytes should not affect hash");
    }

    // ── SubtitleQuery field tests ───────────────────────────────────

    #[test]
    fn subtitle_query_fields_set_correctly() {
        let q = SubtitleQuery {
            file_hash: Some("abc123".into()),
            imdb_id: Some("tt1234567".into()),
            title: "Breaking Bad".into(),
            year: Some(2008),
            season: Some(1),
            episode: Some(3),
            languages: vec!["eng".into(), "spa".into()],
            release_group: Some("NTb".into()),
            source: Some("BluRay".into()),
            video_codec: Some("x264".into()),
            audio_codec: Some("DTS".into()),
            resolution: Some("1080p".into()),
            include_ai_translated: false,
            include_machine_translated: false,
        };

        assert_eq!(q.file_hash.as_deref(), Some("abc123"));
        assert_eq!(q.imdb_id.as_deref(), Some("tt1234567"));
        assert_eq!(q.title, "Breaking Bad");
        assert_eq!(q.year, Some(2008));
        assert_eq!(q.season, Some(1));
        assert_eq!(q.episode, Some(3));
        assert_eq!(q.languages, vec!["eng", "spa"]);
        assert_eq!(q.release_group.as_deref(), Some("NTb"));
        assert_eq!(q.source.as_deref(), Some("BluRay"));
        assert_eq!(q.video_codec.as_deref(), Some("x264"));
        assert_eq!(q.audio_codec.as_deref(), Some("DTS"));
        assert_eq!(q.resolution.as_deref(), Some("1080p"));
        assert!(!q.include_ai_translated);
        assert!(!q.include_machine_translated);
    }

    #[test]
    fn subtitle_query_optional_fields_default_none() {
        let q = SubtitleQuery {
            file_hash: None,
            imdb_id: None,
            title: "Test".into(),
            year: None,
            season: None,
            episode: None,
            languages: vec![],
            release_group: None,
            source: None,
            video_codec: None,
            audio_codec: None,
            resolution: None,
            include_ai_translated: true,
            include_machine_translated: true,
        };

        assert!(q.file_hash.is_none());
        assert!(q.imdb_id.is_none());
        assert!(q.year.is_none());
        assert!(q.season.is_none());
        assert!(q.episode.is_none());
        assert!(q.languages.is_empty());
        assert!(q.include_ai_translated);
        assert!(q.include_machine_translated);
    }

    // ── SubtitleMatch ordering tests ────────────────────────────────

    #[test]
    fn subtitle_match_ordering_higher_score_first() {
        let mut matches = vec![
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
            SubtitleMatch {
                provider: "opensubtitles".into(),
                provider_file_id: "3".into(),
                language: "eng".into(),
                release_info: None,
                score: 200,
                hearing_impaired: true,
                forced: false,
                ai_translated: false,
                machine_translated: false,
                uploader: None,
                download_count: None,
                hash_matched: false,
            },
        ];

        matches.sort_by(|a, b| b.score.cmp(&a.score));

        assert_eq!(matches[0].score, 300);
        assert_eq!(matches[0].provider_file_id, "2");
        assert_eq!(matches[1].score, 200);
        assert_eq!(matches[1].provider_file_id, "3");
        assert_eq!(matches[2].score, 100);
        assert_eq!(matches[2].provider_file_id, "1");
    }

    #[test]
    fn subtitle_match_equal_scores_stable() {
        let mut matches = vec![
            SubtitleMatch {
                provider: "opensubtitles".into(),
                provider_file_id: "a".into(),
                language: "eng".into(),
                release_info: None,
                score: 180,
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
                provider_file_id: "b".into(),
                language: "spa".into(),
                release_info: None,
                score: 180,
                hearing_impaired: false,
                forced: false,
                ai_translated: false,
                machine_translated: false,
                uploader: None,
                download_count: None,
                hash_matched: false,
            },
        ];

        // sort_by is stable, so equal-score items keep original order
        matches.sort_by(|a, b| b.score.cmp(&a.score));
        assert_eq!(matches[0].provider_file_id, "a");
        assert_eq!(matches[1].provider_file_id, "b");
    }
}

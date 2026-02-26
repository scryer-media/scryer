use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use scryer_application::{AppError, AppResult, IndexerClient, IndexerSearchResult, SearchMode};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

pub const NZBGEEK_MIN_REQUEST_INTERVAL_MS: u64 = 1100;
pub const NZBGEEK_BASE_BACKOFF_SECONDS: u64 = 10;
pub const NZBGEEK_MAX_BACKOFF_SECONDS: u64 = 900;
const NZBGEEK_MINUTE_BACKOFF_JITTER_MAX_MS: u64 = 250;

#[derive(Clone)]
pub struct NzbGeekSearchClient {
    source_label: String,
    api_url: String,
    api_key: Option<String>,
    user_agent: String,
    http_client: Client,
    rate_limiter: crate::newznab_rate_limiter::NewznabRateLimiter,
}

impl NzbGeekSearchClient {
    pub fn new(
        api_key: Option<String>,
        api_url: Option<String>,
        min_request_interval_ms: u64,
        base_backoff_seconds: u64,
        max_backoff_seconds: u64,
    ) -> Self {
        let resolved_url = api_url.unwrap_or_else(|| "https://api.nzbgeek.info".to_string());
        let cleaned = resolved_url.trim_end_matches('/').to_string();
        let rate_limiter = crate::newznab_rate_limiter::NewznabRateLimiter::new(
            crate::newznab_rate_limiter::NewznabRateLimiterConfig {
                label: "nzbgeek".to_string(),
                cooldown_ms: min_request_interval_ms.max(250),
                max_concurrent_requests: 4,
                base_backoff_seconds: base_backoff_seconds.max(1),
                max_backoff_seconds: max_backoff_seconds
                    .max(base_backoff_seconds.max(1))
                    .max(NZBGEEK_BASE_BACKOFF_SECONDS),
                jitter_max_ms: NZBGEEK_MINUTE_BACKOFF_JITTER_MAX_MS,
            },
        );
        Self {
            source_label: "nzbgeek".to_string(),
            api_url: cleaned,
            api_key: api_key
                .map(|key| key.trim().to_string())
                .filter(|key| !key.is_empty()),
            user_agent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36".to_string(),
            http_client: Client::new(),
            rate_limiter,
        }
    }

    pub fn from_indexer_config(config: &scryer_domain::IndexerConfig) -> Self {
        let rate_limiter = crate::newznab_rate_limiter::NewznabRateLimiter::from_indexer_config(
            &config.name,
            config.rate_limit_seconds,
            config.rate_limit_burst,
        );
        Self {
            source_label: config.name.clone(),
            api_url: config.base_url.trim_end_matches('/').to_string(),
            api_key: config.api_key_encrypted.clone(),
            user_agent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36".to_string(),
            http_client: Client::new(),
            rate_limiter,
        }
    }

    fn endpoint(&self) -> String {
        if self.api_url.ends_with("/api") {
            self.api_url.clone()
        } else {
            format!("{}/api", self.api_url)
        }
    }



    async fn execute_search_request<'a>(
        &self,
        endpoint: &str,
        params: &NzbGeekSearchQuery<'a>,
    ) -> AppResult<(reqwest::StatusCode, reqwest::header::HeaderMap, String)> {
        let response = self
            .http_client
            .get(endpoint)
            .header("Accept", "application/json, */*; q=0.8")
            .header("Accept-Language", "en-US,en;q=0.9")
            .header("User-Agent", &self.user_agent)
            .query(params)
            .send()
            .await;

        let response = match response {
            Ok(response) => response,
            Err(err) => {
                return Err(AppError::Repository(format!(
                    "nzbgeek request failed: {err}"
                )));
            }
        };

        let status = response.status();
        let headers = response.headers().clone();
        let body = response
            .text()
            .await
            .map_err(|err| AppError::Repository(format!("nzbgeek response read failed: {err}")))?;

        Ok((status, headers, body))
    }
}

#[derive(Serialize)]
struct NzbGeekSearchQuery<'a> {
    t: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    q: Option<&'a str>,
    apikey: &'a str,
    o: &'a str,
    extended: u8,
    limit: usize,
    #[serde(rename = "imdbid", skip_serializing_if = "Option::is_none")]
    imdb_id: Option<&'a str>,
    #[serde(rename = "tvdbid", skip_serializing_if = "Option::is_none")]
    tvdb_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cat: Option<&'a str>,
}

#[async_trait]
impl IndexerClient for NzbGeekSearchClient {
    async fn search(
        &self,
        query: String,
        imdb_id: Option<String>,
        tvdb_id: Option<String>,
        category: Option<String>,
        newznab_categories: Option<Vec<String>>,
        limit: usize,
        _mode: SearchMode,
    ) -> AppResult<Vec<IndexerSearchResult>> {
        let query = query.trim();
        let api_key = self
            .api_key
            .clone()
            .ok_or_else(|| AppError::Validation("NZBGeek API key is not configured".into()))?;
        let imdb_id = imdb_id
            .map(|value| value.trim().trim_start_matches("tt").to_string())
            .filter(|value| !value.is_empty());
        let tvdb_id = tvdb_id
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let category = category
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        if query.is_empty() && imdb_id.is_none() && tvdb_id.is_none() {
            return Ok(vec![]);
        }

        let _rate_limit_guard = self.rate_limiter.acquire().await?;

        let is_movie_category = category
            .as_deref()
            .map(|c| c.eq_ignore_ascii_case("movie"))
            .unwrap_or(false);
        let is_tv_category = category
            .as_deref()
            .map(|c| {
                c.eq_ignore_ascii_case("tv")
                    || c.eq_ignore_ascii_case("series")
                    || c.eq_ignore_ascii_case("anime")
            })
            .unwrap_or(false);
        // Category takes precedence over ID presence so that episode searches
        // for titles with an IMDB ID still use t=tvsearch, not t=movie.
        let search_type = if is_movie_category {
            "movie"
        } else if is_tv_category {
            "tvsearch"
        } else if imdb_id.is_some() {
            "movie"
        } else if tvdb_id.is_some() {
            "tvsearch"
        } else {
            "search"
        };
        // Use user-configured Newznab categories if available, otherwise fall
        // back to hardcoded defaults for generic searches.
        let newznab_cat: Option<String> = if let Some(ref cats) = newznab_categories {
            if cats.is_empty() {
                None
            } else {
                Some(cats.join(","))
            }
        } else {
            match search_type {
                "search" if is_movie_category => Some("2000".to_string()),
                "search" if is_tv_category => Some("5000".to_string()),
                _ => None,
            }
        };
        // t=movie doesn't support tvdbid; t=tvsearch doesn't support imdbid
        let effective_imdb = if search_type == "movie" { imdb_id.as_deref() } else { None };
        let effective_tvdb = if search_type == "tvsearch" { tvdb_id.as_deref() } else { None };
        // For t=movie with imdbid the ID is authoritative — drop the free-text
        // query so appended years don't narrow results to zero.  For t=tvsearch
        // keep q (e.g. "S01E01") because tvdbid only identifies the show.
        let effective_query = if effective_imdb.is_some() {
            None
        } else {
            Some(query).filter(|value| !value.is_empty())
        };
        let mut params = NzbGeekSearchQuery {
            t: search_type,
            q: effective_query,
            apikey: api_key.as_str(),
            o: "json",
            extended: 1,
            limit: limit.clamp(1, 200),
            imdb_id: effective_imdb,
            tvdb_id: effective_tvdb,
            cat: newznab_cat.as_deref(),
        };

        let endpoint = self.endpoint();
        let tvdb_id = tvdb_id.as_deref().unwrap_or("");
        let tvdb_id_for_log = tvdb_id;
        info!(
            endpoint = endpoint.as_str(),
            search_type = search_type,
            query = effective_query.unwrap_or(""),
            imdb_id = effective_imdb.unwrap_or(""),
            tvdb_id = effective_tvdb.unwrap_or(""),
            cat = newznab_cat.as_deref().unwrap_or(""),
            category = category.as_deref().unwrap_or(""),
            limit = params.limit,
            "requesting nzbgeek search"
        );

        let (mut status, mut headers, mut body) =
            match self.execute_search_request(&endpoint, &params).await {
                Ok(value) => value,
                Err(err) => {
                    self.rate_limiter.record_response(None, None, &Default::default(), None)
                        .await?;
                    return Err(err);
                }
            };

        // When Newznab endpoints return a 500 (observed with movie identifier
        // combinations), retry with a generic search query so users still get
        // actionable results in the UI instead of a hard failure.
        if status == reqwest::StatusCode::INTERNAL_SERVER_ERROR
            && search_type != "search"
            && (imdb_id.is_some() || !tvdb_id_for_log.is_empty())
        {
            let fallback_query = if query.is_empty() {
                imdb_id
                    .as_deref()
                    .or_else(|| (!tvdb_id_for_log.is_empty()).then_some(tvdb_id_for_log))
            } else {
                Some(query)
            };
            let fallback_params = NzbGeekSearchQuery {
                t: "search",
                q: fallback_query.filter(|value| !value.is_empty()),
                apikey: api_key.as_str(),
                o: "json",
                extended: 1,
                limit: limit.clamp(1, 200),
                imdb_id: None,
                tvdb_id: None,
                cat: newznab_cat.as_deref(),
            };
            warn!(
                endpoint = endpoint.as_str(),
                query = query,
                original_search_type = search_type,
                category = category.as_deref().unwrap_or(""),
                "nzbgeek search returned 500, retrying with generic search"
            );

            match self.execute_search_request(&endpoint, &fallback_params).await {
                Ok(value) => {
                    params = fallback_params;
                    let (next_status, next_headers, next_body) = value;
                    status = next_status;
                    headers = next_headers;
                    body = next_body;
                }
                Err(err) => {
                    let api_limits = crate::newznab_rate_limiter::parse_apilimits_from_headers(&headers);
                    self.rate_limiter.record_response(Some(status), None, &headers, api_limits.as_ref()).await?;
                    return Err(err);
                }
            }
        }

        let status_text = status.to_string();
        let parsed_error = parse_nzbgeek_error_json(&body);
        let api_limits = crate::newznab_rate_limiter::parse_apilimits_from_headers(&headers);
        self.rate_limiter.record_response(
            Some(status),
            parsed_error.as_ref().map(|(code, _)| code.as_str()),
            &headers,
            api_limits.as_ref(),
        )
        .await?;

        if !status.is_success() {
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                warn!(
                    endpoint = endpoint.as_str(),
                    query = query,
                    tvdb_id = tvdb_id,
                    category = category.as_deref().unwrap_or(""),
                    "nzbgeek rate-limit response received (HTTP 429)"
                );
            }
            let preview = body.chars().take(600).collect::<String>();
            warn!(
                endpoint = endpoint.as_str(),
                status = status_text.as_str(),
                preview = preview.as_str(),
                "nzbgeek returned non-success response"
            );
            return Err(AppError::Repository(format!(
                "nzbgeek rejected request with status {status}"
            )));
        }

        if let Some((code, description)) = parsed_error {
            warn!(
                endpoint = endpoint.as_str(),
                code = code.as_str(),
                description = description.as_str(),
                "nzbgeek returned structured error payload"
            );
            return Err(AppError::Repository(format!(
                "nzbgeek error {}: {}",
                code, description
            )));
        }

        let parsed = parse_newznab_json(&body, params.limit, &self.source_label);
        info!(
            endpoint = endpoint.as_str(),
            source = self.source_label.as_str(),
            query = query,
            count = parsed.len(),
            "indexer search parsed successfully"
        );

        Ok(parsed)
    }
}

// --- NZBGeek JSON response types ---

#[derive(Deserialize)]
struct NzbGeekJsonResponse {
    channel: Option<NzbGeekJsonChannel>,
    error: Option<NzbGeekJsonErrorNode>,
}

#[derive(Deserialize)]
struct NzbGeekJsonChannel {
    item: Option<NzbGeekJsonItems>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum NzbGeekJsonItems {
    Many(Vec<NzbGeekJsonItem>),
    One(Box<NzbGeekJsonItem>),
}

impl NzbGeekJsonItems {
    fn into_vec(self) -> Vec<NzbGeekJsonItem> {
        match self {
            NzbGeekJsonItems::Many(v) => v,
            NzbGeekJsonItems::One(v) => vec![*v],
        }
    }
}

#[derive(Deserialize)]
struct NzbGeekJsonItem {
    title: Option<String>,
    link: Option<String>,
    #[serde(rename = "pubDate")]
    pub_date: Option<String>,
    enclosure: Option<NzbGeekJsonEnclosure>,
    attr: Option<NzbGeekJsonAttributes>,
}

#[derive(Deserialize)]
struct NzbGeekJsonEnclosure {
    #[serde(rename = "@attributes")]
    attributes: Option<NzbGeekJsonEnclosureAttrs>,
}

#[derive(Deserialize)]
struct NzbGeekJsonEnclosureAttrs {
    url: Option<String>,
    length: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum NzbGeekJsonAttributes {
    Many(Vec<NzbGeekJsonAttributeNode>),
    One(Box<NzbGeekJsonAttributeNode>),
}

impl NzbGeekJsonAttributes {
    fn into_vec(self) -> Vec<NzbGeekJsonAttributeNode> {
        match self {
            NzbGeekJsonAttributes::Many(v) => v,
            NzbGeekJsonAttributes::One(v) => vec![*v],
        }
    }
}

#[derive(Deserialize)]
struct NzbGeekJsonAttributeNode {
    #[serde(rename = "@attributes")]
    attributes: Option<NzbGeekJsonAttributeAttrs>,
}

#[derive(Deserialize)]
struct NzbGeekJsonAttributeAttrs {
    name: Option<String>,
    value: Option<String>,
}

#[derive(Deserialize)]
struct NzbGeekJsonErrorNode {
    #[serde(rename = "@attributes")]
    attributes: Option<NzbGeekJsonErrorAttrs>,
}

#[derive(Deserialize)]
struct NzbGeekJsonErrorAttrs {
    code: Option<String>,
    description: Option<String>,
}

pub(crate) fn parse_retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    let retry_after = parse_delay_header(headers.get("Retry-After"));
    if retry_after.is_some() {
        return retry_after;
    }

    let rate_limit_reset = parse_rate_limit_reset(headers.get("X-RateLimit-Reset"));
    if rate_limit_reset.is_some() {
        return rate_limit_reset;
    }

    let rate_limit_remaining = headers
        .get("X-RateLimit-Remaining")
        .and_then(parse_header_str)
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(1);
    if rate_limit_remaining == 0 {
        parse_rate_limit_reset(headers.get("X-RateLimit-Reset"))
    } else {
        None
    }
}

fn parse_delay_header(header_value: Option<&reqwest::header::HeaderValue>) -> Option<Duration> {
    let raw = header_value.and_then(parse_header_str)?;
    parse_delay_text(&raw)
}

fn parse_rate_limit_reset(header_value: Option<&reqwest::header::HeaderValue>) -> Option<Duration> {
    let raw = header_value.and_then(parse_header_str)?;
    parse_reset_delay(&raw).or_else(|| parse_delay_text(&raw))
}

fn parse_delay_text(raw: &str) -> Option<Duration> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Ok(seconds) = trimmed.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }

    parse_http_date_delay(trimmed)
}

fn parse_reset_delay(raw: &str) -> Option<Duration> {
    let seconds = raw.trim().parse::<u64>().ok()?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();

    // NZBGeek occasionally exposes Unix epoch resets.
    if seconds > 1_000_000_000 {
        let delta = seconds.saturating_sub(now);
        if delta == 0 {
            Some(Duration::from_secs(0))
        } else {
            Some(Duration::from_secs(delta))
        }
    } else if seconds <= 300 {
        // short delays are typically relative seconds
        Some(Duration::from_secs(seconds))
    } else {
        None
    }
}

fn parse_http_date_delay(raw: &str) -> Option<Duration> {
    let parsed = DateTime::parse_from_rfc2822(raw)
        .ok()
        .or_else(|| DateTime::parse_from_rfc3339(raw).ok())?;
    let parsed_utc = parsed.with_timezone(&Utc);
    let now = Utc::now();
    let delta = parsed_utc.signed_duration_since(now).num_milliseconds();
    if delta <= 0 {
        Some(Duration::from_secs(0))
    } else {
        u64::try_from(delta).ok().map(Duration::from_millis)
    }
}

fn parse_header_str(value: &reqwest::header::HeaderValue) -> Option<String> {
    value.to_str().map(str::to_string).ok()
}

fn parse_nzbgeek_error_json(body: &str) -> Option<(String, String)> {
    let parsed: NzbGeekJsonResponse = serde_json::from_str(body).ok()?;
    let attrs = parsed.error?.attributes?;
    let code = attrs.code.unwrap_or_else(|| "unknown".into());
    let description = attrs.description.unwrap_or_else(|| "unknown".into());
    Some((code, description))
}

pub(crate) fn parse_newznab_json(body: &str, limit: usize, source_label: &str) -> Vec<IndexerSearchResult> {
    let parsed: NzbGeekJsonResponse = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(err) => {
            warn!(error = %err, "failed to parse nzbgeek json response");
            return vec![];
        }
    };

    let items = match parsed.channel.and_then(|c| c.item) {
        Some(items) => items.into_vec(),
        None => return vec![],
    };

    items
        .into_iter()
        .take(limit)
        .filter_map(|item| {
            let title = item.title?;
            let enclosure_attrs = item.enclosure.and_then(|e| e.attributes);
            let download_url = enclosure_attrs.as_ref().and_then(|a| a.url.clone());
            let size_bytes = enclosure_attrs
                .as_ref()
                .and_then(|a| a.length.as_ref())
                .and_then(|v| v.replace(',', "").parse::<i64>().ok());
            let (thumbs_up, thumbs_down, nzbgeek_languages, nzbgeek_subtitles, nzbgeek_grabs, nzbgeek_password_protected) = item
                .attr
                .map(extract_nzbgeek_metadata)
                .unwrap_or((None, None, None, None, None, None));

            Some(IndexerSearchResult {
                source: source_label.to_string(),
                title,
                link: item.link,
                download_url,
                size_bytes,
                published_at: item.pub_date,
                thumbs_up,
                thumbs_down,
                nzbgeek_languages,
                nzbgeek_subtitles,
                nzbgeek_grabs,
                nzbgeek_password_protected,
                parsed_release_metadata: None,
                quality_profile_decision: None,
            })
        })
        .collect()
}

fn parse_nzbgeek_password(raw: &str) -> Option<String> {
    let value = raw.trim();
    if value.is_empty() || value == "0" {
        None
    } else {
        Some(value.to_string())
    }
}

fn normalize_nzbgeek_delimited_value(raw: &str) -> Option<Vec<String>> {
    let items = raw
        .split(" - ")
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .collect::<Vec<String>>();

    if items.is_empty() {
        None
    } else {
        Some(items)
    }
}

#[allow(clippy::type_complexity)]
fn extract_nzbgeek_metadata(
    attributes: NzbGeekJsonAttributes,
) -> (
    Option<i32>,
    Option<i32>,
    Option<Vec<String>>,
    Option<Vec<String>>,
    Option<i64>,
    Option<String>,
) {
    let mut thumbs_up = None;
    let mut thumbs_down = None;
    let mut nzbgeek_languages = Vec::new();
    let mut nzbgeek_subtitles = Vec::new();
    let mut nzbgeek_grabs = None;
    let mut nzbgeek_password_protected = None;

    for node in attributes.into_vec() {
        let Some(attrs) = node.attributes else {
            continue;
        };
        let Some(name) = attrs.name.as_deref() else {
            continue;
        };
        let Some(value) = attrs.value.as_deref() else {
            continue;
        };
        let normalized_name: String = name
            .chars()
            .filter(|ch| ch.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase();

        match normalized_name.as_str() {
            "thumbsup" | "thumbup" => {
                thumbs_up = value.trim().replace(',', "").parse::<i32>().ok()
            }
            "thumbsdown" | "thumbdown" => {
                thumbs_down = value.trim().replace(',', "").parse::<i32>().ok()
            }
            "language" => {
                if let Some(values) = normalize_nzbgeek_delimited_value(value) {
                    nzbgeek_languages.extend(values);
                }
            }
            "subs" => {
                if let Some(values) = normalize_nzbgeek_delimited_value(value) {
                    nzbgeek_subtitles.extend(values);
                }
            }
            "grabs" => {
                nzbgeek_grabs = value.trim().replace(',', "").parse::<i64>().ok();
            }
            "password" => {
                nzbgeek_password_protected = parse_nzbgeek_password(value);
            }
            _ => {}
        }
    }

    (
        thumbs_up,
        thumbs_down,
        if nzbgeek_languages.is_empty() {
            None
        } else {
            Some(nzbgeek_languages)
        },
        if nzbgeek_subtitles.is_empty() {
            None
        } else {
            Some(nzbgeek_subtitles)
        },
        nzbgeek_grabs,
        nzbgeek_password_protected,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_nzbgeek_json_extracts_basic_fields() {
        let fixture = r#"{
  "channel": {
    "item": [
      {
        "title": "Show S01E01",
        "link": "https://example.com/nzb",
        "pubDate": "Mon, 01 Jan 2007 00:00:00 +0000",
        "attr": [
          {
            "@attributes": {
              "name": "thumbsup",
              "value": "12"
            }
          },
          {
            "@attributes": {
              "name": "thumbsdown",
              "value": "3"
            }
          }
          ,
          {
            "@attributes": {
              "name": "language",
              "value": "English - Japanese"
            }
          },
          {
            "@attributes": {
              "name": "grabs",
              "value": "14321"
            }
          },
          {
            "@attributes": {
              "name": "password",
              "value": "0"
            }
          },
          {
            "@attributes": {
              "name": "subs",
              "value": "English - Spanish"
            }
          }
        ],
        "enclosure": {
          "@attributes": {
            "url": "https://example.com/download.nzb?title=Show%20S01E01&apikey=abc",
            "length": "153600",
            "type": "application/x-nzb"
          }
        }
      },
      {
        "title": "Second",
        "link": "https://example.com/second",
        "enclosure": {
          "@attributes": {
            "url": "https://example.com/second.nzb",
            "length": "2048",
            "type": "application/x-nzb"
          }
        }
      }
    ]
  }
}"#;

        let items = parse_newznab_json(fixture, 2, "nzbgeek");
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].source, "nzbgeek");
        assert_eq!(items[0].title, "Show S01E01");
        assert_eq!(items[0].size_bytes, Some(153600));
        assert_eq!(items[0].thumbs_up, Some(12));
        assert_eq!(items[0].thumbs_down, Some(3));
        assert_eq!(items[0].nzbgeek_grabs, Some(14321));
        assert_eq!(items[0].nzbgeek_password_protected, None);
        assert_eq!(
            items[0].nzbgeek_languages,
            Some(vec!["English".to_string(), "Japanese".to_string()])
        );
        assert_eq!(
            items[0].nzbgeek_subtitles,
            Some(vec!["English".to_string(), "Spanish".to_string()])
        );
        assert_eq!(
            items[0].download_url.as_deref(),
            Some("https://example.com/download.nzb?title=Show%20S01E01&apikey=abc")
        );
    }

    #[test]
    fn parse_nzbgeek_json_handles_single_item() {
        let fixture = r#"{
  "channel": {
    "item": {
      "title": "Solo.Result.2024.1080p",
      "link": "https://example.com/solo",
      "enclosure": {
        "@attributes": {
          "url": "https://example.com/solo.nzb",
          "length": "4096",
          "type": "application/x-nzb"
        }
      }
    }
  }
}"#;

        let items = parse_newznab_json(fixture, 10, "nzbgeek");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "Solo.Result.2024.1080p");
        assert_eq!(items[0].size_bytes, Some(4096));
    }

    #[test]
    fn parse_nzbgeek_retry_after_reads_headers() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            "Retry-After",
            reqwest::header::HeaderValue::from_static("45"),
        );

        assert_eq!(
            parse_retry_after(&headers),
            Some(std::time::Duration::from_secs(45))
        );

        headers.clear();
        headers.insert(
            "X-RateLimit-Remaining",
            reqwest::header::HeaderValue::from_static("0"),
        );
        headers.insert(
            "X-RateLimit-Reset",
            reqwest::header::HeaderValue::from_static("30"),
        );

        assert_eq!(
            parse_retry_after(&headers),
            Some(std::time::Duration::from_secs(30))
        );
    }

    #[test]
    fn parse_nzbgeek_retry_after_parses_http_date() {
        let delay_time = chrono::Utc::now() + chrono::Duration::seconds(120);
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            "Retry-After",
            reqwest::header::HeaderValue::from_str(&delay_time.to_rfc2822()).unwrap(),
        );

        let delay = parse_retry_after(&headers).expect("retry delay should be parsed");
        assert!(delay.as_secs() >= 90);
        assert!(delay.as_secs() <= 150);
    }
}

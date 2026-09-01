//! Provider adapters for media-server watch signals (RFC 137 section 7.3).
//!
//! One [`MediaServerSignalSource`] implementation that dispatches on the
//! connection's provider. Jellyfin is the only arm that reads today; Emby and
//! Plex return an explicit "unsupported" error rather than an empty list,
//! because "this provider has no adapter yet" and "this person has watched
//! nothing" must never be the same answer.
//!
//! # Jellyfin
//!
//! `GET /Users/{userId}/Items` with `Filters=IsPlayed`, recursive over the
//! whole library, restricted to `Movie,Episode`, and asking for the fields the
//! mapper needs: `ProviderIds` for the item, `SeriesId`/`SeriesName` and the
//! season/episode indexes for episodes, and `UserData` for played state, play
//! count, and last-played time. Paged with `StartIndex`/`Limit`.
//!
//! Two bounds, both deliberate:
//!
//! * a **page size** so a large library does not arrive as one enormous body;
//! * a **hard cap** on total items, after which the read stops and logs. A
//!   truncated read is worse than a complete one but far better than an
//!   unbounded fetch against an operator's server; the cap is set high enough
//!   that a realistic played history never reaches it.
//!
//! # Series ids
//!
//! Jellyfin does not put the series' TMDB/TVDB ids on the episode. Rather than
//! one lookup per episode, the adapter collects the distinct `SeriesId`s it saw
//! and resolves them in a single `Ids=` batch, then attaches each series'
//! external ids to its episodes. A series that cannot be resolved leaves its
//! episodes without series ids, which mapping treats as unmapped rather than
//! guessing.
//!
//! # Malformed items
//!
//! An item without an `Id`, or with a `Type` that is neither `Movie` nor
//! `Episode`, is skipped and counted. One unreadable entry must not fail a
//! participant's whole history.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use scryer_application::{AppError, AppResult, MediaServerSignalSource, ProviderPlayedItem};
use scryer_domain::{MediaServerConnection, MediaServerProvider, MediaServerSignalKind};
use scryer_outbound_http::generic_reqwest_client;
use serde_json::Value;
use tracing::warn;
use url::Url;

/// Per-request budget. Higher than the playback probe's five seconds: this is a
/// paged library query, not a memory read, but it still sits inside a
/// background job that must not hang for minutes.
const SIGNAL_FETCH_TIMEOUT: Duration = Duration::from_secs(30);

/// Items requested per page.
pub const JELLYFIN_SIGNAL_PAGE_SIZE: usize = 500;

/// Hard ceiling on items read for one participant in one sweep. Beyond this the
/// read stops and logs a truncation rather than paging forever.
pub const JELLYFIN_SIGNAL_MAX_ITEMS: usize = 20_000;

/// Series metadata lookups are batched; this bounds one batch's `Ids=` list so
/// the query string stays a sane length.
const JELLYFIN_SERIES_BATCH: usize = 100;

const JELLYFIN_ITEM_FIELDS: &str =
    "ProviderIds,UserData,ParentIndexNumber,IndexNumber,SeriesId,SeriesName,ParentId,Path";

/// External id sources Scryer joins on, mapped from Jellyfin's `ProviderIds`
/// keys. Jellyfin's capitalization has drifted across versions, so the match is
/// case-insensitive on the key rather than a fixed spelling.
const PROVIDER_ID_KEYS: [(&str, &str); 3] = [("tmdb", "tmdb"), ("tvdb", "tvdb"), ("imdb", "imdb")];

pub struct HttpMediaServerSignalSource {
    client: reqwest::Client,
}

impl Default for HttpMediaServerSignalSource {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpMediaServerSignalSource {
    pub fn new() -> Self {
        Self {
            client: generic_reqwest_client(),
        }
    }

    async fn fetch_json(&self, url: Url, api_key: &str) -> AppResult<Value> {
        let response = self
            .client
            .get(url)
            .header("Accept", "application/json")
            .header("X-Emby-Token", api_key)
            .timeout(SIGNAL_FETCH_TIMEOUT)
            .send()
            .await
            .map_err(|error| AppError::Repository(transport_reason(&error)))?;
        let status = response.status();
        if !status.is_success() {
            // The reason is a bare status: the request URL carries the
            // participant's id and the header carries the admin key, and
            // neither belongs in an error that reaches a state row.
            return Err(AppError::Repository(format!("status {}", status.as_u16())));
        }
        response
            .json::<Value>()
            .await
            .map_err(|_| AppError::Repository("unreadable response".into()))
    }

    /// Every played movie and episode for one Jellyfin user.
    async fn fetch_jellyfin(
        &self,
        connection: &MediaServerConnection,
        api_key: &str,
        external_user_id: &str,
    ) -> AppResult<Vec<ProviderPlayedItem>> {
        let base = jellyfin_base(&connection.base_url)?;
        let mut items: Vec<ProviderPlayedItem> = Vec::new();
        let mut skipped = 0_usize;
        let mut start_index = 0_usize;

        loop {
            let url = played_items_url(&base, external_user_id, start_index)?;
            let body = self.fetch_json(url, api_key).await?;
            let page = body
                .get("Items")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if page.is_empty() {
                break;
            }

            let (mut normalized, page_skipped) = normalize_jellyfin_items(&page);
            skipped += page_skipped;
            items.append(&mut normalized);

            start_index += page.len();
            if page.len() < JELLYFIN_SIGNAL_PAGE_SIZE {
                break;
            }
            if items.len() >= JELLYFIN_SIGNAL_MAX_ITEMS {
                warn!(
                    connection_id = connection.id.as_str(),
                    cap = JELLYFIN_SIGNAL_MAX_ITEMS,
                    "Jellyfin played history exceeded the per-participant cap; the read was truncated"
                );
                break;
            }
        }

        if skipped > 0 {
            warn!(
                connection_id = connection.id.as_str(),
                skipped, "skipped unreadable Jellyfin played items"
            );
        }

        self.attach_series_ids(&base, api_key, external_user_id, &mut items)
            .await;
        Ok(items)
    }

    /// Resolve the external ids of every distinct series the episodes belong
    /// to, in batches, and attach them.
    ///
    /// Failure here is not fatal: episodes simply stay unmapped, which is the
    /// honest outcome. That is why this returns `()` rather than a result.
    async fn attach_series_ids(
        &self,
        base: &Url,
        api_key: &str,
        external_user_id: &str,
        items: &mut [ProviderPlayedItem],
    ) {
        let series_ids = items
            .iter()
            .filter(|item| item.kind == MediaServerSignalKind::Episode)
            .filter_map(|item| item.series_provider_item_id.clone())
            .collect::<BTreeSet<_>>();
        if series_ids.is_empty() {
            return;
        }

        let mut resolved: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
        let ordered = series_ids.into_iter().collect::<Vec<_>>();
        for chunk in ordered.chunks(JELLYFIN_SERIES_BATCH) {
            let url = match series_lookup_url(base, external_user_id, chunk) {
                Ok(url) => url,
                Err(error) => {
                    warn!(error = %error, "could not build the Jellyfin series lookup URL");
                    return;
                }
            };
            match self.fetch_json(url, api_key).await {
                Ok(body) => {
                    for entry in body
                        .get("Items")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default()
                    {
                        let Some(id) = string_field(&entry, "Id") else {
                            continue;
                        };
                        resolved.insert(id, external_ids(&entry));
                    }
                }
                Err(error) => {
                    warn!(error = %error, "could not resolve Jellyfin series ids; those episodes stay unmapped");
                    return;
                }
            }
        }

        for item in items.iter_mut() {
            let Some(series_id) = item.series_provider_item_id.as_deref() else {
                continue;
            };
            if let Some(ids) = resolved.get(series_id) {
                item.series_external_ids = ids.clone();
            }
        }
    }
}

#[async_trait]
impl MediaServerSignalSource for HttpMediaServerSignalSource {
    async fn fetch_played_items(
        &self,
        connection: &MediaServerConnection,
        external_user_id: &str,
    ) -> AppResult<Vec<ProviderPlayedItem>> {
        let api_key = connection
            .api_key
            .as_deref()
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .ok_or_else(|| AppError::Repository("no credential stored".into()))?;
        let external_user_id = external_user_id.trim();
        if external_user_id.is_empty() {
            return Err(AppError::Validation(
                "a media-server signal read needs a provider user id".into(),
            ));
        }

        match connection.provider {
            MediaServerProvider::Jellyfin => {
                self.fetch_jellyfin(connection, api_key, external_user_id)
                    .await
            }
            // Not `Ok(vec![])`: an unimplemented provider must surface as a
            // per-connection error, never as a person with no watch history.
            MediaServerProvider::Emby | MediaServerProvider::Plex => {
                Err(AppError::Repository(format!(
                    "{} watch signals are not supported yet",
                    connection.provider.as_str()
                )))
            }
        }
    }
}

// ── Normalization ───────────────────────────────────────────────────────────

/// Turn one page of Jellyfin items into normalized observations.
///
/// Returns `(items, skipped)`. Pure, so the whole contract is testable against
/// canned payloads without an HTTP server.
pub fn normalize_jellyfin_items(page: &[Value]) -> (Vec<ProviderPlayedItem>, usize) {
    let mut items = Vec::with_capacity(page.len());
    let mut skipped = 0;
    for entry in page {
        match normalize_jellyfin_item(entry) {
            Some(item) => items.push(item),
            None => skipped += 1,
        }
    }
    (items, skipped)
}

/// One item, or `None` when it cannot be read as a played movie or episode.
pub fn normalize_jellyfin_item(entry: &Value) -> Option<ProviderPlayedItem> {
    let provider_item_id = string_field(entry, "Id")?;
    let kind = match string_field(entry, "Type")?.as_str() {
        "Movie" => MediaServerSignalKind::Movie,
        "Episode" => MediaServerSignalKind::Episode,
        _ => return None,
    };

    let user_data = entry.get("UserData");
    // `Filters=IsPlayed` already narrowed the query, but the row records what
    // the server said rather than what the filter implied.
    let played = user_data
        .and_then(|data| data.get("Played"))
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let play_count = user_data
        .and_then(|data| data.get("PlayCount"))
        .and_then(integer_field)
        .unwrap_or(0)
        .max(0);
    let last_played_at = user_data
        .and_then(|data| data.get("LastPlayedDate"))
        .and_then(Value::as_str)
        .and_then(parse_timestamp);

    Some(ProviderPlayedItem {
        provider_item_id,
        kind,
        name: string_field(entry, "Name"),
        external_ids: external_ids(entry),
        // Filled in by the batched series lookup; an episode whose series never
        // resolves keeps an empty map and stays unmapped.
        series_external_ids: BTreeMap::new(),
        series_provider_item_id: string_field(entry, "SeriesId"),
        // Jellyfin names the season "ParentIndexNumber" on an episode.
        season_number: entry.get("ParentIndexNumber").and_then(integer_field),
        episode_number: entry.get("IndexNumber").and_then(integer_field),
        played,
        play_count,
        last_played_at,
    })
}

/// TMDB/TVDB/IMDb ids from a Jellyfin `ProviderIds` object, lowercased to the
/// source names Scryer stores in `title_external_ids`.
fn external_ids(entry: &Value) -> BTreeMap<String, String> {
    let mut ids = BTreeMap::new();
    let Some(object) = entry.get("ProviderIds").and_then(Value::as_object) else {
        return ids;
    };
    for (raw_key, raw_value) in object {
        let key = raw_key.trim().to_ascii_lowercase();
        let Some((_, source)) = PROVIDER_ID_KEYS
            .iter()
            .find(|(candidate, _)| key == *candidate)
        else {
            continue;
        };
        let value = match raw_value {
            Value::String(text) => text.trim().to_string(),
            Value::Number(number) => number.to_string(),
            _ => continue,
        };
        if !value.is_empty() {
            ids.insert((*source).to_string(), value);
        }
    }
    ids
}

/// A JSON field that may be a number or a numeric string, as Jellyfin's
/// serializers have produced both.
fn integer_field(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_f64().map(|number| number as i64))
        .or_else(|| value.as_str()?.trim().parse::<i64>().ok())
}

fn string_field(entry: &Value, field: &str) -> Option<String> {
    let value = entry.get(field)?.as_str()?.trim();
    (!value.is_empty()).then(|| value.to_string())
}

/// Jellyfin timestamps are ISO-8601, sometimes without a zone marker. A value
/// that cannot be read becomes `None` rather than "now": a wrong last-played
/// time would make stale data look fresh.
fn parse_timestamp(raw: &str) -> Option<DateTime<Utc>> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    if let Ok(parsed) = DateTime::parse_from_rfc3339(raw) {
        return Some(parsed.with_timezone(&Utc));
    }
    chrono::NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S%.f")
        .ok()
        .map(|naive| naive.and_utc())
}

// ── URLs ────────────────────────────────────────────────────────────────────

/// A stored base URL with a trailing slash, so `join` keeps its last path
/// segment. Mirrors the handling in [`crate::media_server_playback`].
fn jellyfin_base(base_url: &str) -> AppResult<Url> {
    let mut base = Url::parse(base_url.trim())
        .map_err(|_| AppError::Validation("media server base URL is invalid".into()))?;
    if !base.path().ends_with('/') {
        base.set_path(&format!("{}/", base.path()));
    }
    Ok(base)
}

fn played_items_url(base: &Url, external_user_id: &str, start_index: usize) -> AppResult<Url> {
    let mut url = base
        .join(&format!("Users/{external_user_id}/Items"))
        .map_err(|_| AppError::Validation("media server base URL is invalid".into()))?;
    url.query_pairs_mut()
        .append_pair("Recursive", "true")
        .append_pair("IsMissing", "false")
        .append_pair("Filters", "IsPlayed")
        .append_pair("IncludeItemTypes", "Movie,Episode")
        .append_pair("Fields", JELLYFIN_ITEM_FIELDS)
        .append_pair("EnableUserData", "true")
        .append_pair("StartIndex", &start_index.to_string())
        .append_pair("Limit", &JELLYFIN_SIGNAL_PAGE_SIZE.to_string());
    Ok(url)
}

fn series_lookup_url(base: &Url, external_user_id: &str, ids: &[String]) -> AppResult<Url> {
    let mut url = base
        .join(&format!("Users/{external_user_id}/Items"))
        .map_err(|_| AppError::Validation("media server base URL is invalid".into()))?;
    url.query_pairs_mut()
        .append_pair("Ids", &ids.join(","))
        .append_pair("Fields", "ProviderIds")
        .append_pair("Limit", &ids.len().to_string());
    Ok(url)
}

/// A short, credential-free classification of a transport failure. Never the
/// raw error: it can carry the request URL.
fn transport_reason(error: &reqwest::Error) -> String {
    if error.is_timeout() {
        "request timed out".into()
    } else if error.is_connect() {
        "connection failed".into()
    } else if error.is_decode() {
        "unreadable response".into()
    } else {
        "request failed".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A full Jellyfin movie row, as the server returns it.
    fn movie_item() -> Value {
        json!({
            "Id": "jf-movie-1",
            "Name": "Example Feature",
            "Type": "Movie",
            "ProviderIds": { "Tmdb": "603", "Imdb": "tt0133093" },
            "UserData": { "Played": true, "PlayCount": 3, "LastPlayedDate": "2026-08-30T21:14:05.0000000Z" }
        })
    }

    fn episode_item() -> Value {
        json!({
            "Id": "jf-episode-1",
            "Name": "Pilot",
            "Type": "Episode",
            "SeriesId": "jf-series-1",
            "SeriesName": "Example Show",
            "ParentIndexNumber": 2,
            "IndexNumber": 5,
            "ProviderIds": { "Tvdb": "999" },
            "UserData": { "Played": true, "PlayCount": 1, "LastPlayedDate": "2026-08-29T10:00:00Z" }
        })
    }

    #[test]
    fn normalizes_a_played_movie() {
        let item = normalize_jellyfin_item(&movie_item()).expect("movie normalizes");
        assert_eq!(item.provider_item_id, "jf-movie-1");
        assert_eq!(item.kind, MediaServerSignalKind::Movie);
        assert_eq!(
            item.external_ids.get("tmdb").map(String::as_str),
            Some("603")
        );
        assert_eq!(
            item.external_ids.get("imdb").map(String::as_str),
            Some("tt0133093")
        );
        assert!(item.played);
        assert_eq!(item.play_count, 3);
        assert!(item.last_played_at.is_some());
        assert!(item.series_provider_item_id.is_none());
    }

    #[test]
    fn normalizes_an_episode_with_its_coordinates() {
        let item = normalize_jellyfin_item(&episode_item()).expect("episode normalizes");
        assert_eq!(item.kind, MediaServerSignalKind::Episode);
        assert_eq!(item.season_number, Some(2));
        assert_eq!(item.episode_number, Some(5));
        assert_eq!(item.series_provider_item_id.as_deref(), Some("jf-series-1"));
        // Series ids are attached by the batched lookup, not by the item row.
        assert!(item.series_external_ids.is_empty());
    }

    #[test]
    fn skips_malformed_and_unsupported_items() {
        let page = vec![
            movie_item(),
            // No Id.
            json!({ "Name": "Nameless", "Type": "Movie" }),
            // A type this wave does not record.
            json!({ "Id": "jf-series-1", "Name": "Example Show", "Type": "Series" }),
            episode_item(),
        ];
        let (items, skipped) = normalize_jellyfin_items(&page);
        assert_eq!(items.len(), 2);
        assert_eq!(skipped, 2);
    }

    #[test]
    fn reads_provider_ids_case_insensitively_and_from_numbers() {
        let item = normalize_jellyfin_item(&json!({
            "Id": "jf-movie-2",
            "Type": "Movie",
            "ProviderIds": { "TMDB": 12345, "Unknown": "x" }
        }))
        .expect("normalizes");
        assert_eq!(
            item.external_ids.get("tmdb").map(String::as_str),
            Some("12345")
        );
        assert_eq!(item.external_ids.len(), 1);
    }

    #[test]
    fn missing_user_data_does_not_invent_a_last_played_time() {
        let item = normalize_jellyfin_item(&json!({
            "Id": "jf-movie-3",
            "Type": "Movie",
            "ProviderIds": { "Tmdb": "1" }
        }))
        .expect("normalizes");
        assert!(item.last_played_at.is_none());
        assert_eq!(item.play_count, 0);
        // The query filtered on IsPlayed, so an item with no UserData block is
        // still a played item.
        assert!(item.played);
    }

    #[test]
    fn parses_jellyfins_zoneless_timestamps() {
        assert!(parse_timestamp("2026-08-30T21:14:05.0000000").is_some());
        assert!(parse_timestamp("2026-08-30T21:14:05Z").is_some());
        assert!(parse_timestamp("not a date").is_none());
        assert!(parse_timestamp("   ").is_none());
    }

    #[test]
    fn played_items_url_carries_every_required_parameter() {
        let base = jellyfin_base("http://jellyfin.example/media").expect("base");
        let url = played_items_url(&base, "user-123", 500).expect("url");
        assert_eq!(url.path(), "/media/Users/user-123/Items");
        let query = url.query().expect("query");
        for expected in [
            "Recursive=true",
            "Filters=IsPlayed",
            "IncludeItemTypes=Movie%2CEpisode",
            "StartIndex=500",
            "Limit=500",
        ] {
            assert!(query.contains(expected), "missing {expected} in {query}");
        }
        assert!(query.contains("ProviderIds"));
        assert!(query.contains("UserData"));
    }

    #[tokio::test]
    async fn unsupported_providers_error_rather_than_report_no_history() {
        let source = HttpMediaServerSignalSource::new();
        let mut connection = connection_fixture();
        connection.provider = MediaServerProvider::Plex;
        let error = source
            .fetch_played_items(&connection, "user-123")
            .await
            .expect_err("plex is unsupported in this wave");
        assert!(format!("{error}").contains("not supported"));
    }

    #[tokio::test]
    async fn a_connection_without_a_credential_is_an_error() {
        let source = HttpMediaServerSignalSource::new();
        let mut connection = connection_fixture();
        connection.api_key = None;
        let error = source
            .fetch_played_items(&connection, "user-123")
            .await
            .expect_err("no credential");
        assert!(format!("{error}").contains("no credential"));
    }

    /// Two pages plus the batched series lookup, end to end over HTTP.
    ///
    /// The page size is 500, so the first page is padded to exactly that many
    /// items to prove the adapter keeps paging until a short page arrives, and
    /// that it advances `StartIndex` rather than re-reading page one forever.
    #[tokio::test]
    async fn pages_until_a_short_page_and_resolves_series_ids_in_one_batch() {
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;

        let mut first_page = (0..JELLYFIN_SIGNAL_PAGE_SIZE - 1)
            .map(|index| {
                json!({
                    "Id": format!("jf-filler-{index}"),
                    "Type": "Movie",
                    "ProviderIds": { "Tmdb": index.to_string() },
                    "UserData": { "Played": true, "PlayCount": 1 }
                })
            })
            .collect::<Vec<_>>();
        first_page.push(movie_item());

        Mock::given(method("GET"))
            .and(path("/Users/user-123/Items"))
            .and(query_param("StartIndex", "0"))
            .and(query_param("Filters", "IsPlayed"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "Items": first_page,
                "TotalRecordCount": JELLYFIN_SIGNAL_PAGE_SIZE + 1
            })))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/Users/user-123/Items"))
            .and(query_param(
                "StartIndex",
                JELLYFIN_SIGNAL_PAGE_SIZE.to_string(),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "Items": [episode_item()],
                "TotalRecordCount": JELLYFIN_SIGNAL_PAGE_SIZE + 1
            })))
            .mount(&server)
            .await;

        // The series lookup is the only request carrying `Ids`.
        Mock::given(method("GET"))
            .and(path("/Users/user-123/Items"))
            .and(query_param("Ids", "jf-series-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "Items": [{
                    "Id": "jf-series-1",
                    "Type": "Series",
                    "ProviderIds": { "Tvdb": "999" }
                }]
            })))
            .mount(&server)
            .await;

        let mut connection = connection_fixture();
        connection.base_url = server.uri();

        let items = HttpMediaServerSignalSource::new()
            .fetch_played_items(&connection, "user-123")
            .await
            .expect("two pages read");

        assert_eq!(items.len(), JELLYFIN_SIGNAL_PAGE_SIZE + 1);
        let episode = items
            .iter()
            .find(|item| item.kind == MediaServerSignalKind::Episode)
            .expect("the second page's episode");
        // The series ids arrived from the batched lookup, not from the episode.
        assert_eq!(
            episode.series_external_ids.get("tvdb").map(String::as_str),
            Some("999")
        );
        assert_eq!(episode.season_number, Some(2));
        assert_eq!(episode.episode_number, Some(5));
    }

    /// A page the server refuses fails the participant rather than reporting a
    /// short history, and the reason never carries the URL or the credential.
    #[tokio::test]
    async fn an_http_failure_is_an_error_with_a_credential_free_reason() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/Users/user-123/Items"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;

        let mut connection = connection_fixture();
        connection.base_url = server.uri();

        let error = HttpMediaServerSignalSource::new()
            .fetch_played_items(&connection, "user-123")
            .await
            .expect_err("401 fails the read");
        let message = format!("{error}");
        assert!(message.contains("status 401"), "{message}");
        assert!(!message.contains("api-key"), "{message}");
        assert!(!message.contains("user-123"), "{message}");
    }

    fn connection_fixture() -> MediaServerConnection {
        MediaServerConnection {
            id: "conn-1".into(),
            provider: MediaServerProvider::Jellyfin,
            display_name: "Example Jellyfin".into(),
            base_url: "http://jellyfin.example".into(),
            external_url: None,
            enabled: true,
            login_enabled: true,
            linking_enabled: true,
            auto_add_enabled: false,
            default_app_permissions: Default::default(),
            default_library_grants: Vec::new(),
            machine_id: None,
            api_key: Some("api-key".into()),
            emby_server_id: None,
            emby_connect_enabled: false,
            path_mappings: Vec::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }
}

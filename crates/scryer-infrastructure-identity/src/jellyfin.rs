//! Jellyfin wire details: the `MediaBrowser` authorization scheme and the
//! catalog scan. Emby has its own module; the two servers share no request
//! path so a change to one server's protocol never reaches the other.

use std::collections::HashSet;

use reqwest::Client;
use scryer_application::{AppError, AppResult, MediaServerCatalogItem, MediaServerCatalogItemKind};
use scryer_domain::ExternalId;
use serde_json::Value;
use url::Url;

const SCRYER_PRODUCT: &str = "Scryer";
const SCRYER_VERSION: &str = env!("CARGO_PKG_VERSION");
const CATALOG_PAGE_SIZE: usize = 500;
const CATALOG_FIELDS: &str = "ProviderIds,SeriesId,ParentIndexNumber,IndexNumber,IndexNumberEnd";

/// The device identity Scryer presents to one Jellyfin connection.
///
/// Stable per connection, so the session Jellyfin opens for a login is the
/// same one the follow-up API-key calls are attributed to.
pub(super) fn device_id(connection_id: &str) -> String {
    format!("SCRYER_{connection_id}")
}

/// Characters that survive a `MediaBrowser` parameter value unencoded: the
/// URL-unreserved set. Everything else — quotes, commas, `+`, spaces — is
/// percent-encoded, because the server unquotes each value and then runs it
/// through `WebUtility.UrlDecode`.
const AUTH_PARAMETER: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

fn auth_parameter(value: &str) -> String {
    percent_encoding::utf8_percent_encode(value, AUTH_PARAMETER).to_string()
}

/// Build an `Authorization: MediaBrowser …` header value.
///
/// Jellyfin 12 gates every legacy credential channel it used to accept — the
/// `X-Emby-Token`, `X-MediaBrowser-Token` and `X-Emby-Authorization` headers
/// and the `api_key` query parameter — behind `EnableLegacyAuthorization`,
/// which its own upgrade migration turns off (jellyfin/jellyfin#15559). This
/// header is what remains. `Token` is its only required parameter.
///
/// `device_id` is omitted when the caller has no connection to bind to: an
/// API key is not a device, and the server fills the field in from its own
/// identity when a key arrives without one.
pub(super) fn authorization(token: Option<&str>, device_id: Option<&str>) -> String {
    let mut parameters = Vec::with_capacity(5);
    if let Some(token) = token {
        parameters.push(format!("Token=\"{}\"", auth_parameter(token)));
    }
    parameters.push(format!("Client=\"{}\"", auth_parameter(SCRYER_PRODUCT)));
    parameters.push(format!("Device=\"{}\"", auth_parameter(SCRYER_PRODUCT)));
    if let Some(device_id) = device_id {
        parameters.push(format!("DeviceId=\"{}\"", auth_parameter(device_id)));
    }
    parameters.push(format!("Version=\"{}\"", auth_parameter(SCRYER_VERSION)));
    format!("MediaBrowser {}", parameters.join(", "))
}

/// The unauthenticated header a Jellyfin login presents: it names the client
/// and device, and carries no token because the login is what mints one.
pub(super) fn login_authorization(connection_id: &str) -> String {
    authorization(None, Some(&device_id(connection_id)))
}

/// Attach Scryer's credential to a Jellyfin request.
///
/// Both channels go on every request, so one code path serves every Jellyfin
/// generation without first probing its version: Jellyfin 12 reads
/// `Authorization` only, Jellyfin 10.x reads either. Sending both is
/// unambiguous rather than merely tolerated — the server reads the legacy
/// header solely when `Authorization` carried no token, so the two can never
/// disagree.
pub(super) trait JellyfinAuth {
    fn jellyfin_auth(self, token: &str, device_id: Option<&str>) -> Self;
}

impl JellyfinAuth for reqwest::RequestBuilder {
    fn jellyfin_auth(self, token: &str, device_id: Option<&str>) -> Self {
        self.header("Authorization", authorization(Some(token), device_id))
            .header("X-Emby-Token", token)
    }
}

fn base_url(value: &str) -> AppResult<Url> {
    let mut url = Url::parse(value.trim())
        .map_err(|error| AppError::Repository(format!("invalid Jellyfin URL: {error}")))?;
    if !url.path().ends_with('/') {
        url.set_path(&format!("{}/", url.path()));
    }
    Ok(url)
}

fn catalog_url(base_url: &Url, path: &str) -> AppResult<Url> {
    base_url
        .join(path)
        .map_err(|error| AppError::Repository(format!("invalid Jellyfin catalog URL: {error}")))
}

pub(super) async fn scan_catalog(
    client: &Client,
    base_url: &str,
    api_key: &str,
    recent_only: bool,
) -> AppResult<Vec<MediaServerCatalogItem>> {
    let base_url = self::base_url(base_url)?;
    let mut start = 0usize;
    let mut catalog = Vec::new();
    loop {
        let mut url = catalog_url(&base_url, "Items")?;
        url.query_pairs_mut()
            .append_pair("Recursive", "true")
            .append_pair("IncludeItemTypes", "Movie,Series,Episode")
            .append_pair("Fields", CATALOG_FIELDS)
            .append_pair("StartIndex", &start.to_string())
            .append_pair("Limit", &CATALOG_PAGE_SIZE.to_string());
        if recent_only {
            url.query_pairs_mut()
                .append_pair("SortBy", "DateCreated,DateLastContentAdded")
                .append_pair("SortOrder", "Descending");
        }
        let response = client
            .get(url)
            .header("Accept", "application/json")
            .jellyfin_auth(api_key, None)
            .send()
            .await
            .map_err(|error| {
                AppError::Repository(format!("Jellyfin catalog scan failed: {error}"))
            })?;
        if !response.status().is_success() {
            return Err(AppError::Repository(format!(
                "Jellyfin catalog scan failed with status {}",
                response.status()
            )));
        }
        let page = response.json::<Value>().await.map_err(|error| {
            AppError::Repository(format!("invalid Jellyfin catalog response: {error}"))
        })?;
        let items = page
            .get("Items")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let page_len = items.len();
        catalog.extend(items.iter().filter_map(catalog_item));
        start += page_len;
        let total = page
            .get("TotalRecordCount")
            .and_then(Value::as_u64)
            .unwrap_or(0) as usize;
        if recent_only || page_len < CATALOG_PAGE_SIZE || (total > 0 && start >= total) {
            return hydrate_parent_series(client, &base_url, api_key, catalog).await;
        }
    }
}

async fn hydrate_parent_series(
    client: &Client,
    base_url: &Url,
    api_key: &str,
    mut catalog: Vec<MediaServerCatalogItem>,
) -> AppResult<Vec<MediaServerCatalogItem>> {
    let mut known_series_ids = catalog
        .iter()
        .filter(|item| item.kind == MediaServerCatalogItemKind::Series)
        .map(|item| item.provider_item_id.clone())
        .collect::<HashSet<_>>();
    let missing_series_ids = catalog
        .iter()
        .filter(|item| item.kind == MediaServerCatalogItemKind::Episode)
        .filter_map(|item| item.series_provider_item_id.clone())
        .filter(|series_id| known_series_ids.insert(series_id.clone()))
        .collect::<Vec<_>>();

    for series_id in missing_series_ids {
        // Jellyfin 10.11 resolves a user before serving `GET /Items/{id}` and
        // answers an API-key caller with 400 "Guid can't be empty". `GET
        // /Items?Ids=` is the list endpoint the scan above already uses; it
        // explicitly supports API keys without a user, so the parent series
        // is fetched through it instead.
        let mut url = catalog_url(base_url, "Items")?;
        url.query_pairs_mut()
            .append_pair("Ids", &series_id)
            .append_pair("Fields", CATALOG_FIELDS);
        let response = client
            .get(url)
            .header("Accept", "application/json")
            .jellyfin_auth(api_key, None)
            .send()
            .await
            .map_err(|error| {
                AppError::Repository(format!("Jellyfin parent-series lookup failed: {error}"))
            })?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            continue;
        }
        if !response.status().is_success() {
            return Err(AppError::Repository(format!(
                "Jellyfin parent-series lookup failed with status {}",
                response.status()
            )));
        }
        let page = response.json::<Value>().await.map_err(|error| {
            AppError::Repository(format!("invalid Jellyfin parent-series response: {error}"))
        })?;
        // A series that no longer exists yields an empty page rather than a 404.
        let Some(series) = page
            .get("Items")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .and_then(catalog_item)
        else {
            continue;
        };
        if series.kind == MediaServerCatalogItemKind::Series {
            catalog.push(series);
        }
    }
    Ok(catalog)
}

/// Ask Jellyfin to re-read specific folders: `POST /Library/Media/Updated`
/// with one `MediaUpdateInfo` per path, the notification its own filesystem
/// watcher raises. Targeted, never a full library scan.
pub(super) async fn refresh_paths(
    client: &Client,
    base_url: &str,
    api_key: &str,
    paths: &[&str],
) -> AppResult<()> {
    let base_url = self::base_url(base_url)?;
    let url = base_url
        .join("Library/Media/Updated")
        .map_err(|error| AppError::Repository(format!("invalid Jellyfin refresh URL: {error}")))?;
    let body = serde_json::json!({
        "Updates": paths
            .iter()
            .map(|path| serde_json::json!({ "Path": path, "UpdateType": "Modified" }))
            .collect::<Vec<_>>(),
    });
    let response = client
        .post(url)
        .header("Accept", "application/json")
        .jellyfin_auth(api_key, None)
        .json(&body)
        .send()
        .await
        .map_err(|error| AppError::Repository(format!("Jellyfin refresh failed: {error}")))?;
    if !response.status().is_success() {
        return Err(AppError::Repository(format!(
            "Jellyfin refresh failed with status {}",
            response.status()
        )));
    }
    Ok(())
}

fn catalog_item(value: &Value) -> Option<MediaServerCatalogItem> {
    let kind = match value.get("Type")?.as_str()? {
        "Movie" => MediaServerCatalogItemKind::Movie,
        "Series" => MediaServerCatalogItemKind::Series,
        "Episode" => MediaServerCatalogItemKind::Episode,
        _ => return None,
    };
    Some(MediaServerCatalogItem {
        kind,
        provider_item_id: value.get("Id")?.as_str()?.trim().to_string(),
        external_ids: provider_ids(value.get("ProviderIds")),
        series_provider_item_id: value
            .get("SeriesId")
            .and_then(Value::as_str)
            .map(str::to_string),
        season_number: value
            .get("ParentIndexNumber")
            .and_then(Value::as_i64)
            .map(|value| value as i32),
        episode_number: value
            .get("IndexNumber")
            .and_then(Value::as_i64)
            .map(|value| value as i32),
        episode_number_end: value
            .get("IndexNumberEnd")
            .and_then(Value::as_i64)
            .map(|value| value as i32),
    })
}

fn provider_ids(value: Option<&Value>) -> Vec<ExternalId> {
    let Some(Value::Object(values)) = value else {
        return Vec::new();
    };
    values
        .iter()
        .filter_map(|(source, value)| {
            value.as_str().map(|value| ExternalId {
                source: source.clone(),
                value: value.to_string(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    fn test_client() -> Client {
        scryer_outbound_http::install_default_rustls_provider();
        Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("test HTTP client")
    }

    /// Match one exact `Authorization` value.
    ///
    /// wiremock's own `header` matcher reads a comma-separated value as a list
    /// of values, so a `MediaBrowser` header never compares equal through it.
    fn authorization_header(expected: String) -> impl Fn(&wiremock::Request) -> bool {
        move |request: &wiremock::Request| {
            request
                .headers
                .get("authorization")
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value == expected)
        }
    }

    #[test]
    fn authorization_pins_the_wire_format() {
        assert_eq!(
            authorization(Some("api-key"), Some("SCRYER_jellyfin-main")),
            format!(
                "MediaBrowser Token=\"api-key\", Client=\"Scryer\", Device=\"Scryer\", \
                 DeviceId=\"SCRYER_jellyfin-main\", Version=\"{SCRYER_VERSION}\""
            )
        );
        // A bare API key is not a device, so the parameter is left off rather
        // than filled with a placeholder.
        assert_eq!(
            authorization(Some("api-key"), None),
            format!(
                "MediaBrowser Token=\"api-key\", Client=\"Scryer\", Device=\"Scryer\", \
                 Version=\"{SCRYER_VERSION}\""
            )
        );
        // A login has no token yet: it is the call that mints one.
        assert_eq!(
            login_authorization("jellyfin-main"),
            format!(
                "MediaBrowser Client=\"Scryer\", Device=\"Scryer\", \
                 DeviceId=\"SCRYER_jellyfin-main\", Version=\"{SCRYER_VERSION}\""
            )
        );
    }

    #[test]
    fn authorization_encodes_values_that_would_break_the_parser() {
        // The server unquotes each value and then URL-decodes it, so a quote
        // or comma in a pasted key must not reach the wire as a delimiter,
        // and a `+` must not come back out as a space.
        let header = authorization(Some("a\"b,c d+e"), Some("SCRYER_x\"y"));
        assert!(
            header.contains("Token=\"a%22b%2Cc%20d%2Be\""),
            "token was not encoded: {header}"
        );
        assert!(
            header.contains("DeviceId=\"SCRYER_x%22y\""),
            "device id was not encoded: {header}"
        );
    }

    #[tokio::test]
    async fn catalog_scan_authorizes_with_the_media_browser_header_and_hydrates_series() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/Items"))
            .and(query_param("StartIndex", "0"))
            .and(authorization_header(authorization(Some("api-key"), None)))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "Items": [
                    {"Type": "Movie", "Id": "movie-1", "ProviderIds": {"Tmdb": "1001"}},
                    {"Type": "Episode", "Id": "episode-1", "SeriesId": "series-1",
                     "ParentIndexNumber": 2, "IndexNumber": 3, "IndexNumberEnd": 4}
                ],
                "TotalRecordCount": 2
            })))
            .expect(1)
            .mount(&server)
            .await;
        // Jellyfin 10.11 rejects `GET /Items/{id}` from an API-key caller with
        // 400, so the parent series must come back through the list endpoint's
        // `Ids=` filter.
        Mock::given(method("GET"))
            .and(path("/Items"))
            .and(query_param("Ids", "series-1"))
            .and(authorization_header(authorization(Some("api-key"), None)))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "Items": [
                    {"Type": "Series", "Id": "series-1", "ProviderIds": {"Tvdb": "2002"}}
                ],
                "TotalRecordCount": 1
            })))
            .expect(1)
            .mount(&server)
            .await;

        let catalog = scan_catalog(&test_client(), &server.uri(), "api-key", false)
            .await
            .expect("scan Jellyfin catalog");

        assert_eq!(catalog.len(), 3);
        assert_eq!(catalog[0].kind, MediaServerCatalogItemKind::Movie);
        assert_eq!(catalog[0].external_ids[0].source, "Tmdb");
        assert_eq!(catalog[1].kind, MediaServerCatalogItemKind::Episode);
        assert_eq!(
            catalog[1].series_provider_item_id.as_deref(),
            Some("series-1")
        );
        assert_eq!(catalog[1].season_number, Some(2));
        assert_eq!(catalog[1].episode_number, Some(3));
        assert_eq!(catalog[1].episode_number_end, Some(4));
        assert_eq!(catalog[2].kind, MediaServerCatalogItemKind::Series);
        assert_eq!(catalog[2].provider_item_id, "series-1");
    }

    #[tokio::test]
    async fn refresh_posts_the_changed_paths_with_the_media_browser_header() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/Library/Media/Updated"))
            .and(authorization_header(authorization(Some("api-key"), None)))
            .and(wiremock::matchers::body_json(json!({
                "Updates": [
                    { "Path": "/data/tv/Some Show", "UpdateType": "Modified" },
                    { "Path": "/data/tv/Other Show", "UpdateType": "Modified" },
                ]
            })))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        refresh_paths(
            &test_client(),
            &server.uri(),
            "api-key",
            &["/data/tv/Some Show", "/data/tv/Other Show"],
        )
        .await
        .expect("refresh");
    }

    #[tokio::test]
    async fn refresh_surfaces_a_rejected_notification() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/Library/Media/Updated"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;

        let error = refresh_paths(
            &test_client(),
            &server.uri(),
            "api-key",
            &["/data/tv/Some Show"],
        )
        .await
        .expect_err("a rejected notification is not a success");
        assert!(error.to_string().contains("401"), "{error}");
    }
}

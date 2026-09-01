//! Live playback observation across configured media servers (RFC 137 §9.10).
//!
//! One [`MediaServerPlaybackProbe`] implementation that fans out over every
//! **enabled** media-server connection and asks each one what it is streaming
//! right now. These are the cheap server-local session endpoints the RFC names:
//! Plex `/status/sessions`, Jellyfin and Emby `/Sessions`.
//!
//! Two rules shape everything here:
//!
//! * **No connection may take down the probe.** A server that is offline,
//!   mis-credentialed, slow, or returning garbage becomes
//!   [`PlaybackProbeStatus::Unreachable`] for that connection alone; the others
//!   still report what they truthfully observed. The probe returns `Err` only
//!   when the connection list itself cannot be read.
//! * **Reasons are terse and credential-free.** An `Unreachable` reason is a
//!   short classification ("request timed out", "status 401"), never a raw
//!   error string that could carry a URL with an embedded token.
//!
//! Nothing here writes anything: no store, no cache, no migration.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use scryer_application::{
    AppResult, ConnectionPlaybackActivity, MediaServerConnectionRepository,
    MediaServerPlaybackProbe, PlaybackActivitySnapshot, PlaybackProbeStatus,
};
use scryer_domain::{MediaServerConnection, MediaServerProvider};
use scryer_outbound_http::generic_reqwest_client;
use serde_json::Value;
use url::Url;

/// Per-connection budget for a session read.
///
/// Deliberately far below [`scryer_outbound_http::STANDARD_HTTP_TIMEOUT`]: this
/// is a LAN call to a server that answers from memory, and it sits in front of
/// a maintenance action. A slow server should be reported as unknown quickly,
/// not block the sweep for a minute.
const PLAYBACK_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Plex's own service host. A Plex connection whose base URL is still this has
/// no selected server, so there is no session endpoint to ask.
const PLEX_SERVICE_HOST: &str = "plex.tv";

pub struct HttpMediaServerPlaybackProbe {
    connections: Arc<dyn MediaServerConnectionRepository>,
    client: reqwest::Client,
}

impl HttpMediaServerPlaybackProbe {
    pub fn new(connections: Arc<dyn MediaServerConnectionRepository>) -> Self {
        Self {
            connections,
            client: generic_reqwest_client(),
        }
    }
}

#[async_trait]
impl MediaServerPlaybackProbe for HttpMediaServerPlaybackProbe {
    async fn active_playback(&self) -> AppResult<PlaybackActivitySnapshot> {
        // An error here is the one thing that fails the whole probe: without
        // the connection list there is nothing to fan out over, and reporting
        // "no connections" would read as Clear.
        let connections = self.connections.list(None).await?;
        let enabled = connections
            .into_iter()
            .filter(|connection| connection.enabled)
            .collect::<Vec<_>>();

        let mut activity = Vec::with_capacity(enabled.len());
        for connection in &enabled {
            activity.push(ConnectionPlaybackActivity {
                connection_id: connection.id.clone(),
                provider: connection.provider.clone(),
                status: probe_connection(&self.client, connection).await,
            });
        }

        Ok(PlaybackActivitySnapshot {
            connections: activity,
            observed_at: Utc::now(),
        })
    }
}

/// Asks one connection what it is streaming. Never returns an error: every
/// failure mode is an `Unreachable` status for this connection.
pub(crate) async fn probe_connection(
    client: &reqwest::Client,
    connection: &MediaServerConnection,
) -> PlaybackProbeStatus {
    let Some(api_key) = connection
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty())
    else {
        return PlaybackProbeStatus::Unreachable("no credential stored".into());
    };

    match connection.provider {
        MediaServerProvider::Plex => probe_plex(client, &connection.base_url, api_key).await,
        MediaServerProvider::Jellyfin | MediaServerProvider::Emby => {
            probe_emby_family(client, &connection.base_url, api_key).await
        }
    }
}

/// Plex: `GET /status/sessions` with the stored token. Active when the
/// container reports a non-zero size.
async fn probe_plex(client: &reqwest::Client, base_url: &str, token: &str) -> PlaybackProbeStatus {
    let url = match session_url(base_url, "status/sessions") {
        Ok(url) => url,
        Err(reason) => return PlaybackProbeStatus::Unreachable(reason),
    };
    if url
        .host_str()
        .is_some_and(|host| host.eq_ignore_ascii_case(PLEX_SERVICE_HOST))
    {
        // plex.tv is the account service, not a media server: this connection
        // has no server selected, so its sessions cannot be observed.
        return PlaybackProbeStatus::Unreachable("no Plex server selected".into());
    }

    let body = match fetch_json(client, url, "X-Plex-Token", token).await {
        Ok(body) => body,
        Err(status) => return status,
    };

    // Plex answers `{"MediaContainer": {"size": N, "Metadata": [...]}}`. The
    // size field is the count; `Metadata` is absent entirely when idle.
    let container = match body.get("MediaContainer") {
        Some(container) => container,
        None => return PlaybackProbeStatus::Unreachable("unreadable response".into()),
    };
    let size = container.get("size").and_then(playback_count).or_else(|| {
        container
            .get("Metadata")
            .and_then(Value::as_array)
            .map(|sessions| sessions.len() as u32)
    });
    match size {
        Some(0) => PlaybackProbeStatus::Idle,
        Some(count) => PlaybackProbeStatus::ActiveSessions(count),
        None => PlaybackProbeStatus::Unreachable("unreadable response".into()),
    }
}

/// Jellyfin and Emby: `GET /Sessions` with the stored API key. A session counts
/// as active only when it carries a `NowPlayingItem`; idle clients stay
/// connected and are listed regardless.
async fn probe_emby_family(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
) -> PlaybackProbeStatus {
    let url = match session_url(base_url, "Sessions") {
        Ok(url) => url,
        Err(reason) => return PlaybackProbeStatus::Unreachable(reason),
    };
    let body = match fetch_json(client, url, "X-Emby-Token", api_key).await {
        Ok(body) => body,
        Err(status) => return status,
    };

    let Some(sessions) = body.as_array() else {
        return PlaybackProbeStatus::Unreachable("unreadable response".into());
    };
    let active = sessions
        .iter()
        .filter(|session| {
            session
                .get("NowPlayingItem")
                .is_some_and(|item| !item.is_null())
        })
        .count() as u32;
    if active == 0 {
        PlaybackProbeStatus::Idle
    } else {
        PlaybackProbeStatus::ActiveSessions(active)
    }
}

/// Sends the session request and decodes its body, mapping every failure to an
/// `Unreachable` status carrying a terse, credential-free reason.
async fn fetch_json(
    client: &reqwest::Client,
    url: Url,
    credential_header: &str,
    credential: &str,
) -> Result<Value, PlaybackProbeStatus> {
    let response = client
        .get(url)
        .header("Accept", "application/json")
        .header(credential_header, credential)
        .timeout(PLAYBACK_PROBE_TIMEOUT)
        .send()
        .await
        .map_err(|error| PlaybackProbeStatus::Unreachable(transport_reason(&error)))?;
    let status = response.status();
    if !status.is_success() {
        return Err(PlaybackProbeStatus::Unreachable(format!(
            "status {}",
            status.as_u16()
        )));
    }
    response
        .json::<Value>()
        .await
        .map_err(|_| PlaybackProbeStatus::Unreachable("unreadable response".into()))
}

/// The session endpoint for a stored base URL. Mirrors the media-server URL
/// handling in [`crate::external_identity`]: a base URL without a trailing
/// slash would otherwise drop its last path segment on `join`.
fn session_url(base_url: &str, path: &str) -> Result<Url, String> {
    let mut base = Url::parse(base_url.trim()).map_err(|_| "invalid base URL".to_string())?;
    if !base.path().ends_with('/') {
        base.set_path(&format!("{}/", base.path()));
    }
    base.join(path).map_err(|_| "invalid base URL".to_string())
}

/// A session count as reported by a server that may answer with a number or a
/// numeric string (Plex's XML-derived JSON does the latter on some versions).
fn playback_count(value: &Value) -> Option<u32> {
    value
        .as_u64()
        .or_else(|| value.as_str()?.trim().parse::<u64>().ok())
        .map(|count| u32::try_from(count).unwrap_or(u32::MAX))
}

/// A short classification of a transport failure. Never the raw error: it can
/// carry the request URL.
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
    use chrono::Utc;
    use scryer_application::{AppError, MediaServerPlaybackProbe};
    use scryer_domain::{AppPermissionMask, MediaServerConnection};
    use serde_json::json;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn connection(
        id: &str,
        provider: MediaServerProvider,
        base_url: &str,
        api_key: Option<&str>,
    ) -> MediaServerConnection {
        let now = Utc::now();
        MediaServerConnection {
            id: id.to_string(),
            provider,
            display_name: id.to_string(),
            base_url: base_url.to_string(),
            external_url: None,
            enabled: true,
            login_enabled: false,
            linking_enabled: false,
            auto_add_enabled: false,
            default_app_permissions: AppPermissionMask::NONE,
            default_library_grants: Vec::new(),
            machine_id: None,
            api_key: api_key.map(str::to_string),
            emby_server_id: None,
            emby_connect_enabled: false,
            path_mappings: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }

    /// Connection repository stub: the probe only ever calls `list`.
    struct StubConnections {
        connections: Vec<MediaServerConnection>,
        fail: bool,
    }

    impl StubConnections {
        fn new(connections: Vec<MediaServerConnection>) -> Self {
            Self {
                connections,
                fail: false,
            }
        }

        fn failing() -> Self {
            Self {
                connections: Vec::new(),
                fail: true,
            }
        }
    }

    #[async_trait]
    impl MediaServerConnectionRepository for StubConnections {
        async fn list(
            &self,
            _: Option<MediaServerProvider>,
        ) -> AppResult<Vec<MediaServerConnection>> {
            if self.fail {
                return Err(AppError::Repository("datastore offline".into()));
            }
            Ok(self.connections.clone())
        }

        async fn get_by_id(&self, _: &str) -> AppResult<Option<MediaServerConnection>> {
            Ok(None)
        }

        async fn create(&self, _: MediaServerConnection) -> AppResult<MediaServerConnection> {
            unreachable!("the playback probe never writes")
        }

        async fn update(&self, _: MediaServerConnection) -> AppResult<MediaServerConnection> {
            unreachable!("the playback probe never writes")
        }

        async fn list_playback_items_for_entity(
            &self,
            _: scryer_domain::MediaServerPlaybackEntityKind,
            _: &str,
        ) -> AppResult<Vec<scryer_domain::MediaServerPlaybackItem>> {
            Ok(Vec::new())
        }

        async fn replace_playback_items_for_connection(
            &self,
            _: &str,
            _: Vec<scryer_domain::MediaServerPlaybackItem>,
        ) -> AppResult<()> {
            unreachable!("the playback probe never writes")
        }

        async fn delete(&self, _: &str) -> AppResult<()> {
            unreachable!("the playback probe never writes")
        }

        async fn has_external_accounts(&self, _: &str) -> AppResult<bool> {
            Ok(false)
        }

        async fn has_notification_channels(&self, _: &str) -> AppResult<bool> {
            Ok(false)
        }
    }

    async fn mount_plex_sessions(server: &MockServer, response: ResponseTemplate) {
        Mock::given(method("GET"))
            .and(path("/status/sessions"))
            .and(header("x-plex-token", "plex-token"))
            .respond_with(response)
            .mount(server)
            .await;
    }

    async fn mount_emby_sessions(server: &MockServer, response: ResponseTemplate) {
        Mock::given(method("GET"))
            .and(path("/Sessions"))
            .and(header("x-emby-token", "api-key"))
            .respond_with(response)
            .mount(server)
            .await;
    }

    // ── Plex ────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn plex_reports_active_sessions_from_the_container_size() {
        let server = MockServer::start().await;
        mount_plex_sessions(
            &server,
            ResponseTemplate::new(200).set_body_json(json!({
                "MediaContainer": {
                    "size": 2,
                    "Metadata": [{"title": "A"}, {"title": "B"}]
                }
            })),
        )
        .await;

        let status = probe_connection(
            &generic_reqwest_client(),
            &connection(
                "plex",
                MediaServerProvider::Plex,
                &server.uri(),
                Some("plex-token"),
            ),
        )
        .await;

        assert_eq!(status, PlaybackProbeStatus::ActiveSessions(2));
    }

    #[tokio::test]
    async fn plex_reports_idle_when_the_container_is_empty() {
        let server = MockServer::start().await;
        mount_plex_sessions(
            &server,
            ResponseTemplate::new(200).set_body_json(json!({"MediaContainer": {"size": 0}})),
        )
        .await;

        let status = probe_connection(
            &generic_reqwest_client(),
            &connection(
                "plex",
                MediaServerProvider::Plex,
                &server.uri(),
                Some("plex-token"),
            ),
        )
        .await;

        assert_eq!(status, PlaybackProbeStatus::Idle);
    }

    #[tokio::test]
    async fn plex_accepts_a_string_size() {
        let server = MockServer::start().await;
        mount_plex_sessions(
            &server,
            ResponseTemplate::new(200).set_body_json(json!({"MediaContainer": {"size": "1"}})),
        )
        .await;

        let status = probe_connection(
            &generic_reqwest_client(),
            &connection(
                "plex",
                MediaServerProvider::Plex,
                &server.uri(),
                Some("plex-token"),
            ),
        )
        .await;

        assert_eq!(status, PlaybackProbeStatus::ActiveSessions(1));
    }

    #[tokio::test]
    async fn plex_auth_failure_is_unreachable_not_idle() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/status/sessions"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;

        let status = probe_connection(
            &generic_reqwest_client(),
            &connection(
                "plex",
                MediaServerProvider::Plex,
                &server.uri(),
                Some("plex-token"),
            ),
        )
        .await;

        assert_eq!(
            status,
            PlaybackProbeStatus::Unreachable("status 401".to_string())
        );
    }

    #[tokio::test]
    async fn plex_malformed_body_is_unreachable() {
        let server = MockServer::start().await;
        mount_plex_sessions(
            &server,
            ResponseTemplate::new(200).set_body_string("<MediaContainer size=\"1\"/>"),
        )
        .await;

        let status = probe_connection(
            &generic_reqwest_client(),
            &connection(
                "plex",
                MediaServerProvider::Plex,
                &server.uri(),
                Some("plex-token"),
            ),
        )
        .await;

        assert_eq!(
            status,
            PlaybackProbeStatus::Unreachable("unreadable response".to_string())
        );
    }

    #[tokio::test]
    async fn plex_json_without_a_container_is_unreachable() {
        let server = MockServer::start().await;
        mount_plex_sessions(&server, ResponseTemplate::new(200).set_body_json(json!({}))).await;

        let status = probe_connection(
            &generic_reqwest_client(),
            &connection(
                "plex",
                MediaServerProvider::Plex,
                &server.uri(),
                Some("plex-token"),
            ),
        )
        .await;

        assert_eq!(
            status,
            PlaybackProbeStatus::Unreachable("unreadable response".to_string())
        );
    }

    #[tokio::test]
    async fn a_plex_connection_without_a_selected_server_is_unreachable() {
        let status = probe_connection(
            &generic_reqwest_client(),
            &connection(
                "plex",
                MediaServerProvider::Plex,
                "https://plex.tv",
                Some("plex-token"),
            ),
        )
        .await;

        assert_eq!(
            status,
            PlaybackProbeStatus::Unreachable("no Plex server selected".to_string())
        );
    }

    // ── Jellyfin / Emby ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn jellyfin_counts_only_sessions_with_a_now_playing_item() {
        let server = MockServer::start().await;
        mount_emby_sessions(
            &server,
            ResponseTemplate::new(200).set_body_json(json!([
                {"Id": "idle-client"},
                {"Id": "watching", "NowPlayingItem": {"Id": "item-1"}},
                {"Id": "null-item", "NowPlayingItem": null},
                {"Id": "also-watching", "NowPlayingItem": {"Id": "item-2"}}
            ])),
        )
        .await;

        let status = probe_connection(
            &generic_reqwest_client(),
            &connection(
                "jellyfin",
                MediaServerProvider::Jellyfin,
                &server.uri(),
                Some("api-key"),
            ),
        )
        .await;

        assert_eq!(status, PlaybackProbeStatus::ActiveSessions(2));
    }

    #[tokio::test]
    async fn jellyfin_connected_but_idle_clients_are_idle() {
        let server = MockServer::start().await;
        mount_emby_sessions(
            &server,
            ResponseTemplate::new(200).set_body_json(json!([{"Id": "idle-1"}, {"Id": "idle-2"}])),
        )
        .await;

        let status = probe_connection(
            &generic_reqwest_client(),
            &connection(
                "jellyfin",
                MediaServerProvider::Jellyfin,
                &server.uri(),
                Some("api-key"),
            ),
        )
        .await;

        assert_eq!(status, PlaybackProbeStatus::Idle);
    }

    #[tokio::test]
    async fn emby_uses_the_same_session_endpoint() {
        let server = MockServer::start().await;
        mount_emby_sessions(
            &server,
            ResponseTemplate::new(200)
                .set_body_json(json!([{"Id": "watching", "NowPlayingItem": {"Id": "x"}}])),
        )
        .await;

        let status = probe_connection(
            &generic_reqwest_client(),
            &connection(
                "emby",
                MediaServerProvider::Emby,
                &server.uri(),
                Some("api-key"),
            ),
        )
        .await;

        assert_eq!(status, PlaybackProbeStatus::ActiveSessions(1));
    }

    #[tokio::test]
    async fn emby_family_auth_failure_is_unreachable() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/Sessions"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&server)
            .await;

        let status = probe_connection(
            &generic_reqwest_client(),
            &connection(
                "emby",
                MediaServerProvider::Emby,
                &server.uri(),
                Some("api-key"),
            ),
        )
        .await;

        assert_eq!(
            status,
            PlaybackProbeStatus::Unreachable("status 403".to_string())
        );
    }

    #[tokio::test]
    async fn emby_family_malformed_body_is_unreachable() {
        let server = MockServer::start().await;
        mount_emby_sessions(
            &server,
            ResponseTemplate::new(200).set_body_json(json!({"Items": []})),
        )
        .await;

        let status = probe_connection(
            &generic_reqwest_client(),
            &connection(
                "jellyfin",
                MediaServerProvider::Jellyfin,
                &server.uri(),
                Some("api-key"),
            ),
        )
        .await;

        assert_eq!(
            status,
            PlaybackProbeStatus::Unreachable("unreadable response".to_string())
        );
    }

    #[tokio::test]
    async fn a_slow_server_times_out_as_unreachable() {
        let server = MockServer::start().await;
        mount_emby_sessions(
            &server,
            ResponseTemplate::new(200)
                .set_body_json(json!([]))
                .set_delay(PLAYBACK_PROBE_TIMEOUT + Duration::from_secs(2)),
        )
        .await;

        let status = probe_connection(
            &generic_reqwest_client(),
            &connection(
                "jellyfin",
                MediaServerProvider::Jellyfin,
                &server.uri(),
                Some("api-key"),
            ),
        )
        .await;

        assert_eq!(
            status,
            PlaybackProbeStatus::Unreachable("request timed out".to_string())
        );
    }

    #[tokio::test]
    async fn a_dead_server_is_unreachable() {
        // Bound and immediately dropped: the port has no listener.
        let dead_uri = {
            let server = MockServer::start().await;
            server.uri()
        };

        let status = probe_connection(
            &generic_reqwest_client(),
            &connection(
                "jellyfin",
                MediaServerProvider::Jellyfin,
                &dead_uri,
                Some("api-key"),
            ),
        )
        .await;

        assert!(
            matches!(status, PlaybackProbeStatus::Unreachable(_)),
            "expected unreachable, got {status:?}"
        );
    }

    #[tokio::test]
    async fn a_connection_without_a_credential_is_unreachable() {
        let status = probe_connection(
            &generic_reqwest_client(),
            &connection(
                "jellyfin",
                MediaServerProvider::Jellyfin,
                "http://127.0.0.1:1",
                None,
            ),
        )
        .await;

        assert_eq!(
            status,
            PlaybackProbeStatus::Unreachable("no credential stored".to_string())
        );
    }

    #[tokio::test]
    async fn an_invalid_base_url_is_unreachable() {
        let status = probe_connection(
            &generic_reqwest_client(),
            &connection(
                "jellyfin",
                MediaServerProvider::Jellyfin,
                "not a url",
                Some("api-key"),
            ),
        )
        .await;

        assert_eq!(
            status,
            PlaybackProbeStatus::Unreachable("invalid base URL".to_string())
        );
    }

    // ── Fan-out ─────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn no_configured_connections_yields_an_empty_snapshot() {
        let probe = HttpMediaServerPlaybackProbe::new(Arc::new(StubConnections::new(Vec::new())));

        let snapshot = probe.active_playback().await.expect("probe");

        assert!(snapshot.connections.is_empty());
    }

    #[tokio::test]
    async fn disabled_connections_are_never_asked() {
        let mut disabled = connection(
            "plex",
            MediaServerProvider::Plex,
            "http://127.0.0.1:1",
            Some("plex-token"),
        );
        disabled.enabled = false;
        let probe =
            HttpMediaServerPlaybackProbe::new(Arc::new(StubConnections::new(vec![disabled])));

        let snapshot = probe.active_playback().await.expect("probe");

        assert!(
            snapshot.connections.is_empty(),
            "a disabled connection must not appear as unreachable: {:?}",
            snapshot.connections
        );
    }

    #[tokio::test]
    async fn multiple_connections_report_independently() {
        let active = MockServer::start().await;
        mount_emby_sessions(
            &active,
            ResponseTemplate::new(200)
                .set_body_json(json!([{"Id": "watching", "NowPlayingItem": {"Id": "x"}}])),
        )
        .await;
        let idle = MockServer::start().await;
        mount_plex_sessions(
            &idle,
            ResponseTemplate::new(200).set_body_json(json!({"MediaContainer": {"size": 0}})),
        )
        .await;
        let broken = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/Sessions"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&broken)
            .await;

        let probe = HttpMediaServerPlaybackProbe::new(Arc::new(StubConnections::new(vec![
            connection(
                "jellyfin",
                MediaServerProvider::Jellyfin,
                &active.uri(),
                Some("api-key"),
            ),
            connection(
                "plex",
                MediaServerProvider::Plex,
                &idle.uri(),
                Some("plex-token"),
            ),
            connection(
                "emby",
                MediaServerProvider::Emby,
                &broken.uri(),
                Some("api-key"),
            ),
        ])));

        let snapshot = probe.active_playback().await.expect("probe");

        assert_eq!(snapshot.connections.len(), 3);
        assert_eq!(
            snapshot.connections[0].status,
            PlaybackProbeStatus::ActiveSessions(1)
        );
        assert_eq!(snapshot.connections[1].status, PlaybackProbeStatus::Idle);
        assert_eq!(
            snapshot.connections[2].status,
            PlaybackProbeStatus::Unreachable("status 500".to_string())
        );
        // Identity is preserved so the fold can name the connection it held on.
        assert_eq!(snapshot.connections[2].connection_id, "emby");
        assert_eq!(snapshot.connections[2].provider, MediaServerProvider::Emby);
    }

    #[tokio::test]
    async fn an_unreadable_connection_list_fails_the_whole_probe() {
        let probe = HttpMediaServerPlaybackProbe::new(Arc::new(StubConnections::failing()));

        // Reporting "no connections" here would read as Clear, so this is the
        // one failure the probe must surface as an error.
        assert!(probe.active_playback().await.is_err());
    }
}

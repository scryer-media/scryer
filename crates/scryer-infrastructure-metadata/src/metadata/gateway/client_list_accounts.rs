//! OAuth is an explicit outbound-policy exception: token POSTs are single-shot,
//! reject redirects, have bounded bodies and deadlines, and never log payloads.
use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use reqwest::{Client, RequestBuilder, StatusCode};
use scryer_application::lists::account_transport::{
    ListAccountAuthGateway, ListAccountCompleteRequest, ListAccountStartRequest,
    ListAccountStartResponse, ListProviderAppConfig, PROVIDER_REJECTED, RECONNECT_REQUIRED,
    rate_limited_failure,
};
use scryer_application::{AppError, AppResult};
use scryer_domain::ListAccountCredential;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use url::Url;

use super::{MetadataGatewayClient, apply_instance_auth_headers};

const RELAY_ORIGIN: &str = "https://smg.scryer.media";
const RESPONSE_LIMIT: usize = 64 * 1024;
const REQUEST_LIMIT: usize = 16 * 1024;
/// Deadline for starts, polls, revokes and every other short call.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
/// Deadline for a direct provider token exchange or renew.
const OAUTH_TIMEOUT: Duration = Duration::from_secs(100);
/// The relay waits up to 90 s on the provider itself, so Scryer waits a
/// little longer on a relayed exchange or renew to receive the relay's own
/// answer rather than its silence.
const RELAY_TIMEOUT: Duration = Duration::from_secs(110);
/// Upper bound on a provider's requested pause, so a hostile or broken
/// `Retry-After` cannot park an account indefinitely.
const RETRY_AFTER_CAP: Duration = Duration::from_secs(24 * 60 * 60);

/// Uses the existing instance enrollment for relay requests. Provider URLs are
/// protocol constants; operator configuration supplies app credentials only.
pub struct HttpListAccountAuthGateway {
    gateway: Arc<MetadataGatewayClient>,
    http: Client,
    inflight: tokio::sync::Semaphore,
    endpoints: Endpoints,
    plex_client_identifier: String,
}

struct Endpoints {
    relay: String,
    plex: String,
    tmdb: String,
    trakt: String,
    anilist: String,
    mal: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            relay: RELAY_ORIGIN.into(),
            plex: "https://plex.tv/api/v2".into(),
            tmdb: "https://api.themoviedb.org/4".into(),
            trakt: "https://auth.trakt.tv/oauth/token".into(),
            anilist: "https://anilist.co/api/v2/oauth/token".into(),
            mal: "https://myanimelist.net/v1/oauth2/token".into(),
        }
    }
}

impl HttpListAccountAuthGateway {
    pub fn new(gateway: Arc<MetadataGatewayClient>) -> AppResult<Self> {
        Self::new_with_client_identifier(gateway, uuid::Uuid::new_v4().to_string())
    }

    pub fn new_with_client_identifier(
        gateway: Arc<MetadataGatewayClient>,
        client_identifier: String,
    ) -> AppResult<Self> {
        require_value(&client_identifier, 512)?;
        let http = Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .connect_timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .user_agent(concat!("Scryer/list-oauth/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| failure("transport_unavailable"))?;
        Ok(Self {
            gateway,
            http,
            inflight: tokio::sync::Semaphore::new(16),
            endpoints: Endpoints::default(),
            plex_client_identifier: client_identifier,
        })
    }

    async fn send(
        &self,
        request: RequestBuilder,
        provider: &str,
        operation: &str,
    ) -> AppResult<Reply> {
        let _permit = self.inflight.try_acquire().map_err(|_| failure("busy"))?;
        let request = request
            .header("Accept", "application/json")
            .build()
            .map_err(|_| failure("invalid_request"))?;
        if request
            .body()
            .and_then(|body| body.as_bytes())
            .is_some_and(|body| body.len() > REQUEST_LIMIT)
        {
            return Err(failure("invalid_request"));
        }
        let mut response = self
            .http
            .execute(request)
            .await
            .map_err(|_| failure("transport_unavailable"))?;
        let status = response.status();
        let retry_after = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(scryer_outbound_http::parse_retry_after)
            .map(|(delay, _)| delay.min(RETRY_AFTER_CAP));
        // Bounded server-owned labels only: no URL, state, body or credentials.
        tracing::debug!(
            provider,
            operation,
            status = status.as_u16(),
            "list OAuth response"
        );
        if response
            .content_length()
            .is_some_and(|len| len > RESPONSE_LIMIT as u64)
        {
            return Err(failure("invalid_provider_response"));
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| failure("transport_unavailable"))?
        {
            if chunk.len() > RESPONSE_LIMIT.saturating_sub(body.len()) {
                return Err(failure("invalid_provider_response"));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(Reply {
            status,
            body,
            retry_after,
        })
    }

    async fn json<T: DeserializeOwned>(
        &self,
        request: RequestBuilder,
        provider: &str,
        operation: &str,
        source: Source,
    ) -> AppResult<T> {
        let reply = self.send(request, provider, operation).await?;
        ensure_success(&reply, source)?;
        serde_json::from_slice(&reply.body).map_err(|_| failure("invalid_provider_response"))
    }

    async fn relay(&self, provider: &str, operation: &str, body: Value) -> AppResult<Reply> {
        if !matches!(provider, "trakt" | "anilist" | "mal" | "simkl")
            || !matches!(operation, "start" | "exchange" | "renew" | "revoke")
        {
            return Err(failure("unsupported_provider"));
        }
        let url = Url::parse(&format!(
            "{}/auth/v1/{provider}/{operation}",
            self.endpoints.relay
        ))
        .map_err(|_| failure("transport_unavailable"))?;
        let body = serde_json::to_vec(&body).map_err(|_| failure("invalid_request"))?;
        if body.len() > REQUEST_LIMIT {
            return Err(failure("invalid_request"));
        }
        let (_, auth) = self
            .gateway
            .get_http_client()
            .await
            .map_err(|_| failure("instance_auth_unavailable"))?;
        let auth = auth.ok_or_else(|| failure("instance_auth_required"))?;
        let request = self
            .http
            .post(url.clone())
            .timeout(if matches!(operation, "exchange" | "renew") {
                RELAY_TIMEOUT
            } else {
                REQUEST_TIMEOUT
            })
            .header("Content-Type", "application/json")
            .body(body.clone());
        let request = apply_instance_auth_headers(request, &auth, "POST", &url, &body)
            .await
            .map_err(|_| failure("instance_auth_unavailable"))?;
        self.send(request, provider, operation).await
    }

    async fn relay_json<T: DeserializeOwned>(
        &self,
        provider: &str,
        operation: &str,
        body: Value,
    ) -> AppResult<T> {
        let reply = self.relay(provider, operation, body).await?;
        ensure_success(&reply, Source::Relay)?;
        serde_json::from_slice(&reply.body).map_err(|_| failure("invalid_provider_response"))
    }

    fn token_request(
        &self,
        provider: &str,
        app: &ListProviderAppConfig,
        mut fields: Vec<(&str, String)>,
    ) -> AppResult<RequestBuilder> {
        require_value(&app.client_id, 512)?;
        if matches!(provider, "trakt" | "anilist") {
            require_value(&app.client_secret, 8192)?;
        }
        fields.push(("client_id", app.client_id.clone()));
        if !app.client_secret.is_empty() {
            require_value(&app.client_secret, 8192)?;
            fields.push(("client_secret", app.client_secret.clone()));
        }
        let endpoint = match provider {
            "trakt" => &self.endpoints.trakt,
            "anilist" => &self.endpoints.anilist,
            "mal" => &self.endpoints.mal,
            _ => return Err(failure("unsupported_provider")),
        };
        // Token requests are exchanges and renews: the provider may be slow, and
        // abandoning one mid-flight can spend a single-use code or token.
        let mut request = self.http.post(endpoint).timeout(OAUTH_TIMEOUT);
        if provider == "mal" {
            let mut body = url::form_urlencoded::Serializer::new(String::new());
            for (key, value) in &fields {
                body.append_pair(key, value);
            }
            request = request
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(body.finish());
        } else {
            let fields: std::collections::BTreeMap<_, _> = fields.into_iter().collect();
            request = request.json(&fields);
        }
        if provider == "trakt" {
            request = request
                .header("trakt-api-key", &app.client_id)
                .header("trakt-api-version", "2");
        }
        Ok(request)
    }

    async fn plex_start(&self, _state: &str) -> AppResult<ListAccountStartResponse> {
        let client_id = self.plex_client_identifier.as_str();
        let pin: PlexPin = self
            .json(
                self.http
                    .post(format!("{}/pins", self.endpoints.plex))
                    .header("X-Plex-Client-Identifier", client_id)
                    .header("X-Plex-Product", "Scryer")
                    .header("Content-Type", "application/x-www-form-urlencoded")
                    .body("strong=true"),
                "plex",
                "start",
                Source::Provider,
            )
            .await?;
        require_value(&pin.code, 256)?;
        if pin.id == 0 || pin.expires_in == 0 {
            return Err(failure("invalid_provider_response"));
        }
        let mut query = url::form_urlencoded::Serializer::new(String::new());
        query
            .append_pair("clientID", client_id)
            .append_pair("code", &pin.code)
            .append_pair("context[device][product]", "Scryer");
        Ok(ListAccountStartResponse {
            authorize_url: format!("https://app.plex.tv/auth#?{}", query.finish()),
            poll_token: Some(encode(&PlexPoll {
                id: pin.id,
                code: pin.code,
                client_id: client_id.into(),
            })?),
            expires_in: pin.expires_in.min(600),
        })
    }

    async fn tmdb_start(
        &self,
        app: Option<&ListProviderAppConfig>,
    ) -> AppResult<ListAccountStartResponse> {
        let token = tmdb_app_token(app)?;
        let reply: TmdbRequestToken = self
            .json(
                self.http
                    .post(format!("{}/auth/request_token", self.endpoints.tmdb))
                    .bearer_auth(token)
                    .json(&json!({})),
                "tmdb",
                "start",
                Source::Provider,
            )
            .await?;
        if !reply.success {
            return Err(failure("invalid_provider_response"));
        }
        require_value(&reply.request_token, 8192)?;
        let mut url = Url::parse("https://www.themoviedb.org/auth/access").expect("constant URL");
        url.query_pairs_mut()
            .append_pair("request_token", &reply.request_token);
        Ok(ListAccountStartResponse {
            authorize_url: url.to_string(),
            poll_token: Some(reply.request_token),
            expires_in: 600,
        })
    }
}

#[async_trait]
impl ListAccountAuthGateway for HttpListAccountAuthGateway {
    async fn start(&self, request: ListAccountStartRequest) -> AppResult<ListAccountStartResponse> {
        let provider = request.provider.as_str();
        match provider {
            "plex" => self.plex_start(&request.state).await,
            "tmdb" => self.tmdb_start(request.app.as_ref()).await,
            "trakt" | "anilist" | "mal" | "simkl" => {
                if let Some(app) = request.app.as_ref() {
                    if provider == "simkl" {
                        return Err(failure("unsupported_provider_app"));
                    }
                    let url = direct_authorize_url(provider, app, &request)?;
                    return Ok(ListAccountStartResponse {
                        authorize_url: url,
                        poll_token: None,
                        expires_in: 600,
                    });
                }
                let reply: RelayStart = self.relay_json(provider, "start", json!({
                    "origin": request.origin, "state": request.state, "code_challenge": request.code_challenge,
                })).await?;
                let client_id = validate_authorize_url(provider, &reply.authorize_url)?;
                if reply.expires_in == 0 || reply.expires_in > 600 {
                    return Err(failure("invalid_provider_response"));
                }
                Ok(ListAccountStartResponse {
                    authorize_url: reply.authorize_url,
                    poll_token: Some(encode(&RelayPoll { client_id })?),
                    expires_in: reply.expires_in,
                })
            }
            _ => Err(failure("unsupported_provider")),
        }
    }

    async fn poll(
        &self,
        provider: &str,
        poll_token: &str,
        app: Option<&ListProviderAppConfig>,
    ) -> AppResult<Option<ListAccountCredential>> {
        match provider {
            "plex" => {
                let poll: PlexPoll = decode(poll_token)?;
                require_value(&poll.client_id, 512)?;
                require_value(&poll.code, 256)?;
                if poll.id == 0 {
                    return Err(failure("invalid_request"));
                }
                let reply: PlexPin = self
                    .json(
                        self.http
                            .get(format!("{}/pins/{}", self.endpoints.plex, poll.id))
                            .header("X-Plex-Client-Identifier", &poll.client_id)
                            .header("X-Plex-Product", "Scryer")
                            .query(&[("code", &poll.code)]),
                        provider,
                        "poll",
                        Source::Provider,
                    )
                    .await?;
                if reply.id != poll.id || reply.code != poll.code {
                    return Err(failure("invalid_provider_response"));
                }
                match reply.auth_token.filter(|value| !value.is_empty()) {
                    Some(token) => {
                        require_value(&token, 8192)?;
                        Ok(Some(ListAccountCredential {
                            access_token: token,
                            direct: true,
                            token_type: Some("bearer".into()),
                            ..Default::default()
                        }))
                    }
                    None => Ok(None),
                }
            }
            "tmdb" => {
                require_value(poll_token, 8192)?;
                let request = self
                    .http
                    .post(format!("{}/auth/access_token", self.endpoints.tmdb))
                    .bearer_auth(tmdb_app_token(app)?)
                    .json(&json!({"request_token": poll_token}));
                let answer = self.send(request, provider, "poll").await?;
                // TMDb's documented status 41 means the user has not approved
                // yet. Only a 401 can carry it; any other status is judged on the
                // status alone, so an HTML 5xx page reads as unavailable.
                if answer.status == StatusCode::UNAUTHORIZED
                    && serde_json::from_slice::<TmdbAccessToken>(&answer.body)
                        .is_ok_and(|reply| reply.status_code == Some(41))
                {
                    return Ok(None);
                }
                ensure_success(&answer, Source::Provider)?;
                let reply: TmdbAccessToken = serde_json::from_slice(&answer.body)
                    .map_err(|_| failure("invalid_provider_response"))?;
                if !reply.success {
                    return Err(failure("authorization_failed"));
                }
                require_value(&reply.access_token, 8192)?;
                require_value(&reply.account_id, 512)?;
                Ok(Some(ListAccountCredential {
                    access_token: reply.access_token,
                    account_id: Some(reply.account_id),
                    direct: true,
                    token_type: Some("bearer".into()),
                    ..Default::default()
                }))
            }
            _ => Err(failure("unsupported_poll")),
        }
    }

    async fn complete(
        &self,
        request: ListAccountCompleteRequest,
    ) -> AppResult<ListAccountCredential> {
        let provider = request.provider.as_str();
        require_value(&request.code, 12000)?;
        if matches!(provider, "plex" | "tmdb") {
            return Err(failure("unsupported_exchange"));
        }
        if request.app.is_none() && matches!(provider, "trakt" | "anilist" | "simkl") {
            let tokens: Tokens = self
                .relay_json(
                    provider,
                    "exchange",
                    json!({
                        "exchange_code": request.code, "code_verifier": request.code_verifier,
                    }),
                )
                .await?;
            let mut credential = tokens.credential(provider, false, TokenGrant::Exchange)?;
            let poll: RelayPoll = decode(
                request
                    .poll_token
                    .as_deref()
                    .ok_or_else(|| failure("invalid_request"))?,
            )?;
            credential.client_id = Some(poll.client_id);
            return Ok(credential);
        }
        let mut app = request.app.unwrap_or_default();
        if provider == "mal" && app.client_id.is_empty() {
            let poll: RelayPoll = decode(
                request
                    .poll_token
                    .as_deref()
                    .ok_or_else(|| failure("invalid_request"))?,
            )?;
            app.client_id = poll.client_id;
            app.redirect_uri = format!("{RELAY_ORIGIN}/auth/v1/mal/auth");
        } else {
            validate_redirect(&app.redirect_uri, &request.origin)?;
        }
        let mut fields = vec![
            ("grant_type", "authorization_code".into()),
            ("code", request.code),
            ("redirect_uri", app.redirect_uri.clone()),
        ];
        if provider == "mal" {
            fields.push(("code_verifier", request.code_verifier));
        }
        let tokens: Tokens = self
            .json(
                self.token_request(provider, &app, fields)?,
                provider,
                "exchange",
                Source::Provider,
            )
            .await?;
        let mut credential = tokens.credential(provider, true, TokenGrant::Exchange)?;
        credential.client_id = Some(app.client_id);
        Ok(credential)
    }

    async fn renew(
        &self,
        provider: &str,
        credential: &ListAccountCredential,
        app: Option<&ListProviderAppConfig>,
    ) -> AppResult<ListAccountCredential> {
        let mut renewed = if !credential.direct && matches!(provider, "trakt" | "simkl") {
            let handle = credential
                .refresh_handle
                .as_deref()
                .filter(|handle| !handle.is_empty())
                .ok_or_else(|| failure("reconnect_required"))?;
            require_value(handle, 12000)?;
            let tokens: Tokens = self
                .relay_json(provider, "renew", json!({"refresh_handle": handle}))
                .await?;
            tokens.credential(provider, false, TokenGrant::Renew)?
        } else if credential.direct && matches!(provider, "trakt" | "mal") {
            let refresh_token = credential
                .refresh_token
                .as_deref()
                .filter(|token| !token.is_empty())
                .ok_or_else(|| failure("reconnect_required"))?;
            require_value(refresh_token, 8192)?;
            let mut app = app.cloned().unwrap_or_default();
            if provider == "mal" && app.client_id.is_empty() {
                app.client_id = credential
                    .client_id
                    .clone()
                    .ok_or_else(|| failure("reconnect_required"))?;
            }
            if credential
                .client_id
                .as_deref()
                .is_some_and(|id| id != app.client_id)
            {
                return Err(failure("reconnect_required"));
            }
            let mut fields = vec![
                ("grant_type", "refresh_token".into()),
                ("refresh_token", refresh_token.into()),
            ];
            if provider == "trakt" {
                fields.push(("redirect_uri", app.redirect_uri.clone()));
            }
            let tokens: Tokens = self
                .json(
                    self.token_request(provider, &app, fields)?,
                    provider,
                    "renew",
                    Source::DirectRenew,
                )
                .await?;
            tokens.credential(provider, true, TokenGrant::Renew)?
        } else {
            return Err(failure("reconnect_required"));
        };
        renewed.client_id = credential.client_id.clone();
        renewed.account_id = credential.account_id.clone();
        Ok(renewed)
    }

    async fn revoke(
        &self,
        provider: &str,
        credential: &ListAccountCredential,
        _app: Option<&ListProviderAppConfig>,
    ) -> AppResult<()> {
        if provider == "simkl" && !credential.direct {
            let handle = credential
                .refresh_handle
                .as_deref()
                .ok_or_else(|| failure("reconnect_required"))?;
            require_value(handle, 12000)?;
            let reply = self
                .relay(provider, "revoke", json!({"refresh_handle": handle}))
                .await?;
            ensure_success(&reply, Source::Relay)?;
        }
        Ok(())
    }
}

#[derive(Deserialize)]
struct RelayStart {
    authorize_url: String,
    expires_in: u64,
}
#[derive(Serialize, Deserialize)]
struct RelayPoll {
    client_id: String,
}
#[derive(Serialize, Deserialize)]
struct PlexPoll {
    id: u64,
    code: String,
    client_id: String,
}
#[derive(Deserialize)]
struct PlexPin {
    id: u64,
    code: String,
    #[serde(rename = "expiresIn", default)]
    expires_in: u64,
    #[serde(rename = "authToken", default)]
    auth_token: Option<String>,
}
#[derive(Deserialize)]
struct TmdbRequestToken {
    success: bool,
    request_token: String,
}
#[derive(Deserialize)]
struct TmdbAccessToken {
    #[serde(default)]
    success: bool,
    #[serde(default)]
    status_code: Option<u16>,
    #[serde(default)]
    access_token: String,
    #[serde(default)]
    account_id: String,
}
#[derive(Deserialize)]
struct Tokens {
    access_token: String,
    token_type: String,
    expires_in: i64,
    #[serde(default)]
    created_at: Option<i64>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    refresh_handle: Option<String>,
    #[serde(default)]
    refresh_expires_in: Option<i64>,
}
/// Simkl refresh credentials live for exactly this long from an exchange.
const SIMKL_REFRESH_LIFETIME_SECS: i64 = 180 * 24 * 60 * 60;

/// Which grant produced a token reply. A renew without a new refresh
/// credential keeps the original deadline, so it may report less than the
/// full Simkl lifetime; an exchange always starts a fresh one.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TokenGrant {
    Exchange,
    Renew,
}

impl Tokens {
    fn credential(
        self,
        provider: &str,
        direct: bool,
        grant: TokenGrant,
    ) -> AppResult<ListAccountCredential> {
        self.credential_at(provider, direct, grant, Utc::now())
    }

    fn credential_at(
        self,
        provider: &str,
        direct: bool,
        grant: TokenGrant,
        now: DateTime<Utc>,
    ) -> AppResult<ListAccountCredential> {
        require_value(&self.access_token, 8192)?;
        if !self.token_type.eq_ignore_ascii_case("bearer")
            || self.expires_in <= 0
            || self.scope.as_ref().is_some_and(|s| s.len() > 512)
        {
            return Err(failure("invalid_provider_response"));
        }
        if direct {
            if self.refresh_handle.is_some() {
                return Err(failure("invalid_provider_response"));
            }
            if matches!(provider, "trakt" | "mal") {
                require_value(self.refresh_token.as_deref().unwrap_or_default(), 8192)?;
            }
        } else {
            if self.refresh_token.is_some() {
                return Err(failure("invalid_provider_response"));
            }
            if matches!(provider, "trakt" | "simkl") {
                require_value(self.refresh_handle.as_deref().unwrap_or_default(), 12000)?;
            }
        }
        if provider == "simkl" {
            let refresh_lifetime_ok = match (grant, self.refresh_expires_in) {
                (TokenGrant::Exchange, Some(secs)) => secs == SIMKL_REFRESH_LIFETIME_SECS,
                (TokenGrant::Renew, Some(secs)) => secs > 0 && secs <= SIMKL_REFRESH_LIFETIME_SECS,
                (_, None) => false,
            };
            if self.scope.as_deref() != Some("media:read") || !refresh_lifetime_ok {
                return Err(failure("invalid_provider_response"));
            }
        }
        // Allow small provider clock skew, but never extend a token lifetime by
        // accepting a future issue time. Older issue times retain their expiry.
        let created = self
            .created_at
            .map(|v| {
                DateTime::from_timestamp(v, 0)
                    .filter(|at| v >= 0 && *at <= now + chrono::Duration::minutes(5))
                    .map(|at| at.min(now))
                    .ok_or_else(|| failure("invalid_provider_response"))
            })
            .transpose()?
            .unwrap_or(now);
        let lifetime = chrono::Duration::try_seconds(self.expires_in)
            .ok_or_else(|| failure("invalid_provider_response"))?;
        let expires_at = created
            .checked_add_signed(lifetime)
            .ok_or_else(|| failure("invalid_provider_response"))?;
        if expires_at <= now {
            return Err(failure("invalid_provider_response"));
        }
        let refresh_expires_at = self
            .refresh_expires_in
            .map(|v| {
                if v <= 0 {
                    return Err(failure("invalid_provider_response"));
                }
                let lifetime = chrono::Duration::try_seconds(v)
                    .ok_or_else(|| failure("invalid_provider_response"))?;
                now.checked_add_signed(lifetime)
                    .ok_or_else(|| failure("invalid_provider_response"))
            })
            .transpose()?;
        Ok(ListAccountCredential {
            access_token: self.access_token,
            token_type: Some(self.token_type),
            scope: self.scope,
            refresh_token: self.refresh_token,
            refresh_handle: self.refresh_handle,
            direct,
            expires_at: Some(expires_at),
            refresh_expires_at,
            ..Default::default()
        })
    }
}

fn failure(code: &str) -> AppError {
    scryer_application::lists::account_transport::auth_failure(code)
}
fn require_value(value: &str, max: usize) -> AppResult<()> {
    if value.is_empty() || value.len() > max || value.bytes().any(|b| b <= 32 || b == 127) {
        return Err(failure("invalid_request"));
    }
    Ok(())
}
/// A bounded HTTP answer: status, body, and any pause the server asked for.
struct Reply {
    status: StatusCode,
    body: Vec<u8>,
    retry_after: Option<Duration>,
}

/// Who answered, which decides how an error code is read.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Source {
    /// A provider endpoint called directly.
    Provider,
    /// A provider token endpoint renewing a stored grant. Only an OAuth error
    /// body makes a refusal final; a bare or HTML 4xx is a proxy or outage page.
    DirectRenew,
    /// The SMG relay, whose error codes describe the relay as well as the provider.
    Relay,
}

fn ensure_success(reply: &Reply, source: Source) -> AppResult<()> {
    let status = reply.status;
    if status.is_success() {
        return Ok(());
    }
    if status == StatusCode::TOO_MANY_REQUESTS {
        return Err(rate_limited_failure(reply.retry_after));
    }
    let error = serde_json::from_slice::<Value>(&reply.body)
        .ok()
        .and_then(|body| body.get("error").and_then(Value::as_str).map(str::to_owned));
    let code = match error.as_deref() {
        Some("invalid_grant" | "invalid_refresh_handle" | "expired_token") => {
            Some(RECONNECT_REQUIRED)
        }
        Some("access_denied") => Some("access_denied"),
        // The relay's documented transient answers about itself, whatever
        // status carries them. They say nothing about the provider or the grant.
        Some("relay_busy" | "relay_key_unavailable" | "relay_unavailable")
            if source == Source::Relay =>
        {
            Some("relay_unavailable")
        }
        Some("relay_busy" | "relay_key_unavailable" | "relay_unavailable") => {
            Some("provider_unavailable")
        }
        // A rotated or mistyped client secret is the app's fault, not the
        // grant's, so it never expires the accounts linked through that app.
        Some(
            "provider_not_configured"
            | "relay_not_configured"
            | "provider_configuration_error"
            | "invalid_client"
            | "unauthorized_client",
        ) => Some("provider_not_configured"),
        // The relay's own refusals say nothing about the provider grant.
        Some(code) if source == Source::Relay => match code {
            "instance_auth_required" => Some("instance_auth_required"),
            "invalid_request" | "request_too_large" => Some("invalid_request"),
            "not_found" | "method_not_allowed" => Some("unsupported_provider"),
            "invalid_exchange" => Some("authorization_failed"),
            // The provider's own trouble, relayed: it counts against the grant.
            "provider_unavailable" | "invalid_provider_response" | "invalid_scope" => {
                Some("provider_unavailable")
            }
            _ => None,
        },
        _ => None,
    };
    if let Some(code) = code {
        return Err(failure(code));
    }
    let refused = status.is_client_error() && status != StatusCode::REQUEST_TIMEOUT;
    let code = match source {
        // Only an explicit grant code above ends a relayed grant. SMG's instance
        // authentication answers 401 and 403 in free text (clock skew, unknown
        // key, replayed nonce); any other unrecognised answer, 4xx or 5xx, is
        // relay trouble that says nothing about the provider grant.
        Source::Relay if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) => {
            "instance_auth_required"
        }
        Source::Relay => "relay_unavailable",
        Source::DirectRenew if refused && error.is_some() => PROVIDER_REJECTED,
        Source::DirectRenew => "provider_unavailable",
        Source::Provider if refused => PROVIDER_REJECTED,
        Source::Provider => "provider_unavailable",
    };
    Err(failure(code))
}
fn encode<T: Serialize>(value: &T) -> AppResult<String> {
    serde_json::to_vec(value)
        .map(|v| URL_SAFE_NO_PAD.encode(v))
        .map_err(|_| failure("invalid_request"))
}
fn decode<T: DeserializeOwned>(value: &str) -> AppResult<T> {
    if value.len() > 16000 {
        return Err(failure("invalid_request"));
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| failure("invalid_request"))?;
    serde_json::from_slice(&bytes).map_err(|_| failure("invalid_request"))
}
fn tmdb_app_token(app: Option<&ListProviderAppConfig>) -> AppResult<&str> {
    let token = app
        .and_then(|app| app.access_token.as_deref())
        .ok_or_else(|| failure("provider_not_configured"))?;
    require_value(token, 8192)?;
    Ok(token)
}
fn authorize_endpoint(provider: &str) -> AppResult<&'static str> {
    match provider {
        "trakt" => Ok("https://trakt.tv/oauth/authorize"),
        "anilist" => Ok("https://anilist.co/api/v2/oauth/authorize"),
        "mal" => Ok("https://myanimelist.net/v1/oauth2/authorize"),
        "simkl" => Ok("https://simkl.com/oauth2/authorize"),
        _ => Err(failure("unsupported_provider")),
    }
}
fn validate_authorize_url(provider: &str, raw: &str) -> AppResult<String> {
    if raw.len() > 16000 {
        return Err(failure("invalid_provider_response"));
    }
    let parsed = Url::parse(raw).map_err(|_| failure("invalid_provider_response"))?;
    let expected = Url::parse(authorize_endpoint(provider)?).expect("constant URL");
    if parsed.origin() != expected.origin()
        || parsed.path() != expected.path()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.fragment().is_some()
    {
        return Err(failure("invalid_provider_response"));
    }
    let values = parsed
        .query_pairs()
        .filter(|(key, _)| key == "client_id")
        .map(|(_, value)| value.into_owned())
        .collect::<Vec<_>>();
    if values.len() != 1 {
        return Err(failure("invalid_provider_response"));
    }
    require_value(&values[0], 512)?;
    Ok(values[0].clone())
}
fn validate_redirect(raw: &str, origin: &str) -> AppResult<()> {
    let callback = Url::parse(raw).map_err(|_| failure("invalid_provider_app"))?;
    let origin = Url::parse(origin).map_err(|_| failure("invalid_provider_app"))?;
    if !matches!(callback.scheme(), "http" | "https")
        || callback.origin() != origin.origin()
        || !callback.path().ends_with("/lists/oauth/callback")
        || !callback.username().is_empty()
        || callback.password().is_some()
        || callback.query().is_some()
        || callback.fragment().is_some()
    {
        return Err(failure("invalid_provider_app"));
    }
    Ok(())
}
fn direct_authorize_url(
    provider: &str,
    app: &ListProviderAppConfig,
    request: &ListAccountStartRequest,
) -> AppResult<String> {
    require_value(&app.client_id, 512)?;
    require_value(&request.state, 512)?;
    validate_redirect(&app.redirect_uri, &request.origin)?;
    if matches!(provider, "trakt" | "anilist") {
        require_value(&app.client_secret, 8192)?;
    }
    let mut url = Url::parse(authorize_endpoint(provider)?).expect("constant URL");
    url.query_pairs_mut()
        .append_pair("client_id", &app.client_id)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", &app.redirect_uri)
        .append_pair("state", &request.state);
    if provider == "mal" {
        url.query_pairs_mut()
            .append_pair("code_challenge", &request.code_challenge)
            .append_pair("code_challenge_method", "plain");
    }
    Ok(url.to_string())
}

#[cfg(test)]
#[path = "client_list_accounts_tests.rs"]
mod tests;

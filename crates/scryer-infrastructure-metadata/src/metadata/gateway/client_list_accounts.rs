//! OAuth is an explicit outbound-policy exception: token POSTs are single-shot,
//! reject redirects, have bounded bodies and deadlines, and never log payloads.
use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use reqwest::{Client, RequestBuilder, StatusCode};
use scryer_application::lists::account_transport::{
    ListAccountAuthGateway, ListAccountCompleteRequest, ListAccountStartRequest,
    ListAccountStartResponse, ListProviderAppConfig,
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
const OAUTH_TIMEOUT: Duration = Duration::from_secs(20);

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
            .timeout(OAUTH_TIMEOUT)
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
    ) -> AppResult<(StatusCode, Vec<u8>)> {
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
            .map_err(|_| failure("provider_unavailable"))?;
        let status = response.status();
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
            .map_err(|_| failure("provider_unavailable"))?
        {
            if chunk.len() > RESPONSE_LIMIT.saturating_sub(body.len()) {
                return Err(failure("invalid_provider_response"));
            }
            body.extend_from_slice(&chunk);
        }
        Ok((status, body))
    }

    async fn json<T: DeserializeOwned>(
        &self,
        request: RequestBuilder,
        provider: &str,
        operation: &str,
    ) -> AppResult<T> {
        let (status, body) = self.send(request, provider, operation).await?;
        ensure_success(status, &body)?;
        serde_json::from_slice(&body).map_err(|_| failure("invalid_provider_response"))
    }

    async fn relay(
        &self,
        provider: &str,
        operation: &str,
        body: Value,
    ) -> AppResult<(StatusCode, Vec<u8>)> {
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
        let (status, body) = self.relay(provider, operation, body).await?;
        ensure_success(status, &body)?;
        serde_json::from_slice(&body).map_err(|_| failure("invalid_provider_response"))
    }

    fn token_request(
        &self,
        provider: &str,
        app: &ListProviderAppConfig,
        mut fields: Vec<(&str, String)>,
    ) -> AppResult<RequestBuilder> {
        require_value(&app.client_id, 512)?;
        if provider == "anilist" {
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
        let mut request = self.http.post(endpoint);
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
                let (status, body) = self.send(request, provider, "poll").await?;
                let reply: TmdbAccessToken = serde_json::from_slice(&body)
                    .map_err(|_| failure("invalid_provider_response"))?;
                // TMDb's documented status 41 means the user has not approved yet.
                if status == StatusCode::UNAUTHORIZED && reply.status_code == Some(41) {
                    return Ok(None);
                }
                ensure_success(status, &body)?;
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
            let mut credential = tokens.credential(provider, false)?;
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
        if matches!(provider, "trakt" | "mal") {
            fields.push(("code_verifier", request.code_verifier));
        }
        let tokens: Tokens = self
            .json(
                self.token_request(provider, &app, fields)?,
                provider,
                "exchange",
            )
            .await?;
        let mut credential = tokens.credential(provider, true)?;
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
                .ok_or_else(|| failure("reconnect_required"))?;
            require_value(handle, 12000)?;
            let tokens: Tokens = self
                .relay_json(provider, "renew", json!({"refresh_handle": handle}))
                .await?;
            tokens.credential(provider, false)?
        } else if credential.direct && matches!(provider, "trakt" | "mal") {
            let refresh_token = credential
                .refresh_token
                .as_deref()
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
                )
                .await?;
            tokens.credential(provider, true)?
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
            let (status, body) = self
                .relay(provider, "revoke", json!({"refresh_handle": handle}))
                .await?;
            ensure_success(status, &body)?;
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
impl Tokens {
    fn credential(self, provider: &str, direct: bool) -> AppResult<ListAccountCredential> {
        self.credential_at(provider, direct, Utc::now())
    }

    fn credential_at(
        self,
        provider: &str,
        direct: bool,
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
        if provider == "simkl"
            && (self.scope.as_deref() != Some("media:read")
                || self.refresh_expires_in != Some(180 * 24 * 60 * 60))
        {
            return Err(failure("invalid_provider_response"));
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
    AppError::Validation(format!("list account authentication failed: {code}"))
}
fn require_value(value: &str, max: usize) -> AppResult<()> {
    if value.is_empty() || value.len() > max || value.bytes().any(|b| b <= 32 || b == 127) {
        return Err(failure("invalid_request"));
    }
    Ok(())
}
fn ensure_success(status: StatusCode, body: &[u8]) -> AppResult<()> {
    if status.is_success() {
        return Ok(());
    }
    if status == StatusCode::TOO_MANY_REQUESTS {
        return Err(failure("rate_limited"));
    }
    let code = serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|body| match body.get("error").and_then(Value::as_str) {
            Some("invalid_grant" | "invalid_refresh_handle" | "expired_token") => {
                Some("reconnect_required")
            }
            Some("access_denied") => Some("access_denied"),
            Some(
                "provider_not_configured"
                | "relay_not_configured"
                | "provider_configuration_error"
                | "invalid_client"
                | "unauthorized_client",
            ) => Some("provider_not_configured"),
            _ => None,
        })
        .unwrap_or("provider_unavailable");
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
        "trakt" => Ok("https://auth.trakt.tv/oauth/authorize"),
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
    if provider == "anilist" {
        require_value(&app.client_secret, 8192)?;
    }
    let mut url = Url::parse(authorize_endpoint(provider)?).expect("constant URL");
    url.query_pairs_mut()
        .append_pair("client_id", &app.client_id)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", &app.redirect_uri)
        .append_pair("state", &request.state);
    // MyAnimeList accepts only the plain method; Trakt accepts only S256.
    let method = match provider {
        "mal" => Some("plain"),
        "trakt" => Some("S256"),
        _ => None,
    };
    if let Some(method) = method {
        require_value(&request.code_challenge, 128)?;
        url.query_pairs_mut()
            .append_pair("code_challenge", &request.code_challenge)
            .append_pair("code_challenge_method", method);
    }
    Ok(url.to_string())
}

#[cfg(test)]
#[path = "client_list_accounts_tests.rs"]
mod tests;

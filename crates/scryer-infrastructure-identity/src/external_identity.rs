use std::collections::HashSet;

use async_trait::async_trait;
use quick_xml::Reader;
use quick_xml::XmlVersion;
use quick_xml::events::Event;
use reqwest::StatusCode;
use scryer_application::{
    AppError, AppResult, EmbyApiKeyExchange, EmbyApiKeyExchangeCleanup, EmbyAvatar,
    EmbyConnectIdentityVerification, EmbyConnectServer, EmbyServerIdentity, EmbyServerUser,
    ExternalIdentityVerifier, JellyfinServerUser, MediaServerCatalogItem,
    MediaServerCatalogItemKind, PlexServerDiscovery, PlexServerUser, VerifiedExternalIdentity,
};
use scryer_domain::{
    ExternalAccountProvider, ExternalId, MediaServerConnection, MediaServerProvider,
};
use scryer_outbound_http::generic_reqwest_client;
use serde::Deserialize;
use serde_json::Value;
use url::Url;

const PLEX_BASE_URL: &str = "https://plex.tv";
const SCRYER_PRODUCT: &str = "Scryer";
const SCRYER_VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct HttpExternalIdentityVerifier {
    client: reqwest::Client,
    plex_base_url: Url,
    emby_connect_base_url: Url,
}

impl HttpExternalIdentityVerifier {
    pub fn new() -> Self {
        Self {
            client: generic_reqwest_client(),
            plex_base_url: Url::parse(PLEX_BASE_URL).expect("valid Plex base URL"),
            emby_connect_base_url: Url::parse("https://connect.emby.media/service/")
                .expect("valid Emby Connect base URL"),
        }
    }
}

impl Default for HttpExternalIdentityVerifier {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpExternalIdentityVerifier {
    #[cfg(test)]
    fn with_plex_base_url(plex_base_url: Url) -> Self {
        Self {
            client: generic_reqwest_client(),
            plex_base_url,
            emby_connect_base_url: Url::parse("https://connect.emby.media/service/")
                .expect("valid Emby Connect base URL"),
        }
    }

    fn plex_url(&self, path: &str) -> AppResult<Url> {
        self.plex_base_url
            .join(path.trim_start_matches('/'))
            .map_err(|error| AppError::Repository(format!("invalid Plex endpoint URL: {error}")))
    }

    async fn find_jellyfin_api_key(
        &self,
        keys_url: &Url,
        admin_token: &str,
        app_name: &str,
    ) -> AppResult<Option<String>> {
        let response = self
            .client
            .get(keys_url.clone())
            .header("Accept", "application/json")
            .header("X-Emby-Token", admin_token)
            .send()
            .await
            .map_err(|error| {
                AppError::Repository(format!("failed to list Jellyfin API keys: {error}"))
            })?;
        match response.status() {
            StatusCode::OK => {}
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                return Err(AppError::Unauthorized(
                    "Jellyfin admin token cannot list API keys".into(),
                ));
            }
            status => {
                return Err(AppError::Repository(format!(
                    "Jellyfin API key listing failed with status {status}"
                )));
            }
        }

        let keys = response
            .json::<JellyfinApiKeyQueryResult>()
            .await
            .map_err(|error| {
                AppError::Repository(format!("invalid Jellyfin API key list response: {error}"))
            })?;
        Ok(keys
            .items
            .into_iter()
            .filter(|key| {
                key.app_name
                    .as_deref()
                    .is_some_and(|name| name.eq_ignore_ascii_case(app_name))
            })
            .filter_map(|key| {
                let token = key.access_token?.trim().to_string();
                (!token.is_empty()).then_some((key.date_created.unwrap_or_default(), token))
            })
            .max_by(|left, right| left.0.cmp(&right.0))
            .map(|(_, token)| token))
    }
}

#[async_trait]
impl ExternalIdentityVerifier for HttpExternalIdentityVerifier {
    async fn scan_media_server_catalog(
        &self,
        connection: &MediaServerConnection,
    ) -> AppResult<Vec<MediaServerCatalogItem>> {
        scan_media_server_catalog(self, connection, false).await
    }

    async fn scan_media_server_catalog_incremental(
        &self,
        connection: &MediaServerConnection,
    ) -> AppResult<Vec<MediaServerCatalogItem>> {
        scan_media_server_catalog(self, connection, true).await
    }

    async fn refresh_media_server_paths(
        &self,
        connection: &MediaServerConnection,
        paths: &[String],
    ) -> AppResult<()> {
        refresh_media_server_paths(self, connection, paths).await
    }

    async fn verify_plex(
        &self,
        connection_id: &str,
        machine_id: Option<&str>,
        plex_auth_token: &str,
    ) -> AppResult<VerifiedExternalIdentity> {
        let connection_id = connection_id.trim();
        let token = plex_auth_token.trim();
        if token.is_empty() {
            return Err(AppError::Unauthorized("Plex auth token is required".into()));
        }

        let account_response = self
            .client
            .get(self.plex_url("users/account.json")?)
            .header("Accept", "application/json")
            .header("X-Plex-Token", token)
            .send()
            .await
            .map_err(|error| AppError::Repository(format!("failed to reach Plex: {error}")))?;

        match account_response.status() {
            StatusCode::OK => {}
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                return Err(AppError::Unauthorized("invalid Plex auth token".into()));
            }
            status => {
                return Err(AppError::Repository(format!(
                    "Plex account validation failed with status {status}"
                )));
            }
        }

        let account_json = account_response.json::<Value>().await.map_err(|error| {
            AppError::Repository(format!("invalid Plex account response: {error}"))
        })?;
        let user = account_json
            .get("user")
            .and_then(Value::as_object)
            .ok_or_else(|| {
                AppError::Repository("Plex account response did not include a user".into())
            })?;
        let external_user_id = json_value_string(user.get("id"))
            .or_else(|| json_value_string(user.get("uuid")))
            .ok_or_else(|| {
                AppError::Repository("Plex account response did not include a user id".into())
            })?;
        let username = json_value_string(user.get("username"))
            .or_else(|| json_value_string(user.get("title")))
            .or_else(|| json_value_string(user.get("email")))
            .unwrap_or_else(|| external_user_id.clone());
        let display_name = json_value_string(user.get("title")).or_else(|| Some(username.clone()));
        let avatar_url = json_value_string(user.get("thumb"));

        if let Some(machine_id) = machine_id.map(str::trim).filter(|value| !value.is_empty()) {
            let resources_response = self
                .client
                .get(self.plex_url("api/resources?includeHttps=1")?)
                .header("Accept", "application/xml")
                .header("X-Plex-Token", token)
                .send()
                .await
                .map_err(|error| {
                    AppError::Repository(format!("failed to reach Plex resources: {error}"))
                })?;
            match resources_response.status() {
                StatusCode::OK => {}
                StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                    return Err(AppError::Unauthorized("invalid Plex auth token".into()));
                }
                status => {
                    return Err(AppError::Repository(format!(
                        "Plex resources validation failed with status {status}"
                    )));
                }
            }
            let resources_xml = resources_response.text().await.map_err(|error| {
                AppError::Repository(format!("invalid Plex resources response: {error}"))
            })?;
            if !plex_resources_include_machine(&resources_xml, machine_id)? {
                return Err(AppError::Unauthorized(
                    "Plex account does not have access to the configured server".into(),
                ));
            }
        }

        Ok(VerifiedExternalIdentity {
            provider: ExternalAccountProvider::Plex,
            connection_id: connection_id.to_string(),
            external_user_id,
            username,
            display_name,
            avatar_url,
            // Plex verifies through a PIN exchange, so no password fact exists.
            remote_password_configured: None,
        })
    }

    async fn discover_plex_servers(
        &self,
        plex_auth_token: &str,
    ) -> AppResult<Vec<PlexServerDiscovery>> {
        let token = plex_auth_token.trim();
        if token.is_empty() {
            return Err(AppError::Unauthorized("Plex auth token is required".into()));
        }
        let resources_response = self
            .client
            .get(self.plex_url("api/resources?includeHttps=1")?)
            .header("Accept", "application/xml")
            .header("X-Plex-Token", token)
            .send()
            .await
            .map_err(|error| {
                AppError::Repository(format!("failed to reach Plex resources: {error}"))
            })?;
        match resources_response.status() {
            StatusCode::OK => {}
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                return Err(AppError::Unauthorized("invalid Plex auth token".into()));
            }
            status => {
                return Err(AppError::Repository(format!(
                    "Plex resources discovery failed with status {status}"
                )));
            }
        }
        let resources_xml = resources_response.text().await.map_err(|error| {
            AppError::Repository(format!("invalid Plex resources response: {error}"))
        })?;
        plex_server_discoveries(&resources_xml)
    }

    async fn verify_jellyfin(
        &self,
        connection_id: &str,
        base_url: &str,
        username: &str,
        password: &str,
    ) -> AppResult<VerifiedExternalIdentity> {
        let base_url = jellyfin_base_url(base_url)?;
        let auth_url = base_url.join("Users/AuthenticateByName").map_err(|error| {
            AppError::Validation(format!("Jellyfin authentication URL is invalid: {error}"))
        })?;

        let response = self
            .client
            .post(auth_url)
            .header(
                "Authorization",
                jellyfin_authorization_header(connection_id),
            )
            .header("Accept", "application/json")
            .json(&JellyfinAuthRequest { username, password })
            .send()
            .await
            .map_err(|error| {
                AppError::Repository(format!("failed to reach Jellyfin connection: {error}"))
            })?;

        match response.status() {
            StatusCode::OK => {}
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                return Err(AppError::Unauthorized(
                    "invalid Jellyfin credentials".into(),
                ));
            }
            StatusCode::BAD_REQUEST => {
                return Err(AppError::Unauthorized(
                    "invalid Jellyfin credentials".into(),
                ));
            }
            status => {
                return Err(AppError::Repository(format!(
                    "Jellyfin authentication failed with status {status}"
                )));
            }
        }

        let auth = response
            .json::<JellyfinAuthResponse>()
            .await
            .map_err(|error| {
                AppError::Repository(format!("invalid Jellyfin authentication response: {error}"))
            })?;
        // Read before `auth.user.name` is moved out below.
        // Reported, not enforced: linking a passwordless Jellyfin account is
        // allowed. Only the login use case refuses one.
        let remote_password_configured = jellyfin_remote_password_configured(&auth.user);
        let remote_username = auth
            .user
            .name
            .unwrap_or_else(|| username.trim().to_string());
        let avatar_url = jellyfin_user_avatar_url(
            &base_url,
            &auth.user.id,
            auth.user.primary_image_tag.as_deref(),
        );

        Ok(VerifiedExternalIdentity {
            provider: ExternalAccountProvider::Jellyfin,
            connection_id: connection_id.trim().to_string(),
            external_user_id: auth.user.id,
            username: remote_username.clone(),
            display_name: Some(remote_username),
            avatar_url,
            remote_password_configured,
        })
    }

    async fn test_jellyfin_connection(&self, base_url: &str) -> AppResult<()> {
        let base_url = jellyfin_base_url(base_url)?;
        let info_url = base_url.join("System/Info/Public").map_err(|error| {
            AppError::Validation(format!("Jellyfin system info URL is invalid: {error}"))
        })?;
        let response = self
            .client
            .get(info_url)
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|error| {
                AppError::Repository(format!("failed to reach Jellyfin connection: {error}"))
            })?;

        if !response.status().is_success() {
            return Err(AppError::Repository(format!(
                "Jellyfin connection test failed with status {}",
                response.status()
            )));
        }

        let info = response
            .json::<JellyfinPublicInfo>()
            .await
            .map_err(|error| {
                AppError::Repository(format!("invalid Jellyfin system info response: {error}"))
            })?;
        let product_name = info
            .product_name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .ok_or_else(|| {
                AppError::Validation(
                    "the supplied URL did not identify itself as a Jellyfin server".into(),
                )
            })?;
        if !product_name.to_ascii_lowercase().contains("jellyfin") {
            return Err(AppError::Validation(
                "the supplied URL did not identify itself as a Jellyfin server".into(),
            ));
        }
        Ok(())
    }

    async fn test_jellyfin_api_key(&self, base_url: &str, api_key: &str) -> AppResult<()> {
        let base_url = jellyfin_base_url(base_url)?;
        let users_url = base_url.join("Users").map_err(|error| {
            AppError::Validation(format!("Jellyfin users URL is invalid: {error}"))
        })?;
        let response = self
            .client
            .get(users_url)
            .header("Accept", "application/json")
            .header("X-Emby-Token", api_key.trim())
            .send()
            .await
            .map_err(|error| {
                AppError::Repository(format!("failed to validate Jellyfin API key: {error}"))
            })?;

        match response.status() {
            StatusCode::OK => Ok(()),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(AppError::Unauthorized(
                "Jellyfin API key is invalid or does not have user-list access".into(),
            )),
            status => Err(AppError::Repository(format!(
                "Jellyfin API key validation failed with status {status}"
            ))),
        }
    }

    async fn exchange_jellyfin_admin_api_key(
        &self,
        connection_id: &str,
        base_url: &str,
        username: &str,
        password: &str,
    ) -> AppResult<String> {
        let base_url = jellyfin_base_url(base_url)?;
        let auth_url = base_url.join("Users/AuthenticateByName").map_err(|error| {
            AppError::Validation(format!("Jellyfin authentication URL is invalid: {error}"))
        })?;
        let response = self
            .client
            .post(auth_url)
            .header(
                "Authorization",
                jellyfin_authorization_header(connection_id),
            )
            .header("Accept", "application/json")
            .json(&JellyfinAuthRequest { username, password })
            .send()
            .await
            .map_err(|error| {
                AppError::Repository(format!("failed to reach Jellyfin connection: {error}"))
            })?;

        match response.status() {
            StatusCode::OK => {}
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                return Err(AppError::Unauthorized(
                    "invalid Jellyfin admin credentials".into(),
                ));
            }
            status => {
                return Err(AppError::Repository(format!(
                    "Jellyfin admin authentication failed with status {status}"
                )));
            }
        }

        let auth = response
            .json::<JellyfinAuthResponse>()
            .await
            .map_err(|error| {
                AppError::Repository(format!("invalid Jellyfin authentication response: {error}"))
            })?;
        if !auth
            .user
            .policy
            .as_ref()
            .is_some_and(|policy| policy.is_administrator)
        {
            return Err(AppError::Unauthorized(
                "Jellyfin account must be an administrator to create a Scryer API key".into(),
            ));
        }
        let Some(token) = auth
            .access_token
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return Err(AppError::Repository(
                "Jellyfin did not return an admin access token; paste an API key manually".into(),
            ));
        };

        let keys_url = base_url.join("Auth/Keys").map_err(|error| {
            AppError::Validation(format!("Jellyfin API key URL is invalid: {error}"))
        })?;
        if let Some(existing) = self
            .find_jellyfin_api_key(&keys_url, token, SCRYER_PRODUCT)
            .await?
        {
            return Ok(existing);
        }

        let response = self
            .client
            .post(keys_url.clone())
            .header("Accept", "application/json")
            .header("X-Emby-Token", token)
            .query(&[("app", SCRYER_PRODUCT)])
            .send()
            .await
            .map_err(|error| {
                AppError::Repository(format!("failed to create Jellyfin API key: {error}"))
            })?;
        if !response.status().is_success() {
            return Err(AppError::Repository(format!(
                "Jellyfin did not create a usable API key (status {}); paste an API key manually",
                response.status()
            )));
        }

        self.find_jellyfin_api_key(&keys_url, token, SCRYER_PRODUCT)
            .await?
            .ok_or_else(|| {
                AppError::Repository(
                    "Jellyfin created an API key but did not expose it through Auth/Keys; paste an API key manually".into(),
                )
            })
    }

    async fn list_jellyfin_users(
        &self,
        base_url: &str,
        api_key: &str,
        search: Option<&str>,
    ) -> AppResult<Vec<JellyfinServerUser>> {
        let base_url = jellyfin_base_url(base_url)?;
        let users_url = base_url.join("Users").map_err(|error| {
            AppError::Validation(format!("Jellyfin users URL is invalid: {error}"))
        })?;
        let response = self
            .client
            .get(users_url)
            .header("Accept", "application/json")
            .header("X-Emby-Token", api_key.trim())
            .send()
            .await
            .map_err(|error| {
                AppError::Repository(format!("failed to list Jellyfin users: {error}"))
            })?;
        match response.status() {
            StatusCode::OK => {}
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                return Err(AppError::Unauthorized(
                    "Jellyfin API key is invalid or cannot list users".into(),
                ));
            }
            status => {
                return Err(AppError::Repository(format!(
                    "Jellyfin user listing failed with status {status}"
                )));
            }
        }

        let mut users = response
            .json::<Vec<JellyfinUser>>()
            .await
            .map_err(|error| {
                AppError::Repository(format!("invalid Jellyfin user list response: {error}"))
            })?
            .into_iter()
            .map(|user| {
                let avatar_url = jellyfin_user_avatar_url(
                    &base_url,
                    &user.id,
                    user.primary_image_tag.as_deref(),
                );
                let username = user.name.unwrap_or_else(|| user.id.clone());
                JellyfinServerUser {
                    id: user.id,
                    username: username.clone(),
                    display_name: Some(username),
                    avatar_url,
                }
            })
            .collect::<Vec<_>>();
        if let Some(search) = search.map(str::trim).filter(|value| !value.is_empty()) {
            let search = search.to_ascii_lowercase();
            users.retain(|user| {
                user.username.to_ascii_lowercase().contains(&search)
                    || user.id.to_ascii_lowercase().contains(&search)
            });
        }
        Ok(users)
    }

    async fn resolve_emby_api_base(
        &self,
        connection_id: &str,
        base_url: &str,
    ) -> AppResult<EmbyServerIdentity> {
        super::emby::resolve_api_base(&self.client, connection_id, base_url).await
    }

    async fn test_emby_api_key(
        &self,
        connection_id: &str,
        base_url: &str,
        api_key: &str,
        expected_server_id: Option<&str>,
    ) -> AppResult<EmbyServerIdentity> {
        super::emby::test_api_key(
            &self.client,
            connection_id,
            base_url,
            api_key,
            expected_server_id,
        )
        .await
    }

    async fn exchange_emby_local_admin_api_key(
        &self,
        connection_id: &str,
        base_url: &str,
        username: &str,
        password: &str,
    ) -> AppResult<EmbyApiKeyExchange> {
        super::emby::exchange_local_admin_api_key(
            &self.client,
            connection_id,
            base_url,
            username,
            password,
        )
        .await
    }

    async fn discover_emby_connect_servers(
        &self,
        username_or_email: &str,
        password: &str,
    ) -> AppResult<Vec<EmbyConnectServer>> {
        super::emby::discover_connect_servers(
            &self.client,
            &self.emby_connect_base_url,
            username_or_email,
            password,
        )
        .await
    }

    async fn exchange_emby_connect_admin_api_key(
        &self,
        connection_id: &str,
        base_url: &str,
        server_id: &str,
        username_or_email: &str,
        password: &str,
    ) -> AppResult<EmbyApiKeyExchange> {
        super::emby::exchange_connect_admin_api_key(
            &self.client,
            &self.emby_connect_base_url,
            connection_id,
            base_url,
            server_id,
            username_or_email,
            password,
        )
        .await
    }

    async fn finish_emby_api_key_exchange(
        &self,
        connection_id: &str,
        cleanup: EmbyApiKeyExchangeCleanup,
        compensate_created_key: bool,
    ) {
        super::emby::finish_api_key_exchange(
            &self.client,
            connection_id,
            cleanup,
            compensate_created_key,
        )
        .await;
    }

    async fn verify_emby_local_identity(
        &self,
        connection_id: &str,
        base_url: &str,
        expected_server_id: &str,
        username: &str,
        password: &str,
    ) -> AppResult<VerifiedExternalIdentity> {
        super::emby::verify_local_identity(
            &self.client,
            connection_id,
            base_url,
            expected_server_id,
            username,
            password,
        )
        .await
    }

    async fn verify_emby_connect_identity(
        &self,
        connection_id: &str,
        base_url: &str,
        expected_server_id: &str,
        username_or_email: &str,
        password: &str,
    ) -> AppResult<EmbyConnectIdentityVerification> {
        super::emby::verify_connect_identity(
            &self.client,
            &self.emby_connect_base_url,
            connection_id,
            base_url,
            expected_server_id,
            username_or_email,
            password,
        )
        .await
    }

    async fn test_emby_connect_identity(
        &self,
        connection_id: &str,
        base_url: &str,
        expected_server_id: &str,
        username_or_email: &str,
        password: &str,
    ) -> AppResult<EmbyConnectIdentityVerification> {
        self.verify_emby_connect_identity(
            connection_id,
            base_url,
            expected_server_id,
            username_or_email,
            password,
        )
        .await
    }

    async fn list_emby_users(
        &self,
        connection_id: &str,
        base_url: &str,
        api_key: &str,
        search: Option<&str>,
    ) -> AppResult<Vec<EmbyServerUser>> {
        super::emby::list_users(&self.client, connection_id, base_url, api_key, search).await
    }

    async fn fetch_emby_user_avatar(
        &self,
        _connection_id: &str,
        base_url: &str,
        api_key: &str,
        user_id: &str,
        image_tag: &str,
    ) -> AppResult<Option<EmbyAvatar>> {
        super::emby::fetch_avatar(&self.client, base_url, api_key, user_id, image_tag).await
    }

    async fn list_plex_users(
        &self,
        plex_auth_token: &str,
        search: Option<&str>,
    ) -> AppResult<Vec<PlexServerUser>> {
        let token = plex_auth_token.trim();
        if token.is_empty() {
            return Err(AppError::Unauthorized("Plex auth token is required".into()));
        }
        let response = self
            .client
            .get(self.plex_url("api/users/")?)
            .header("Accept", "application/xml")
            .header("X-Plex-Token", token)
            .send()
            .await
            .map_err(|error| AppError::Repository(format!("failed to list Plex users: {error}")))?;
        match response.status() {
            StatusCode::OK => {}
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                return Err(AppError::Unauthorized(
                    "Plex token is invalid or cannot list users".into(),
                ));
            }
            status => {
                return Err(AppError::Repository(format!(
                    "Plex user listing failed with status {status}"
                )));
            }
        }

        let users_xml = response.text().await.map_err(|error| {
            AppError::Repository(format!("invalid Plex user list response: {error}"))
        })?;
        plex_server_users(&users_xml, search)
    }
}

#[derive(serde::Serialize)]
struct JellyfinAuthRequest<'a> {
    #[serde(rename = "Username")]
    username: &'a str,
    #[serde(rename = "Pw")]
    password: &'a str,
}

#[derive(Deserialize)]
struct JellyfinPublicInfo {
    #[serde(rename = "ProductName")]
    product_name: Option<String>,
}

#[derive(Deserialize)]
struct JellyfinAuthResponse {
    #[serde(rename = "User")]
    user: JellyfinUser,
    #[serde(rename = "AccessToken")]
    access_token: Option<String>,
}

#[derive(Deserialize)]
struct JellyfinApiKeyQueryResult {
    #[serde(rename = "Items")]
    items: Vec<JellyfinApiKeyInfo>,
}

#[derive(Deserialize)]
struct JellyfinApiKeyInfo {
    #[serde(rename = "AccessToken")]
    access_token: Option<String>,
    #[serde(rename = "AppName")]
    app_name: Option<String>,
    #[serde(rename = "DateCreated")]
    date_created: Option<String>,
}

#[derive(Deserialize)]
struct JellyfinUser {
    #[serde(rename = "Id")]
    id: String,
    #[serde(rename = "Name")]
    name: Option<String>,
    #[serde(rename = "PrimaryImageTag")]
    primary_image_tag: Option<String>,
    #[serde(rename = "Policy")]
    policy: Option<JellyfinUserPolicy>,
    #[serde(rename = "HasPassword")]
    has_password: Option<bool>,
    #[serde(rename = "HasConfiguredPassword")]
    has_configured_password: Option<bool>,
}

/// Jellyfin reports both flags as `false` for an account with no password, and
/// both as `true` once one is set. Treat an explicit `false` from either as
/// "no password", and absence of both as unknown so servers that omit the
/// fields keep working.
fn jellyfin_remote_password_configured(user: &JellyfinUser) -> Option<bool> {
    match (user.has_password, user.has_configured_password) {
        (None, None) => None,
        (has_password, has_configured_password) => {
            Some(has_password.unwrap_or(true) && has_configured_password.unwrap_or(true))
        }
    }
}

#[derive(Deserialize)]
struct JellyfinUserPolicy {
    #[serde(rename = "IsAdministrator")]
    is_administrator: bool,
}

fn jellyfin_base_url(base_url: &str) -> AppResult<Url> {
    let mut base_url = Url::parse(base_url)
        .map_err(|error| AppError::Validation(format!("Jellyfin base URL is invalid: {error}")))?;
    if base_url.query().is_some() || base_url.fragment().is_some() {
        return Err(AppError::Validation(
            "Jellyfin connection base URL must not include a query or fragment".into(),
        ));
    }
    if !base_url.path().ends_with('/') {
        base_url.set_path(&format!("{}/", base_url.path()));
    }
    Ok(base_url)
}

fn jellyfin_user_avatar_url(
    base_url: &Url,
    user_id: &str,
    primary_image_tag: Option<&str>,
) -> Option<String> {
    let tag = primary_image_tag
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let mut image_url = base_url
        .join(&format!("Users/{user_id}/Images/Primary"))
        .ok()?;
    image_url.query_pairs_mut().append_pair("tag", tag);
    Some(image_url.to_string())
}

fn jellyfin_authorization_header(connection_id: &str) -> String {
    let device_id = format!("SCRYER_{}", connection_id.replace('"', ""));
    format!(
        "MediaBrowser Client=\"{SCRYER_PRODUCT}\", Device=\"{SCRYER_PRODUCT}\", DeviceId=\"{device_id}\", Version=\"{SCRYER_VERSION}\""
    )
}

fn json_value_string(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(value) => {
            let value = value.trim();
            (!value.is_empty()).then(|| value.to_string())
        }
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

struct ParsedPlexServerUser {
    user: PlexServerUser,
    email: Option<String>,
}

fn plex_server_users(users_xml: &str, search: Option<&str>) -> AppResult<Vec<PlexServerUser>> {
    let mut users = Vec::new();
    let mut reader = Reader::from_str(users_xml);
    reader.config_mut().trim_text(true);
    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) | Ok(Event::Empty(element)) => {
                if element.name().as_ref() != "User" {
                    continue;
                }
                let mut id = None;
                let mut username = None;
                let mut title = None;
                let mut email = None;
                let mut thumb = None;
                for attribute in element.attributes() {
                    let attribute = attribute.map_err(|error| {
                        AppError::Repository(format!("invalid Plex users XML: {error}"))
                    })?;
                    let value = attribute
                        .normalized_value(XmlVersion::Implicit1_0)
                        .map_err(|error| {
                            AppError::Repository(format!("invalid Plex users XML: {error}"))
                        })?
                        .trim()
                        .to_string();
                    if value.is_empty() {
                        continue;
                    }
                    match attribute.key.as_ref() {
                        "id" => id = Some(value),
                        "username" => username = Some(value),
                        "title" => title = Some(value),
                        "email" => email = Some(value),
                        "thumb" => thumb = Some(value),
                        _ => {}
                    }
                }

                let Some(id) = id else {
                    continue;
                };
                let username = username
                    .or_else(|| title.clone())
                    .or_else(|| email.clone())
                    .unwrap_or_else(|| id.clone());
                let display_name = title.clone().or_else(|| Some(username.clone()));
                users.push(ParsedPlexServerUser {
                    user: PlexServerUser {
                        id,
                        username,
                        display_name,
                        avatar_url: thumb,
                    },
                    email,
                });
            }
            Ok(Event::Eof) => {
                if let Some(search) = search.map(str::trim).filter(|value| !value.is_empty()) {
                    let search = search.to_ascii_lowercase();
                    users.retain(|entry| {
                        [
                            entry.user.id.as_str(),
                            entry.user.username.as_str(),
                            entry.user.display_name.as_deref().unwrap_or_default(),
                            entry.email.as_deref().unwrap_or_default(),
                        ]
                        .into_iter()
                        .any(|value| value.to_ascii_lowercase().contains(&search))
                    });
                }
                let mut users = users
                    .into_iter()
                    .map(|entry| entry.user)
                    .collect::<Vec<_>>();
                users.sort_by(|left, right| {
                    left.username
                        .to_ascii_lowercase()
                        .cmp(&right.username.to_ascii_lowercase())
                        .then_with(|| left.id.cmp(&right.id))
                });
                return Ok(users);
            }
            Err(error) => {
                return Err(AppError::Repository(format!(
                    "invalid Plex users XML: {error}"
                )));
            }
            _ => {}
        }
    }
}

fn plex_server_discoveries(resources_xml: &str) -> AppResult<Vec<PlexServerDiscovery>> {
    let mut servers = Vec::new();
    let mut reader = Reader::from_str(resources_xml);
    reader.config_mut().trim_text(true);
    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) | Ok(Event::Empty(element)) => {
                if element.name().as_ref() == "Device"
                    && let Some(device) = parse_plex_device(&element)?
                {
                    servers.push(device.into_discovery());
                }
            }
            Ok(Event::Eof) => {
                servers.sort_by(|left, right| {
                    left.name
                        .to_ascii_lowercase()
                        .cmp(&right.name.to_ascii_lowercase())
                        .then_with(|| left.id.cmp(&right.id))
                });
                servers.dedup_by(|left, right| left.id == right.id);
                return Ok(servers);
            }
            Err(error) => {
                return Err(AppError::Repository(format!(
                    "invalid Plex resources XML: {error}"
                )));
            }
            _ => {}
        }
    }
}

struct PendingPlexServerDiscovery {
    id: String,
    name: String,
}

impl PendingPlexServerDiscovery {
    fn into_discovery(self) -> PlexServerDiscovery {
        PlexServerDiscovery {
            id: self.id,
            name: self.name,
        }
    }
}

fn parse_plex_device(
    element: &quick_xml::events::BytesStart<'_>,
) -> AppResult<Option<PendingPlexServerDiscovery>> {
    let mut machine_id = None;
    let mut name = None;
    let mut product = None;
    let mut provides = None;
    for attribute in element.attributes() {
        let attribute = attribute.map_err(|error| {
            AppError::Repository(format!("invalid Plex resources XML: {error}"))
        })?;
        let value = attribute
            .normalized_value(XmlVersion::Implicit1_0)
            .map_err(|error| AppError::Repository(format!("invalid Plex resources XML: {error}")))?
            .trim()
            .to_string();
        if value.is_empty() {
            continue;
        }
        match attribute.key.as_ref() {
            "machineIdentifier" | "clientIdentifier" => machine_id = Some(value),
            "name" => name = Some(value),
            "product" => product = Some(value),
            "provides" => provides = Some(value),
            _ => {}
        }
    }
    if provides
        .as_deref()
        .is_some_and(|value| !value.split(',').any(|part| part.trim() == "server"))
    {
        return Ok(None);
    }
    let Some(machine_id) = machine_id else {
        return Ok(None);
    };
    let name = name.or(product).unwrap_or_else(|| machine_id.clone());
    Ok(Some(PendingPlexServerDiscovery {
        id: machine_id,
        name,
    }))
}

fn plex_resources_include_machine(resources_xml: &str, machine_id: &str) -> AppResult<bool> {
    let mut reader = Reader::from_str(resources_xml);
    reader.config_mut().trim_text(true);
    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) | Ok(Event::Empty(element)) => {
                for attribute in element.attributes() {
                    let attribute = attribute.map_err(|error| {
                        AppError::Repository(format!("invalid Plex resources XML: {error}"))
                    })?;
                    if matches!(
                        attribute.key.as_ref(),
                        "machineIdentifier" | "clientIdentifier"
                    ) {
                        let value = attribute
                            .normalized_value(XmlVersion::Implicit1_0)
                            .map_err(|error| {
                                AppError::Repository(format!("invalid Plex resources XML: {error}"))
                            })?;
                        if value == machine_id {
                            return Ok(true);
                        }
                    }
                }
            }
            Ok(Event::Eof) => return Ok(false),
            Err(error) => {
                return Err(AppError::Repository(format!(
                    "invalid Plex resources XML: {error}"
                )));
            }
            _ => {}
        }
    }
}

/// FR-088: ask one media server to re-read specific folders.
///
/// Targeted, never a full library scan. Every provider here has a first-class
/// API for "this path changed", and that is the only thing this uses:
///
/// | Provider | Call |
/// |---|---|
/// | Jellyfin / Emby | `POST /Library/Media/Updated` with one `MediaUpdateInfo` per path — the same notification the servers' own filesystem watchers raise. |
/// | Plex | `GET /library/sections/{key}/refresh?path=…`, Plex's partial scan, against the section whose configured location contains the path. |
///
/// A Plex path that falls outside every section is skipped rather than widened
/// into a section-wide scan: a folder no section covers is a folder that server
/// does not serve.
async fn refresh_media_server_paths(
    verifier: &HttpExternalIdentityVerifier,
    connection: &MediaServerConnection,
    paths: &[String],
) -> AppResult<()> {
    let paths = paths
        .iter()
        .map(|path| path.trim())
        .filter(|path| !path.is_empty())
        .collect::<Vec<_>>();
    if paths.is_empty() {
        return Ok(());
    }

    let api_key = connection
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .ok_or_else(|| {
            AppError::Validation("media server refresh requires an API key".into())
        })?;

    match connection.provider {
        MediaServerProvider::Jellyfin | MediaServerProvider::Emby => {
            refresh_emby_paths(&verifier.client, &connection.base_url, api_key, &paths).await
        }
        MediaServerProvider::Plex => {
            let machine_id = connection
                .machine_id
                .as_deref()
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .ok_or_else(|| {
                    AppError::Validation("Plex refresh requires a selected server".into())
                })?;
            refresh_plex_paths(verifier, machine_id, api_key, &paths).await
        }
    }
}

/// Jellyfin and Emby both accept the library-update notification their own
/// watchers post, so one request carries every changed folder.
async fn refresh_emby_paths(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
    paths: &[&str],
) -> AppResult<()> {
    let base_url = media_server_url(base_url)?;
    let url = base_url.join("Library/Media/Updated").map_err(|error| {
        AppError::Repository(format!("invalid media server refresh URL: {error}"))
    })?;
    let body = serde_json::json!({
        "Updates": paths
            .iter()
            .map(|path| serde_json::json!({ "Path": path, "UpdateType": "Modified" }))
            .collect::<Vec<_>>(),
    });
    let response = client
        .post(url)
        .header("Accept", "application/json")
        .header("X-Emby-Token", api_key)
        .json(&body)
        .send()
        .await
        .map_err(|error| {
            AppError::Repository(format!("media server refresh failed: {error}"))
        })?;
    if !response.status().is_success() {
        return Err(AppError::Repository(format!(
            "media server refresh failed with status {}",
            response.status()
        )));
    }
    Ok(())
}

/// Plex has no "these paths changed" endpoint, but it does have a per-section
/// partial scan, which is the same thing once the section is known.
async fn refresh_plex_paths(
    verifier: &HttpExternalIdentityVerifier,
    machine_id: &str,
    token: &str,
    paths: &[&str],
) -> AppResult<()> {
    let server_url = plex_server_base_url(verifier, machine_id, token).await?;
    let sections = plex_json(
        &verifier.client,
        server_url
            .join("library/sections")
            .map_err(|error| AppError::Repository(error.to_string()))?,
        token,
    )
    .await?;
    let locations = plex_section_locations(&sections);

    for path in paths {
        let Some(section_key) = plex_section_for_path(&locations, path) else {
            tracing::debug!(
                path,
                "no Plex library section covers this folder; nothing to refresh"
            );
            continue;
        };
        let mut endpoint = server_url
            .join(&format!("library/sections/{section_key}/refresh"))
            .map_err(|error| AppError::Repository(error.to_string()))?;
        endpoint.query_pairs_mut().append_pair("path", path);
        let response = verifier
            .client
            .get(endpoint)
            .header("Accept", "application/json")
            .header("X-Plex-Token", token)
            .send()
            .await
            .map_err(|error| {
                AppError::Repository(format!("Plex partial scan failed: {error}"))
            })?;
        if !response.status().is_success() {
            return Err(AppError::Repository(format!(
                "Plex partial scan failed with status {}",
                response.status()
            )));
        }
    }
    Ok(())
}

/// `(section key, configured location)` for every movie or show section.
fn plex_section_locations(sections: &Value) -> Vec<(String, String)> {
    let mut locations = Vec::new();
    for section in plex_entries(sections, "Directory") {
        if !matches!(
            section.get("type").and_then(Value::as_str),
            Some("movie") | Some("show")
        ) {
            continue;
        }
        let Some(key) = section.get("key").and_then(Value::as_str) else {
            continue;
        };
        for location in section
            .get("Location")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
        {
            if let Some(path) = location.get("path").and_then(Value::as_str) {
                locations.push((key.to_string(), path.trim().to_string()));
            }
        }
    }
    locations
}

/// The most specific section whose location contains `path`.
fn plex_section_for_path(locations: &[(String, String)], path: &str) -> Option<String> {
    locations
        .iter()
        .filter(|(_, location)| {
            let location = location.trim_end_matches(['/', '\\']);
            !location.is_empty()
                && (path == location
                    || path
                        .strip_prefix(location)
                        .is_some_and(|rest| rest.starts_with(['/', '\\'])))
        })
        .max_by_key(|(_, location)| location.len())
        .map(|(key, _)| key.clone())
}

/// The reachable base URL of the configured Plex server, via plex.tv's resource
/// list. Shared by the catalog scan and the FR-088 partial scan.
async fn plex_server_base_url(
    verifier: &HttpExternalIdentityVerifier,
    machine_id: &str,
    token: &str,
) -> AppResult<Url> {
    let response = verifier
        .client
        .get(verifier.plex_url("api/resources?includeHttps=1")?)
        .header("Accept", "application/xml")
        .header("X-Plex-Token", token)
        .send()
        .await
        .map_err(|error| {
            AppError::Repository(format!("failed to reach Plex resources: {error}"))
        })?;
    if !response.status().is_success() {
        return Err(AppError::Repository(format!(
            "Plex resources discovery failed with status {}",
            response.status()
        )));
    }
    let resources = response.text().await.map_err(|error| {
        AppError::Repository(format!("invalid Plex resources response: {error}"))
    })?;
    let server_url = plex_server_url(&resources, machine_id)?.ok_or_else(|| {
        AppError::Repository("configured Plex server has no reachable URI".into())
    })?;
    media_server_url(&server_url)
}

async fn scan_media_server_catalog(
    verifier: &HttpExternalIdentityVerifier,
    connection: &MediaServerConnection,
    recent_only: bool,
) -> AppResult<Vec<MediaServerCatalogItem>> {
    let api_key = connection
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .ok_or_else(|| {
            AppError::Validation("media server catalog scan requires an API key".into())
        })?;
    match connection.provider {
        MediaServerProvider::Jellyfin | MediaServerProvider::Emby => {
            scan_emby_catalog(&verifier.client, &connection.base_url, api_key, recent_only).await
        }
        MediaServerProvider::Plex => {
            let machine_id = connection
                .machine_id
                .as_deref()
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .ok_or_else(|| {
                    AppError::Validation("Plex catalog scan requires a selected server".into())
                })?;
            scan_plex_catalog(verifier, machine_id, api_key, recent_only).await
        }
    }
}

async fn scan_emby_catalog(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
    recent_only: bool,
) -> AppResult<Vec<MediaServerCatalogItem>> {
    let base_url = media_server_url(base_url)?;
    let mut start = 0usize;
    let mut catalog = Vec::new();
    const PAGE_SIZE: usize = 500;
    loop {
        let mut url = base_url.join("Items").map_err(|error| {
            AppError::Repository(format!("invalid media server catalog URL: {error}"))
        })?;
        url.query_pairs_mut()
            .append_pair("Recursive", "true")
            .append_pair("IncludeItemTypes", "Movie,Series,Episode")
            .append_pair(
                "Fields",
                "ProviderIds,SeriesId,ParentIndexNumber,IndexNumber,IndexNumberEnd",
            )
            .append_pair("StartIndex", &start.to_string())
            .append_pair("Limit", &PAGE_SIZE.to_string());
        if recent_only {
            url.query_pairs_mut()
                .append_pair("SortBy", "DateCreated,DateLastContentAdded")
                .append_pair("SortOrder", "Descending");
        }
        let response = client
            .get(url)
            .header("Accept", "application/json")
            .header("X-Emby-Token", api_key)
            .send()
            .await
            .map_err(|error| {
                AppError::Repository(format!("media server catalog scan failed: {error}"))
            })?;
        if !response.status().is_success() {
            return Err(AppError::Repository(format!(
                "media server catalog scan failed with status {}",
                response.status()
            )));
        }
        let page = response.json::<Value>().await.map_err(|error| {
            AppError::Repository(format!("invalid media server catalog response: {error}"))
        })?;
        let items = page
            .get("Items")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let page_len = items.len();
        catalog.extend(items.iter().filter_map(emby_catalog_item));
        start += page_len;
        let total = page
            .get("TotalRecordCount")
            .and_then(Value::as_u64)
            .unwrap_or(0) as usize;
        if recent_only || page_len < PAGE_SIZE || (total > 0 && start >= total) {
            return hydrate_emby_parent_series(client, &base_url, api_key, catalog).await;
        }
    }
}

async fn hydrate_emby_parent_series(
    client: &reqwest::Client,
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
        let mut url = base_url
            .join(&format!("Items/{series_id}"))
            .map_err(|error| {
                AppError::Repository(format!("invalid media server catalog URL: {error}"))
            })?;
        url.query_pairs_mut().append_pair(
            "Fields",
            "ProviderIds,SeriesId,ParentIndexNumber,IndexNumber,IndexNumberEnd",
        );
        let response = client
            .get(url)
            .header("Accept", "application/json")
            .header("X-Emby-Token", api_key)
            .send()
            .await
            .map_err(|error| {
                AppError::Repository(format!("media server parent-series lookup failed: {error}"))
            })?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            continue;
        }
        if !response.status().is_success() {
            return Err(AppError::Repository(format!(
                "media server parent-series lookup failed with status {}",
                response.status()
            )));
        }
        let value = response.json::<Value>().await.map_err(|error| {
            AppError::Repository(format!(
                "invalid media server parent-series response: {error}"
            ))
        })?;
        if let Some(series) = emby_catalog_item(&value)
            && series.kind == MediaServerCatalogItemKind::Series
        {
            catalog.push(series);
        }
    }
    Ok(catalog)
}

fn emby_catalog_item(value: &Value) -> Option<MediaServerCatalogItem> {
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

async fn scan_plex_catalog(
    verifier: &HttpExternalIdentityVerifier,
    machine_id: &str,
    token: &str,
    recent_only: bool,
) -> AppResult<Vec<MediaServerCatalogItem>> {
    let server_url = plex_server_base_url(verifier, machine_id, token).await?;
    let sections = plex_json(
        &verifier.client,
        server_url
            .join("library/sections")
            .map_err(|error| AppError::Repository(error.to_string()))?,
        token,
    )
    .await?;
    let mut catalog = Vec::new();
    let mut catalog_keys = HashSet::new();
    for section in plex_entries(&sections, "Directory") {
        let section_type = section.get("type").and_then(Value::as_str);
        if !matches!(section_type, Some("movie") | Some("show")) {
            continue;
        }
        let Some(key) = section.get("key").and_then(Value::as_str) else {
            continue;
        };
        let endpoint_name = if recent_only { "recentlyAdded" } else { "all" };
        let endpoint = server_url
            .join(&format!("library/sections/{key}/{endpoint_name}"))
            .map_err(|error| AppError::Repository(error.to_string()))?;
        let values = plex_paginated_entries(&verifier.client, endpoint, token, recent_only).await?;
        for value in values {
            let Some(item) =
                hydrate_plex_catalog_item(&verifier.client, &server_url, token, &value).await?
            else {
                continue;
            };
            if item.kind == MediaServerCatalogItemKind::Episode
                && let Some(series_id) = item.series_provider_item_id.as_deref()
            {
                let series_key = (MediaServerCatalogItemKind::Series, series_id.to_string());
                if !catalog_keys.contains(&series_key)
                    && let Some(series) =
                        plex_metadata_catalog_item(&verifier.client, &server_url, token, series_id)
                            .await?
                {
                    catalog_keys.insert(series_key);
                    catalog.push(series);
                }
            }
            let is_series = item.kind == MediaServerCatalogItemKind::Series;
            let series_id = item.provider_item_id.clone();
            if catalog_keys.insert((item.kind, item.provider_item_id.clone())) {
                catalog.push(item);
            }
            if is_series {
                let endpoint = server_url
                    .join(&format!("library/metadata/{series_id}/allLeaves"))
                    .map_err(|error| AppError::Repository(error.to_string()))?;
                for episode in plex_paginated_entries(&verifier.client, endpoint, token, false)
                    .await?
                    .iter()
                    .filter_map(plex_catalog_item)
                {
                    if catalog_keys.insert((episode.kind, episode.provider_item_id.clone())) {
                        catalog.push(episode);
                    }
                }
            }
        }
    }
    Ok(catalog)
}

async fn hydrate_plex_catalog_item(
    client: &reqwest::Client,
    server_url: &Url,
    token: &str,
    value: &Value,
) -> AppResult<Option<MediaServerCatalogItem>> {
    let Some(item) = plex_catalog_item(value) else {
        return Ok(None);
    };
    if item.kind == MediaServerCatalogItemKind::Episode || !item.external_ids.is_empty() {
        return Ok(Some(item));
    }
    Ok(
        plex_metadata_catalog_item(client, server_url, token, &item.provider_item_id)
            .await?
            .or(Some(item)),
    )
}

async fn plex_metadata_catalog_item(
    client: &reqwest::Client,
    server_url: &Url,
    token: &str,
    provider_item_id: &str,
) -> AppResult<Option<MediaServerCatalogItem>> {
    let endpoint = server_url
        .join(&format!("library/metadata/{provider_item_id}"))
        .map_err(|error| AppError::Repository(error.to_string()))?;
    let metadata = plex_json(client, endpoint, token).await?;
    Ok(plex_entries(&metadata, "Metadata")
        .first()
        .and_then(plex_catalog_item))
}

async fn plex_paginated_entries(
    client: &reqwest::Client,
    url: Url,
    token: &str,
    single_page: bool,
) -> AppResult<Vec<Value>> {
    const PAGE_SIZE: usize = 500;
    let mut start = 0usize;
    let mut entries = Vec::new();
    loop {
        let mut page_url = url.clone();
        page_url
            .query_pairs_mut()
            .append_pair("X-Plex-Container-Start", &start.to_string())
            .append_pair("X-Plex-Container-Size", &PAGE_SIZE.to_string())
            // Plex omits external identifiers unless this opt-in is present.
            // Request them with each catalog page so full scans avoid one
            // metadata request per movie or series just to obtain GUIDs.
            .append_pair("includeGuids", "1");
        let page = plex_json(client, page_url, token).await?;
        let page_entries = plex_entries(&page, "Metadata");
        let page_len = page_entries.len();
        let container = page.get("MediaContainer");
        let offset = container
            .and_then(|container| json_value_usize(container.get("offset")))
            .unwrap_or(start);
        let total = container.and_then(|container| json_value_usize(container.get("totalSize")));
        if start > 0 && offset < start {
            return Ok(entries);
        }
        entries.extend(page_entries);
        if single_page
            || page_len == 0
            || page_len < PAGE_SIZE
            || total.is_some_and(|total| offset + page_len >= total)
        {
            return Ok(entries);
        }
        start = offset + page_len;
    }
}

fn json_value_usize(value: Option<&Value>) -> Option<usize> {
    value
        .and_then(|value| {
            value
                .as_u64()
                .map(|value| value.to_string())
                .or_else(|| value.as_str().map(str::to_string))
        })?
        .parse()
        .ok()
}

async fn plex_json(client: &reqwest::Client, url: Url, token: &str) -> AppResult<Value> {
    let response = client
        .get(url)
        .header("Accept", "application/json")
        .header("X-Plex-Token", token)
        .send()
        .await
        .map_err(|error| AppError::Repository(format!("Plex catalog scan failed: {error}")))?;
    if !response.status().is_success() {
        return Err(AppError::Repository(format!(
            "Plex catalog scan failed with status {}",
            response.status()
        )));
    }
    response
        .json()
        .await
        .map_err(|error| AppError::Repository(format!("invalid Plex catalog response: {error}")))
}

fn plex_entries(value: &Value, key: &str) -> Vec<Value> {
    value
        .get("MediaContainer")
        .and_then(|container| container.get(key))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn plex_catalog_item(value: &Value) -> Option<MediaServerCatalogItem> {
    let kind = match value.get("type")?.as_str()? {
        "movie" => MediaServerCatalogItemKind::Movie,
        "show" => MediaServerCatalogItemKind::Series,
        "episode" => MediaServerCatalogItemKind::Episode,
        _ => return None,
    };
    Some(MediaServerCatalogItem {
        kind,
        provider_item_id: json_value_string(value.get("ratingKey"))?,
        external_ids: plex_provider_ids(value.get("Guid"))
            .into_iter()
            .chain(
                value
                    .get("guid")
                    .and_then(Value::as_str)
                    .and_then(plex_external_id),
            )
            .collect(),
        series_provider_item_id: value
            .get("grandparentRatingKey")
            .and_then(|value| json_value_string(Some(value))),
        season_number: value
            .get("parentIndex")
            .and_then(Value::as_i64)
            .map(|value| value as i32),
        episode_number: value
            .get("index")
            .and_then(Value::as_i64)
            .map(|value| value as i32),
        episode_number_end: None,
    })
}

fn plex_provider_ids(value: Option<&Value>) -> Vec<ExternalId> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|value| value.get("id").and_then(Value::as_str))
        .filter_map(plex_external_id)
        .collect()
}

fn plex_external_id(value: &str) -> Option<ExternalId> {
    let (raw_source, raw_value) = value.trim().split_once("://")?;
    let source = match raw_source.trim().to_ascii_lowercase().as_str() {
        "tmdb" | "themoviedb" | "com.plexapp.agents.tmdb" | "com.plexapp.agents.themoviedb" => {
            "tmdb"
        }
        "tvdb" | "thetvdb" | "com.plexapp.agents.thetvdb" => "tvdb",
        "imdb" | "com.plexapp.agents.imdb" => "imdb",
        _ => return None,
    };
    let external_id = raw_value
        .split('?')
        .next()
        .unwrap_or_default()
        .split('/')
        .next()
        .unwrap_or_default()
        .trim();
    if external_id.is_empty() {
        return None;
    }
    Some(ExternalId {
        source: source.into(),
        value: external_id.into(),
    })
}

fn provider_ids(value: Option<&Value>) -> Vec<ExternalId> {
    let Some(value) = value else {
        return Vec::new();
    };
    match value {
        Value::Object(values) => values
            .iter()
            .filter_map(|(source, value)| {
                value.as_str().map(|value| ExternalId {
                    source: source.clone(),
                    value: value.to_string(),
                })
            })
            .collect(),
        Value::Array(values) => values
            .iter()
            .filter_map(|value| value.get("id").and_then(Value::as_str))
            .filter_map(|value| {
                value.split_once("://").map(|(source, id)| ExternalId {
                    source: source.into(),
                    value: id.into(),
                })
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn media_server_url(value: &str) -> AppResult<Url> {
    let mut url = Url::parse(value.trim())
        .map_err(|error| AppError::Repository(format!("invalid media server URL: {error}")))?;
    if !url.path().ends_with('/') {
        url.set_path(&format!("{}/", url.path()));
    }
    Ok(url)
}

fn plex_server_url(resources_xml: &str, machine_id: &str) -> AppResult<Option<String>> {
    let mut reader = Reader::from_str(resources_xml);
    let mut selected = false;
    let mut uris = Vec::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) => {
                if element.name().as_ref() == "Device" {
                    selected = element.attributes().flatten().any(|attribute| {
                        matches!(
                            attribute.key.as_ref(),
                            "machineIdentifier" | "clientIdentifier"
                        ) && attribute
                            .normalized_value(XmlVersion::Implicit1_0)
                            .ok()
                            .as_deref()
                            == Some(machine_id)
                    });
                } else if selected
                    && element.name().as_ref() == "Connection"
                    && let Some(uri) = element
                        .attributes()
                        .flatten()
                        .find(|attribute| attribute.key.as_ref() == "uri")
                        .and_then(|attribute| {
                            attribute.normalized_value(XmlVersion::Implicit1_0).ok()
                        })
                        .map(|value| value.into_owned())
                {
                    uris.push(uri);
                }
            }
            Ok(Event::Empty(element)) if selected && element.name().as_ref() == "Connection" => {
                if let Some(uri) = element
                    .attributes()
                    .flatten()
                    .find(|attribute| attribute.key.as_ref() == "uri")
                    .and_then(|attribute| attribute.normalized_value(XmlVersion::Implicit1_0).ok())
                    .map(|value| value.into_owned())
                {
                    uris.push(uri);
                }
            }
            Ok(Event::End(element)) if element.name().as_ref() == "Device" => selected = false,
            Ok(Event::Eof) => {
                let https_uri = uris.iter().find(|uri| uri.starts_with("https://")).cloned();
                return Ok(https_uri.or_else(|| uris.into_iter().next()));
            }
            Err(error) => {
                return Err(AppError::Repository(format!(
                    "invalid Plex resources XML: {error}"
                )));
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use serde_json::json;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    fn verifier_with_plex(plex_base_url: Url) -> HttpExternalIdentityVerifier {
        HttpExternalIdentityVerifier::with_plex_base_url(plex_base_url)
    }

    #[test]
    fn plex_episode_catalog_item_accepts_string_parent_rating_key() {
        let item = plex_catalog_item(&json!({
            "type": "episode",
            "ratingKey": "episode-12",
            "grandparentRatingKey": "series-7",
            "parentIndex": 2,
            "index": 4
        }))
        .expect("episode should parse");

        assert_eq!(item.series_provider_item_id.as_deref(), Some("series-7"));
        assert_eq!(item.season_number, Some(2));
        assert_eq!(item.episode_number, Some(4));
    }

    #[test]
    fn plex_legacy_guids_map_only_compatible_external_ids() {
        assert_eq!(
            plex_external_id("com.plexapp.agents.themoviedb://550?lang=en"),
            Some(ExternalId {
                source: "tmdb".into(),
                value: "550".into(),
            })
        );
        assert_eq!(plex_external_id("plex://movie/internal-id"), None);
        assert_eq!(
            plex_provider_ids(Some(&json!([
                { "id": "plex://movie/internal-id" },
                { "id": "tmdb://550" }
            ]))),
            vec![ExternalId {
                source: "tmdb".into(),
                value: "550".into(),
            }]
        );
    }

    /// FR-088 on the Emby/Jellyfin side: one library-update notification
    /// carrying exactly the folders that changed, and nothing that would start a
    /// full library scan.
    #[tokio::test]
    async fn emby_refresh_posts_the_changed_paths_as_a_library_update() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/Library/Media/Updated"))
            .and(header("X-Emby-Token", "api-key"))
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

        refresh_emby_paths(
            &generic_reqwest_client(),
            &server.uri(),
            "api-key",
            &["/data/tv/Some Show", "/data/tv/Other Show"],
        )
        .await
        .expect("refresh");
    }

    #[tokio::test]
    async fn emby_refresh_surfaces_a_rejected_notification() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/Library/Media/Updated"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;

        let error = refresh_emby_paths(
            &generic_reqwest_client(),
            &server.uri(),
            "api-key",
            &["/data/tv/Some Show"],
        )
        .await
        .expect_err("a rejected notification is not a success");
        assert!(error.to_string().contains("401"), "{error}");
    }

    #[test]
    fn plex_sections_are_matched_by_their_configured_locations() {
        let sections = json!({
            "MediaContainer": {
                "Directory": [
                    {
                        "key": "1",
                        "type": "movie",
                        "Location": [{ "path": "/data/movies" }]
                    },
                    {
                        "key": "2",
                        "type": "show",
                        "Location": [{ "path": "/data/tv" }, { "path": "/data/tv-4k" }]
                    },
                    {
                        "key": "3",
                        "type": "artist",
                        "Location": [{ "path": "/data/music" }]
                    }
                ]
            }
        });
        let locations = plex_section_locations(&sections);
        assert_eq!(locations.len(), 3, "music sections are not video libraries");

        assert_eq!(
            plex_section_for_path(&locations, "/data/tv/Some Show").as_deref(),
            Some("2")
        );
        assert_eq!(
            plex_section_for_path(&locations, "/data/tv-4k/Some Show").as_deref(),
            Some("2")
        );
        assert_eq!(
            plex_section_for_path(&locations, "/data/movies").as_deref(),
            Some("1")
        );
        assert_eq!(
            plex_section_for_path(&locations, "/data/music/Some Band"),
            None,
            "a non-video section is never partially scanned"
        );
        assert_eq!(
            plex_section_for_path(&locations, "/elsewhere/Some Show"),
            None,
            "a folder no section covers is skipped, never widened into a full scan"
        );
    }

    #[test]
    fn the_most_specific_plex_section_wins() {
        let locations = vec![
            ("1".to_string(), "/data".to_string()),
            ("2".to_string(), "/data/tv".to_string()),
        ];
        assert_eq!(
            plex_section_for_path(&locations, "/data/tv/Some Show").as_deref(),
            Some("2")
        );
    }

    #[tokio::test]
    async fn plex_catalog_pagination_reads_every_page() {
        let server = MockServer::start().await;
        let first_page = (0..500)
            .map(|index| json!({ "ratingKey": index.to_string() }))
            .collect::<Vec<_>>();
        Mock::given(method("GET"))
            .and(path("/library"))
            .and(query_param("X-Plex-Container-Start", "0"))
            .and(query_param("X-Plex-Container-Size", "500"))
            .and(query_param("includeGuids", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "MediaContainer": {
                    "offset": 0,
                    "size": 500,
                    "totalSize": 501,
                    "Metadata": first_page
                }
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/library"))
            .and(query_param("X-Plex-Container-Start", "500"))
            .and(query_param("X-Plex-Container-Size", "500"))
            .and(query_param("includeGuids", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "MediaContainer": {
                    "offset": 500,
                    "size": 1,
                    "totalSize": 501,
                    "Metadata": [{ "ratingKey": "500" }]
                }
            })))
            .expect(1)
            .mount(&server)
            .await;

        let entries = plex_paginated_entries(
            &generic_reqwest_client(),
            Url::parse(&format!("{}/library", server.uri())).expect("mock URL"),
            "token",
            false,
        )
        .await
        .expect("paginate Plex library");

        assert_eq!(entries.len(), 501);
        assert_eq!(
            entries.last().and_then(|item| item.get("ratingKey")),
            Some(&json!("500"))
        );
    }

    #[tokio::test]
    async fn plex_catalog_hydrates_missing_external_guids_from_metadata() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/library/metadata/10"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "MediaContainer": {
                    "Metadata": [{
                        "type": "movie",
                        "ratingKey": "10",
                        "guid": "plex://movie/internal",
                        "Guid": [{ "id": "tmdb://550" }]
                    }]
                }
            })))
            .expect(1)
            .mount(&server)
            .await;
        let server_url = media_server_url(&server.uri()).expect("server URL");

        let item = hydrate_plex_catalog_item(
            &generic_reqwest_client(),
            &server_url,
            "token",
            &json!({
                "type": "movie",
                "ratingKey": "10",
                "guid": "plex://movie/internal",
                "Guid": [{ "id": "plex://movie/internal" }]
            }),
        )
        .await
        .expect("hydrate metadata")
        .expect("movie should parse");

        assert_eq!(
            item.external_ids,
            vec![ExternalId {
                source: "tmdb".into(),
                value: "550".into(),
            }]
        );
    }

    #[tokio::test]
    async fn jellyfin_verification_succeeds() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/Users/AuthenticateByName"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "User": {
                    "Id": "jf-user",
                    "Name": "Jelly User",
                    "PrimaryImageTag": "tag"
                }
            })))
            .mount(&server)
            .await;
        let verifier = HttpExternalIdentityVerifier::new();

        let verified = verifier
            .verify_jellyfin("jellyfin-main", &server.uri(), "jelly", "secret")
            .await
            .expect("verify jellyfin");

        assert_eq!(verified.provider, ExternalAccountProvider::Jellyfin);
        assert_eq!(verified.external_user_id, "jf-user");
        assert_eq!(verified.username, "Jelly User");
        let expected_avatar_url = format!("{}/Users/jf-user/Images/Primary?tag=tag", server.uri());
        assert_eq!(
            verified.avatar_url.as_deref(),
            Some(expected_avatar_url.as_str())
        );
        // Payload omits the password flags, so the fact is unknown rather than
        // "no password" — a server that never reports it must stay usable.
        assert_eq!(verified.remote_password_configured, None);
    }

    #[tokio::test]
    async fn jellyfin_verification_reports_whether_the_account_has_a_password() {
        // Field names and values here match what Jellyfin 10.11.5 actually
        // returns from Users/AuthenticateByName.
        for (has_password, has_configured_password, expected) in
            [(false, false, Some(false)), (true, true, Some(true))]
        {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/Users/AuthenticateByName"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "User": {
                        "Id": "jf-user",
                        "Name": "Jelly User",
                        "HasPassword": has_password,
                        "HasConfiguredPassword": has_configured_password
                    }
                })))
                .mount(&server)
                .await;
            let verifier = HttpExternalIdentityVerifier::new();

            let verified = verifier
                .verify_jellyfin("jellyfin-main", &server.uri(), "jelly", "secret")
                .await
                .expect("verify jellyfin");

            // The verifier reports the fact and never refuses on it; refusing is
            // the login use case's job, so linking keeps working either way.
            assert_eq!(
                verified.remote_password_configured, expected,
                "HasPassword={has_password} HasConfiguredPassword={has_configured_password}"
            );
        }
    }

    #[tokio::test]
    async fn jellyfin_user_listing_returns_avatar_urls() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/Users"))
            .and(header("x-emby-token", "api-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
                "Id": "jf-user",
                "Name": "Jelly User",
                "PrimaryImageTag": "avatar-tag"
            }])))
            .mount(&server)
            .await;
        let verifier = HttpExternalIdentityVerifier::new();

        let users = verifier
            .list_jellyfin_users(&server.uri(), "api-key", None)
            .await
            .expect("list jellyfin users");

        assert_eq!(users.len(), 1);
        assert_eq!(users[0].username, "Jelly User");
        let expected_avatar_url = format!(
            "{}/Users/jf-user/Images/Primary?tag=avatar-tag",
            server.uri()
        );
        assert_eq!(
            users[0].avatar_url.as_deref(),
            Some(expected_avatar_url.as_str())
        );
    }

    #[tokio::test]
    async fn jellyfin_connection_test_succeeds() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/System/Info/Public"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "ProductName": "Jellyfin Server"
            })))
            .mount(&server)
            .await;
        let verifier = HttpExternalIdentityVerifier::new();

        verifier
            .test_jellyfin_connection(&server.uri())
            .await
            .expect("test jellyfin connection");
    }

    #[tokio::test]
    async fn jellyfin_connection_test_reports_failure_status() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/System/Info/Public"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        let verifier = HttpExternalIdentityVerifier::new();

        let result = verifier.test_jellyfin_connection(&server.uri()).await;

        assert!(matches!(result, Err(AppError::Repository(_))));
    }

    #[tokio::test]
    async fn jellyfin_invalid_credentials_are_unauthorized() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/Users/AuthenticateByName"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        let verifier = HttpExternalIdentityVerifier::new();

        let result = verifier
            .verify_jellyfin("jellyfin-main", &server.uri(), "jelly", "bad")
            .await;

        assert!(matches!(result, Err(AppError::Unauthorized(_))));
    }

    #[tokio::test]
    async fn jellyfin_admin_exchange_creates_key_then_reads_keys() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/Users/AuthenticateByName"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "AccessToken": "admin-token",
                "User": {
                    "Id": "admin-user",
                    "Name": "Admin User",
                    "Policy": { "IsAdministrator": true }
                }
            })))
            .expect(1)
            .mount(&server)
            .await;
        let key_list_calls = Arc::new(AtomicUsize::new(0));
        let key_list_calls_for_mock = Arc::clone(&key_list_calls);
        Mock::given(method("GET"))
            .and(path("/Auth/Keys"))
            .and(header("x-emby-token", "admin-token"))
            .respond_with(move |_request: &wiremock::Request| {
                if key_list_calls_for_mock.fetch_add(1, Ordering::SeqCst) == 0 {
                    ResponseTemplate::new(200).set_body_json(json!({
                        "Items": [],
                        "TotalRecordCount": 0
                    }))
                } else {
                    ResponseTemplate::new(200).set_body_json(json!({
                        "Items": [{
                            "AppName": "Scryer",
                            "AccessToken": "generated-token",
                            "DateCreated": "2026-05-30T00:00:00.0000000Z"
                        }],
                        "TotalRecordCount": 1
                    }))
                }
            })
            .expect(2)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/Auth/Keys"))
            .and(header("x-emby-token", "admin-token"))
            .and(query_param("app", SCRYER_PRODUCT))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        let verifier = HttpExternalIdentityVerifier::new();

        let api_key = verifier
            .exchange_jellyfin_admin_api_key("jellyfin-main", &server.uri(), "admin", "secret")
            .await
            .expect("exchange jellyfin admin credentials for api key");

        assert_eq!(api_key, "generated-token");
        assert_eq!(key_list_calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn plex_token_verification_succeeds() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/users/account.json"))
            .and(header("x-plex-token", "token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "user": {
                    "id": 123,
                    "username": "plexuser",
                    "title": "Plex User",
                    "thumb": "https://plex.tv/avatar.jpg"
                }
            })))
            .mount(&server)
            .await;
        let verifier = verifier_with_plex(Url::parse(&server.uri()).expect("mock URL"));

        let verified = verifier
            .verify_plex("plex-main", None, "token")
            .await
            .expect("verify plex");

        assert_eq!(verified.provider, ExternalAccountProvider::Plex);
        assert_eq!(verified.external_user_id, "123");
        assert_eq!(verified.username, "plexuser");
        assert_eq!(verified.display_name.as_deref(), Some("Plex User"));
        assert_eq!(
            verified.avatar_url.as_deref(),
            Some("https://plex.tv/avatar.jpg")
        );
    }

    #[tokio::test]
    async fn plex_invalid_token_is_unauthorized() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/users/account.json"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        let verifier = verifier_with_plex(Url::parse(&server.uri()).expect("mock URL"));

        let result = verifier.verify_plex("plex-main", None, "bad-token").await;

        assert!(matches!(result, Err(AppError::Unauthorized(_))));
    }

    #[tokio::test]
    async fn plex_user_listing_returns_shared_users() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/users/"))
            .and(header("x-plex-token", "token"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"<MediaContainer>
                    <User id="42" username="plexfriend" title="Plex Friend" thumb="https://plex.tv/friend.jpg" email="friend@example.test" />
                    <User id="99" title="Title Fallback" />
                </MediaContainer>"#,
            ))
            .mount(&server)
            .await;
        let verifier = verifier_with_plex(Url::parse(&server.uri()).expect("mock URL"));

        let users = verifier
            .list_plex_users("token", Some("friend@example"))
            .await
            .expect("list plex users");

        assert_eq!(users.len(), 1);
        assert_eq!(users[0].id, "42");
        assert_eq!(users[0].username, "plexfriend");
        assert_eq!(users[0].display_name.as_deref(), Some("Plex Friend"));
        assert_eq!(
            users[0].avatar_url.as_deref(),
            Some("https://plex.tv/friend.jpg")
        );
    }

    #[tokio::test]
    async fn plex_user_listing_invalid_token_is_unauthorized() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/users/"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        let verifier = verifier_with_plex(Url::parse(&server.uri()).expect("mock URL"));

        let result = verifier.list_plex_users("bad-token", None).await;

        assert!(matches!(result, Err(AppError::Unauthorized(_))));
    }

    #[tokio::test]
    async fn plex_machine_match_succeeds() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/users/account.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "user": { "id": "plex-user", "username": "plexuser" }
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/resources"))
            .and(query_param("includeHttps", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"<MediaContainer><Device machineIdentifier="machine-1" /></MediaContainer>"#,
            ))
            .mount(&server)
            .await;
        let verifier = verifier_with_plex(Url::parse(&server.uri()).expect("mock URL"));

        verifier
            .verify_plex("plex-main", Some("machine-1"), "token")
            .await
            .expect("machine should match");
    }

    #[tokio::test]
    async fn plex_client_identifier_machine_match_succeeds() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/users/account.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "user": { "id": "plex-user", "username": "plexuser" }
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/resources"))
            .and(query_param("includeHttps", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"<MediaContainer><Device clientIdentifier="machine-1" provides="server" /></MediaContainer>"#,
            ))
            .mount(&server)
            .await;
        let verifier = verifier_with_plex(Url::parse(&server.uri()).expect("mock URL"));

        verifier
            .verify_plex("plex-main", Some("machine-1"), "token")
            .await
            .expect("client identifier should match");
    }

    #[test]
    fn plex_server_discovery_uses_client_identifier() {
        let servers = plex_server_discoveries(
            r#"<MediaContainer><Device clientIdentifier="machine-1" name="E2E Plex" provides="server"><Connection protocol="http" address="plex-auth" port="32400" uri="http://plex-auth:32400" local="0" /><Connection protocol="https" address="172.21.0.2" port="32400" uri="https://172-21-0-2.example.plex.direct:32400" local="1" /></Device></MediaContainer>"#,
        )
        .expect("resources should parse");

        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].id, "machine-1");
        assert_eq!(servers[0].name, "E2E Plex");
    }

    #[tokio::test]
    async fn plex_machine_mismatch_is_unauthorized() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/users/account.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "user": { "id": "plex-user", "username": "plexuser" }
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/resources"))
            .and(query_param("includeHttps", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"<MediaContainer><Device machineIdentifier="other-machine" /></MediaContainer>"#,
            ))
            .mount(&server)
            .await;
        let verifier = verifier_with_plex(Url::parse(&server.uri()).expect("mock URL"));

        let result = verifier
            .verify_plex("plex-main", Some("machine-1"), "token")
            .await;

        assert!(matches!(result, Err(AppError::Unauthorized(_))));
    }
}

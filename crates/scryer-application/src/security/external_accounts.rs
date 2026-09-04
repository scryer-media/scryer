use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExternalAuthRuntimeConnection {
    pub id: String,
    pub provider: scryer_domain::ExternalAccountProvider,
    pub display_name: String,
    pub login_enabled: bool,
    pub linking_enabled: bool,
    pub emby_connect_enabled: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExternalAuthRuntimeSettings {
    pub login_providers: Vec<scryer_domain::ExternalAccountProvider>,
    pub linking_providers: Vec<scryer_domain::ExternalAccountProvider>,
    pub connections: Vec<ExternalAuthRuntimeConnection>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExternalAuthUse {
    Login,
    Linking,
    Invite,
}

fn normalize_provider_username(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

impl AppUseCase {
    pub async fn get_external_auth_runtime_settings(
        &self,
    ) -> AppResult<ExternalAuthRuntimeSettings> {
        let media_connections = self
            .services
            .integrations
            .media_server_connections
            .list(None)
            .await?;
        let mut connections = Vec::new();
        let mut login_providers = Vec::new();
        let mut linking_providers = Vec::new();

        for connection in media_connections
            .into_iter()
            .filter(|connection| connection.enabled)
        {
            if !connection.login_enabled && !connection.linking_enabled {
                continue;
            }
            let provider = match connection.provider {
                scryer_domain::MediaServerProvider::Jellyfin => {
                    scryer_domain::ExternalAccountProvider::Jellyfin
                }
                scryer_domain::MediaServerProvider::Plex => {
                    if connection.machine_id.is_none() {
                        continue;
                    }
                    scryer_domain::ExternalAccountProvider::Plex
                }
                scryer_domain::MediaServerProvider::Emby => {
                    if connection.emby_server_id.is_none() {
                        continue;
                    }
                    scryer_domain::ExternalAccountProvider::Emby
                }
            };
            if connection.login_enabled && !login_providers.contains(&provider) {
                login_providers.push(provider.clone());
            }
            if connection.linking_enabled && !linking_providers.contains(&provider) {
                linking_providers.push(provider.clone());
            }
            connections.push(ExternalAuthRuntimeConnection {
                id: connection.id,
                provider,
                display_name: connection.display_name,
                login_enabled: connection.login_enabled,
                linking_enabled: connection.linking_enabled,
                emby_connect_enabled: connection.emby_connect_enabled,
            });
        }

        Ok(ExternalAuthRuntimeSettings {
            login_providers,
            linking_providers,
            connections,
        })
    }

    async fn auth_connection_for_use(
        &self,
        provider: scryer_domain::ExternalAccountProvider,
        connection_id: &str,
        usage: ExternalAuthUse,
    ) -> AppResult<scryer_domain::MediaServerConnection> {
        let connection_id = connection_id.trim();
        let connection = self
            .services
            .integrations
            .media_server_connections
            .get_by_id(connection_id)
            .await?
            .ok_or_else(|| {
                AppError::Validation(format!(
                    "{} connection is not configured",
                    provider.as_str()
                ))
            })?;
        let expected_provider = match provider {
            scryer_domain::ExternalAccountProvider::Jellyfin => {
                scryer_domain::MediaServerProvider::Jellyfin
            }
            scryer_domain::ExternalAccountProvider::Plex => {
                scryer_domain::MediaServerProvider::Plex
            }
            scryer_domain::ExternalAccountProvider::Emby => {
                scryer_domain::MediaServerProvider::Emby
            }
        };
        if connection.provider != expected_provider {
            return Err(AppError::Validation(format!(
                "{} connection is not configured for external auth",
                provider.as_str()
            )));
        }
        if !connection.enabled {
            return Err(AppError::Validation(format!(
                "{} connection is disabled",
                provider.as_str()
            )));
        }
        let enabled = match usage {
            ExternalAuthUse::Login | ExternalAuthUse::Invite => connection.login_enabled,
            ExternalAuthUse::Linking => connection.linking_enabled,
        };
        if !enabled {
            return Err(AppError::Validation(format!(
                "{} is not enabled for {}",
                provider.as_str(),
                match usage {
                    ExternalAuthUse::Login => "login",
                    ExternalAuthUse::Linking => "linking",
                    ExternalAuthUse::Invite => "invites",
                }
            )));
        }
        if provider == scryer_domain::ExternalAccountProvider::Plex
            && connection.machine_id.is_none()
        {
            return Err(AppError::Validation(
                "Plex server discovery is required before using Plex for auth".into(),
            ));
        }
        Ok(connection)
    }

    fn ensure_verified_identity_matches_request(
        &self,
        expected_provider: &scryer_domain::ExternalAccountProvider,
        expected_connection_id: &str,
        verified: &VerifiedExternalIdentity,
    ) -> AppResult<()> {
        if &verified.provider != expected_provider {
            return Err(AppError::Validation(
                "verified external identity provider did not match the requested provider".into(),
            ));
        }
        if verified.connection_id.trim() != expected_connection_id.trim() {
            return Err(AppError::Validation(
                "verified external identity connection did not match the requested connection"
                    .into(),
            ));
        }
        Ok(())
    }

    pub async fn list_linked_accounts(
        &self,
        actor: &User,
        user_id: Option<&str>,
    ) -> AppResult<Vec<scryer_domain::UserExternalAccount>> {
        let target_user_id = user_id.unwrap_or(&actor.id);
        if target_user_id != actor.id {
            self.require_app_permission(actor, scryer_domain::AppPermission::ManageUsers)
                .await?;
        } else {
            self.require_actor_capability(actor, scryer_domain::ActorCapability::ManageOwnAccount)
                .await?;
        }
        self.services
            .identity
            .external_accounts
            .list_by_user_id(target_user_id)
            .await
    }

    pub async fn list_external_account_invites(
        &self,
        actor: &User,
    ) -> AppResult<Vec<scryer_domain::UserExternalAccount>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageUsers)
            .await?;
        let users = self.services.identity.users.list_all().await?;
        let mut accounts = Vec::new();
        for user in users {
            accounts.extend(
                self.services
                    .identity
                    .external_accounts
                    .list_by_user_id(&user.id)
                    .await?,
            );
        }
        accounts.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.provider.as_str().cmp(right.provider.as_str()))
                .then_with(|| left.username.cmp(&right.username))
        });
        Ok(accounts)
    }

    pub async fn repair_legacy_jellyfin_external_account_invites(&self) -> AppResult<()> {
        let connections = self
            .services
            .integrations
            .media_server_connections
            .list(Some(scryer_domain::MediaServerProvider::Jellyfin))
            .await?
            .into_iter()
            .map(|connection| (connection.id.clone(), connection))
            .collect::<std::collections::HashMap<_, _>>();
        let users = self.services.identity.users.list_all().await?;

        for user in users {
            let accounts = self
                .services
                .identity
                .external_accounts
                .list_by_user_id(&user.id)
                .await?;
            for mut account in accounts {
                if account.provider != scryer_domain::ExternalAccountProvider::Jellyfin
                    || account.external_user_id.is_some()
                    || account.status != scryer_domain::ExternalAccountStatus::PendingClaim
                {
                    continue;
                }

                let resolved = if let Some(connection) = connections.get(&account.connection_id)
                    && let Some(api_key) = connection.api_key.as_deref()
                {
                    match self
                        .services
                        .integrations
                        .external_identity_verifier
                        .list_jellyfin_users(&connection.base_url, api_key, Some(&account.username))
                        .await
                    {
                        Ok(users) => {
                            let normalized = normalize_provider_username(&account.username);
                            let matches = users
                                .into_iter()
                                .filter(|user| {
                                    normalize_provider_username(&user.username) == normalized
                                })
                                .collect::<Vec<_>>();
                            (matches.len() == 1).then(|| matches[0].clone())
                        }
                        Err(error) => {
                            tracing::warn!(
                                error = %error,
                                account_id = %account.id,
                                connection_id = %account.connection_id,
                                "failed to resolve legacy Jellyfin invite"
                            );
                            None
                        }
                    }
                } else {
                    None
                };

                if let Some(resolved) = resolved {
                    account.external_user_id = Some(resolved.id);
                    account.username = resolved.username;
                    account.display_name = resolved.display_name;
                    account.avatar_url = resolved.avatar_url;
                    account.updated_at = Utc::now();
                    tracing::info!(
                        account_id = %account.id,
                        connection_id = %account.connection_id,
                        "repaired legacy Jellyfin invite with immutable external user id"
                    );
                } else {
                    account.status = scryer_domain::ExternalAccountStatus::Disabled;
                    account.updated_at = Utc::now();
                    tracing::warn!(
                        account_id = %account.id,
                        connection_id = %account.connection_id,
                        "disabled legacy Jellyfin invite that could not be resolved to one immutable external user id"
                    );
                }

                self.services
                    .identity
                    .external_accounts
                    .update(account)
                    .await?;
            }
        }

        Ok(())
    }

    pub async fn create_external_account_invite(
        &self,
        actor: &User,
        user_id: &str,
        provider: scryer_domain::ExternalAccountProvider,
        connection_id: String,
        provider_user_identifier: String,
        provider_user_id: Option<String>,
    ) -> AppResult<scryer_domain::UserExternalAccount> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageUsers)
            .await?;
        let connection_id = normalize_connection_id(connection_id);
        let _provider_user_identifier = provider_user_identifier.trim();
        let connection = self
            .auth_connection_for_use(provider.clone(), &connection_id, ExternalAuthUse::Invite)
            .await?;
        self.services
            .identity
            .users
            .get_by_id(user_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("user {user_id}")))?;

        let (external_user_id, username, display_name, avatar_url) = match provider {
            scryer_domain::ExternalAccountProvider::Jellyfin => {
                let jellyfin_user_id = provider_user_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| {
                        AppError::Validation(
                            "Jellyfin invites require selecting a Jellyfin user from the picker"
                                .into(),
                        )
                    })?;
                let api_key = connection.api_key.as_deref().ok_or_else(|| {
                    AppError::Validation(
                        "Jellyfin invites require a saved Jellyfin API key so Scryer can resolve the immutable user id".into(),
                    )
                })?;
                let user = self
                    .services
                    .integrations
                    .external_identity_verifier
                    .list_jellyfin_users(&connection.base_url, api_key, Some(jellyfin_user_id))
                    .await?
                    .into_iter()
                    .find(|user| user.id.eq_ignore_ascii_case(jellyfin_user_id))
                    .ok_or_else(|| {
                        AppError::Validation(
                            "selected Jellyfin user was not found on the configured server".into(),
                        )
                    })?;
                if self
                    .services
                    .identity
                    .external_accounts
                    .get_by_provider_identity(provider.clone(), &connection_id, &user.id)
                    .await?
                    .is_some()
                {
                    return Err(AppError::Validation(
                        "Jellyfin account is already linked or invited".into(),
                    ));
                }
                (
                    Some(user.id),
                    user.username,
                    user.display_name,
                    user.avatar_url,
                )
            }
            scryer_domain::ExternalAccountProvider::Plex => {
                let plex_user_id = provider_user_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| {
                        AppError::Validation(
                            "Plex invites require selecting a Plex user from the picker".into(),
                        )
                    })?;
                let plex_auth_token = connection.api_key.as_deref().ok_or_else(|| {
                    AppError::Validation(
                        "Plex invites require a saved Plex token so Scryer can resolve the immutable user id".into(),
                    )
                })?;
                let user = self
                    .services
                    .integrations
                    .external_identity_verifier
                    .list_plex_users(plex_auth_token, Some(plex_user_id))
                    .await?
                    .into_iter()
                    .find(|user| user.id.eq_ignore_ascii_case(plex_user_id))
                    .ok_or_else(|| {
                        AppError::Validation(
                            "selected Plex user was not found on the configured account".into(),
                        )
                    })?;
                if self
                    .services
                    .identity
                    .external_accounts
                    .get_by_provider_identity(provider.clone(), &connection_id, &user.id)
                    .await?
                    .is_some()
                {
                    return Err(AppError::Validation(
                        "Plex account is already linked or invited".into(),
                    ));
                }
                (
                    Some(user.id),
                    user.username,
                    user.display_name,
                    user.avatar_url,
                )
            }
            scryer_domain::ExternalAccountProvider::Emby => {
                let emby_user_id = provider_user_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| {
                        AppError::Validation(
                            "Emby invites require selecting an Emby user from the picker".into(),
                        )
                    })?;
                let api_key = connection.api_key.as_deref().ok_or_else(|| {
                    AppError::Validation("Emby invites require a saved integration API key".into())
                })?;
                let user = self
                    .services
                    .integrations
                    .external_identity_verifier
                    .list_emby_users(
                        &connection.id,
                        &connection.base_url,
                        api_key,
                        Some(emby_user_id),
                    )
                    .await?
                    .into_iter()
                    .find(|user| user.id.eq_ignore_ascii_case(emby_user_id))
                    .ok_or_else(|| {
                        AppError::Validation(
                            "selected Emby user was not found on the configured server".into(),
                        )
                    })?;
                if self
                    .services
                    .identity
                    .external_accounts
                    .get_by_provider_identity(provider.clone(), &connection_id, &user.id)
                    .await?
                    .is_some()
                {
                    return Err(AppError::Validation(
                        "Emby account is already linked or invited".into(),
                    ));
                }
                (
                    Some(user.id),
                    user.username,
                    user.display_name,
                    user.avatar_url,
                )
            }
        };

        let mut account = scryer_domain::UserExternalAccount::pending_claim(
            user_id.to_string(),
            provider,
            connection_id,
            external_user_id,
            username,
        );
        account.display_name = display_name;
        account.avatar_url = avatar_url;
        self.services
            .identity
            .external_accounts
            .create(account)
            .await
    }

    pub async fn link_plex_account(
        &self,
        actor: &User,
        connection_id: String,
        plex_auth_token: String,
    ) -> AppResult<scryer_domain::UserExternalAccount> {
        self.require_actor_capability(actor, scryer_domain::ActorCapability::ManageOwnAccount)
            .await?;
        let provider = scryer_domain::ExternalAccountProvider::Plex;
        let connection_id = normalize_connection_id(connection_id);
        let connection = self
            .auth_connection_for_use(provider.clone(), &connection_id, ExternalAuthUse::Linking)
            .await?;
        let verified = self
            .services
            .integrations
            .external_identity_verifier
            .verify_plex(
                &connection_id,
                connection.machine_id.as_deref(),
                &plex_auth_token,
            )
            .await?;
        self.ensure_verified_identity_matches_request(&provider, &connection_id, &verified)?;
        self.link_verified_external_account(actor, verified).await
    }

    pub async fn link_jellyfin_account(
        &self,
        actor: &User,
        connection_id: String,
        username: String,
        password: String,
    ) -> AppResult<scryer_domain::UserExternalAccount> {
        self.require_actor_capability(actor, scryer_domain::ActorCapability::ManageOwnAccount)
            .await?;
        let provider = scryer_domain::ExternalAccountProvider::Jellyfin;
        let connection_id = normalize_connection_id(connection_id);
        let connection = self
            .auth_connection_for_use(provider.clone(), &connection_id, ExternalAuthUse::Linking)
            .await?;
        let verified = self
            .services
            .integrations
            .external_identity_verifier
            .verify_jellyfin(&connection_id, &connection.base_url, &username, &password)
            .await?;
        self.ensure_verified_identity_matches_request(&provider, &connection_id, &verified)?;
        self.link_verified_external_account(actor, verified).await
    }

    pub async fn link_emby_account(
        &self,
        actor: &User,
        connection_id: String,
        mode: EmbyConnectionMode,
        username: String,
        password: String,
    ) -> AppResult<scryer_domain::UserExternalAccount> {
        self.require_actor_capability(actor, scryer_domain::ActorCapability::ManageOwnAccount)
            .await?;
        let provider = scryer_domain::ExternalAccountProvider::Emby;
        let connection_id = normalize_connection_id(connection_id);
        let connection = self
            .auth_connection_for_use(provider.clone(), &connection_id, ExternalAuthUse::Linking)
            .await?;
        let (verified, refreshed_base_url) = self
            .verify_emby_identity(&connection, mode, &username, &password)
            .await
            .map_err(|_| AppError::Unauthorized("Emby sign-in failed".into()))?;
        if password.is_empty() && verified.remote_password_configured != Some(false) {
            return Err(AppError::Unauthorized("Emby sign-in failed".into()));
        }
        self.refresh_emby_base_url_if_needed(&connection, refreshed_base_url.as_deref())
            .await;
        self.ensure_verified_identity_matches_request(&provider, &connection_id, &verified)?;
        self.link_verified_external_account(actor, verified).await
    }

    async fn verify_emby_identity(
        &self,
        connection: &scryer_domain::MediaServerConnection,
        mode: EmbyConnectionMode,
        username: &str,
        password: &str,
    ) -> AppResult<(VerifiedExternalIdentity, Option<String>)> {
        let server_id = connection.emby_server_id.as_deref().ok_or_else(|| {
            AppError::Validation("Emby connection has no verified server identity".into())
        })?;
        match mode {
            EmbyConnectionMode::Local => self
                .services
                .integrations
                .external_identity_verifier
                .verify_emby_local_identity(
                    &connection.id,
                    &connection.base_url,
                    server_id,
                    username,
                    password,
                )
                .await
                .map(|identity| (identity, None)),
            EmbyConnectionMode::Connect => {
                if !connection.emby_connect_enabled {
                    return Err(AppError::Validation(
                        "Emby Connect sign-in is disabled for this connection".into(),
                    ));
                }
                self.services
                    .integrations
                    .external_identity_verifier
                    .verify_emby_connect_identity(
                        &connection.id,
                        &connection.base_url,
                        server_id,
                        username,
                        password,
                    )
                    .await
                    .map(|verification| {
                        let refreshed = (verification.resolved_api_base_url != connection.base_url)
                            .then_some(verification.resolved_api_base_url);
                        (verification.identity, refreshed)
                    })
            }
        }
    }

    async fn refresh_emby_base_url_if_needed(
        &self,
        connection: &scryer_domain::MediaServerConnection,
        refreshed_base_url: Option<&str>,
    ) {
        let Some(refreshed_base_url) = refreshed_base_url else {
            return;
        };
        let Some(server_id) = connection.emby_server_id.as_deref() else {
            return;
        };
        if let Err(error) = self
            .services
            .integrations
            .media_server_connections
            .compare_and_set_emby_base_url(
                &connection.id,
                &connection.base_url,
                server_id,
                refreshed_base_url,
            )
            .await
        {
            tracing::warn!(
                connection_id = connection.id,
                operation = "emby_connect_address_refresh",
                error_class = %error,
                "Emby Connect address refresh failed after successful authentication"
            );
        }
    }

    async fn link_verified_external_account(
        &self,
        actor: &User,
        verified: VerifiedExternalIdentity,
    ) -> AppResult<scryer_domain::UserExternalAccount> {
        let existing = self
            .services
            .identity
            .external_accounts
            .get_by_provider_identity(
                verified.provider.clone(),
                &verified.connection_id,
                &verified.external_user_id,
            )
            .await?;

        if let Some(mut existing) = existing {
            return self
                .refresh_verified_external_account(actor, &mut existing, verified)
                .await;
        }

        let now = Utc::now();
        let account = scryer_domain::UserExternalAccount {
            id: scryer_domain::Id::new().0,
            user_id: actor.id.clone(),
            provider: verified.provider.clone(),
            connection_id: verified.connection_id.clone(),
            external_user_id: Some(verified.external_user_id.clone()),
            username: verified.username.clone(),
            display_name: verified.display_name.clone(),
            avatar_url: verified.avatar_url.clone(),
            status: scryer_domain::ExternalAccountStatus::Active,
            verified_at: Some(now),
            last_login_at: None,
            created_at: now,
            updated_at: now,
        };
        let created_account_id = account.id.clone();
        let mut account = self
            .services
            .identity
            .external_accounts
            .create_or_get_by_provider_identity(account)
            .await?;
        if account.id == created_account_id {
            return Ok(account);
        }
        self.refresh_verified_external_account(actor, &mut account, verified)
            .await
    }

    async fn refresh_verified_external_account(
        &self,
        actor: &User,
        existing: &mut scryer_domain::UserExternalAccount,
        verified: VerifiedExternalIdentity,
    ) -> AppResult<scryer_domain::UserExternalAccount> {
        if existing.user_id != actor.id {
            return Err(AppError::Validation(
                "external account is already linked to another Scryer user".into(),
            ));
        }
        if matches!(
            existing.status,
            scryer_domain::ExternalAccountStatus::Disabled
        ) {
            return Err(AppError::Validation(
                "external account is disabled and must be repaired by an administrator".into(),
            ));
        }
        existing.external_user_id = Some(verified.external_user_id);
        existing.username = verified.username;
        existing.display_name = verified.display_name;
        existing.avatar_url = verified.avatar_url;
        existing.status = scryer_domain::ExternalAccountStatus::Active;
        let now = Utc::now();
        existing.verified_at = Some(now);
        existing.updated_at = now;
        self.services
            .identity
            .external_accounts
            .update(existing.clone())
            .await
    }

    /// Links the OAuth caller to the Jellyfin identity bound to that OAuth
    /// grant. The caller cannot select either a Scryer user or a connection.
    pub async fn link_current_oauth_jellyfin_account(
        &self,
        actor: &User,
        client_id: &str,
        grant_id: &str,
        jellyfin_user_id: &str,
    ) -> AppResult<scryer_domain::UserExternalAccount> {
        let canonical_jellyfin_user_id = crate::canonicalize_jellyfin_user_id(jellyfin_user_id)
            .ok_or_else(|| AppError::Validation("Jellyfin user ID is invalid".into()))?;
        let grant = self
            .services
            .identity
            .oauth
            .get_refresh_grant(grant_id)
            .await?
            .ok_or_else(|| AppError::Unauthorized("OAuth grant is unavailable".into()))?;
        if grant.user_id != actor.id
            || grant.client_id != client_id
            || grant.revoked_at.is_some()
            || !grant.authorization_source.is_authenticated()
            || !crate::oauth::oauth_scope_has_jellyfin_link(&grant.scope)
        {
            return Err(AppError::Unauthorized(
                "OAuth grant is not eligible for Jellyfin linking".into(),
            ));
        }
        let connection_id = grant.jellyfin_connection_id.clone().ok_or_else(|| {
            AppError::Unauthorized("OAuth grant is not eligible for Jellyfin linking".into())
        })?;
        let external_url = grant.jellyfin_external_url.clone().ok_or_else(|| {
            AppError::Unauthorized("OAuth grant is not eligible for Jellyfin linking".into())
        })?;
        let base_url = grant.jellyfin_base_url.clone().ok_or_else(|| {
            AppError::Unauthorized("OAuth grant is not eligible for Jellyfin linking".into())
        })?;
        let api_key_hash = grant.jellyfin_api_key_hash.clone().ok_or_else(|| {
            AppError::Unauthorized("OAuth grant is not eligible for Jellyfin linking".into())
        })?;
        let client = self
            .oauth_client_info(client_id)
            .await?
            .ok_or_else(|| AppError::Unauthorized("OAuth client is unavailable".into()))?;
        let expected_redirect = format!(
            "{}/Scryer/Auth/Callback",
            external_url.trim_end_matches('/')
        );
        // Same rule as authorization: the stored kind identifies the plugin client, and any of its
        // registered callbacks may carry the grant.
        if client.kind != crate::oauth::OAuthClientKind::JellyfinPlugin
            || !client.enabled
            || !client.redirect_uris.contains(&grant.redirect_uri)
            || grant.redirect_uri != expected_redirect
        {
            return Err(AppError::Unauthorized(
                "OAuth grant is not eligible for Jellyfin linking".into(),
            ));
        }
        let connection = self
            .services
            .integrations
            .media_server_connections
            .get_by_id(&connection_id)
            .await?
            .ok_or_else(|| AppError::Unauthorized("Jellyfin connection is unavailable".into()))?;
        if connection.provider != scryer_domain::MediaServerProvider::Jellyfin
            || !connection.enabled
            || !connection.linking_enabled
            || connection.base_url != base_url
            || connection
                .external_url
                .as_deref()
                .map(|url| url.trim_end_matches('/'))
                != Some(external_url.trim_end_matches('/'))
        {
            return Err(AppError::Unauthorized(
                "Jellyfin connection is unavailable".into(),
            ));
        }
        let api_key = connection
            .api_key
            .as_deref()
            .filter(|key| !key.trim().is_empty())
            .ok_or_else(|| AppError::Unauthorized("Jellyfin connection is unavailable".into()))?;
        if self.oauth_token_hash("jellyfin_link_api_key", api_key) != api_key_hash {
            return Err(AppError::Unauthorized(
                "Jellyfin connection authorization changed".into(),
            ));
        }
        let verified = self
            .services
            .integrations
            .external_identity_verifier
            .verify_jellyfin_user_with_api_key(
                &connection.id,
                &connection.base_url,
                api_key,
                &canonical_jellyfin_user_id,
            )
            .await?;
        if verified.provider != scryer_domain::ExternalAccountProvider::Jellyfin
            || verified.connection_id != connection.id
            || verified.external_user_id != canonical_jellyfin_user_id
        {
            return Err(AppError::Unauthorized(
                "Jellyfin user verification failed".into(),
            ));
        }
        // Re-resolve the immutable grant and live connection after I/O so an
        // administrator's concurrent disable, redirect change, or revoke wins.
        let current_grant = self
            .services
            .identity
            .oauth
            .get_refresh_grant(grant_id)
            .await?
            .ok_or_else(|| AppError::Unauthorized("OAuth grant is unavailable".into()))?;
        let current_client = self
            .oauth_client_info(client_id)
            .await?
            .ok_or_else(|| AppError::Unauthorized("OAuth client is unavailable".into()))?;
        let current_connection = self
            .services
            .integrations
            .media_server_connections
            .get_by_id(&connection_id)
            .await?
            .ok_or_else(|| AppError::Unauthorized("Jellyfin connection is unavailable".into()))?;
        if !oauth_jellyfin_link_grant_matches(&current_grant, &grant)
            || current_client.source != crate::oauth::OAuthClientSource::Custom
            || !current_client.enabled
            || current_client.redirect_uris.len() != 1
            || current_client.redirect_uris.first() != Some(&current_grant.redirect_uri)
            || current_grant.redirect_uri != expected_redirect
            || current_connection.provider != scryer_domain::MediaServerProvider::Jellyfin
            || !current_connection.enabled
            || !current_connection.linking_enabled
            || current_connection.base_url != base_url
            || current_connection
                .external_url
                .as_deref()
                .map(|url| url.trim_end_matches('/'))
                != Some(external_url.trim_end_matches('/'))
            || current_connection.api_key.as_deref().is_none_or(|key| {
                self.oauth_token_hash("jellyfin_link_api_key", key) != api_key_hash
            })
        {
            return Err(AppError::Unauthorized(
                "Jellyfin link authorization changed".into(),
            ));
        }
        self.link_verified_external_account(actor, verified)
            .await
            .map_err(|error| match error {
                AppError::Validation(message)
                    if message == "external account is already linked to another Scryer user"
                        || message
                            == "external account is already linked to a different provider identity" =>
                {
                    AppError::Validation("Jellyfin account could not be linked".into())
                }
                other => other,
            })
    }

    pub async fn federated_login_with_plex(
        &self,
        connection_id: String,
        plex_auth_token: String,
    ) -> AppResult<(User, Option<String>)> {
        let provider = scryer_domain::ExternalAccountProvider::Plex;
        let connection_id = normalize_connection_id(connection_id);
        let connection = self
            .auth_connection_for_use(provider.clone(), &connection_id, ExternalAuthUse::Login)
            .await?;
        let verified = self
            .services
            .integrations
            .external_identity_verifier
            .verify_plex(
                &connection_id,
                connection.machine_id.as_deref(),
                &plex_auth_token,
            )
            .await?;
        self.ensure_verified_identity_matches_request(&provider, &connection_id, &verified)?;
        self.login_verified_external_account(verified, connection)
            .await
    }

    pub async fn federated_login_with_jellyfin(
        &self,
        connection_id: String,
        username: String,
        password: String,
    ) -> AppResult<(User, Option<String>)> {
        // A Jellyfin account with no password authenticates against an empty
        // `Pw`, so without this an attacker who knows such a username could sign
        // in with no secret at all. Linking a passwordless account stays allowed
        // (see `link_jellyfin_account`); only signing in with one is refused.
        //
        // Both refusals below are flattened to the generic `LOGIN_FAILED` by
        // `to_login_gql_error`, so neither reveals whether the account exists or
        // whether it happens to be passwordless.
        if password.trim().is_empty() {
            return Err(AppError::Unauthorized(
                "invalid Jellyfin credentials".into(),
            ));
        }

        let provider = scryer_domain::ExternalAccountProvider::Jellyfin;
        let connection_id = normalize_connection_id(connection_id);
        let connection = self
            .auth_connection_for_use(provider.clone(), &connection_id, ExternalAuthUse::Login)
            .await?;
        let verified = self
            .services
            .integrations
            .external_identity_verifier
            .verify_jellyfin(&connection_id, &connection.base_url, &username, &password)
            .await?;
        // Defence in depth: refuse when Jellyfin itself reports the account has
        // no password, regardless of what the caller submitted. `None` means the
        // server did not report the fact and is deliberately not treated as
        // passwordless.
        if verified.remote_password_configured == Some(false) {
            return Err(AppError::Unauthorized(
                "invalid Jellyfin credentials".into(),
            ));
        }
        self.ensure_verified_identity_matches_request(&provider, &connection_id, &verified)?;
        self.login_verified_external_account(verified, connection)
            .await
    }

    pub async fn federated_login_with_emby(
        &self,
        connection_id: String,
        mode: EmbyConnectionMode,
        username: String,
        password: String,
    ) -> AppResult<(User, Option<String>)> {
        if password.is_empty() {
            return Err(AppError::Unauthorized("Emby sign-in failed".into()));
        }
        let provider = scryer_domain::ExternalAccountProvider::Emby;
        let connection_id = normalize_connection_id(connection_id);
        let connection = self
            .auth_connection_for_use(provider.clone(), &connection_id, ExternalAuthUse::Login)
            .await?;
        let (verified, refreshed_base_url) = self
            .verify_emby_identity(&connection, mode, &username, &password)
            .await
            .map_err(|_| AppError::Unauthorized("Emby sign-in failed".into()))?;
        if mode == EmbyConnectionMode::Local && verified.remote_password_configured == Some(false) {
            return Err(AppError::Unauthorized("Emby sign-in failed".into()));
        }
        self.refresh_emby_base_url_if_needed(&connection, refreshed_base_url.as_deref())
            .await;
        self.ensure_verified_identity_matches_request(&provider, &connection_id, &verified)?;
        self.login_verified_external_account(verified, connection)
            .await
    }

    async fn login_verified_external_account(
        &self,
        verified: VerifiedExternalIdentity,
        connection: scryer_domain::MediaServerConnection,
    ) -> AppResult<(User, Option<String>)> {
        let provider = verified.provider.clone();
        let account = self
            .services
            .identity
            .external_accounts
            .get_by_provider_identity(
                provider.clone(),
                &verified.connection_id,
                &verified.external_user_id,
            )
            .await?;

        let mut auto_added_user = None;
        let mut account = if let Some(account) = account {
            account
        } else if connection.auto_add_enabled {
            let (user, account) = self
                .create_auto_added_external_account(&verified, &connection)
                .await?;
            auto_added_user = Some(user);
            account
        } else {
            return Err(AppError::Unauthorized(
                "external account is not invited".into(),
            ));
        };

        let user = if let Some(user) = auto_added_user {
            user
        } else {
            self.services
                .identity
                .users
                .get_by_id(&account.user_id)
                .await?
                .ok_or_else(|| AppError::NotFound(format!("user {}", account.user_id)))?
        };
        if !user.login_status().is_enabled() {
            return Err(AppError::Unauthorized("credentials unavailable".into()));
        }

        match account.status {
            scryer_domain::ExternalAccountStatus::Disabled => {
                return Err(AppError::Unauthorized(
                    "external account is disabled".into(),
                ));
            }
            scryer_domain::ExternalAccountStatus::PendingClaim => {
                account.status = scryer_domain::ExternalAccountStatus::Active;
            }
            scryer_domain::ExternalAccountStatus::Active => {}
        }
        account.external_user_id = Some(verified.external_user_id);
        account.username = verified.username;
        account.display_name = verified.display_name;
        account.avatar_url = verified.avatar_url;
        let now = Utc::now();
        account.verified_at = Some(now);
        account.last_login_at = Some(now);
        account.updated_at = now;
        self.services
            .identity
            .external_accounts
            .update(account.clone())
            .await?;

        let auth_session_version = self
            .services
            .identity
            .users
            .auth_session_version(&user.id)
            .await?;
        self.cache_jwt_signing_key(&user).await?;
        Ok((user, auth_session_version))
    }

    async fn create_auto_added_external_account(
        &self,
        verified: &VerifiedExternalIdentity,
        connection: &scryer_domain::MediaServerConnection,
    ) -> AppResult<(User, scryer_domain::UserExternalAccount)> {
        if !connection.auto_add_enabled {
            return Err(AppError::Unauthorized(
                "external account is not invited".into(),
            ));
        }
        let username = self.unique_auto_added_username(&verified.username).await?;
        let user = User {
            id: scryer_domain::Id::new().0,
            username,
            password_hash: None,
            password_change_required: false,
            account_kind: scryer_domain::UserAccountKind::ExternalAutoProvisioned,
            authorization: Default::default(),
        };
        let grants = connection
            .default_library_grants
            .iter()
            .map(|grant| scryer_domain::LibraryGrant {
                user_id: user.id.clone(),
                library_id: grant.library_id.clone(),
                permissions: grant.permissions,
            })
            .collect();

        let now = Utc::now();
        self.services
            .identity
            .external_accounts
            .create_auto_added_user_with_account(
                user.clone(),
                connection.default_app_permissions,
                grants,
                scryer_domain::UserExternalAccount {
                    id: scryer_domain::Id::new().0,
                    user_id: user.id,
                    provider: verified.provider.clone(),
                    connection_id: verified.connection_id.clone(),
                    external_user_id: Some(verified.external_user_id.clone()),
                    username: verified.username.clone(),
                    display_name: verified.display_name.clone(),
                    avatar_url: verified.avatar_url.clone(),
                    status: scryer_domain::ExternalAccountStatus::Active,
                    verified_at: Some(now),
                    last_login_at: Some(now),
                    created_at: now,
                    updated_at: now,
                },
            )
            .await
    }

    async fn unique_auto_added_username(&self, provider_username: &str) -> AppResult<String> {
        let base = provider_username.trim();
        let base = if base.is_empty() { "media-user" } else { base };
        if !Self::is_reserved_recovery_username(base)
            && self
                .services
                .identity
                .users
                .get_by_username(base)
                .await?
                .is_none()
        {
            return Ok(base.to_string());
        }
        for suffix in 2..=9999 {
            let candidate = format!("{base}-{suffix}");
            if Self::is_reserved_recovery_username(&candidate) {
                continue;
            }
            if self
                .services
                .identity
                .users
                .get_by_username(&candidate)
                .await?
                .is_none()
            {
                return Ok(candidate);
            }
        }
        Err(AppError::Validation(
            "could not allocate a Scryer username for the external account".into(),
        ))
    }

    pub async fn unlink_external_account(
        &self,
        actor: &User,
        linked_account_id: &str,
    ) -> AppResult<()> {
        let account = self
            .services
            .identity
            .external_accounts
            .get_by_id(linked_account_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("external account {linked_account_id}")))?;
        if account.user_id != actor.id {
            self.require_app_permission(actor, scryer_domain::AppPermission::ManageUsers)
                .await?;
        } else {
            self.require_actor_capability(actor, scryer_domain::ActorCapability::ManageOwnAccount)
                .await?;
        }
        if matches!(account.status, scryer_domain::ExternalAccountStatus::Active) {
            let accounts = self
                .services
                .identity
                .external_accounts
                .list_by_user_id(&account.user_id)
                .await?;
            let active_count = accounts
                .iter()
                .filter(|account| {
                    matches!(account.status, scryer_domain::ExternalAccountStatus::Active)
                })
                .count();
            if active_count <= 1 {
                self.require_local_fallback_credential(&account.user_id)
                    .await?;
            }
        }
        self.services
            .identity
            .external_accounts
            .delete(linked_account_id)
            .await
    }

    async fn require_local_fallback_credential(&self, user_id: &str) -> AppResult<()> {
        let user = self
            .services
            .identity
            .users
            .get_by_id(user_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("user {user_id}")))?;
        if user.password_hash.is_some() {
            return Ok(());
        }
        if !self
            .services
            .identity
            .webauthn
            .list_credentials_for_user(user_id)
            .await?
            .is_empty()
        {
            return Ok(());
        }
        Err(AppError::Validation(
            "cannot unlink the last external login without a local password or passkey".into(),
        ))
    }
}

fn normalize_connection_id(value: impl AsRef<str>) -> String {
    value.as_ref().trim().to_string()
}

fn oauth_jellyfin_link_grant_matches(
    current: &OAuthRefreshGrantRecord,
    approved: &OAuthRefreshGrantRecord,
) -> bool {
    current.id == approved.id
        && current.family_id == approved.family_id
        && current.user_id == approved.user_id
        && current.client_id == approved.client_id
        && current.authorization_source == approved.authorization_source
        && current.auth_session_version == approved.auth_session_version
        && current.redirect_uri == approved.redirect_uri
        && current.scope == approved.scope
        && current.jellyfin_connection_id == approved.jellyfin_connection_id
        && current.jellyfin_external_url == approved.jellyfin_external_url
        && current.jellyfin_base_url == approved.jellyfin_base_url
        && current.jellyfin_api_key_hash == approved.jellyfin_api_key_hash
        && current.revoked_at == approved.revoked_at
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use tokio::sync::Mutex;

    use super::*;
    use crate::null_repositories::test_nulls::{
        NullDownloadClient, NullDownloadClientConfigRepository, NullIndexerClient,
        NullQualityProfileRepository, NullReleaseAttemptRepository, NullShowRepository,
        NullTitleRepository, NullUserRepository,
    };
    use crate::{
        AppServices, ExternalIdentityVerifier, IndexerConfig, IndexerConfigRepository,
        IndexerConfigUpdate, JellyfinServerUser, JwtAuthConfig, MediaServerConnectionRepository,
        OAuthAuthorizationCodeRecord, OAuthClientRegistrationRecord, OAuthConnectedAppRecord,
        OAuthRefreshGrantRecord, OAuthRefreshRotationOutcome, OAuthRefreshTokenRecord,
        OAuthRepository, PlexServerUser, SettingsRepository, UserExternalAccountRepository,
        UserRepository,
    };
    use scryer_domain::{
        AppPermission, AppPermissionMask, ExternalAccountProvider, ExternalAccountStatus,
        LibraryPermissionMask, MediaServerConnection, UserAuthorization, UserExternalAccount,
    };

    type TestSettingsKey = (String, String, Option<String>);
    type TestSettingsValues = HashMap<TestSettingsKey, String>;

    #[derive(Default)]
    struct TestSettingsRepository {
        values: Mutex<TestSettingsValues>,
    }

    #[derive(Default)]
    struct TestExternalAccountRepository {
        accounts: Mutex<Vec<UserExternalAccount>>,
        first_lookup_race: Option<Arc<TestExternalAccountLookupRace>>,
        claim_winner: Mutex<Option<UserExternalAccount>>,
    }

    struct TestExternalAccountLookupRace {
        barrier: tokio::sync::Barrier,
        remaining_waiters: Mutex<usize>,
    }

    #[derive(Default)]
    struct TestUserRepository {
        users: Mutex<Vec<User>>,
    }

    #[derive(Default)]
    struct TestMediaServerConnectionRepository {
        connections: Mutex<Vec<MediaServerConnection>>,
        get_by_id_responses: Mutex<Vec<MediaServerConnection>>,
    }

    struct TestOAuthRepository {
        grants: Mutex<Vec<OAuthRefreshGrantRecord>>,
        registration: Option<OAuthClientRegistrationRecord>,
        registration_responses: Mutex<Vec<OAuthClientRegistrationRecord>>,
    }

    #[derive(Default)]
    struct TestExternalIdentityVerifier {
        jellyfin_users: Vec<JellyfinServerUser>,
        plex_users: Vec<PlexServerUser>,
        emby_users: Vec<crate::EmbyServerUser>,
        /// Usernames the fake Jellyfin server reports as having no password.
        passwordless_jellyfin_usernames: Vec<String>,
        /// Usernames the fake Emby server reports as having no password.
        passwordless_emby_usernames: Vec<String>,
    }

    impl TestExternalAccountRepository {
        fn new(accounts: Vec<UserExternalAccount>) -> Self {
            Self {
                accounts: Mutex::new(accounts),
                first_lookup_race: None,
                claim_winner: Mutex::new(None),
            }
        }

        fn with_first_lookup_race(accounts: Vec<UserExternalAccount>, waiters: usize) -> Self {
            Self {
                accounts: Mutex::new(accounts),
                first_lookup_race: Some(Arc::new(TestExternalAccountLookupRace {
                    barrier: tokio::sync::Barrier::new(waiters),
                    remaining_waiters: Mutex::new(waiters),
                })),
                claim_winner: Mutex::new(None),
            }
        }

        fn with_claim_winner(winner: UserExternalAccount) -> Self {
            Self {
                accounts: Mutex::new(Vec::new()),
                first_lookup_race: None,
                claim_winner: Mutex::new(Some(winner)),
            }
        }

        async fn wait_for_racing_initial_lookup(&self) {
            let Some(race) = self.first_lookup_race.as_ref() else {
                return;
            };
            let should_wait = {
                let mut remaining_waiters = race.remaining_waiters.lock().await;
                if *remaining_waiters == 0 {
                    false
                } else {
                    *remaining_waiters -= 1;
                    true
                }
            };
            if should_wait {
                race.barrier.wait().await;
            }
        }
    }

    impl TestMediaServerConnectionRepository {
        fn new(connections: Vec<MediaServerConnection>) -> Self {
            Self {
                connections: Mutex::new(connections),
                get_by_id_responses: Mutex::new(Vec::new()),
            }
        }

        fn with_get_by_id_responses(
            connections: Vec<MediaServerConnection>,
            get_by_id_responses: Vec<MediaServerConnection>,
        ) -> Self {
            Self {
                connections: Mutex::new(connections),
                get_by_id_responses: Mutex::new(get_by_id_responses),
            }
        }
    }

    impl TestOAuthRepository {
        fn new(
            grants: Vec<OAuthRefreshGrantRecord>,
            registration: Option<OAuthClientRegistrationRecord>,
        ) -> Self {
            Self {
                grants: Mutex::new(grants),
                registration,
                registration_responses: Mutex::new(Vec::new()),
            }
        }

        fn with_registration_responses(
            grants: Vec<OAuthRefreshGrantRecord>,
            registration: Option<OAuthClientRegistrationRecord>,
            registration_responses: Vec<OAuthClientRegistrationRecord>,
        ) -> Self {
            Self {
                grants: Mutex::new(grants),
                registration,
                registration_responses: Mutex::new(registration_responses),
            }
        }

        async fn replace_grants(&self, grants: Vec<OAuthRefreshGrantRecord>) {
            *self.grants.lock().await = grants;
        }
    }

    impl TestExternalIdentityVerifier {
        fn with_jellyfin_users(jellyfin_users: Vec<JellyfinServerUser>) -> Self {
            Self {
                jellyfin_users,
                plex_users: Vec::new(),
                emby_users: Vec::new(),
                passwordless_jellyfin_usernames: Vec::new(),
                passwordless_emby_usernames: Vec::new(),
            }
        }

        fn with_users(
            jellyfin_users: Vec<JellyfinServerUser>,
            plex_users: Vec<PlexServerUser>,
        ) -> Self {
            Self {
                jellyfin_users,
                plex_users,
                emby_users: Vec::new(),
                passwordless_jellyfin_usernames: Vec::new(),
                passwordless_emby_usernames: Vec::new(),
            }
        }

        fn with_passwordless_jellyfin_users(
            jellyfin_users: Vec<JellyfinServerUser>,
            passwordless_jellyfin_usernames: Vec<String>,
        ) -> Self {
            Self {
                jellyfin_users,
                plex_users: Vec::new(),
                emby_users: Vec::new(),
                passwordless_jellyfin_usernames,
                passwordless_emby_usernames: Vec::new(),
            }
        }

        fn with_emby_users(
            emby_users: Vec<crate::EmbyServerUser>,
            passwordless_emby_usernames: Vec<String>,
        ) -> Self {
            Self {
                jellyfin_users: Vec::new(),
                plex_users: Vec::new(),
                emby_users,
                passwordless_jellyfin_usernames: Vec::new(),
                passwordless_emby_usernames,
            }
        }
    }

    impl TestUserRepository {
        fn new(users: Vec<User>) -> Self {
            Self {
                users: Mutex::new(users),
            }
        }
    }

    struct TestIndexerConfigRepository;

    #[async_trait::async_trait]
    impl UserExternalAccountRepository for TestExternalAccountRepository {
        async fn create(&self, account: UserExternalAccount) -> AppResult<UserExternalAccount> {
            self.accounts.lock().await.push(account.clone());
            Ok(account)
        }

        async fn create_or_get_by_provider_identity(
            &self,
            account: UserExternalAccount,
        ) -> AppResult<UserExternalAccount> {
            if let Some(winner) = self.claim_winner.lock().await.take() {
                self.accounts.lock().await.push(winner.clone());
                return Ok(winner);
            }
            let mut accounts = self.accounts.lock().await;
            if let Some(existing) = accounts.iter().find(|existing| {
                existing.provider == account.provider
                    && existing.connection_id == account.connection_id
                    && existing.external_user_id == account.external_user_id
            }) {
                return Ok(existing.clone());
            }
            accounts.push(account.clone());
            Ok(account)
        }

        async fn list_by_user_id(&self, user_id: &str) -> AppResult<Vec<UserExternalAccount>> {
            Ok(self
                .accounts
                .lock()
                .await
                .iter()
                .filter(|account| account.user_id == user_id)
                .cloned()
                .collect())
        }

        async fn list_verified_by_connection(
            &self,
            provider: ExternalAccountProvider,
            connection_id: &str,
        ) -> AppResult<Vec<UserExternalAccount>> {
            Ok(self
                .accounts
                .lock()
                .await
                .iter()
                .filter(|account| {
                    account.provider == provider
                        && account.connection_id == connection_id
                        && account.status == ExternalAccountStatus::Active
                        && account.verified_at.is_some()
                        && account
                            .external_user_id
                            .as_deref()
                            .is_some_and(|value| !value.trim().is_empty())
                })
                .cloned()
                .collect())
        }

        async fn get_by_id(&self, id: &str) -> AppResult<Option<UserExternalAccount>> {
            Ok(self
                .accounts
                .lock()
                .await
                .iter()
                .find(|account| account.id == id)
                .cloned())
        }

        async fn get_by_provider_identity(
            &self,
            provider: ExternalAccountProvider,
            connection_id: &str,
            external_user_id: &str,
        ) -> AppResult<Option<UserExternalAccount>> {
            let account = self
                .accounts
                .lock()
                .await
                .iter()
                .find(|account| {
                    account.provider == provider
                        && account.connection_id == connection_id
                        && account.external_user_id.as_deref() == Some(external_user_id)
                })
                .cloned();
            if account.is_none() {
                self.wait_for_racing_initial_lookup().await;
            }
            Ok(account)
        }

        async fn get_pending_claim_by_provider_username(
            &self,
            provider: ExternalAccountProvider,
            connection_id: &str,
            username: &str,
        ) -> AppResult<Option<UserExternalAccount>> {
            let normalized_username = normalize_provider_username(username);
            Ok(self
                .accounts
                .lock()
                .await
                .iter()
                .find(|account| {
                    account.provider == provider
                        && account.connection_id == connection_id
                        && account.external_user_id.is_none()
                        && account.status == ExternalAccountStatus::PendingClaim
                        && normalize_provider_username(&account.username) == normalized_username
                })
                .cloned())
        }

        async fn update(&self, account: UserExternalAccount) -> AppResult<UserExternalAccount> {
            let mut accounts = self.accounts.lock().await;
            if let Some(existing) = accounts
                .iter_mut()
                .find(|candidate| candidate.id == account.id)
            {
                *existing = account.clone();
            }
            Ok(account)
        }

        async fn create_auto_added_user_with_account(
            &self,
            user: User,
            _app_permissions: AppPermissionMask,
            _library_grants: Vec<scryer_domain::LibraryGrant>,
            account: UserExternalAccount,
        ) -> AppResult<(User, UserExternalAccount)> {
            let account = self.create(account).await?;
            Ok((user, account))
        }

        async fn delete(&self, id: &str) -> AppResult<()> {
            self.accounts
                .lock()
                .await
                .retain(|account| account.id != id);
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl OAuthRepository for TestOAuthRepository {
        async fn create_api_key(&self, _: crate::ApiKeyRecord) -> AppResult<crate::ApiKeyRecord> {
            Err(AppError::Repository("not configured".into()))
        }

        async fn get_api_key_by_lookup_id(
            &self,
            _: &str,
        ) -> AppResult<Option<crate::ApiKeyRecord>> {
            Ok(None)
        }

        async fn list_api_keys(&self, _: &str) -> AppResult<Vec<crate::ApiKeyRecord>> {
            Ok(Vec::new())
        }

        async fn list_environment_api_keys(&self) -> AppResult<Vec<crate::ApiKeyRecord>> {
            Ok(Vec::new())
        }

        async fn upsert_environment_api_key(
            &self,
            _: crate::ApiKeyRecord,
        ) -> AppResult<crate::ApiKeyRecord> {
            Err(AppError::Repository("not configured".into()))
        }

        async fn revoke_api_key(
            &self,
            _: &str,
            _: &str,
            _: chrono::DateTime<Utc>,
        ) -> AppResult<bool> {
            Ok(false)
        }

        async fn touch_api_key_last_used(
            &self,
            _: &str,
            _: chrono::DateTime<Utc>,
        ) -> AppResult<bool> {
            Ok(false)
        }

        async fn create_client_registration(
            &self,
            _: OAuthClientRegistrationRecord,
        ) -> AppResult<OAuthClientRegistrationRecord> {
            Err(AppError::Repository("not configured".into()))
        }

        async fn get_client_registration(
            &self,
            client_id: &str,
        ) -> AppResult<Option<OAuthClientRegistrationRecord>> {
            let mut responses = self.registration_responses.lock().await;
            if !responses.is_empty() {
                let registration = responses.remove(0);
                return Ok((registration.client_id == client_id).then_some(registration));
            }
            drop(responses);

            Ok(self
                .registration
                .as_ref()
                .filter(|registration| registration.client_id == client_id)
                .cloned())
        }

        async fn list_client_registrations(&self) -> AppResult<Vec<OAuthClientRegistrationRecord>> {
            Ok(self.registration.clone().into_iter().collect())
        }

        async fn update_client_registration(
            &self,
            _: OAuthClientRegistrationRecord,
            _: chrono::DateTime<Utc>,
        ) -> AppResult<Option<OAuthClientRegistrationRecord>> {
            Ok(None)
        }

        async fn delete_client_registration(
            &self,
            _: &str,
            _: chrono::DateTime<Utc>,
            _: &str,
        ) -> AppResult<bool> {
            Ok(false)
        }

        async fn is_refresh_grant_active(&self, _: &str, _: &str) -> AppResult<bool> {
            Ok(false)
        }

        async fn create_authorization_code(
            &self,
            _: OAuthAuthorizationCodeRecord,
        ) -> AppResult<OAuthAuthorizationCodeRecord> {
            Err(AppError::Repository("not configured".into()))
        }

        async fn get_authorization_code(
            &self,
            _: &str,
        ) -> AppResult<Option<OAuthAuthorizationCodeRecord>> {
            Ok(None)
        }

        async fn consume_authorization_code(
            &self,
            _: &str,
            _: chrono::DateTime<Utc>,
        ) -> AppResult<bool> {
            Ok(false)
        }

        async fn consume_authorization_code_and_create_refresh_grant(
            &self,
            _: OAuthAuthorizationCodeRecord,
            _: chrono::DateTime<Utc>,
            _: OAuthRefreshGrantRecord,
            _: OAuthRefreshTokenRecord,
            _: bool,
        ) -> AppResult<Option<OAuthRefreshGrantRecord>> {
            Ok(None)
        }

        async fn create_refresh_grant(
            &self,
            _: OAuthRefreshGrantRecord,
            _: OAuthRefreshTokenRecord,
            _: bool,
        ) -> AppResult<OAuthRefreshGrantRecord> {
            Err(AppError::Repository("not configured".into()))
        }

        async fn get_refresh_token(
            &self,
            _: &str,
        ) -> AppResult<Option<(OAuthRefreshTokenRecord, OAuthRefreshGrantRecord)>> {
            Ok(None)
        }

        async fn get_refresh_grant(&self, _: &str) -> AppResult<Option<OAuthRefreshGrantRecord>> {
            let mut grants = self.grants.lock().await;
            match grants.len() {
                0 => Ok(None),
                1 => Ok(grants.first().cloned()),
                _ => Ok(Some(grants.remove(0))),
            }
        }

        async fn rotate_refresh_token(
            &self,
            _: &str,
            _: chrono::DateTime<Utc>,
            _: OAuthRefreshTokenRecord,
        ) -> AppResult<OAuthRefreshRotationOutcome> {
            Ok(OAuthRefreshRotationOutcome::Unavailable)
        }

        async fn revoke_refresh_grant(
            &self,
            _: &str,
            _: &str,
            _: chrono::DateTime<Utc>,
            _: &str,
        ) -> AppResult<bool> {
            Ok(false)
        }

        async fn revoke_refresh_family(
            &self,
            _: &str,
            _: chrono::DateTime<Utc>,
            _: &str,
        ) -> AppResult<u64> {
            Ok(0)
        }

        async fn revoke_user_refresh_grants(
            &self,
            _: &str,
            _: chrono::DateTime<Utc>,
            _: &str,
        ) -> AppResult<u64> {
            Ok(0)
        }

        async fn revoke_authless_refresh_grants(
            &self,
            _: chrono::DateTime<Utc>,
            _: &str,
        ) -> AppResult<u64> {
            Ok(0)
        }

        async fn touch_refresh_grant_last_used(
            &self,
            _: &str,
            _: &str,
            _: chrono::DateTime<Utc>,
        ) -> AppResult<bool> {
            Ok(false)
        }

        async fn list_connected_apps(&self, _: &str) -> AppResult<Vec<OAuthConnectedAppRecord>> {
            Ok(Vec::new())
        }
    }

    #[async_trait::async_trait]
    impl ExternalIdentityVerifier for TestExternalIdentityVerifier {
        async fn verify_plex(
            &self,
            _: &str,
            _: Option<&str>,
            _: &str,
        ) -> AppResult<VerifiedExternalIdentity> {
            Err(AppError::Repository(
                "external identity verification is not configured".into(),
            ))
        }

        async fn verify_jellyfin(
            &self,
            connection_id: &str,
            _: &str,
            username: &str,
            _: &str,
        ) -> AppResult<VerifiedExternalIdentity> {
            let user = self
                .jellyfin_users
                .iter()
                .find(|user| user.username.eq_ignore_ascii_case(username))
                .ok_or_else(|| AppError::Unauthorized("invalid Jellyfin credentials".into()))?;
            let remote_password_configured = Some(
                !self
                    .passwordless_jellyfin_usernames
                    .iter()
                    .any(|candidate| candidate.eq_ignore_ascii_case(username)),
            );
            Ok(VerifiedExternalIdentity {
                provider: ExternalAccountProvider::Jellyfin,
                connection_id: connection_id.to_string(),
                external_user_id: user.id.clone(),
                username: user.username.clone(),
                display_name: user.display_name.clone(),
                avatar_url: user.avatar_url.clone(),
                remote_password_configured,
            })
        }

        async fn verify_jellyfin_user_with_api_key(
            &self,
            connection_id: &str,
            _: &str,
            _: &str,
            canonical_user_id: &str,
        ) -> AppResult<VerifiedExternalIdentity> {
            let user = self
                .jellyfin_users
                .iter()
                .find(|user| user.id == canonical_user_id)
                .ok_or_else(|| {
                    AppError::Unauthorized("Jellyfin user verification failed".into())
                })?;
            Ok(VerifiedExternalIdentity {
                provider: ExternalAccountProvider::Jellyfin,
                connection_id: connection_id.to_string(),
                external_user_id: user.id.clone(),
                username: user.username.clone(),
                display_name: user.display_name.clone(),
                avatar_url: user.avatar_url.clone(),
                remote_password_configured: None,
            })
        }

        async fn discover_plex_servers(
            &self,
            _: &str,
        ) -> AppResult<Vec<crate::PlexServerDiscovery>> {
            Ok(Vec::new())
        }

        async fn test_jellyfin_connection(&self, _: &str) -> AppResult<()> {
            Ok(())
        }

        async fn test_jellyfin_api_key(&self, _: &str, _: &str) -> AppResult<()> {
            Ok(())
        }

        async fn exchange_jellyfin_admin_api_key(
            &self,
            _: &str,
            _: &str,
            _: &str,
            _: &str,
        ) -> AppResult<String> {
            Err(AppError::Repository(
                "external identity verification is not configured".into(),
            ))
        }

        async fn list_jellyfin_users(
            &self,
            _: &str,
            api_key: &str,
            search: Option<&str>,
        ) -> AppResult<Vec<JellyfinServerUser>> {
            if api_key == "fail" {
                return Err(AppError::Repository("Jellyfin listing failed".into()));
            }
            if api_key == "stall" {
                return std::future::pending::<AppResult<Vec<JellyfinServerUser>>>().await;
            }
            let Some(search) = search.map(str::trim).filter(|value| !value.is_empty()) else {
                return Ok(self.jellyfin_users.clone());
            };
            let search = search.to_ascii_lowercase();
            Ok(self
                .jellyfin_users
                .iter()
                .filter(|user| {
                    user.id.to_ascii_lowercase().contains(&search)
                        || user.username.to_ascii_lowercase().contains(&search)
                })
                .cloned()
                .collect())
        }

        async fn verify_emby_local_identity(
            &self,
            connection_id: &str,
            _: &str,
            _: &str,
            username: &str,
            _: &str,
        ) -> AppResult<VerifiedExternalIdentity> {
            let user = self
                .emby_users
                .iter()
                .find(|user| user.username.eq_ignore_ascii_case(username))
                .ok_or_else(|| {
                    AppError::Repository(
                        "upstream Emby detail containing request-secret must stay private".into(),
                    )
                })?;
            Ok(VerifiedExternalIdentity {
                provider: ExternalAccountProvider::Emby,
                connection_id: connection_id.to_string(),
                external_user_id: user.id.clone(),
                username: user.username.clone(),
                display_name: user.display_name.clone(),
                avatar_url: user.avatar_url.clone(),
                remote_password_configured: Some(
                    !self
                        .passwordless_emby_usernames
                        .iter()
                        .any(|candidate| candidate.eq_ignore_ascii_case(username)),
                ),
            })
        }

        async fn verify_emby_connect_identity(
            &self,
            connection_id: &str,
            base_url: &str,
            expected_server_id: &str,
            username_or_email: &str,
            password: &str,
        ) -> AppResult<crate::EmbyConnectIdentityVerification> {
            let identity = self
                .verify_emby_local_identity(
                    connection_id,
                    base_url,
                    expected_server_id,
                    username_or_email,
                    password,
                )
                .await?;
            Ok(crate::EmbyConnectIdentityVerification {
                identity,
                resolved_api_base_url: base_url.to_string(),
            })
        }

        async fn list_emby_users(
            &self,
            _: &str,
            _: &str,
            api_key: &str,
            search: Option<&str>,
        ) -> AppResult<Vec<crate::EmbyServerUser>> {
            if api_key == "fail" {
                return Err(AppError::Repository("Emby listing failed".into()));
            }
            let Some(search) = search.map(str::trim).filter(|value| !value.is_empty()) else {
                return Ok(self.emby_users.clone());
            };
            let search = search.to_ascii_lowercase();
            Ok(self
                .emby_users
                .iter()
                .filter(|user| {
                    user.id.to_ascii_lowercase().contains(&search)
                        || user.username.to_ascii_lowercase().contains(&search)
                })
                .cloned()
                .collect())
        }

        async fn list_plex_users(
            &self,
            plex_auth_token: &str,
            search: Option<&str>,
        ) -> AppResult<Vec<PlexServerUser>> {
            if plex_auth_token == "fail" {
                return Err(AppError::Repository("Plex listing failed".into()));
            }
            if plex_auth_token == "stall" {
                return std::future::pending::<AppResult<Vec<PlexServerUser>>>().await;
            }
            let Some(search) = search.map(str::trim).filter(|value| !value.is_empty()) else {
                return Ok(self.plex_users.clone());
            };
            let search = search.to_ascii_lowercase();
            Ok(self
                .plex_users
                .iter()
                .filter(|user| {
                    user.id.to_ascii_lowercase().contains(&search)
                        || user.username.to_ascii_lowercase().contains(&search)
                })
                .cloned()
                .collect())
        }
    }

    #[async_trait::async_trait]
    impl MediaServerConnectionRepository for TestMediaServerConnectionRepository {
        async fn list(
            &self,
            provider: Option<scryer_domain::MediaServerProvider>,
        ) -> AppResult<Vec<MediaServerConnection>> {
            Ok(self
                .connections
                .lock()
                .await
                .iter()
                .filter(|connection| {
                    provider
                        .as_ref()
                        .is_none_or(|provider| &connection.provider == provider)
                })
                .cloned()
                .collect())
        }

        async fn get_by_id(&self, id: &str) -> AppResult<Option<MediaServerConnection>> {
            let mut responses = self.get_by_id_responses.lock().await;
            if !responses.is_empty() {
                let connection = responses.remove(0);
                return Ok((connection.id == id).then_some(connection));
            }
            drop(responses);
            Ok(self
                .connections
                .lock()
                .await
                .iter()
                .find(|connection| connection.id == id)
                .cloned())
        }

        async fn create(
            &self,
            connection: MediaServerConnection,
        ) -> AppResult<MediaServerConnection> {
            self.connections.lock().await.push(connection.clone());
            Ok(connection)
        }

        async fn update(
            &self,
            connection: MediaServerConnection,
        ) -> AppResult<MediaServerConnection> {
            let mut connections = self.connections.lock().await;
            if let Some(existing) = connections
                .iter_mut()
                .find(|candidate| candidate.id == connection.id)
            {
                *existing = connection.clone();
            }
            Ok(connection)
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
            Ok(())
        }

        async fn delete(&self, id: &str) -> AppResult<()> {
            self.connections
                .lock()
                .await
                .retain(|connection| connection.id != id);
            Ok(())
        }

        async fn has_external_accounts(&self, _: &str) -> AppResult<bool> {
            Ok(false)
        }

        async fn has_notification_channels(&self, _: &str) -> AppResult<bool> {
            Ok(false)
        }
    }

    #[async_trait::async_trait]
    impl UserRepository for TestUserRepository {
        async fn get_by_username(&self, username: &str) -> AppResult<Option<User>> {
            Ok(self
                .users
                .lock()
                .await
                .iter()
                .find(|user| user.username == username)
                .cloned())
        }

        async fn create(&self, user: User) -> AppResult<User> {
            self.users.lock().await.push(user.clone());
            Ok(user)
        }

        async fn list_all(&self) -> AppResult<Vec<User>> {
            Ok(self.users.lock().await.clone())
        }

        async fn get_by_id(&self, id: &str) -> AppResult<Option<User>> {
            Ok(self
                .users
                .lock()
                .await
                .iter()
                .find(|user| user.id == id)
                .cloned())
        }

        async fn auth_session_version(&self, _user_id: &str) -> AppResult<Option<String>> {
            Ok(None)
        }

        async fn update_password_and_invalidate_sessions(
            &self,
            id: &str,
            password_hash: String,
            password_change_required: bool,
            _auth_session_version: &str,
        ) -> AppResult<User> {
            let mut users = self.users.lock().await;
            let user = users
                .iter_mut()
                .find(|user| user.id == id)
                .ok_or_else(|| AppError::NotFound(format!("user {id}")))?;
            user.password_hash = Some(password_hash);
            user.password_change_required = password_change_required;
            Ok(user.clone())
        }

        async fn update_own_password_and_invalidate_sessions(
            &self,
            id: &str,
            password_hash: String,
            password_change_required: bool,
            _auth_session_version: &str,
            expected_password_hash: Option<&str>,
        ) -> AppResult<User> {
            let mut users = self.users.lock().await;
            let user = users
                .iter_mut()
                .find(|user| user.id == id)
                .ok_or_else(|| AppError::NotFound(format!("user {id}")))?;
            let precondition_matches = match expected_password_hash {
                Some(expected) => user.password_hash.as_deref() == Some(expected),
                None => user.password_hash.is_none(),
            };
            if !precondition_matches {
                return Err(AppError::ReauthenticationRequired(
                    "account credentials changed; authenticate again".into(),
                ));
            }
            user.password_hash = Some(password_hash);
            user.password_change_required = password_change_required;
            Ok(user.clone())
        }

        async fn complete_required_password_change(
            &self,
            id: &str,
            password_hash: String,
            expected_auth_session_version: &Option<String>,
            _auth_session_version: &str,
        ) -> AppResult<User> {
            if expected_auth_session_version.is_some() {
                return Err(AppError::Unauthorized(
                    "authentication session was invalidated".into(),
                ));
            }
            let mut users = self.users.lock().await;
            let user = users
                .iter_mut()
                .find(|user| user.id == id)
                .ok_or_else(|| AppError::NotFound(format!("user {id}")))?;
            if !user.password_change_required {
                return Err(AppError::Unauthorized(
                    "password change is no longer required".into(),
                ));
            }
            user.password_hash = Some(password_hash);
            user.password_change_required = false;
            Ok(user.clone())
        }

        async fn update_login_status_and_rotate_session(
            &self,
            id: &str,
            status: scryer_domain::UserLoginStatus,
            _auth_session_version: &str,
        ) -> AppResult<User> {
            let mut users = self.users.lock().await;
            let user = users
                .iter_mut()
                .find(|user| user.id == id)
                .ok_or_else(|| AppError::NotFound(format!("user {id}")))?;
            user.set_login_status(status);
            Ok(user.clone())
        }

        async fn delete(&self, id: &str) -> AppResult<()> {
            self.users.lock().await.retain(|user| user.id != id);
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl IndexerConfigRepository for TestIndexerConfigRepository {
        async fn list(&self, _: Option<String>) -> AppResult<Vec<IndexerConfig>> {
            Ok(Vec::new())
        }

        async fn get_by_id(&self, _: &str) -> AppResult<Option<IndexerConfig>> {
            Ok(None)
        }

        async fn create(&self, config: IndexerConfig) -> AppResult<IndexerConfig> {
            Ok(config)
        }

        async fn touch_last_error(&self, _: &str) -> AppResult<()> {
            Ok(())
        }

        async fn update(&self, _: IndexerConfigUpdate) -> AppResult<IndexerConfig> {
            Err(AppError::Repository("not configured".into()))
        }

        async fn delete(&self, _: &str) -> AppResult<()> {
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl SettingsRepository for TestSettingsRepository {
        async fn get_setting_json(
            &self,
            scope: &str,
            key_name: &str,
            scope_id: Option<String>,
        ) -> AppResult<Option<String>> {
            Ok(self
                .values
                .lock()
                .await
                .get(&(scope.to_string(), key_name.to_string(), scope_id))
                .cloned())
        }

        async fn upsert_setting_json(
            &self,
            scope: &str,
            key_name: &str,
            scope_id: Option<String>,
            value_json: String,
            _source: &str,
            _updated_by_user_id: Option<String>,
        ) -> AppResult<()> {
            self.values.lock().await.insert(
                (scope.to_string(), key_name.to_string(), scope_id),
                value_json,
            );
            Ok(())
        }

        async fn delete_setting_value(
            &self,
            scope: &str,
            key_name: &str,
            scope_id: Option<String>,
        ) -> AppResult<()> {
            self.values
                .lock()
                .await
                .remove(&(scope.to_string(), key_name.to_string(), scope_id));
            Ok(())
        }

        async fn delete_values_for_scope_id(&self, scope_id: &str) -> AppResult<u32> {
            let mut values = self.values.lock().await;
            let before = values.len();
            values.retain(|(_, _, current_scope_id), _| {
                current_scope_id.as_deref() != Some(scope_id)
            });
            Ok((before - values.len()) as u32)
        }
    }

    fn test_app(settings: Arc<dyn SettingsRepository>) -> AppUseCase {
        test_app_with_external_accounts(
            settings,
            Arc::new(crate::null_repositories::NullUserExternalAccountRepository),
        )
    }

    fn test_app_with_external_accounts(
        settings: Arc<dyn SettingsRepository>,
        external_accounts: Arc<dyn UserExternalAccountRepository>,
    ) -> AppUseCase {
        test_app_with_identity(settings, Arc::new(NullUserRepository), external_accounts)
    }

    fn test_app_with_identity(
        settings: Arc<dyn SettingsRepository>,
        users: Arc<dyn UserRepository>,
        external_accounts: Arc<dyn UserExternalAccountRepository>,
    ) -> AppUseCase {
        test_app_with_identity_and_media_servers(
            settings,
            users,
            external_accounts,
            vec![
                test_media_server_connection(
                    scryer_domain::MediaServerProvider::Jellyfin,
                    "jellyfin-main",
                ),
                test_media_server_connection(scryer_domain::MediaServerProvider::Plex, "plex-main"),
            ],
        )
    }

    fn test_app_with_identity_and_media_servers(
        settings: Arc<dyn SettingsRepository>,
        users: Arc<dyn UserRepository>,
        external_accounts: Arc<dyn UserExternalAccountRepository>,
        media_server_connections: Vec<MediaServerConnection>,
    ) -> AppUseCase {
        test_app_with_identity_media_servers_and_verifier(
            settings,
            users,
            external_accounts,
            media_server_connections,
            Arc::new(TestExternalIdentityVerifier::default()),
        )
    }

    fn test_app_with_identity_media_servers_and_verifier(
        settings: Arc<dyn SettingsRepository>,
        users: Arc<dyn UserRepository>,
        external_accounts: Arc<dyn UserExternalAccountRepository>,
        media_server_connections: Vec<MediaServerConnection>,
        external_identity_verifier: Arc<dyn ExternalIdentityVerifier>,
    ) -> AppUseCase {
        test_app_with_identity_oauth_media_servers_and_verifier(
            settings,
            users,
            external_accounts,
            Arc::new(TestMediaServerConnectionRepository::new(
                media_server_connections,
            )),
            external_identity_verifier,
            Arc::new(crate::null_repositories::NullOAuthRepository),
        )
    }

    fn test_app_with_identity_oauth_media_servers_and_verifier(
        settings: Arc<dyn SettingsRepository>,
        users: Arc<dyn UserRepository>,
        external_accounts: Arc<dyn UserExternalAccountRepository>,
        media_server_connections: Arc<dyn MediaServerConnectionRepository>,
        external_identity_verifier: Arc<dyn ExternalIdentityVerifier>,
        oauth: Arc<dyn OAuthRepository>,
    ) -> AppUseCase {
        let assembly = AppServices::builder(
            Arc::new(NullTitleRepository),
            Arc::new(NullShowRepository),
            users,
            Arc::new(TestIndexerConfigRepository),
            Arc::new(NullIndexerClient),
            Arc::new(NullDownloadClient),
            Arc::new(NullDownloadClientConfigRepository),
            Arc::new(NullReleaseAttemptRepository),
            settings,
            Arc::new(NullQualityProfileRepository),
            String::new(),
        )
        .with_external_account_store(external_accounts)
        .with_oauth_store(oauth)
        .with_external_identity_verifier(external_identity_verifier)
        .with_media_server_connection_store(media_server_connections)
        .build_partial_for_tests();

        AppUseCase::new(
            assembly,
            JwtAuthConfig {
                issuer: "scryer-test".to_string(),
                access_ttl_seconds: 3600,
                jwt_signing_salt: "test-salt".to_string(),
            },
            Arc::new(FacetRegistry::new()),
        )
    }

    fn admin_user() -> User {
        User {
            id: "admin".to_string(),
            username: "admin".to_string(),
            password_hash: Some("hash".to_string()),
            password_change_required: false,
            account_kind: Default::default(),
            authorization: UserAuthorization {
                app: AppPermissionMask::from_permissions([
                    AppPermission::ManageSystemSettings,
                    AppPermission::ManageUsers,
                ]),
                libraries: HashMap::new(),
                default_library: LibraryPermissionMask::NONE,
                actor_capabilities: scryer_domain::ActorCapabilityMask::MANAGE_OWN_ACCOUNT,
                login_status: Default::default(),
                loaded: true,
            },
        }
    }

    fn regular_user(id: &str) -> User {
        User {
            id: id.to_string(),
            username: id.to_string(),
            password_hash: None,
            password_change_required: false,
            account_kind: Default::default(),
            authorization: UserAuthorization {
                app: AppPermissionMask::NONE,
                libraries: HashMap::new(),
                default_library: LibraryPermissionMask::NONE,
                actor_capabilities: scryer_domain::ActorCapabilityMask::MANAGE_OWN_ACCOUNT,
                login_status: Default::default(),
                loaded: true,
            },
        }
    }

    fn active_jellyfin_account(user_id: &str) -> UserExternalAccount {
        let now = Utc::now();
        UserExternalAccount {
            id: format!("{user_id}-jellyfin"),
            user_id: user_id.to_string(),
            provider: ExternalAccountProvider::Jellyfin,
            connection_id: "jellyfin-main".to_string(),
            external_user_id: Some(format!("{user_id}-remote")),
            username: user_id.to_string(),
            display_name: None,
            avatar_url: None,
            status: ExternalAccountStatus::Active,
            verified_at: Some(now),
            last_login_at: Some(now),
            created_at: now,
            updated_at: now,
        }
    }

    fn test_media_server_connection(
        provider: scryer_domain::MediaServerProvider,
        id: &str,
    ) -> scryer_domain::MediaServerConnection {
        scryer_domain::MediaServerConnection {
            id: id.to_string(),
            provider: provider.clone(),
            display_name: id.to_string(),
            base_url: match provider {
                scryer_domain::MediaServerProvider::Plex => "https://plex.tv".to_string(),
                scryer_domain::MediaServerProvider::Jellyfin => {
                    "https://jellyfin.example.test".to_string()
                }
                scryer_domain::MediaServerProvider::Emby => "https://emby.example.test".to_string(),
            },
            external_url: None,
            enabled: true,
            login_enabled: true,
            linking_enabled: true,
            auto_add_enabled: false,
            default_app_permissions: AppPermissionMask::NONE,
            default_library_grants: Vec::new(),
            machine_id: match provider {
                scryer_domain::MediaServerProvider::Plex => Some("machine-1".to_string()),
                scryer_domain::MediaServerProvider::Jellyfin
                | scryer_domain::MediaServerProvider::Emby => None,
            },
            api_key: None,
            emby_server_id: (provider == scryer_domain::MediaServerProvider::Emby)
                .then(|| "emby-server".to_string()),
            emby_connect_enabled: provider == scryer_domain::MediaServerProvider::Emby,
            path_mappings: Vec::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn auto_added_username_skips_reserved_recovery_admin() {
        let app = test_app_with_identity(
            Arc::new(TestSettingsRepository::default()),
            Arc::new(TestUserRepository::new(vec![regular_user(
                "recovery-admin-2",
            )])),
            Arc::new(TestExternalAccountRepository::default()),
        );

        let username = app
            .unique_auto_added_username("recovery-admin")
            .await
            .expect("allocate auto-added username");

        assert_eq!(username, "recovery-admin-3");
    }

    #[tokio::test]
    async fn jellyfin_invite_requires_selected_external_id() {
        let admin = admin_user();
        let target = regular_user("user-1");
        let external_accounts = Arc::new(TestExternalAccountRepository::default());
        let app = test_app_with_identity(
            Arc::new(TestSettingsRepository::default()),
            Arc::new(TestUserRepository::new(vec![target.clone()])),
            external_accounts.clone(),
        );
        let account = app
            .create_external_account_invite(
                &admin,
                &target.id,
                ExternalAccountProvider::Jellyfin,
                "jellyfin-main".to_string(),
                " JellyUser ".to_string(),
                None,
            )
            .await;

        assert!(
            matches!(account, Err(AppError::Validation(message)) if message.contains("picker"))
        );
    }

    #[tokio::test]
    async fn jellyfin_invite_uses_selected_external_id() {
        let admin = admin_user();
        let target = regular_user("user-1");
        let mut connection = test_media_server_connection(
            scryer_domain::MediaServerProvider::Jellyfin,
            "jellyfin-main",
        );
        connection.api_key = Some("jellyfin-api-key".to_string());
        let app = test_app_with_identity_media_servers_and_verifier(
            Arc::new(TestSettingsRepository::default()),
            Arc::new(TestUserRepository::new(vec![target.clone()])),
            Arc::new(TestExternalAccountRepository::default()),
            vec![connection],
            Arc::new(TestExternalIdentityVerifier::with_jellyfin_users(vec![
                JellyfinServerUser {
                    id: "jellyfin-user-id".to_string(),
                    username: "JellyUser".to_string(),
                    display_name: Some("Jelly User".to_string()),
                    avatar_url: Some("https://jellyfin.example.test/avatar.png".to_string()),
                },
            ])),
        );
        let account = app
            .create_external_account_invite(
                &admin,
                &target.id,
                ExternalAccountProvider::Jellyfin,
                "jellyfin-main".to_string(),
                " JellyUser ".to_string(),
                Some("jellyfin-user-id".to_string()),
            )
            .await
            .expect("create Jellyfin invite");

        assert_eq!(account.provider, ExternalAccountProvider::Jellyfin);
        assert_eq!(account.connection_id, "jellyfin-main");
        assert_eq!(
            account.external_user_id.as_deref(),
            Some("jellyfin-user-id")
        );
        assert_eq!(account.username, "JellyUser");
        assert_eq!(account.display_name.as_deref(), Some("Jelly User"));
        assert_eq!(account.status, ExternalAccountStatus::PendingClaim);
        assert_eq!(account.last_login_at, None);
    }

    #[tokio::test]
    async fn media_server_users_fail_open_across_connections() {
        let admin = admin_user();
        let mut jellyfin_connection = test_media_server_connection(
            scryer_domain::MediaServerProvider::Jellyfin,
            "jellyfin-main",
        );
        jellyfin_connection.api_key = Some("jellyfin-api-key".to_string());
        let mut failing_jellyfin_connection = test_media_server_connection(
            scryer_domain::MediaServerProvider::Jellyfin,
            "jellyfin-fail",
        );
        failing_jellyfin_connection.api_key = Some("fail".to_string());
        let mut plex_connection =
            test_media_server_connection(scryer_domain::MediaServerProvider::Plex, "plex-main");
        plex_connection.api_key = Some("plex-token".to_string());
        let missing_plex_connection =
            test_media_server_connection(scryer_domain::MediaServerProvider::Plex, "plex-missing");
        let app = test_app_with_identity_media_servers_and_verifier(
            Arc::new(TestSettingsRepository::default()),
            Arc::new(TestUserRepository::default()),
            Arc::new(TestExternalAccountRepository::default()),
            vec![
                jellyfin_connection,
                failing_jellyfin_connection,
                plex_connection,
                missing_plex_connection,
            ],
            Arc::new(TestExternalIdentityVerifier::with_users(
                vec![JellyfinServerUser {
                    id: "jellyfin-user-id".to_string(),
                    username: "jellyfin-user".to_string(),
                    display_name: None,
                    avatar_url: None,
                }],
                vec![PlexServerUser {
                    id: "plex-user-id".to_string(),
                    username: "plex-user".to_string(),
                    display_name: None,
                    avatar_url: None,
                }],
            )),
        );

        let groups = app
            .list_media_server_users(&admin, None)
            .await
            .expect("list media server users");
        let groups_by_id = groups
            .iter()
            .map(|group| (group.connection_id.as_str(), group))
            .collect::<HashMap<_, _>>();

        assert_eq!(
            groups_by_id["jellyfin-main"].status,
            crate::MediaServerUserGroupStatus::Ready
        );
        assert_eq!(
            groups_by_id["jellyfin-main"].users[0].id,
            "jellyfin-user-id"
        );
        assert_eq!(
            groups_by_id["plex-main"].status,
            crate::MediaServerUserGroupStatus::Ready
        );
        assert_eq!(groups_by_id["plex-main"].users[0].id, "plex-user-id");
        assert_eq!(
            groups_by_id["plex-missing"].status,
            crate::MediaServerUserGroupStatus::MissingCredentials
        );
        assert!(groups_by_id["plex-missing"].users.is_empty());
        assert_eq!(
            groups_by_id["jellyfin-fail"].status,
            crate::MediaServerUserGroupStatus::Error
        );
        assert!(groups_by_id["jellyfin-fail"].users.is_empty());
    }

    #[tokio::test]
    async fn media_server_users_timeout_does_not_block_successful_connections() {
        let admin = admin_user();
        let mut stalled_jellyfin_connection = test_media_server_connection(
            scryer_domain::MediaServerProvider::Jellyfin,
            "jellyfin-stall",
        );
        stalled_jellyfin_connection.api_key = Some("stall".to_string());
        let mut plex_connection =
            test_media_server_connection(scryer_domain::MediaServerProvider::Plex, "plex-main");
        plex_connection.api_key = Some("plex-token".to_string());
        let app = test_app_with_identity_media_servers_and_verifier(
            Arc::new(TestSettingsRepository::default()),
            Arc::new(TestUserRepository::default()),
            Arc::new(TestExternalAccountRepository::default()),
            vec![stalled_jellyfin_connection, plex_connection],
            Arc::new(TestExternalIdentityVerifier::with_users(
                Vec::new(),
                vec![PlexServerUser {
                    id: "plex-user-id".to_string(),
                    username: "plex-user".to_string(),
                    display_name: None,
                    avatar_url: None,
                }],
            )),
        );

        let started_at = std::time::Instant::now();
        let groups = app
            .list_media_server_users(&admin, None)
            .await
            .expect("list media server users");

        assert!(
            started_at.elapsed() < std::time::Duration::from_secs(1),
            "stalled media server lookup should be bounded by the per-connection timeout"
        );
        let groups_by_id = groups
            .iter()
            .map(|group| (group.connection_id.as_str(), group))
            .collect::<HashMap<_, _>>();

        assert_eq!(
            groups_by_id["plex-main"].status,
            crate::MediaServerUserGroupStatus::Ready
        );
        assert_eq!(groups_by_id["plex-main"].users[0].id, "plex-user-id");
        assert_eq!(
            groups_by_id["jellyfin-stall"].status,
            crate::MediaServerUserGroupStatus::Error
        );
        assert!(groups_by_id["jellyfin-stall"].users.is_empty());
        assert!(
            groups_by_id["jellyfin-stall"]
                .error_message
                .as_deref()
                .is_some_and(|message| message.contains("Timed out after"))
        );
    }

    #[tokio::test]
    async fn auto_added_external_user_cannot_be_given_local_password() {
        let admin = admin_user();
        let mut target = regular_user("jellyfin-user");
        target.account_kind = scryer_domain::UserAccountKind::ExternalAutoProvisioned;
        let app = test_app_with_identity(
            Arc::new(TestSettingsRepository::default()),
            Arc::new(TestUserRepository::new(vec![target.clone()])),
            Arc::new(TestExternalAccountRepository::new(vec![
                active_jellyfin_account(&target.id),
            ])),
        );

        let result = app
            .set_user_password(&admin, &target.id, "local-password".to_string())
            .await;

        assert!(
            matches!(
                result,
                Err(AppError::Validation(ref message))
                    if message == "externally managed users cannot set a Scryer password"
            ),
            "expected externally managed password validation, got {result:?}"
        );
    }

    #[tokio::test]
    async fn passwordless_linked_local_user_can_be_given_initial_password() {
        let admin = admin_user();
        let target = regular_user("linked-passwordless-user");
        let app = test_app_with_identity(
            Arc::new(TestSettingsRepository::default()),
            Arc::new(TestUserRepository::new(vec![target.clone()])),
            Arc::new(TestExternalAccountRepository::new(vec![
                active_jellyfin_account(&target.id),
            ])),
        );

        let updated = app
            .set_user_password(&admin, &target.id, "local-password".to_string())
            .await
            .expect("local linked user can receive an initial password");

        assert_eq!(updated.account_kind, scryer_domain::UserAccountKind::Local);
        assert!(updated.password_hash.is_some());
    }

    #[tokio::test]
    async fn passwordless_local_user_can_set_initial_own_password() {
        let target = regular_user("passwordless-admin");
        let app = test_app_with_identity(
            Arc::new(TestSettingsRepository::default()),
            Arc::new(TestUserRepository::new(vec![target.clone()])),
            Arc::new(TestExternalAccountRepository::default()),
        );

        let updated = app
            .set_initial_own_password(&target, "local-password".to_string())
            .await
            .expect("local passwordless user can set an initial password");

        assert_eq!(updated.account_kind, scryer_domain::UserAccountKind::Local);
        assert!(updated.password_hash.is_some());
    }

    #[tokio::test]
    async fn auto_added_external_user_cannot_set_initial_own_password() {
        let mut target = regular_user("auto-added-self");
        target.account_kind = scryer_domain::UserAccountKind::ExternalAutoProvisioned;
        let app = test_app_with_identity(
            Arc::new(TestSettingsRepository::default()),
            Arc::new(TestUserRepository::new(vec![target.clone()])),
            Arc::new(TestExternalAccountRepository::new(vec![
                active_jellyfin_account(&target.id),
            ])),
        );

        let result = app
            .set_initial_own_password(&target, "local-password".to_string())
            .await;

        assert!(
            matches!(
                result,
                Err(AppError::Validation(ref message))
                    if message == "externally managed users cannot set a Scryer password"
            ),
            "expected externally managed password validation, got {result:?}"
        );
    }

    #[tokio::test]
    async fn password_backed_linked_user_can_rotate_local_password() {
        let admin = admin_user();
        let mut target = regular_user("linked-user");
        target.password_hash = Some("existing-local-password-hash".to_string());
        let app = test_app_with_identity(
            Arc::new(TestSettingsRepository::default()),
            Arc::new(TestUserRepository::new(vec![target.clone()])),
            Arc::new(TestExternalAccountRepository::new(vec![
                active_jellyfin_account(&target.id),
            ])),
        );

        let updated = app
            .set_user_password(&admin, &target.id, "new-local-password".to_string())
            .await
            .expect("linked local user should be allowed to rotate password");

        assert!(updated.password_hash.is_some());
        assert_ne!(updated.password_hash, target.password_hash);
    }

    #[tokio::test]
    async fn plex_invite_requires_selected_external_id() {
        let admin = admin_user();
        let target = regular_user("user-1");
        let app = test_app_with_identity(
            Arc::new(TestSettingsRepository::default()),
            Arc::new(TestUserRepository::new(vec![target.clone()])),
            Arc::new(TestExternalAccountRepository::default()),
        );
        let account = app
            .create_external_account_invite(
                &admin,
                &target.id,
                ExternalAccountProvider::Plex,
                "plex-main".to_string(),
                "plex-user-1".to_string(),
                None,
            )
            .await;

        assert!(
            matches!(account, Err(AppError::Validation(message)) if message.contains("picker"))
        );
    }

    #[tokio::test]
    async fn plex_invite_uses_selected_external_id() {
        let admin = admin_user();
        let target = regular_user("user-1");
        let mut connection =
            test_media_server_connection(scryer_domain::MediaServerProvider::Plex, "plex-main");
        connection.api_key = Some("plex-token".to_string());
        let app = test_app_with_identity_media_servers_and_verifier(
            Arc::new(TestSettingsRepository::default()),
            Arc::new(TestUserRepository::new(vec![target.clone()])),
            Arc::new(TestExternalAccountRepository::default()),
            vec![connection],
            Arc::new(TestExternalIdentityVerifier::with_users(
                Vec::new(),
                vec![PlexServerUser {
                    id: "plex-user-1".to_string(),
                    username: "plexuser".to_string(),
                    display_name: Some("Plex User".to_string()),
                    avatar_url: Some("https://plex.tv/avatar.jpg".to_string()),
                }],
            )),
        );
        let account = app
            .create_external_account_invite(
                &admin,
                &target.id,
                ExternalAccountProvider::Plex,
                "plex-main".to_string(),
                "plexuser".to_string(),
                Some("plex-user-1".to_string()),
            )
            .await
            .expect("create plex invite");

        assert_eq!(account.provider, ExternalAccountProvider::Plex);
        assert_eq!(account.connection_id, "plex-main");
        assert_eq!(account.external_user_id.as_deref(), Some("plex-user-1"));
        assert_eq!(account.username, "plexuser");
        assert_eq!(account.display_name.as_deref(), Some("Plex User"));
        assert_eq!(account.status, ExternalAccountStatus::PendingClaim);
        assert_eq!(account.last_login_at, None);
    }

    #[tokio::test]
    async fn admin_can_list_external_account_invites_across_users() {
        let admin = admin_user();
        let first = regular_user("user-1");
        let second = regular_user("user-2");
        let external_accounts = Arc::new(TestExternalAccountRepository::default());
        let mut jellyfin_connection = test_media_server_connection(
            scryer_domain::MediaServerProvider::Jellyfin,
            "jellyfin-main",
        );
        jellyfin_connection.api_key = Some("jellyfin-api-key".to_string());
        let mut plex_connection =
            test_media_server_connection(scryer_domain::MediaServerProvider::Plex, "plex-main");
        plex_connection.api_key = Some("plex-token".to_string());
        let app = test_app_with_identity_media_servers_and_verifier(
            Arc::new(TestSettingsRepository::default()),
            Arc::new(TestUserRepository::new(vec![first.clone(), second.clone()])),
            external_accounts,
            vec![jellyfin_connection, plex_connection],
            Arc::new(TestExternalIdentityVerifier::with_users(
                vec![JellyfinServerUser {
                    id: "first-jellyfin-id".to_string(),
                    username: "first-jellyfin".to_string(),
                    display_name: None,
                    avatar_url: None,
                }],
                vec![PlexServerUser {
                    id: "second-plex-id".to_string(),
                    username: "second-plex".to_string(),
                    display_name: None,
                    avatar_url: None,
                }],
            )),
        );
        app.create_external_account_invite(
            &admin,
            &first.id,
            ExternalAccountProvider::Jellyfin,
            "jellyfin-main".to_string(),
            "first-jellyfin".to_string(),
            Some("first-jellyfin-id".to_string()),
        )
        .await
        .expect("create first invite");
        app.create_external_account_invite(
            &admin,
            &second.id,
            ExternalAccountProvider::Plex,
            "plex-main".to_string(),
            "second-plex".to_string(),
            Some("second-plex-id".to_string()),
        )
        .await
        .expect("create second invite");

        let invites = app
            .list_external_account_invites(&admin)
            .await
            .expect("list external account invites");
        let user_ids = invites
            .iter()
            .map(|account| account.user_id.as_str())
            .collect::<Vec<_>>();
        assert!(user_ids.contains(&first.id.as_str()));
        assert!(user_ids.contains(&second.id.as_str()));

        let result = app.list_external_account_invites(&first).await;
        assert!(matches!(result, Err(AppError::Unauthorized(_))));
    }

    #[tokio::test]
    async fn external_auth_runtime_settings_are_derived_from_media_servers() {
        let mut jellyfin_login = test_media_server_connection(
            scryer_domain::MediaServerProvider::Jellyfin,
            "jellyfin-login",
        );
        jellyfin_login.display_name = "Jellyfin Login".to_string();
        jellyfin_login.login_enabled = true;
        jellyfin_login.linking_enabled = false;

        let mut jellyfin_link = test_media_server_connection(
            scryer_domain::MediaServerProvider::Jellyfin,
            "jellyfin-link",
        );
        jellyfin_link.display_name = "Jellyfin Link".to_string();
        jellyfin_link.login_enabled = false;
        jellyfin_link.linking_enabled = true;

        let mut disabled_jellyfin = test_media_server_connection(
            scryer_domain::MediaServerProvider::Jellyfin,
            "jellyfin-disabled",
        );
        disabled_jellyfin.enabled = false;

        let mut auth_flags_off = test_media_server_connection(
            scryer_domain::MediaServerProvider::Jellyfin,
            "jellyfin-off",
        );
        auth_flags_off.login_enabled = false;
        auth_flags_off.linking_enabled = false;

        let mut plex =
            test_media_server_connection(scryer_domain::MediaServerProvider::Plex, "plex-main");
        plex.display_name = "Plex Main".to_string();

        let mut plex_without_machine = test_media_server_connection(
            scryer_domain::MediaServerProvider::Plex,
            "plex-no-machine",
        );
        plex_without_machine.machine_id = None;

        let emby =
            test_media_server_connection(scryer_domain::MediaServerProvider::Emby, "emby-main");

        let app = test_app_with_identity_and_media_servers(
            Arc::new(TestSettingsRepository::default()),
            Arc::new(NullUserRepository),
            Arc::new(TestExternalAccountRepository::default()),
            vec![
                jellyfin_login,
                jellyfin_link,
                disabled_jellyfin,
                auth_flags_off,
                plex,
                plex_without_machine,
                emby,
            ],
        );

        let settings = app
            .get_external_auth_runtime_settings()
            .await
            .expect("load runtime settings");

        assert_eq!(
            settings.login_providers,
            vec![
                ExternalAccountProvider::Jellyfin,
                ExternalAccountProvider::Plex,
                ExternalAccountProvider::Emby,
            ]
        );
        assert_eq!(
            settings.linking_providers,
            vec![
                ExternalAccountProvider::Jellyfin,
                ExternalAccountProvider::Plex,
                ExternalAccountProvider::Emby,
            ]
        );
        assert_eq!(
            settings
                .connections
                .iter()
                .map(|connection| connection.id.as_str())
                .collect::<Vec<_>>(),
            vec!["jellyfin-login", "jellyfin-link", "plex-main", "emby-main"]
        );
        assert_eq!(settings.connections[0].display_name, "Jellyfin Login");
        assert!(settings.connections[0].login_enabled);
        assert!(!settings.connections[0].linking_enabled);
        assert!(!settings.connections[1].login_enabled);
        assert!(settings.connections[1].linking_enabled);
        assert_eq!(
            settings.connections[2].provider,
            ExternalAccountProvider::Plex
        );
        assert_eq!(
            settings.connections[3].provider,
            ExternalAccountProvider::Emby
        );
        assert!(settings.connections[3].emby_connect_enabled);
    }

    #[tokio::test]
    async fn external_auth_runtime_settings_empty_when_no_connection_exposes_auth() {
        let mut disabled = test_media_server_connection(
            scryer_domain::MediaServerProvider::Jellyfin,
            "jellyfin-disabled",
        );
        disabled.enabled = false;

        let mut flags_off = test_media_server_connection(
            scryer_domain::MediaServerProvider::Jellyfin,
            "jellyfin-off",
        );
        flags_off.login_enabled = false;
        flags_off.linking_enabled = false;

        let mut plex_without_machine = test_media_server_connection(
            scryer_domain::MediaServerProvider::Plex,
            "plex-no-machine",
        );
        plex_without_machine.machine_id = None;

        let app = test_app_with_identity_and_media_servers(
            Arc::new(TestSettingsRepository::default()),
            Arc::new(NullUserRepository),
            Arc::new(TestExternalAccountRepository::default()),
            vec![disabled, flags_off, plex_without_machine],
        );

        let settings = app
            .get_external_auth_runtime_settings()
            .await
            .expect("load runtime settings");

        assert!(settings.login_providers.is_empty());
        assert!(settings.linking_providers.is_empty());
        assert!(settings.connections.is_empty());
    }

    #[tokio::test]
    async fn link_rejects_connection_not_on_allowlist_before_verification() {
        let app = test_app(Arc::new(TestSettingsRepository::default()));
        let admin = admin_user();

        let result = app
            .link_jellyfin_account(
                &admin,
                "jellyfin-other".to_string(),
                "someone".to_string(),
                "secret".to_string(),
            )
            .await;

        assert!(
            matches!(result, Err(AppError::Validation(message)) if message.contains("not configured"))
        );
    }

    #[tokio::test]
    async fn jellyfin_login_rejects_empty_password_before_touching_the_connection() {
        let app = test_app(Arc::new(TestSettingsRepository::default()));

        for password in ["", "   "] {
            let result = app
                .federated_login_with_jellyfin(
                    "jellyfin-main".to_string(),
                    "someone".to_string(),
                    password.to_string(),
                )
                .await;

            // No connection is configured here, so if the guard ran any later
            // this would surface the "not configured" validation error instead.
            assert!(
                matches!(&result, Err(AppError::Unauthorized(message)) if message.contains("invalid Jellyfin credentials")),
                "expected password {password:?} to be refused up front, got {result:?}"
            );
        }
    }

    #[tokio::test]
    async fn jellyfin_login_rejects_account_reported_passwordless_by_the_server() {
        let app = test_app_with_identity_media_servers_and_verifier(
            Arc::new(TestSettingsRepository::default()),
            Arc::new(TestUserRepository::default()),
            Arc::new(TestExternalAccountRepository::default()),
            vec![test_media_server_connection(
                scryer_domain::MediaServerProvider::Jellyfin,
                "jellyfin-main",
            )],
            Arc::new(
                TestExternalIdentityVerifier::with_passwordless_jellyfin_users(
                    vec![
                        JellyfinServerUser {
                            id: "no-password-id".to_string(),
                            username: "NoPassword".to_string(),
                            display_name: None,
                            avatar_url: None,
                        },
                        JellyfinServerUser {
                            id: "has-password-id".to_string(),
                            username: "HasPassword".to_string(),
                            display_name: None,
                            avatar_url: None,
                        },
                    ],
                    vec!["NoPassword".to_string()],
                ),
            ),
        );

        // Non-empty password, so only the server-reported fact can refuse this.
        let passwordless = app
            .federated_login_with_jellyfin(
                "jellyfin-main".to_string(),
                "NoPassword".to_string(),
                "anything".to_string(),
            )
            .await;
        assert!(
            matches!(&passwordless, Err(AppError::Unauthorized(message)) if message.contains("invalid Jellyfin credentials")),
            "passwordless account must not be able to sign in, got {passwordless:?}"
        );

        // Control: identical call shape against an account the server reports as
        // having a password. It clears the guard and fails later on invitation,
        // proving the refusal above is specific to being passwordless.
        let with_password = app
            .federated_login_with_jellyfin(
                "jellyfin-main".to_string(),
                "HasPassword".to_string(),
                "anything".to_string(),
            )
            .await;
        assert!(
            matches!(&with_password, Err(AppError::Unauthorized(message)) if message.contains("not invited")),
            "password-configured account should reach the invitation check, got {with_password:?}"
        );
    }

    #[tokio::test]
    async fn jellyfin_linking_still_allows_a_passwordless_account() {
        let admin = admin_user();
        let app = test_app_with_identity_media_servers_and_verifier(
            Arc::new(TestSettingsRepository::default()),
            Arc::new(TestUserRepository::new(vec![admin.clone()])),
            Arc::new(TestExternalAccountRepository::default()),
            vec![test_media_server_connection(
                scryer_domain::MediaServerProvider::Jellyfin,
                "jellyfin-main",
            )],
            Arc::new(
                TestExternalIdentityVerifier::with_passwordless_jellyfin_users(
                    vec![JellyfinServerUser {
                        id: "no-password-id".to_string(),
                        username: "NoPassword".to_string(),
                        display_name: None,
                        avatar_url: None,
                    }],
                    vec!["NoPassword".to_string()],
                ),
            ),
        );

        // Empty password too: linking a passwordless account is intentionally
        // supported, and only signing in with one is refused.
        let account = app
            .link_jellyfin_account(
                &admin,
                "jellyfin-main".to_string(),
                "NoPassword".to_string(),
                String::new(),
            )
            .await
            .expect("linking a passwordless Jellyfin account stays allowed");

        assert_eq!(account.provider, ExternalAccountProvider::Jellyfin);
        assert_eq!(account.external_user_id.as_deref(), Some("no-password-id"));
    }

    #[tokio::test]
    async fn link_rejects_disabled_existing_account() {
        let admin = admin_user();
        let now = Utc::now();
        let app = test_app_with_external_accounts(
            Arc::new(TestSettingsRepository::default()),
            Arc::new(TestExternalAccountRepository::new(vec![
                UserExternalAccount {
                    id: "linked-account".to_string(),
                    user_id: admin.id.clone(),
                    provider: ExternalAccountProvider::Jellyfin,
                    connection_id: "jellyfin-main".to_string(),
                    external_user_id: Some("remote-user".to_string()),
                    username: "remote-user".to_string(),
                    display_name: None,
                    avatar_url: None,
                    status: scryer_domain::ExternalAccountStatus::Disabled,
                    verified_at: None,
                    last_login_at: None,
                    created_at: now,
                    updated_at: now,
                },
            ])),
        );

        let result = app
            .link_verified_external_account(
                &admin,
                VerifiedExternalIdentity {
                    provider: ExternalAccountProvider::Jellyfin,
                    connection_id: "jellyfin-main".to_string(),
                    external_user_id: "remote-user".to_string(),
                    username: "remote-user".to_string(),
                    display_name: Some("Remote User".to_string()),
                    avatar_url: None,
                    remote_password_configured: None,
                },
            )
            .await;

        assert!(
            matches!(result, Err(AppError::Validation(message)) if message.contains("disabled"))
        );
    }

    #[tokio::test]
    async fn concurrent_first_links_converge_on_one_owner() {
        let first_actor = regular_user("first-user");
        let second_actor = regular_user("second-user");
        let external_accounts = Arc::new(TestExternalAccountRepository::with_first_lookup_race(
            Vec::new(),
            2,
        ));
        let app = test_app_with_external_accounts(
            Arc::new(TestSettingsRepository::default()),
            external_accounts.clone(),
        );
        let verified = || VerifiedExternalIdentity {
            provider: ExternalAccountProvider::Jellyfin,
            connection_id: "jellyfin-main".to_string(),
            external_user_id: "same-remote-user".to_string(),
            username: "same-user".to_string(),
            display_name: Some("Same User".to_string()),
            avatar_url: None,
            remote_password_configured: None,
        };

        let (first, second) = tokio::join!(
            app.link_verified_external_account(&first_actor, verified()),
            app.link_verified_external_account(&second_actor, verified()),
        );

        assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
        assert_eq!(
            usize::from(matches!(
                &first,
                Err(AppError::Validation(message))
                    if message == "external account is already linked to another Scryer user"
            )) + usize::from(matches!(
                &second,
                Err(AppError::Validation(message))
                    if message == "external account is already linked to another Scryer user"
            )),
            1,
            "the losing first-link attempt must normalize to the ordinary cross-user conflict"
        );

        let accounts = external_accounts.accounts.lock().await;
        assert_eq!(accounts.len(), 1);
        assert_eq!(
            accounts[0].external_user_id.as_deref(),
            Some("same-remote-user")
        );
        assert!(
            accounts[0].user_id == first_actor.id || accounts[0].user_id == second_actor.id,
            "the unique provider identity must have exactly one owner"
        );
    }

    #[tokio::test]
    async fn atomic_claim_same_actor_refreshes_returned_account_metadata() {
        let actor = regular_user("same-user");
        let mut winner = active_jellyfin_account(&actor.id);
        winner.external_user_id = Some("remote-user".to_string());
        winner.username = "stale-username".to_string();
        winner.display_name = Some("Stale Name".to_string());
        winner.avatar_url = Some("https://jellyfin.example.test/stale.png".to_string());
        let external_accounts = Arc::new(TestExternalAccountRepository::with_claim_winner(winner));
        let app = test_app_with_external_accounts(
            Arc::new(TestSettingsRepository::default()),
            external_accounts.clone(),
        );

        let linked = app
            .link_verified_external_account(
                &actor,
                VerifiedExternalIdentity {
                    provider: ExternalAccountProvider::Jellyfin,
                    connection_id: "jellyfin-main".to_string(),
                    external_user_id: "remote-user".to_string(),
                    username: "fresh-username".to_string(),
                    display_name: Some("Fresh Name".to_string()),
                    avatar_url: Some("https://jellyfin.example.test/fresh.png".to_string()),
                    remote_password_configured: None,
                },
            )
            .await
            .expect("same actor should refresh the account returned by the atomic claim");

        assert_eq!(linked.username, "fresh-username");
        assert_eq!(linked.display_name.as_deref(), Some("Fresh Name"));
        assert_eq!(
            linked.avatar_url.as_deref(),
            Some("https://jellyfin.example.test/fresh.png")
        );
        assert_eq!(linked.status, ExternalAccountStatus::Active);
        assert!(linked.verified_at.is_some());

        let accounts = external_accounts.accounts.lock().await;
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0], linked);
    }

    #[tokio::test]
    async fn atomic_claim_rejects_a_disabled_returned_account() {
        let actor = regular_user("disabled-user");
        let mut winner = active_jellyfin_account(&actor.id);
        winner.external_user_id = Some("remote-user".to_string());
        winner.status = ExternalAccountStatus::Disabled;
        let external_accounts = Arc::new(TestExternalAccountRepository::with_claim_winner(winner));
        let app = test_app_with_external_accounts(
            Arc::new(TestSettingsRepository::default()),
            external_accounts.clone(),
        );

        let result = app
            .link_verified_external_account(
                &actor,
                VerifiedExternalIdentity {
                    provider: ExternalAccountProvider::Jellyfin,
                    connection_id: "jellyfin-main".to_string(),
                    external_user_id: "remote-user".to_string(),
                    username: "attempted-refresh".to_string(),
                    display_name: None,
                    avatar_url: None,
                    remote_password_configured: None,
                },
            )
            .await;

        assert!(
            matches!(result, Err(AppError::Validation(message)) if message.contains("disabled")),
            "a disabled account returned by the atomic claim must remain disabled"
        );
        let accounts = external_accounts.accounts.lock().await;
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].status, ExternalAccountStatus::Disabled);
        assert_eq!(accounts[0].username, "disabled-user");
    }

    #[tokio::test]
    async fn link_jellyfin_does_not_claim_pending_username_invite_without_external_id() {
        let admin = admin_user();
        let now = Utc::now();
        let app = test_app_with_external_accounts(
            Arc::new(TestSettingsRepository::default()),
            Arc::new(TestExternalAccountRepository::new(vec![
                UserExternalAccount {
                    id: "pending-account".to_string(),
                    user_id: admin.id.clone(),
                    provider: ExternalAccountProvider::Jellyfin,
                    connection_id: "jellyfin-main".to_string(),
                    external_user_id: None,
                    username: "Remote User".to_string(),
                    display_name: None,
                    avatar_url: None,
                    status: scryer_domain::ExternalAccountStatus::PendingClaim,
                    verified_at: None,
                    last_login_at: None,
                    created_at: now,
                    updated_at: now,
                },
            ])),
        );

        let account = app
            .link_verified_external_account(
                &admin,
                VerifiedExternalIdentity {
                    provider: ExternalAccountProvider::Jellyfin,
                    connection_id: "jellyfin-main".to_string(),
                    external_user_id: "jellyfin-user-id".to_string(),
                    username: "remote user".to_string(),
                    display_name: Some("Remote User".to_string()),
                    avatar_url: Some("https://jellyfin.example.test/avatar.png".to_string()),
                    remote_password_configured: None,
                },
            )
            .await
            .expect("link Jellyfin account by immutable id");

        assert_ne!(account.id, "pending-account");
        assert_eq!(
            account.external_user_id.as_deref(),
            Some("jellyfin-user-id")
        );
        assert_eq!(account.status, ExternalAccountStatus::Active);
    }

    #[tokio::test]
    async fn pending_claim_login_activates_and_refreshes_metadata() {
        let user = User {
            id: "user-1".to_string(),
            username: "local-user".to_string(),
            password_hash: None,
            password_change_required: false,
            account_kind: Default::default(),
            authorization: UserAuthorization {
                app: AppPermissionMask::NONE,
                libraries: HashMap::new(),
                default_library: LibraryPermissionMask::NONE,
                actor_capabilities: scryer_domain::ActorCapabilityMask::MANAGE_OWN_ACCOUNT,
                login_status: Default::default(),
                loaded: true,
            },
        };
        let now = Utc::now();
        let external_accounts = Arc::new(TestExternalAccountRepository::new(vec![
            UserExternalAccount {
                id: "pending-account".to_string(),
                user_id: user.id.clone(),
                provider: ExternalAccountProvider::Jellyfin,
                connection_id: "jellyfin-main".to_string(),
                external_user_id: Some("remote-user".to_string()),
                username: "Fresh-Name".to_string(),
                display_name: None,
                avatar_url: None,
                status: scryer_domain::ExternalAccountStatus::PendingClaim,
                verified_at: None,
                last_login_at: None,
                created_at: now,
                updated_at: now,
            },
        ]));
        let app = test_app_with_identity(
            Arc::new(TestSettingsRepository::default()),
            Arc::new(TestUserRepository::new(vec![user.clone()])),
            external_accounts.clone(),
        );

        let logged_in = app
            .login_verified_external_account(
                VerifiedExternalIdentity {
                    provider: ExternalAccountProvider::Jellyfin,
                    connection_id: "jellyfin-main".to_string(),
                    external_user_id: "remote-user".to_string(),
                    username: "fresh-name".to_string(),
                    display_name: Some("Fresh Name".to_string()),
                    avatar_url: Some("https://jellyfin.example.test/avatar".to_string()),
                    remote_password_configured: None,
                },
                test_media_server_connection(
                    scryer_domain::MediaServerProvider::Jellyfin,
                    "jellyfin-main",
                ),
            )
            .await
            .expect("login succeeds");

        assert_eq!(logged_in.0.id, user.id);
        let updated = external_accounts
            .get_by_provider_identity(
                ExternalAccountProvider::Jellyfin,
                "jellyfin-main",
                "remote-user",
            )
            .await
            .expect("load account")
            .expect("account exists");
        assert_eq!(updated.status, scryer_domain::ExternalAccountStatus::Active);
        assert_eq!(updated.external_user_id.as_deref(), Some("remote-user"));
        assert_eq!(updated.username, "fresh-name");
        assert_eq!(updated.display_name.as_deref(), Some("Fresh Name"));
        assert_eq!(
            updated.avatar_url.as_deref(),
            Some("https://jellyfin.example.test/avatar")
        );
        assert!(updated.verified_at.is_some());
        assert!(updated.last_login_at.is_some());
    }

    #[tokio::test]
    async fn active_login_refreshes_external_account_metadata() {
        let user = User {
            id: "user-1".to_string(),
            username: "local-user".to_string(),
            password_hash: None,
            password_change_required: false,
            account_kind: Default::default(),
            authorization: UserAuthorization {
                app: AppPermissionMask::NONE,
                libraries: HashMap::new(),
                default_library: LibraryPermissionMask::NONE,
                actor_capabilities: scryer_domain::ActorCapabilityMask::MANAGE_OWN_ACCOUNT,
                login_status: Default::default(),
                loaded: true,
            },
        };
        let now = Utc::now();
        let external_accounts = Arc::new(TestExternalAccountRepository::new(vec![
            UserExternalAccount {
                id: "active-account".to_string(),
                user_id: user.id.clone(),
                provider: ExternalAccountProvider::Plex,
                connection_id: "plex-main".to_string(),
                external_user_id: Some("remote-user".to_string()),
                username: "old-name".to_string(),
                display_name: None,
                avatar_url: None,
                status: scryer_domain::ExternalAccountStatus::Active,
                verified_at: Some(now),
                last_login_at: None,
                created_at: now,
                updated_at: now,
            },
        ]));
        let app = test_app_with_identity(
            Arc::new(TestSettingsRepository::default()),
            Arc::new(TestUserRepository::new(vec![user.clone()])),
            external_accounts.clone(),
        );

        app.login_verified_external_account(
            VerifiedExternalIdentity {
                provider: ExternalAccountProvider::Plex,
                connection_id: "plex-main".to_string(),
                external_user_id: "remote-user".to_string(),
                username: "fresh-plex".to_string(),
                display_name: Some("Fresh Plex".to_string()),
                avatar_url: Some("https://plex.example.test/avatar".to_string()),
                remote_password_configured: None,
            },
            test_media_server_connection(scryer_domain::MediaServerProvider::Plex, "plex-main"),
        )
        .await
        .expect("login succeeds");

        let updated = external_accounts
            .get_by_provider_identity(ExternalAccountProvider::Plex, "plex-main", "remote-user")
            .await
            .expect("load account")
            .expect("account exists");
        assert_eq!(updated.status, scryer_domain::ExternalAccountStatus::Active);
        assert_eq!(updated.username, "fresh-plex");
        assert_eq!(updated.display_name.as_deref(), Some("Fresh Plex"));
        assert_eq!(
            updated.avatar_url.as_deref(),
            Some("https://plex.example.test/avatar")
        );
        assert!(updated.last_login_at.is_some());
    }

    #[tokio::test]
    async fn disabled_user_rejects_external_login_without_updating_account() {
        let mut user = regular_user("disabled-external-user");
        user.set_login_status(scryer_domain::UserLoginStatus::Disabled);
        let account = UserExternalAccount {
            id: "disabled-account".to_string(),
            user_id: user.id.clone(),
            provider: ExternalAccountProvider::Plex,
            connection_id: "plex-main".to_string(),
            external_user_id: Some("remote-user".to_string()),
            username: "old-name".to_string(),
            display_name: None,
            avatar_url: None,
            status: ExternalAccountStatus::Active,
            verified_at: None,
            last_login_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let external_accounts = Arc::new(TestExternalAccountRepository::new(vec![account]));
        let app = test_app_with_identity(
            Arc::new(TestSettingsRepository::default()),
            Arc::new(TestUserRepository::new(vec![user])),
            external_accounts.clone(),
        );

        let result = app
            .login_verified_external_account(
                VerifiedExternalIdentity {
                    provider: ExternalAccountProvider::Plex,
                    connection_id: "plex-main".to_string(),
                    external_user_id: "remote-user".to_string(),
                    username: "fresh-name".to_string(),
                    display_name: Some("Fresh Name".to_string()),
                    avatar_url: None,
                    remote_password_configured: None,
                },
                test_media_server_connection(scryer_domain::MediaServerProvider::Plex, "plex-main"),
            )
            .await;
        assert!(matches!(result, Err(AppError::Unauthorized(_))));

        let stored = external_accounts
            .get_by_provider_identity(ExternalAccountProvider::Plex, "plex-main", "remote-user")
            .await
            .expect("load account")
            .expect("account exists");
        assert_eq!(stored.username, "old-name");
        assert!(stored.last_login_at.is_none());
    }

    #[tokio::test]
    async fn verified_identity_must_match_requested_connection() {
        let app = test_app(Arc::new(TestSettingsRepository::default()));

        let result = app.ensure_verified_identity_matches_request(
            &ExternalAccountProvider::Jellyfin,
            "jellyfin-main",
            &VerifiedExternalIdentity {
                provider: ExternalAccountProvider::Jellyfin,
                connection_id: "jellyfin-other".to_string(),
                external_user_id: "remote-user".to_string(),
                username: "remote-user".to_string(),
                display_name: None,
                avatar_url: None,
                remote_password_configured: None,
            },
        );

        assert!(
            matches!(result, Err(AppError::Validation(message)) if message.contains("did not match"))
        );
    }

    #[tokio::test]
    async fn verified_identity_must_match_requested_provider() {
        let app = test_app(Arc::new(TestSettingsRepository::default()));

        let result = app.ensure_verified_identity_matches_request(
            &ExternalAccountProvider::Jellyfin,
            "jellyfin-main",
            &VerifiedExternalIdentity {
                provider: ExternalAccountProvider::Plex,
                connection_id: "jellyfin-main".to_string(),
                external_user_id: "remote-user".to_string(),
                username: "remote-user".to_string(),
                display_name: None,
                avatar_url: None,
                remote_password_configured: None,
            },
        );

        assert!(
            matches!(result, Err(AppError::Validation(message)) if message.contains("provider"))
        );
    }

    #[tokio::test]
    async fn emby_local_and_connect_login_converge_on_the_local_user_id() {
        let user = regular_user("scryer-user");
        let now = Utc::now();
        let account = UserExternalAccount {
            id: "emby-account".to_string(),
            user_id: user.id.clone(),
            provider: ExternalAccountProvider::Emby,
            connection_id: "emby-main".to_string(),
            external_user_id: Some("local-emby-user-id".to_string()),
            username: "EmbyUser".to_string(),
            display_name: None,
            avatar_url: None,
            status: ExternalAccountStatus::Active,
            verified_at: Some(now),
            last_login_at: None,
            created_at: now,
            updated_at: now,
        };
        let external_accounts = Arc::new(TestExternalAccountRepository::new(vec![account]));
        let app = test_app_with_identity_media_servers_and_verifier(
            Arc::new(TestSettingsRepository::default()),
            Arc::new(TestUserRepository::new(vec![user.clone()])),
            external_accounts.clone(),
            vec![test_media_server_connection(
                scryer_domain::MediaServerProvider::Emby,
                "emby-main",
            )],
            Arc::new(TestExternalIdentityVerifier::with_emby_users(
                vec![crate::EmbyServerUser {
                    id: "local-emby-user-id".to_string(),
                    username: "EmbyUser".to_string(),
                    display_name: Some("Emby User".to_string()),
                    avatar_url: Some("/api/media-servers/emby-main/users/local/avatar".to_string()),
                }],
                Vec::new(),
            )),
        );

        let local = app
            .federated_login_with_emby(
                "emby-main".to_string(),
                EmbyConnectionMode::Local,
                "EmbyUser".to_string(),
                "   ".to_string(),
            )
            .await
            .expect("whitespace-only local Emby password is still non-empty");
        let connect = app
            .federated_login_with_emby(
                "emby-main".to_string(),
                EmbyConnectionMode::Connect,
                "EmbyUser".to_string(),
                "\t ".to_string(),
            )
            .await
            .expect("whitespace-only Connect password is still non-empty");
        let empty = app
            .federated_login_with_emby(
                "emby-main".to_string(),
                EmbyConnectionMode::Local,
                "EmbyUser".to_string(),
                String::new(),
            )
            .await;

        assert_eq!(local.0.id, user.id);
        assert_eq!(connect.0.id, user.id);
        assert!(
            matches!(empty, Err(AppError::Unauthorized(message)) if message == "Emby sign-in failed")
        );
        let linked = external_accounts
            .list_by_user_id(&user.id)
            .await
            .expect("list linked Emby identities");
        assert_eq!(linked.len(), 1);
        assert_eq!(linked[0].provider, ExternalAccountProvider::Emby);
        assert_eq!(
            linked[0].external_user_id.as_deref(),
            Some("local-emby-user-id")
        );
    }

    #[tokio::test]
    async fn disabling_emby_connect_does_not_disable_local_login() {
        let user = regular_user("local-only-user");
        let now = Utc::now();
        let account = UserExternalAccount {
            id: "local-only-emby-account".to_string(),
            user_id: user.id.clone(),
            provider: ExternalAccountProvider::Emby,
            connection_id: "emby-main".to_string(),
            external_user_id: Some("local-only-id".to_string()),
            username: "LocalOnly".to_string(),
            display_name: None,
            avatar_url: None,
            status: ExternalAccountStatus::Active,
            verified_at: Some(now),
            last_login_at: None,
            created_at: now,
            updated_at: now,
        };
        let mut connection =
            test_media_server_connection(scryer_domain::MediaServerProvider::Emby, "emby-main");
        connection.emby_connect_enabled = false;
        let app = test_app_with_identity_media_servers_and_verifier(
            Arc::new(TestSettingsRepository::default()),
            Arc::new(TestUserRepository::new(vec![user.clone()])),
            Arc::new(TestExternalAccountRepository::new(vec![account])),
            vec![connection],
            Arc::new(TestExternalIdentityVerifier::with_emby_users(
                vec![crate::EmbyServerUser {
                    id: "local-only-id".to_string(),
                    username: "LocalOnly".to_string(),
                    display_name: None,
                    avatar_url: None,
                }],
                Vec::new(),
            )),
        );

        let local = app
            .federated_login_with_emby(
                "emby-main".to_string(),
                EmbyConnectionMode::Local,
                "LocalOnly".to_string(),
                "password".to_string(),
            )
            .await
            .expect("local authentication remains enabled");
        assert_eq!(local.0.id, user.id);

        let connect = app
            .federated_login_with_emby(
                "emby-main".to_string(),
                EmbyConnectionMode::Connect,
                "LocalOnly".to_string(),
                "password".to_string(),
            )
            .await;
        assert!(
            matches!(connect, Err(AppError::Unauthorized(message)) if message == "Emby sign-in failed")
        );
    }

    #[tokio::test]
    async fn emby_passwordless_account_can_link_but_cannot_login() {
        let actor = admin_user();
        let verifier = Arc::new(TestExternalIdentityVerifier::with_emby_users(
            vec![crate::EmbyServerUser {
                id: "passwordless-id".to_string(),
                username: "Passwordless".to_string(),
                display_name: None,
                avatar_url: None,
            }],
            vec!["Passwordless".to_string()],
        ));
        let app = test_app_with_identity_media_servers_and_verifier(
            Arc::new(TestSettingsRepository::default()),
            Arc::new(TestUserRepository::new(vec![actor.clone()])),
            Arc::new(TestExternalAccountRepository::default()),
            vec![test_media_server_connection(
                scryer_domain::MediaServerProvider::Emby,
                "emby-main",
            )],
            verifier,
        );

        let linked = app
            .link_emby_account(
                &actor,
                "emby-main".to_string(),
                EmbyConnectionMode::Local,
                "Passwordless".to_string(),
                String::new(),
            )
            .await
            .expect("Emby confirms the empty password belongs to a passwordless user");
        assert_eq!(linked.external_user_id.as_deref(), Some("passwordless-id"));

        let login = app
            .federated_login_with_emby(
                "emby-main".to_string(),
                EmbyConnectionMode::Local,
                "Passwordless".to_string(),
                "not-a-secret".to_string(),
            )
            .await;
        assert!(
            matches!(login, Err(AppError::Unauthorized(message)) if message == "Emby sign-in failed")
        );
    }

    #[tokio::test]
    async fn emby_link_flattens_upstream_failures_without_echoing_details() {
        let actor = admin_user();
        let app = test_app_with_identity_media_servers_and_verifier(
            Arc::new(TestSettingsRepository::default()),
            Arc::new(TestUserRepository::new(vec![actor.clone()])),
            Arc::new(TestExternalAccountRepository::default()),
            vec![test_media_server_connection(
                scryer_domain::MediaServerProvider::Emby,
                "emby-main",
            )],
            Arc::new(TestExternalIdentityVerifier::default()),
        );

        for mode in [EmbyConnectionMode::Local, EmbyConnectionMode::Connect] {
            let error = app
                .link_emby_account(
                    &actor,
                    "emby-main".to_string(),
                    mode,
                    "missing-user".to_string(),
                    "request-secret".to_string(),
                )
                .await
                .expect_err("fake upstream failure");
            assert!(
                matches!(&error, AppError::Unauthorized(message) if message == "Emby sign-in failed")
            );
            assert!(!error.to_string().contains("request-secret"));
            assert!(!error.to_string().contains("upstream Emby detail"));
        }
    }

    #[tokio::test]
    async fn emby_invite_uses_selected_local_user_id_and_metadata() {
        let admin = admin_user();
        let target = regular_user("invite-target");
        let mut connection =
            test_media_server_connection(scryer_domain::MediaServerProvider::Emby, "emby-main");
        connection.api_key = Some("emby-api-key".to_string());
        let app = test_app_with_identity_media_servers_and_verifier(
            Arc::new(TestSettingsRepository::default()),
            Arc::new(TestUserRepository::new(vec![target.clone()])),
            Arc::new(TestExternalAccountRepository::default()),
            vec![connection],
            Arc::new(TestExternalIdentityVerifier::with_emby_users(
                vec![crate::EmbyServerUser {
                    id: "selected-local-id".to_string(),
                    username: "Invitee".to_string(),
                    display_name: Some("Emby Invitee".to_string()),
                    avatar_url: Some(
                        "/api/media-servers/emby-main/users/selected/avatar".to_string(),
                    ),
                }],
                Vec::new(),
            )),
        );

        let account = app
            .create_external_account_invite(
                &admin,
                &target.id,
                ExternalAccountProvider::Emby,
                "emby-main".to_string(),
                "ignored picker label".to_string(),
                Some("selected-local-id".to_string()),
            )
            .await
            .expect("create Emby invite");
        assert_eq!(
            account.external_user_id.as_deref(),
            Some("selected-local-id")
        );
        assert_eq!(account.username, "Invitee");
        assert_eq!(account.display_name.as_deref(), Some("Emby Invitee"));
        assert_eq!(account.status, ExternalAccountStatus::PendingClaim);
    }

    fn oauth_jellyfin_link_connection(api_key: &str) -> MediaServerConnection {
        let mut connection = test_media_server_connection(
            scryer_domain::MediaServerProvider::Jellyfin,
            "jellyfin-main",
        );
        connection.external_url = Some("https://jellyfin.example.test".to_string());
        connection.api_key = Some(api_key.to_string());
        connection
    }

    fn oauth_jellyfin_link_registration() -> OAuthClientRegistrationRecord {
        let now = Utc::now();
        OAuthClientRegistrationRecord {
            client_id: "jellyfin-plugin".to_string(),
            display_name: "Jellyfin plugin".to_string(),
            redirect_uris: vec!["https://jellyfin.example.test/Scryer/Auth/Callback".to_string()],
            enabled: true,
            kind: crate::oauth::OAuthClientKind::JellyfinPlugin,
            created_at: now,
            updated_at: now,
        }
    }

    fn oauth_jellyfin_link_grant(
        user_id: &str,
        client_id: &str,
        api_key_hash: String,
    ) -> OAuthRefreshGrantRecord {
        let now = Utc::now();
        OAuthRefreshGrantRecord {
            id: "jellyfin-link-grant".to_string(),
            family_id: "jellyfin-link-family".to_string(),
            user_id: user_id.to_string(),
            authorization_source: OAuthAuthorizationSource::Authenticated,
            client_id: client_id.to_string(),
            redirect_uri: "https://jellyfin.example.test/Scryer/Auth/Callback".to_string(),
            scope: "library jellyfin-link".to_string(),
            jellyfin_connection_id: Some("jellyfin-main".to_string()),
            jellyfin_external_url: Some("https://jellyfin.example.test".to_string()),
            jellyfin_base_url: Some("https://jellyfin.example.test".to_string()),
            jellyfin_api_key_hash: Some(api_key_hash),
            auth_session_version: "1".to_string(),
            created_at: now,
            updated_at: now,
            last_used_at: None,
            revoked_at: None,
            revoked_reason: None,
        }
    }

    fn oauth_jellyfin_link_verifier(user_id: &str) -> Arc<dyn ExternalIdentityVerifier> {
        Arc::new(TestExternalIdentityVerifier::with_jellyfin_users(vec![
            JellyfinServerUser {
                id: user_id.to_string(),
                username: "Jellyfin user".to_string(),
                display_name: Some("Jellyfin User".to_string()),
                avatar_url: None,
            },
        ]))
    }

    #[tokio::test]
    async fn oauth_jellyfin_link_rejects_ineligible_grants_before_remote_verification() {
        let actor = regular_user("oauth-user");
        let jellyfin_user_id = "0123456789abcdef0123456789abcdef";
        for (name, mutate) in [
            (
                "scope",
                Box::new(|grant: &mut OAuthRefreshGrantRecord| {
                    grant.scope = "library".to_string();
                }) as Box<dyn Fn(&mut OAuthRefreshGrantRecord)>,
            ),
            (
                "actor",
                Box::new(|grant: &mut OAuthRefreshGrantRecord| {
                    grant.user_id = "other-user".to_string();
                }),
            ),
            (
                "client",
                Box::new(|grant: &mut OAuthRefreshGrantRecord| {
                    grant.client_id = "other-client".to_string();
                }),
            ),
            (
                "binding",
                Box::new(|grant: &mut OAuthRefreshGrantRecord| {
                    grant.jellyfin_connection_id = None;
                }),
            ),
        ] {
            let oauth = Arc::new(TestOAuthRepository::new(
                Vec::new(),
                Some(oauth_jellyfin_link_registration()),
            ));
            let app = test_app_with_identity_oauth_media_servers_and_verifier(
                Arc::new(TestSettingsRepository::default()),
                Arc::new(NullUserRepository),
                Arc::new(TestExternalAccountRepository::default()),
                Arc::new(TestMediaServerConnectionRepository::new(vec![
                    oauth_jellyfin_link_connection("admin-key"),
                ])),
                oauth_jellyfin_link_verifier(jellyfin_user_id),
                oauth.clone(),
            );
            let mut grant = oauth_jellyfin_link_grant(
                &actor.id,
                "jellyfin-plugin",
                app.oauth_token_hash("jellyfin_link_api_key", "admin-key"),
            );
            mutate(&mut grant);
            oauth.replace_grants(vec![grant]).await;

            let result = app
                .link_current_oauth_jellyfin_account(
                    &actor,
                    "jellyfin-plugin",
                    "jellyfin-link-grant",
                    jellyfin_user_id,
                )
                .await;
            assert!(
                matches!(result, Err(AppError::Unauthorized(_))),
                "{name} should reject the OAuth grant, got {result:?}"
            );
        }
    }

    #[tokio::test]
    async fn oauth_jellyfin_link_rechecks_base_url_and_api_key_after_verification() {
        let actor = regular_user("oauth-user");
        let jellyfin_user_id = "0123456789abcdef0123456789abcdef";
        for (name, mut changed_connection) in [
            ("base URL", oauth_jellyfin_link_connection("admin-key")),
            ("API key", oauth_jellyfin_link_connection("rotated-key")),
        ] {
            if name == "base URL" {
                changed_connection.base_url = "https://different-jellyfin.example.test".to_string();
            }
            let connection = oauth_jellyfin_link_connection("admin-key");
            let oauth = Arc::new(TestOAuthRepository::new(
                Vec::new(),
                Some(oauth_jellyfin_link_registration()),
            ));
            let app = test_app_with_identity_oauth_media_servers_and_verifier(
                Arc::new(TestSettingsRepository::default()),
                Arc::new(NullUserRepository),
                Arc::new(TestExternalAccountRepository::default()),
                Arc::new(
                    TestMediaServerConnectionRepository::with_get_by_id_responses(
                        vec![connection.clone()],
                        vec![connection, changed_connection],
                    ),
                ),
                oauth_jellyfin_link_verifier(jellyfin_user_id),
                oauth.clone(),
            );
            let grant = oauth_jellyfin_link_grant(
                &actor.id,
                "jellyfin-plugin",
                app.oauth_token_hash("jellyfin_link_api_key", "admin-key"),
            );
            oauth.replace_grants(vec![grant]).await;

            let result = app
                .link_current_oauth_jellyfin_account(
                    &actor,
                    "jellyfin-plugin",
                    "jellyfin-link-grant",
                    jellyfin_user_id,
                )
                .await;
            assert!(
                matches!(result, Err(AppError::Unauthorized(ref message))
                    if message == "Jellyfin link authorization changed"),
                "changed {name} must reject after verification, got {result:?}"
            );
        }
    }

    #[tokio::test]
    async fn oauth_jellyfin_link_rechecks_redirect_allowlist_after_verification() {
        let actor = regular_user("oauth-user");
        let jellyfin_user_id = "0123456789abcdef0123456789abcdef";
        let registration = oauth_jellyfin_link_registration();
        let mut changed_registration = registration.clone();
        changed_registration.redirect_uris =
            vec!["https://jellyfin.example.test/Scryer/Auth/ChangedCallback".to_string()];
        let oauth = Arc::new(TestOAuthRepository::with_registration_responses(
            Vec::new(),
            Some(registration.clone()),
            vec![registration, changed_registration],
        ));
        let app = test_app_with_identity_oauth_media_servers_and_verifier(
            Arc::new(TestSettingsRepository::default()),
            Arc::new(NullUserRepository),
            Arc::new(TestExternalAccountRepository::default()),
            Arc::new(TestMediaServerConnectionRepository::new(vec![
                oauth_jellyfin_link_connection("admin-key"),
            ])),
            oauth_jellyfin_link_verifier(jellyfin_user_id),
            oauth.clone(),
        );
        let grant = oauth_jellyfin_link_grant(
            &actor.id,
            "jellyfin-plugin",
            app.oauth_token_hash("jellyfin_link_api_key", "admin-key"),
        );
        oauth.replace_grants(vec![grant]).await;

        let result = app
            .link_current_oauth_jellyfin_account(
                &actor,
                "jellyfin-plugin",
                "jellyfin-link-grant",
                jellyfin_user_id,
            )
            .await;

        assert!(
            matches!(result, Err(AppError::Unauthorized(ref message))
                if message == "Jellyfin link authorization changed"),
            "redirect changes during verification must reject before durable linking, got {result:?}"
        );
    }

    #[tokio::test]
    async fn oauth_jellyfin_link_hides_cross_user_identity_conflicts() {
        let actor = regular_user("oauth-user");
        let jellyfin_user_id = "0123456789abcdef0123456789abcdef";
        let mut existing = active_jellyfin_account("other-user");
        existing.external_user_id = Some(jellyfin_user_id.to_string());
        let oauth = Arc::new(TestOAuthRepository::new(
            Vec::new(),
            Some(oauth_jellyfin_link_registration()),
        ));
        let app = test_app_with_identity_oauth_media_servers_and_verifier(
            Arc::new(TestSettingsRepository::default()),
            Arc::new(NullUserRepository),
            Arc::new(TestExternalAccountRepository::new(vec![existing])),
            Arc::new(TestMediaServerConnectionRepository::new(vec![
                oauth_jellyfin_link_connection("admin-key"),
            ])),
            oauth_jellyfin_link_verifier(jellyfin_user_id),
            oauth.clone(),
        );
        oauth
            .replace_grants(vec![oauth_jellyfin_link_grant(
                &actor.id,
                "jellyfin-plugin",
                app.oauth_token_hash("jellyfin_link_api_key", "admin-key"),
            )])
            .await;

        let result = app
            .link_current_oauth_jellyfin_account(
                &actor,
                "jellyfin-plugin",
                "jellyfin-link-grant",
                jellyfin_user_id,
            )
            .await;
        assert!(
            matches!(result, Err(AppError::Validation(ref message))
                if message == "Jellyfin account could not be linked"),
            "cross-user conflict must stay generic, got {result:?}"
        );
    }

    #[tokio::test]
    async fn emby_auto_add_resolves_scryer_username_collision() {
        let existing = User {
            username: "EmbyUser".to_string(),
            ..regular_user("existing-user")
        };
        let mut connection =
            test_media_server_connection(scryer_domain::MediaServerProvider::Emby, "emby-main");
        connection.auto_add_enabled = true;
        let external_accounts = Arc::new(TestExternalAccountRepository::default());
        let app = test_app_with_identity_media_servers_and_verifier(
            Arc::new(TestSettingsRepository::default()),
            Arc::new(TestUserRepository::new(vec![existing])),
            external_accounts.clone(),
            vec![connection],
            Arc::new(TestExternalIdentityVerifier::with_emby_users(
                vec![crate::EmbyServerUser {
                    id: "auto-add-local-id".to_string(),
                    username: "EmbyUser".to_string(),
                    display_name: Some("Auto Add".to_string()),
                    avatar_url: None,
                }],
                Vec::new(),
            )),
        );

        let added = app
            .federated_login_with_emby(
                "emby-main".to_string(),
                EmbyConnectionMode::Local,
                "EmbyUser".to_string(),
                "password".to_string(),
            )
            .await
            .expect("auto-add Emby account");
        assert_eq!(added.0.username, "EmbyUser-2");
        assert_eq!(
            added.0.account_kind,
            scryer_domain::UserAccountKind::ExternalAutoProvisioned
        );
        let accounts = external_accounts
            .list_by_user_id(&added.0.id)
            .await
            .expect("list auto-added account");
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].provider, ExternalAccountProvider::Emby);
        assert_eq!(
            accounts[0].external_user_id.as_deref(),
            Some("auto-add-local-id")
        );
    }
}

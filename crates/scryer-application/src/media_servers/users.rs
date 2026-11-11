use super::*;

impl AppUseCase {
    pub async fn list_media_server_users(
        &self,
        actor: &User,
        search: Option<&str>,
    ) -> AppResult<Vec<MediaServerUserGroup>> {
        self.require_app_permission(actor, AppPermission::ManageUsers)
            .await?;
        let search = search.map(str::trim).filter(|value| !value.is_empty());
        let mut connections = self
            .services
            .integrations
            .media_server_connections
            .list(None)
            .await?;
        connections.sort_by(|left, right| {
            left.provider
                .as_str()
                .cmp(right.provider.as_str())
                .then_with(|| {
                    left.display_name
                        .to_ascii_lowercase()
                        .cmp(&right.display_name.to_ascii_lowercase())
                })
                .then_with(|| left.id.cmp(&right.id))
        });

        let mut tasks = tokio::task::JoinSet::new();
        let verifier = Arc::clone(&self.services.integrations.external_identity_verifier);
        let search = search.map(ToString::to_string);
        for connection in connections {
            if !connection.enabled || !connection.login_enabled {
                continue;
            }

            let Some(provider) = external_account_provider_for_media_server(&connection.provider)
            else {
                continue;
            };

            let verifier = Arc::clone(&verifier);
            let search = search.clone();
            tasks.spawn(async move {
                list_media_server_user_group_with_timeout(verifier, connection, provider, search)
                    .await
            });
        }

        let mut groups = Vec::new();
        while let Some(result) = tasks.join_next().await {
            match result {
                Ok(group) => groups.push(group),
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        "media server user lookup task failed before returning a group"
                    );
                }
            }
        }

        groups.sort_by(|left, right| {
            left.provider
                .as_str()
                .cmp(right.provider.as_str())
                .then_with(|| {
                    left.connection_name
                        .to_ascii_lowercase()
                        .cmp(&right.connection_name.to_ascii_lowercase())
                })
                .then_with(|| left.connection_id.cmp(&right.connection_id))
        });

        Ok(groups)
    }

    /// The accounts one Jellyfin or Emby connection's server knows, read with
    /// the connection's stored API key, for choosing which account to link.
    ///
    /// Choosing a name proves nothing: the link itself still signs in as that
    /// account. Listing a server's accounts is for whoever manages the
    /// connection, so it needs the same permission the connection mutations
    /// do. `include_linked` reads which accounts already carry a verified
    /// link; without it every account reports `linked: false`.
    pub async fn list_media_server_connection_accounts(
        &self,
        actor: &User,
        connection_id: &str,
        include_linked: bool,
    ) -> AppResult<Vec<MediaServerConnectionAccount>> {
        self.require_app_permission(actor, AppPermission::ManageSystemSettings)
            .await?;
        let connection_id = connection_id.trim();
        let connection = self
            .services
            .integrations
            .media_server_connections
            .get_by_id(connection_id)
            .await?
            .ok_or_else(|| {
                AppError::not_found_reason(
                    crate::error_reasons::media_servers::CONNECTION_NOT_FOUND,
                    format!("media server connection {connection_id}"),
                )
            })?;
        let api_key = connection
            .api_key
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let verifier = &self.services.integrations.external_identity_verifier;
        let mut users: Vec<MediaServerUser> = match connection.provider {
            MediaServerProvider::Jellyfin => {
                let api_key = api_key.ok_or_else(|| {
                    AppError::validation_refused(
                        crate::error_reasons::media_servers::JELLYFIN_USER_LISTING_REQUIRES_SAVED_API_KEY,
                        "Jellyfin user listing requires a saved API key; save an API key to load Jellyfin users",
                    )
                })?;
                verifier
                    .list_jellyfin_users(&connection.base_url, api_key, None)
                    .await?
                    .into_iter()
                    .map(MediaServerUser::from)
                    .collect()
            }
            MediaServerProvider::Emby => {
                let api_key = api_key.ok_or_else(|| {
                    AppError::validation_refused(
                        crate::error_reasons::media_servers::EMBY_USER_LISTING_REQUIRES_SAVED_INTEGRATION_API_KEY,
                        "Emby user listing requires a saved integration API key",
                    )
                })?;
                verifier
                    .list_emby_users(&connection.id, &connection.base_url, api_key, None)
                    .await?
                    .into_iter()
                    .map(MediaServerUser::from)
                    .collect()
            }
            MediaServerProvider::Plex => {
                return Err(AppError::validation_refused(
                    crate::error_reasons::media_servers::ACCOUNT_LISTING_REQUIRES_JELLYFIN_OR_EMBY_CONNECTION,
                    "choosing an account from the server's list needs a Jellyfin or Emby connection",
                ));
            }
        };
        users.sort_by(|left, right| {
            left.username
                .to_ascii_lowercase()
                .cmp(&right.username.to_ascii_lowercase())
                .then_with(|| left.id.cmp(&right.id))
        });

        let linked: std::collections::HashSet<String> = match (
            include_linked,
            connection.provider.external_account_provider(),
        ) {
            (true, Some(provider)) => self
                .services
                .identity
                .external_accounts
                .list_verified_by_connection(provider, &connection.id)
                .await?
                .into_iter()
                .filter_map(|account| account.external_user_id)
                .map(|id| id.trim().to_string())
                .collect(),
            _ => std::collections::HashSet::new(),
        };
        Ok(users
            .into_iter()
            .map(|user| MediaServerConnectionAccount {
                linked: linked.contains(user.id.trim()),
                user,
            })
            .collect())
    }
}

pub(super) async fn list_media_server_user_group_with_timeout(
    verifier: Arc<dyn ExternalIdentityVerifier>,
    connection: MediaServerConnection,
    provider: scryer_domain::ExternalAccountProvider,
    search: Option<String>,
) -> MediaServerUserGroup {
    let timeout_group = empty_media_server_user_group(&connection, provider.clone());
    match tokio::time::timeout(
        MEDIA_SERVER_USER_LIST_TIMEOUT,
        list_media_server_user_group(verifier, connection, provider, search),
    )
    .await
    {
        Ok(group) => group,
        Err(_) => {
            let mut group = timeout_group;
            group.status = MediaServerUserGroupStatus::Error;
            group.error_message = Some(format!(
                "Timed out after {} loading users from this server",
                media_server_user_list_timeout_label()
            ));
            group
        }
    }
}

pub(super) fn media_server_user_list_timeout_label() -> String {
    let seconds = MEDIA_SERVER_USER_LIST_TIMEOUT.as_secs();
    if seconds > 0 {
        return format!("{seconds} seconds");
    }
    format!(
        "{} milliseconds",
        MEDIA_SERVER_USER_LIST_TIMEOUT.as_millis()
    )
}

pub(super) async fn list_media_server_user_group(
    verifier: Arc<dyn ExternalIdentityVerifier>,
    connection: MediaServerConnection,
    provider: scryer_domain::ExternalAccountProvider,
    search: Option<String>,
) -> MediaServerUserGroup {
    let mut group = empty_media_server_user_group(&connection, provider.clone());
    match provider {
        scryer_domain::ExternalAccountProvider::Jellyfin => {
            let Some(api_key) = connection
                .api_key
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                group.status = MediaServerUserGroupStatus::MissingCredentials;
                group.error_message =
                    Some("Save a Jellyfin API key to load users from this server".into());
                return group;
            };

            match verifier
                .list_jellyfin_users(&connection.base_url, api_key, search.as_deref())
                .await
            {
                Ok(users) => {
                    group.users = users.into_iter().map(MediaServerUser::from).collect();
                }
                Err(error) => {
                    group.status = MediaServerUserGroupStatus::Error;
                    group.error_message = Some(error.to_string());
                }
            }
        }
        scryer_domain::ExternalAccountProvider::Plex => {
            if connection
                .machine_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .is_none()
            {
                group.status = MediaServerUserGroupStatus::Error;
                group.error_message =
                    Some("Discover and select a Plex server before loading users".into());
                return group;
            }

            let Some(plex_auth_token) = connection
                .api_key
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                group.status = MediaServerUserGroupStatus::MissingCredentials;
                group.error_message =
                    Some("Save a Plex token to load users from this server".into());
                return group;
            };

            match verifier
                .list_plex_users(plex_auth_token, search.as_deref())
                .await
            {
                Ok(users) => {
                    group.users = users.into_iter().map(MediaServerUser::from).collect();
                }
                Err(error) => {
                    group.status = MediaServerUserGroupStatus::Error;
                    group.error_message = Some(error.to_string());
                }
            }
        }
        scryer_domain::ExternalAccountProvider::Emby => {
            let Some(api_key) = connection
                .api_key
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                group.status = MediaServerUserGroupStatus::MissingCredentials;
                group.error_message =
                    Some("Save an Emby integration API key to load users from this server".into());
                return group;
            };
            match verifier
                .list_emby_users(
                    &connection.id,
                    &connection.base_url,
                    api_key,
                    search.as_deref(),
                )
                .await
            {
                Ok(users) => {
                    group.users = users.into_iter().map(MediaServerUser::from).collect();
                }
                Err(error) => {
                    group.status = MediaServerUserGroupStatus::Error;
                    group.error_message = Some(error.to_string());
                }
            }
        }
    }

    group.users.sort_by(|left, right| {
        left.username
            .to_ascii_lowercase()
            .cmp(&right.username.to_ascii_lowercase())
            .then_with(|| left.id.cmp(&right.id))
    });
    group
}

pub(super) fn empty_media_server_user_group(
    connection: &MediaServerConnection,
    provider: scryer_domain::ExternalAccountProvider,
) -> MediaServerUserGroup {
    MediaServerUserGroup {
        connection_id: connection.id.clone(),
        connection_name: connection.display_name.clone(),
        provider,
        status: MediaServerUserGroupStatus::Ready,
        error_message: None,
        users: Vec::new(),
    }
}

pub(super) fn external_account_provider_for_media_server(
    provider: &MediaServerProvider,
) -> Option<scryer_domain::ExternalAccountProvider> {
    match provider {
        MediaServerProvider::Jellyfin => Some(scryer_domain::ExternalAccountProvider::Jellyfin),
        MediaServerProvider::Plex => Some(scryer_domain::ExternalAccountProvider::Plex),
        MediaServerProvider::Emby => Some(scryer_domain::ExternalAccountProvider::Emby),
    }
}

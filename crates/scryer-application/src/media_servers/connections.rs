use super::*;

impl AppUseCase {
    pub async fn list_media_server_connections(
        &self,
        actor: &User,
        provider: Option<MediaServerProvider>,
    ) -> AppResult<Vec<MediaServerConnection>> {
        self.require_app_permission(actor, AppPermission::ManageSystemSettings)
            .await?;
        self.services
            .integrations
            .media_server_connections
            .list(provider)
            .await
    }

    pub async fn get_media_server_connection(
        &self,
        actor: &User,
        id: &str,
    ) -> AppResult<Option<MediaServerConnection>> {
        self.require_app_permission(actor, AppPermission::ManageSystemSettings)
            .await?;
        self.services
            .integrations
            .media_server_connections
            .get_by_id(id.trim())
            .await
    }

    pub async fn create_media_server_connection(
        &self,
        actor: &User,
        draft: MediaServerConnectionDraft,
    ) -> AppResult<MediaServerConnection> {
        self.require_media_server_permission(actor, &draft).await?;
        let now = Utc::now();
        let connection_id = scryer_domain::Id::new().0;
        let plex_selection = self
            .resolve_plex_server_selection(
                &draft.provider,
                None,
                draft.machine_id.clone(),
                draft.plex_auth_token.as_deref(),
                draft.plex_server_id.as_deref(),
            )
            .await?;
        let api_key_supplied = draft
            .api_key
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty());
        let mut api_key = match draft.provider {
            MediaServerProvider::Plex => draft.api_key.clone().or(draft.plex_auth_token.clone()),
            MediaServerProvider::Jellyfin | MediaServerProvider::Emby => draft.api_key.clone(),
        };
        let mut base_url = draft.base_url.clone();
        let mut emby_server_id = None;
        let mut emby_connect_enabled = false;
        let mut emby_exchange_cleanup = None;
        if draft.provider == MediaServerProvider::Emby {
            let resolved = self
                .resolve_emby_credentials_for_setup(
                    &connection_id,
                    draft.base_url.as_str(),
                    draft.emby_connection_mode,
                    draft.emby_local_setup_method,
                    draft.emby_connect_enabled,
                    draft.api_key.as_deref(),
                    draft.admin_username.as_deref(),
                    draft.admin_password.as_deref(),
                    draft.emby_connect_username_or_email.as_deref(),
                    draft.emby_connect_password.as_deref(),
                    draft.emby_connect_server_id.as_deref(),
                )
                .await?;
            base_url = resolved.base_url;
            api_key = Some(resolved.api_key);
            emby_server_id = Some(resolved.server_id);
            emby_connect_enabled = resolved.connect_enabled;
            emby_exchange_cleanup = resolved.cleanup;
        }
        let normalized = self
            .normalize_media_server_connection(
                connection_id.clone(),
                draft.provider,
                draft.display_name,
                base_url,
                draft.external_url,
                draft.enabled,
                draft.login_enabled,
                draft.linking_enabled,
                draft.auto_add_enabled,
                draft.default_app_permissions,
                draft.default_library_grants,
                plex_selection.machine_id,
                api_key,
                emby_server_id,
                emby_connect_enabled,
                draft.path_mappings,
                now,
                now,
            )
            .await;
        let mut connection = match normalized {
            Ok(connection) => connection,
            Err(error) => {
                if let Some(cleanup) = emby_exchange_cleanup.take() {
                    self.services
                        .integrations
                        .external_identity_verifier
                        .finish_emby_api_key_exchange(&connection_id, cleanup, true)
                        .await;
                }
                return Err(error);
            }
        };
        connection.api_key = self
            .jellyfin_api_key_from_credentials_or_input(
                &connection,
                draft.admin_username.as_deref(),
                draft.admin_password.as_deref(),
                connection.api_key.clone(),
                api_key_supplied,
            )
            .await?;

        let create_result = async {
            self.test_media_server_connection_internal(
                &connection,
                draft.plex_auth_token.as_deref(),
                false,
            )
            .await?;
            self.services
                .integrations
                .media_server_connections
                .create(connection)
                .await
        }
        .await;
        let created = match create_result {
            Ok(created) => {
                if let Some(cleanup) = emby_exchange_cleanup.take() {
                    self.services
                        .integrations
                        .external_identity_verifier
                        .finish_emby_api_key_exchange(&connection_id, cleanup, false)
                        .await;
                }
                created
            }
            Err(error) => {
                if let Some(cleanup) = emby_exchange_cleanup.take() {
                    self.services
                        .integrations
                        .external_identity_verifier
                        .finish_emby_api_key_exchange(&connection_id, cleanup, true)
                        .await;
                }
                return Err(error);
            }
        };
        if created.enabled
            && let Err(error) = self
                .refresh_media_server_playback_mappings_for_connection(&created)
                .await
        {
            tracing::warn!(
                connection_id = created.id.as_str(),
                error = %error,
                "initial media server playback catalog scan failed"
            );
        }
        self.emit_configuration_changed_event(
            actor,
            "media_server_connection",
            Some(created.id.clone()),
            scryer_domain::ConfigurationChangeAction::Saved,
        )
        .await;
        Ok(created)
    }

    pub async fn update_media_server_connection(
        &self,
        actor: &User,
        patch: MediaServerConnectionPatch,
    ) -> AppResult<MediaServerConnection> {
        self.require_app_permission(actor, AppPermission::ManageSystemSettings)
            .await?;
        let id = patch.id.trim().to_string();
        if id.is_empty() {
            return Err(AppError::Validation(
                "media server connection id is required".into(),
            ));
        }
        let existing = self
            .services
            .integrations
            .media_server_connections
            .get_by_id(&id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("media server connection {id}")))?;

        let provider = patch
            .provider
            .clone()
            .unwrap_or_else(|| existing.provider.clone());
        if media_server_update_requires_manage_permissions(&existing, &patch, &provider) {
            self.require_app_permission(actor, AppPermission::ManagePermissions)
                .await?;
        }
        if provider != existing.provider
            && self
                .services
                .integrations
                .media_server_connections
                .has_external_accounts(&id)
                .await?
        {
            return Err(AppError::Validation(
                "cannot change provider for a connection with linked accounts".into(),
            ));
        }
        let requested_machine_id = if patch.clear_machine_id {
            None
        } else {
            patch.machine_id.clone().or(existing.machine_id.clone())
        };
        let plex_selection = self
            .resolve_plex_server_selection(
                &provider,
                existing.machine_id.as_deref(),
                requested_machine_id,
                patch.plex_auth_token.as_deref(),
                patch.plex_server_id.as_deref(),
            )
            .await?;
        let api_key_supplied = patch
            .api_key
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty());
        let existing_api_key = if existing.provider == provider {
            existing.api_key.clone()
        } else {
            None
        };
        let mut api_key = if patch.clear_api_key {
            None
        } else if provider == MediaServerProvider::Plex {
            patch
                .plex_auth_token
                .clone()
                .or(patch.api_key.clone())
                .or(existing_api_key)
        } else {
            patch.api_key.clone().or(existing_api_key)
        };
        let enabled = patch.enabled.unwrap_or(existing.enabled);
        if provider == MediaServerProvider::Emby && enabled && patch.clear_api_key {
            return Err(AppError::Validation(
                "an enabled Emby connection must retain a verified API key".into(),
            ));
        }
        let mut base_url = patch
            .base_url
            .clone()
            .unwrap_or_else(|| existing.base_url.clone());
        let mut emby_server_id = (provider == MediaServerProvider::Emby)
            .then(|| existing.emby_server_id.clone())
            .flatten();
        let mut emby_connect_enabled = if provider == MediaServerProvider::Emby {
            patch
                .emby_connect_enabled
                .unwrap_or(existing.emby_connect_enabled)
        } else {
            false
        };
        let rotate_emby_credentials = provider == MediaServerProvider::Emby
            && (patch.emby_connection_mode.is_some()
                || existing.provider != MediaServerProvider::Emby);
        let mut emby_exchange_cleanup = None;
        if rotate_emby_credentials {
            let resolved = self
                .resolve_emby_credentials_for_setup(
                    &id,
                    &base_url,
                    patch.emby_connection_mode,
                    patch.emby_local_setup_method,
                    patch.emby_connect_enabled,
                    patch.api_key.as_deref(),
                    patch.admin_username.as_deref(),
                    patch.admin_password.as_deref(),
                    patch.emby_connect_username_or_email.as_deref(),
                    patch.emby_connect_password.as_deref(),
                    patch.emby_connect_server_id.as_deref(),
                )
                .await?;
            base_url = resolved.base_url;
            api_key = Some(resolved.api_key);
            emby_server_id = Some(resolved.server_id);
            emby_connect_enabled = resolved.connect_enabled;
            emby_exchange_cleanup = resolved.cleanup;
        }
        if provider == MediaServerProvider::Emby
            && patch.base_url.is_some()
            && !rotate_emby_credentials
        {
            let stored_api_key = api_key.as_deref().ok_or_else(|| {
                AppError::Validation("changing an Emby server URL requires a stored API key".into())
            })?;
            let identity = self
                .services
                .integrations
                .external_identity_verifier
                .test_emby_api_key(&id, &base_url, stored_api_key, emby_server_id.as_deref())
                .await?;
            base_url = identity.api_base_url;
            emby_server_id = Some(identity.server_id);
        }
        if emby_connect_enabled && emby_server_id.is_none() {
            return Err(AppError::Validation(
                "Emby Connect login requires a verified server identity".into(),
            ));
        }

        let normalized = self
            .normalize_media_server_connection(
                id.clone(),
                provider,
                patch
                    .display_name
                    .unwrap_or_else(|| existing.display_name.clone()),
                base_url,
                patch.external_url.or_else(|| existing.external_url.clone()),
                enabled,
                patch.login_enabled.unwrap_or(existing.login_enabled),
                patch.linking_enabled.unwrap_or(existing.linking_enabled),
                patch.auto_add_enabled.unwrap_or(existing.auto_add_enabled),
                patch
                    .default_app_permissions
                    .unwrap_or(existing.default_app_permissions),
                patch
                    .default_library_grants
                    .unwrap_or_else(|| existing.default_library_grants.clone()),
                plex_selection.machine_id,
                api_key,
                emby_server_id,
                emby_connect_enabled,
                patch
                    .path_mappings
                    .unwrap_or_else(|| existing.path_mappings.clone()),
                existing.created_at,
                Utc::now(),
            )
            .await;
        let mut connection = match normalized {
            Ok(connection) => connection,
            Err(error) => {
                if let Some(cleanup) = emby_exchange_cleanup.take() {
                    self.services
                        .integrations
                        .external_identity_verifier
                        .finish_emby_api_key_exchange(&id, cleanup, true)
                        .await;
                }
                return Err(error);
            }
        };

        connection.api_key = self
            .jellyfin_api_key_from_credentials_or_input(
                &connection,
                patch.admin_username.as_deref(),
                patch.admin_password.as_deref(),
                connection.api_key.clone(),
                api_key_supplied,
            )
            .await?;

        let update_result = async {
            self.test_media_server_connection_internal(
                &connection,
                patch.plex_auth_token.as_deref(),
                false,
            )
            .await?;
            self.services
                .integrations
                .media_server_connections
                .update(connection)
                .await
        }
        .await;
        let updated = match update_result {
            Ok(updated) => {
                if let Some(cleanup) = emby_exchange_cleanup.take() {
                    self.services
                        .integrations
                        .external_identity_verifier
                        .finish_emby_api_key_exchange(&id, cleanup, false)
                        .await;
                }
                updated
            }
            Err(error) => {
                if let Some(cleanup) = emby_exchange_cleanup.take() {
                    self.services
                        .integrations
                        .external_identity_verifier
                        .finish_emby_api_key_exchange(&id, cleanup, true)
                        .await;
                }
                return Err(error);
            }
        };
        if updated.enabled
            && let Err(error) = self
                .refresh_media_server_playback_mappings_for_connection(&updated)
                .await
        {
            tracing::warn!(
                connection_id = updated.id.as_str(),
                error = %error,
                "media server playback catalog scan after connection update failed"
            );
        }
        self.emit_configuration_changed_event(
            actor,
            "media_server_connection",
            Some(updated.id.clone()),
            scryer_domain::ConfigurationChangeAction::Updated,
        )
        .await;
        Ok(updated)
    }

    pub async fn delete_media_server_connection(&self, actor: &User, id: &str) -> AppResult<()> {
        self.require_app_permission(actor, AppPermission::ManageSystemSettings)
            .await?;
        let id = id.trim();
        if self
            .services
            .integrations
            .media_server_connections
            .has_external_accounts(id)
            .await?
        {
            return Err(AppError::Validation(
                "media server connection is referenced by linked accounts; disable it instead"
                    .into(),
            ));
        }
        if self
            .services
            .integrations
            .media_server_connections
            .has_notification_channels(id)
            .await?
        {
            return Err(AppError::Validation(
                "media server connection is referenced by notification channels; disable it instead"
                    .into(),
            ));
        }
        self.services
            .integrations
            .media_server_connections
            .delete(id)
            .await?;
        self.emit_configuration_changed_event(
            actor,
            "media_server_connection",
            Some(id.to_string()),
            scryer_domain::ConfigurationChangeAction::Deleted,
        )
        .await;
        Ok(())
    }

    pub async fn test_media_server_connection(
        &self,
        actor: &User,
        id: &str,
        plex_auth_token: Option<&str>,
    ) -> AppResult<()> {
        self.require_app_permission(actor, AppPermission::ManageSystemSettings)
            .await?;
        let mut connection = self
            .services
            .integrations
            .media_server_connections
            .get_by_id(id.trim())
            .await?
            .ok_or_else(|| AppError::NotFound(format!("media server connection {}", id.trim())))?;
        if connection.provider == MediaServerProvider::Emby {
            let api_key = connection
                .api_key
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    AppError::Validation(
                        "Emby connection test requires a saved integration API key".into(),
                    )
                })?;
            let identity = self
                .services
                .integrations
                .external_identity_verifier
                .test_emby_api_key(
                    &connection.id,
                    &connection.base_url,
                    api_key,
                    connection.emby_server_id.as_deref(),
                )
                .await?;
            if connection.emby_server_id.is_none() || connection.base_url != identity.api_base_url {
                connection.base_url = identity.api_base_url;
                connection.emby_server_id = Some(identity.server_id);
                connection.updated_at = Utc::now();
                self.services
                    .integrations
                    .media_server_connections
                    .update(connection)
                    .await?;
            }
            return Ok(());
        }
        self.test_media_server_connection_internal(&connection, plex_auth_token, true)
            .await
    }

    pub(super) async fn require_media_server_permission(
        &self,
        actor: &User,
        draft: &MediaServerConnectionDraft,
    ) -> AppResult<()> {
        self.require_app_permission(actor, AppPermission::ManageSystemSettings)
            .await?;
        if !draft.default_app_permissions.is_empty()
            || draft
                .default_library_grants
                .iter()
                .any(|grant| !grant.permissions.is_empty())
        {
            self.require_app_permission(actor, AppPermission::ManagePermissions)
                .await?;
        }
        Ok(())
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "normalization mirrors the API payload"
    )]
    pub(super) async fn normalize_media_server_connection(
        &self,
        id: String,
        provider: MediaServerProvider,
        display_name: String,
        base_url: String,
        external_url: Option<String>,
        enabled: bool,
        login_enabled: bool,
        linking_enabled: bool,
        auto_add_enabled: bool,
        default_app_permissions: AppPermissionMask,
        default_library_grants: Vec<MediaServerDefaultLibraryGrant>,
        machine_id: Option<String>,
        api_key: Option<String>,
        emby_server_id: Option<String>,
        emby_connect_enabled: bool,
        path_mappings: Vec<MediaServerPathMapping>,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
    ) -> AppResult<MediaServerConnection> {
        let id = id.trim().to_string();
        if id.is_empty() {
            return Err(AppError::Validation(
                "media server connection id is required".into(),
            ));
        }
        let base_url = normalize_media_server_base_url(&provider, base_url)?;
        let external_url = normalize_media_server_external_url(external_url)?;
        let display_name = display_name.trim().to_string();
        let display_name = if display_name.is_empty() {
            default_media_server_display_name(&provider).to_string()
        } else {
            display_name
        };
        let machine_id = normalize_optional_string(machine_id);
        let api_key = normalize_optional_string(api_key);
        let path_mappings = normalize_path_mappings(path_mappings)?;
        let default_library_grants = normalize_default_library_grants(default_library_grants);

        let (
            login_enabled,
            linking_enabled,
            auto_add_enabled,
            default_app_permissions,
            default_library_grants,
        ) = if provider.supports_external_auth() {
            (
                login_enabled,
                linking_enabled,
                auto_add_enabled,
                default_app_permissions,
                default_library_grants,
            )
        } else {
            (false, false, false, AppPermissionMask::NONE, Vec::new())
        };
        if provider == MediaServerProvider::Plex
            && (login_enabled || linking_enabled || auto_add_enabled)
            && machine_id.is_none()
        {
            return Err(AppError::Validation(
                "Discover and select a Plex server before enabling login, linking, or auto-add"
                    .into(),
            ));
        }

        Ok(MediaServerConnection {
            id,
            provider: provider.clone(),
            display_name,
            base_url,
            external_url,
            enabled,
            login_enabled,
            linking_enabled,
            auto_add_enabled,
            default_app_permissions,
            default_library_grants,
            machine_id: match provider {
                MediaServerProvider::Plex => machine_id,
                MediaServerProvider::Jellyfin | MediaServerProvider::Emby => None,
            },
            api_key: match provider {
                MediaServerProvider::Jellyfin
                | MediaServerProvider::Emby
                | MediaServerProvider::Plex => api_key,
            },
            emby_server_id: match provider {
                MediaServerProvider::Emby => normalize_optional_string(emby_server_id),
                MediaServerProvider::Jellyfin | MediaServerProvider::Plex => None,
            },
            emby_connect_enabled: provider == MediaServerProvider::Emby && emby_connect_enabled,
            path_mappings: match provider {
                MediaServerProvider::Jellyfin
                | MediaServerProvider::Emby
                | MediaServerProvider::Plex => path_mappings,
            },
            created_at,
            updated_at,
        })
    }

    pub(super) async fn test_media_server_connection_internal(
        &self,
        connection: &MediaServerConnection,
        plex_auth_token: Option<&str>,
        require_plex_token: bool,
    ) -> AppResult<()> {
        match connection.provider {
            MediaServerProvider::Jellyfin => {
                self.services
                    .integrations
                    .external_identity_verifier
                    .test_jellyfin_connection(&connection.base_url)
                    .await?;
                if let Some(api_key) = connection.api_key.as_deref() {
                    self.services
                        .integrations
                        .external_identity_verifier
                        .test_jellyfin_api_key(&connection.base_url, api_key)
                        .await?;
                }
            }
            MediaServerProvider::Plex => {
                let has_auth_capability = connection.login_enabled
                    || connection.linking_enabled
                    || connection.auto_add_enabled;
                if has_auth_capability && connection.machine_id.is_none() {
                    return Err(AppError::Validation(
                        "Discover and select a Plex server before enabling login, linking, or auto-add"
                            .into(),
                    ));
                }
                let token = plex_auth_token
                    .map(str::trim)
                    .filter(|value| !value.is_empty());
                if require_plex_token && token.is_none() {
                    return Err(AppError::Validation(
                        "Sign in with Plex to test this connection".into(),
                    ));
                }
                if let Some(token) = token {
                    let servers = self
                        .services
                        .integrations
                        .external_identity_verifier
                        .discover_plex_servers(token)
                        .await?;
                    if let Some(machine_id) = connection.machine_id.as_deref()
                        && !servers.iter().any(|server| server.id == machine_id)
                    {
                        return Err(AppError::Unauthorized(
                            "Plex account does not have access to the selected server".into(),
                        ));
                    }
                }
            }
            MediaServerProvider::Emby => {
                let api_key = connection
                    .api_key
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty());
                let Some(api_key) = api_key else {
                    if !connection.enabled {
                        return Ok(());
                    }
                    return Err(AppError::Validation(
                        "Emby connection requires a verified integration API key".into(),
                    ));
                };
                let identity = self
                    .services
                    .integrations
                    .external_identity_verifier
                    .test_emby_api_key(
                        &connection.id,
                        &connection.base_url,
                        api_key,
                        connection.emby_server_id.as_deref(),
                    )
                    .await?;
                if connection.emby_server_id.as_deref() != Some(identity.server_id.as_str()) {
                    return Err(AppError::Validation(
                        "Emby server identity does not match the saved connection".into(),
                    ));
                }
            }
        }
        Ok(())
    }
}

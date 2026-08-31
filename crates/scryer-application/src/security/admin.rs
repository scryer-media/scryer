use super::*;
use crate::event_views::history_event_from_domain_event;
use scryer_domain::ConfigurationChangeAction;

const DEFAULT_ADMIN_USERNAME: &str = "admin";
const RECOVERY_ADMIN_USERNAME: &str = "recovery-admin";
const ANONYMOUS_AUDIT_USERNAME: &str = "anonymous";

impl AppUseCase {
    fn required_startup_admin_app_permissions() -> scryer_domain::AppPermissionMask {
        scryer_domain::UserAuthorization::full_admin().app
    }

    pub fn is_reserved_recovery_username(username: &str) -> bool {
        Self::normalize_local_username(username).eq_ignore_ascii_case(RECOVERY_ADMIN_USERNAME)
    }

    pub(crate) fn is_default_admin_username(username: &str) -> bool {
        Self::normalize_local_username(username).eq_ignore_ascii_case(DEFAULT_ADMIN_USERNAME)
    }

    pub(crate) fn is_reserved_local_username(username: &str) -> bool {
        let normalized = Self::normalize_local_username(username);
        normalized.eq_ignore_ascii_case(RECOVERY_ADMIN_USERNAME)
            || normalized.eq_ignore_ascii_case(ANONYMOUS_AUDIT_USERNAME)
    }

    fn reserved_local_username_error(username: &str) -> AppError {
        if Self::normalize_local_username(username).eq_ignore_ascii_case(ANONYMOUS_AUDIT_USERNAME) {
            AppError::Validation("anonymous is reserved for authless audit attribution".into())
        } else {
            AppError::Validation(format!(
                "{RECOVERY_ADMIN_USERNAME} is reserved for instance recovery"
            ))
        }
    }

    async fn ensure_user_admin_permission_masks(&self, user: &User) -> AppResult<()> {
        let admin_authorization = scryer_domain::UserAuthorization::full_admin();
        self.services
            .catalog
            .libraries
            .set_app_permission_mask_for_user(&user.id, admin_authorization.app)
            .await?;
        let mut seen_library_ids = std::collections::HashSet::new();
        let grants = self
            .services
            .catalog
            .libraries
            .list(None)
            .await?
            .into_iter()
            .filter_map(|library| {
                if !seen_library_ids.insert(library.id.clone()) {
                    return None;
                }
                Some(scryer_domain::LibraryGrant {
                    user_id: user.id.clone(),
                    library_id: library.id,
                    permissions: admin_authorization.default_library,
                })
            })
            .collect();
        self.services
            .catalog
            .libraries
            .set_grants_for_user(&user.id, grants)
            .await
    }

    pub async fn system_health(&self, actor: &User) -> AppResult<SystemHealth> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        let titles = self.services.catalog.titles.list(None, None).await?;
        let users = self.services.identity.users.list_all().await?;
        let recent_activity = self.recent_activity_page(12, 0).await?;

        let mut titles_movie = 0usize;
        let mut titles_series = 0usize;
        let mut titles_anime = 0usize;
        let titles_other = 0usize;
        let mut monitored_titles = 0usize;
        let mut recent_event_preview = Vec::with_capacity(std::cmp::min(3, recent_activity.len()));

        for title in &titles {
            if title.monitored {
                monitored_titles += 1;
            }

            match title.facet {
                MediaFacet::Movie => titles_movie += 1,
                MediaFacet::Series => titles_series += 1,
                MediaFacet::Anime => titles_anime += 1,
            }
        }

        for event in recent_activity.iter().take(3) {
            recent_event_preview.push(event.message.clone());
        }

        let datastore_info = self.services.config.system_info.datastore_info().await.ok();
        let db_migration_version = datastore_info
            .as_ref()
            .and_then(|info| info.current_migration_key.clone());
        let datastore_engine = datastore_info
            .map(|info| info.engine)
            .unwrap_or_else(|| "unknown".to_string());
        let indexer_stats = self.services.integrations.indexer_stats.all_stats();

        Ok(SystemHealth {
            service_ready: true,
            db_path: datastore_engine.clone(),
            datastore_engine,
            datastore_migration_key: db_migration_version.clone(),
            runtime_path_style: RuntimePathStyle::current(),
            total_titles: titles.len(),
            monitored_titles,
            total_users: users.len(),
            titles_movie,
            titles_series,
            titles_anime,
            titles_other,
            recent_events: recent_activity.len(),
            recent_event_preview,
            db_migration_version,
            indexer_stats,
        })
    }

    pub async fn disk_space(&self, actor: &User) -> AppResult<Vec<DiskSpaceInfo>> {
        let libraries = self
            .list_libraries_for_permission(actor, None, scryer_domain::LibraryPermission::View)
            .await?;

        let mut seen_paths = std::collections::HashSet::new();
        let mut results = Vec::new();

        for library in libraries {
            for root in library.roots {
                let path = root.path;
                if !seen_paths.insert(path.clone()) {
                    continue;
                }

                if let Some(space) = filesystem_space(&path) {
                    let total = space.total_bytes;
                    let free = space.available_bytes;
                    let used = total.saturating_sub(free);
                    results.push(DiskSpaceInfo {
                        path,
                        label: library.name.clone(),
                        total_bytes: total,
                        free_bytes: free,
                        used_bytes: used,
                    });
                } else {
                    tracing::warn!(path = path.as_str(), "failed to query disk space");
                }
            }
        }

        Ok(results)
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<HistoryEvent> {
        let (tx, rx) = broadcast::channel(128);
        let app = self.clone();
        tokio::spawn(async move {
            let mut wake_rx = app.runtime.events.domain_event_broadcast.subscribe();
            let mut cursor = 0_i64;

            loop {
                let events = match app
                    .services
                    .events
                    .domain_events
                    .list_after_sequence(cursor, 100)
                    .await
                {
                    Ok(events) if !events.is_empty() => events,
                    Ok(_) => match wake_rx.recv().await {
                        Ok(sequence) => {
                            if sequence > cursor {
                                cursor = sequence.saturating_sub(1);
                            }
                            continue;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            tracing::debug!(
                                "history event subscription lagged, skipped {n} wakeups"
                            );
                            continue;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    },
                    Err(error) => {
                        tracing::warn!("history event subscription replay failed: {error}");
                        break;
                    }
                };

                for event in events {
                    cursor = event.sequence;
                    if let Some(history) = history_event_from_domain_event(&event)
                        && tx.send(history).is_err()
                    {
                        return;
                    }
                }
            }
        });
        rx
    }

    pub async fn ensure_default_admin(&self, username: &str, password: &str) -> AppResult<User> {
        let username = Self::normalize_local_username(username);
        if username.is_empty() {
            return Err(AppError::Validation("admin username is required".into()));
        }
        if Self::is_reserved_local_username(username) {
            return Err(Self::reserved_local_username_error(username));
        }
        if password.is_empty() {
            return Err(AppError::Validation("admin password is required".into()));
        }
        if let Some(mut found) = self
            .services
            .identity
            .users
            .get_by_username(username)
            .await?
        {
            self.ensure_user_admin_permission_masks(&found).await?;
            // Migration-seeded admin may lack a password hash — set one.
            if found.password_hash.is_none() {
                let auth_session_version = Id::new().0;
                found = self
                    .services
                    .identity
                    .users
                    .update_password_and_invalidate_sessions(
                        &found.id,
                        self.hash_password(password)?,
                        false,
                        &auth_session_version,
                    )
                    .await?;
            }
            self.refresh_cached_jwt_signing_key(&found).await?;
            return Ok(found);
        }

        let user = User {
            id: Id::new().0,
            username: username.to_string(),
            password_hash: Some(self.hash_password(password)?),
            password_change_required: false,
            account_kind: Default::default(),
            authorization: Default::default(),
        };

        let user = self.services.identity.users.create(user).await?;
        self.ensure_user_admin_permission_masks(&user).await?;
        self.cache_jwt_signing_key(&user).await?;
        self.emit_configuration_changed_event(
            None,
            "user",
            Some(user.id.clone()),
            ConfigurationChangeAction::Saved,
        )
        .await;
        Ok(user)
    }

    async fn ensure_default_admin_actor(&self) -> AppResult<User> {
        let username = DEFAULT_ADMIN_USERNAME;
        if let Some(found) = self
            .services
            .identity
            .users
            .get_by_username(username)
            .await?
        {
            self.ensure_user_admin_permission_masks(&found).await?;
            self.refresh_cached_jwt_signing_key(&found).await?;
            return Ok(found);
        }

        let user = User {
            id: Id::new().0,
            username: username.to_string(),
            password_hash: None,
            password_change_required: false,
            account_kind: Default::default(),
            authorization: Default::default(),
        };

        let user = self.services.identity.users.create(user).await?;
        self.ensure_user_admin_permission_masks(&user).await?;
        self.cache_jwt_signing_key(&user).await?;
        self.emit_configuration_changed_event(
            None,
            "user",
            Some(user.id.clone()),
            ConfigurationChangeAction::Saved,
        )
        .await;
        Ok(user)
    }

    pub async fn find_or_create_default_user(&self) -> AppResult<User> {
        self.ensure_default_admin_actor().await
    }

    pub async fn find_default_user(&self) -> AppResult<Option<User>> {
        self.services
            .identity
            .users
            .get_by_username(DEFAULT_ADMIN_USERNAME)
            .await
    }

    pub async fn usable_admin_login_exists(&self) -> AppResult<bool> {
        self.usable_admin_login_exists_excluding(None).await
    }

    async fn usable_admin_login_exists_excluding(
        &self,
        excluded_user_id: Option<&str>,
    ) -> AppResult<bool> {
        let required_permissions = Self::required_startup_admin_app_permissions();
        for user in self.services.identity.users.list_all().await? {
            if excluded_user_id == Some(user.id.as_str()) || !user.login_status().is_enabled() {
                continue;
            }
            let Some(password_hash) = user.password_hash.as_deref() else {
                continue;
            };
            if !user.account_kind.allows_local_credentials() {
                continue;
            }
            if self.validate_password_hash(password_hash).is_err() {
                continue;
            }

            let user = self.attach_user_authorization(user).await?;
            if user.authorization.app.contains(required_permissions) {
                return Ok(true);
            }
        }

        Ok(false)
    }

    pub async fn recover_reserved_admin_access(&self, password: &str) -> AppResult<User> {
        self.validate_new_local_password(password).await?;
        let mut recovery_admin = if let Some(existing) = self
            .services
            .identity
            .users
            .get_by_username(RECOVERY_ADMIN_USERNAME)
            .await?
        {
            if !existing.account_kind.allows_local_credentials() {
                return Err(AppError::Validation(
                    "recovery admin user is externally managed and cannot be repaired for local login"
                        .into(),
                ));
            }
            self.update_user_password_hash(&existing, &existing.id, password.to_string(), false)
                .await?
        } else {
            let user = User {
                id: Id::new().0,
                username: RECOVERY_ADMIN_USERNAME.to_string(),
                password_hash: Some(self.hash_password(password)?),
                password_change_required: false,
                account_kind: scryer_domain::UserAccountKind::Local,
                authorization: Default::default(),
            };
            let user = self.services.identity.users.create(user).await?;
            self.cache_jwt_signing_key(&user).await?;
            self.emit_configuration_changed_event(
                None,
                "user",
                Some(user.id.clone()),
                ConfigurationChangeAction::Saved,
            )
            .await;
            user
        };
        self.ensure_user_admin_permission_masks(&recovery_admin)
            .await?;

        self.services
            .identity
            .totp
            .delete_credential_for_user(&recovery_admin.id)
            .await?;
        self.services
            .identity
            .totp
            .replace_recovery_codes(&recovery_admin.id, Vec::new())
            .await?;
        self.services
            .identity
            .totp
            .delete_enrollment_challenges_for_user(&recovery_admin.id)
            .await?;
        self.services
            .identity
            .totp
            .clear_failed_attempts(&recovery_admin.id)
            .await?;

        for passkey in self
            .services
            .identity
            .webauthn
            .list_credentials_for_user(&recovery_admin.id)
            .await?
        {
            self.services
                .identity
                .webauthn
                .delete_credential_for_user(&passkey.id, &recovery_admin.id)
                .await?;
        }

        self.services
            .config
            .settings
            .upsert_setting_json(
                SETTINGS_SCOPE_SYSTEM,
                MFA_REQUIRE_CONFIG_STEP_UP_KEY,
                None,
                serde_json::to_string(&false).unwrap_or_else(|_| "false".to_string()),
                "startup_recovery",
                Some(recovery_admin.id.clone()),
            )
            .await?;

        if !recovery_admin.login_status().is_enabled() {
            recovery_admin = self
                .services
                .identity
                .users
                .update_login_status_and_rotate_session(
                    &recovery_admin.id,
                    scryer_domain::UserLoginStatus::Enabled,
                    &Id::new().0,
                )
                .await?;
        }
        self.refresh_cached_jwt_signing_key(&recovery_admin).await?;
        Ok(recovery_admin)
    }

    pub async fn list_users(&self, actor: &User) -> AppResult<Vec<User>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageUsers)
            .await?;
        self.services.identity.users.list_all().await
    }

    pub async fn user_auth_factor_status(&self, user_id: &str) -> AppResult<UserAuthFactorStatus> {
        let has_mfa = self
            .services
            .identity
            .totp
            .get_credential_for_user(user_id)
            .await?
            .is_some();
        let has_passkey = !self
            .services
            .identity
            .webauthn
            .list_credentials_for_user(user_id)
            .await?
            .is_empty();
        Ok(UserAuthFactorStatus {
            has_mfa,
            has_passkey,
        })
    }

    pub async fn get_user(&self, actor: &User, user_id: &str) -> AppResult<Option<User>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageUsers)
            .await?;
        self.services.identity.users.get_by_id(user_id).await
    }

    pub async fn create_user(
        &self,
        actor: &User,
        username: String,
        password: String,
        app_permissions: scryer_domain::AppPermissionMask,
        library_grants: Vec<scryer_domain::LibraryGrant>,
    ) -> AppResult<User> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageUsers)
            .await?;
        if !app_permissions.is_empty()
            || library_grants
                .iter()
                .any(|grant| !grant.permissions.is_empty())
        {
            self.require_app_permission(actor, scryer_domain::AppPermission::ManagePermissions)
                .await?;
        }

        let username = Self::normalize_local_username(&username).to_string();
        if username.is_empty() {
            return Err(AppError::Validation("username is required".to_string()));
        }
        if Self::is_reserved_local_username(&username) {
            return Err(Self::reserved_local_username_error(&username));
        }
        self.validate_new_local_password(&password).await?;
        let password_hash = self.hash_password(&password)?;

        if self
            .services
            .identity
            .users
            .get_by_username(&username)
            .await?
            .is_some()
        {
            return Err(AppError::Validation(format!(
                "user {} already exists",
                username
            )));
        }

        let user = User {
            id: Id::new().0,
            username: username.clone(),
            password_hash: Some(password_hash),
            password_change_required: true,
            account_kind: scryer_domain::UserAccountKind::Local,
            authorization: Default::default(),
        };

        let user = self.services.identity.users.create(user).await?;
        self.services
            .catalog
            .libraries
            .set_app_permission_mask_for_user(&user.id, app_permissions)
            .await?;
        let grants = library_grants
            .into_iter()
            .map(|mut grant| {
                grant.user_id = user.id.clone();
                grant.permissions = grant.permissions.normalized_for_storage();
                grant
            })
            .collect();
        self.services
            .catalog
            .libraries
            .set_grants_for_user(&user.id, grants)
            .await?;
        self.cache_jwt_signing_key(&user).await?;
        self.emit_configuration_changed_event(
            actor,
            "user",
            Some(user.id.clone()),
            ConfigurationChangeAction::Saved,
        )
        .await;
        Ok(user)
    }

    /// Set a user's password without actor checks. Used only for first-run bootstrap.
    pub async fn bootstrap_user_password(&self, user_id: &str, password: &str) -> AppResult<User> {
        let password_hash = self.hash_password(password)?;
        let auth_session_version = Id::new().0;
        let user = self
            .services
            .identity
            .users
            .update_password_and_invalidate_sessions(
                user_id,
                password_hash,
                false,
                &auth_session_version,
            )
            .await?;
        self.revoke_oauth_refresh_grants_for_user(user_id, "password_changed")
            .await?;
        self.refresh_cached_jwt_signing_key(&user).await?;
        Ok(user)
    }

    pub async fn change_own_password(
        &self,
        actor: &User,
        password: String,
        current_password: String,
    ) -> AppResult<User> {
        self.require_actor_capability(actor, scryer_domain::ActorCapability::ManageOwnAccount)
            .await?;
        if password.is_empty() {
            return Err(AppError::Validation("password is required".into()));
        }

        let existing = self
            .services
            .identity
            .users
            .get_by_id(&actor.id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("user {}", actor.id)))?;

        if !existing.account_kind.allows_local_credentials() {
            return Err(AppError::Validation(
                "externally managed users cannot set a Scryer password".into(),
            ));
        }

        let hash = existing
            .password_hash
            .as_deref()
            .ok_or_else(|| AppError::Validation("account has no password set".into()))?;
        if !self.validate_password(&current_password, hash)? {
            return Err(AppError::Unauthorized(
                "current password is incorrect".into(),
            ));
        }

        self.update_own_password_hash(actor, password, Some(hash))
            .await
    }

    pub async fn set_initial_own_password(
        &self,
        actor: &User,
        password: String,
    ) -> AppResult<User> {
        self.require_actor_capability(actor, scryer_domain::ActorCapability::ManageOwnAccount)
            .await?;
        let existing = self
            .services
            .identity
            .users
            .get_by_id(&actor.id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("user {}", actor.id)))?;

        if !existing.account_kind.allows_local_credentials() {
            return Err(AppError::Validation(
                "externally managed users cannot set a Scryer password".into(),
            ));
        }

        if existing.password_hash.is_some() {
            return Err(AppError::Validation("current password is required".into()));
        }

        self.update_own_password_hash(actor, password, None).await
    }

    pub async fn set_user_password(
        &self,
        actor: &User,
        user_id: &str,
        password: String,
    ) -> AppResult<User> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageUsers)
            .await?;

        if user_id == actor.id {
            return Err(AppError::Validation(
                "use change_own_password to update your own password".into(),
            ));
        }

        if password.is_empty() {
            return Err(AppError::Validation("password is required".into()));
        }
        let existing = self
            .services
            .identity
            .users
            .get_by_id(user_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("user {user_id}")))?;
        if !existing.account_kind.allows_local_credentials() {
            return Err(AppError::Validation(
                "externally managed users cannot set a Scryer password".into(),
            ));
        }

        self.validate_new_local_password(&password).await?;
        let password_hash = self.hash_password(&password)?;
        let auth_session_version = Id::new().0;
        let user = self
            .services
            .identity
            .users
            .update_password_and_invalidate_sessions(
                user_id,
                password_hash,
                true,
                &auth_session_version,
            )
            .await?;
        self.refresh_cached_jwt_signing_key(&user).await?;
        self.revoke_oauth_refresh_grants_for_user(user_id, "password_changed")
            .await?;
        self.emit_configuration_changed_event(
            actor,
            "temporary_password",
            Some(user.id.clone()),
            ConfigurationChangeAction::Updated,
        )
        .await;
        Ok(user)
    }

    pub async fn complete_required_password_change(
        &self,
        actor: &User,
        password: String,
        expected_auth_session_version: &Option<String>,
    ) -> AppResult<(User, Option<String>)> {
        if password.is_empty() {
            return Err(AppError::Validation("password is required".into()));
        }
        let existing = self
            .services
            .identity
            .users
            .get_by_id(&actor.id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("user {}", actor.id)))?;
        if !existing.account_kind.allows_local_credentials() {
            return Err(AppError::Validation(
                "externally managed users cannot set a Scryer password".into(),
            ));
        }

        self.validate_new_local_password(&password).await?;
        if let Some(password_hash) = existing.password_hash.as_deref()
            && self.validate_password(&password, password_hash)?
        {
            return Err(AppError::Validation(
                "new password must differ from the temporary password".into(),
            ));
        }
        let auth_session_version = Id::new().0;
        let user = self
            .services
            .identity
            .users
            .complete_required_password_change(
                &actor.id,
                self.hash_password(&password)?,
                expected_auth_session_version,
                &auth_session_version,
            )
            .await?;
        self.refresh_cached_jwt_signing_key(&user).await?;
        self.revoke_oauth_refresh_grants_for_user(&actor.id, "password_changed")
            .await?;
        self.emit_configuration_changed_event(
            actor,
            "temporary_password",
            Some(user.id.clone()),
            ConfigurationChangeAction::Updated,
        )
        .await;
        Ok((user, Some(auth_session_version)))
    }

    async fn update_own_password_hash(
        &self,
        actor: &User,
        password: String,
        expected_password_hash: Option<&str>,
    ) -> AppResult<User> {
        if password.is_empty() {
            return Err(AppError::Validation("password is required".into()));
        }

        self.validate_new_local_password(&password).await?;
        let password_hash = self.hash_password(&password)?;
        let auth_session_version = Id::new().0;
        let user = self
            .services
            .identity
            .users
            .update_own_password_and_invalidate_sessions(
                &actor.id,
                password_hash,
                false,
                &auth_session_version,
                expected_password_hash,
            )
            .await?;
        self.refresh_cached_jwt_signing_key(&user).await?;
        self.revoke_oauth_refresh_grants_for_user(&actor.id, "password_changed")
            .await?;
        self.emit_configuration_changed_event(
            actor,
            "user_password",
            Some(user.id.clone()),
            ConfigurationChangeAction::Updated,
        )
        .await;

        Ok(user)
    }

    async fn update_user_password_hash(
        &self,
        actor: &User,
        user_id: &str,
        password: String,
        password_change_required: bool,
    ) -> AppResult<User> {
        if password.is_empty() {
            return Err(AppError::Validation("password is required".into()));
        }

        let existing = self
            .services
            .identity
            .users
            .get_by_id(user_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("user {user_id}")))?;
        if !existing.account_kind.allows_local_credentials() {
            return Err(AppError::Validation(
                "externally managed users cannot set a Scryer password".into(),
            ));
        }

        self.validate_new_local_password(&password).await?;
        let password_hash = self.hash_password(&password)?;
        let auth_session_version = Id::new().0;
        let user = self
            .services
            .identity
            .users
            .update_password_and_invalidate_sessions(
                user_id,
                password_hash,
                password_change_required,
                &auth_session_version,
            )
            .await?;
        self.refresh_cached_jwt_signing_key(&user).await?;
        self.revoke_oauth_refresh_grants_for_user(user_id, "password_changed")
            .await?;
        self.emit_configuration_changed_event(
            actor,
            "user_password",
            Some(user.id.clone()),
            ConfigurationChangeAction::Updated,
        )
        .await;

        Ok(user)
    }
    pub async fn set_user_app_permissions(
        &self,
        actor: &User,
        user_id: &str,
        permissions: scryer_domain::AppPermissionMask,
    ) -> AppResult<User> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageUsers)
            .await?;
        self.require_app_permission(actor, scryer_domain::AppPermission::ManagePermissions)
            .await?;

        let user = self
            .services
            .identity
            .users
            .get_by_id(user_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("user {}", user_id)))?;

        if user.id == actor.id {
            return Err(AppError::Validation("cannot modify own permissions".into()));
        }

        self.services
            .catalog
            .libraries
            .set_app_permission_mask_for_user(user_id, permissions)
            .await?;
        self.evict_cached_jwt_signing_key(user_id).await;
        self.refresh_cached_jwt_signing_key(&user).await?;
        self.emit_configuration_changed_event(
            actor,
            "user_permissions",
            Some(user.id.clone()),
            ConfigurationChangeAction::Updated,
        )
        .await;

        Ok(user)
    }

    pub async fn set_user_library_permissions(
        &self,
        actor: &User,
        user_id: &str,
        grants: Vec<scryer_domain::LibraryGrant>,
    ) -> AppResult<User> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageUsers)
            .await?;
        self.require_app_permission(actor, scryer_domain::AppPermission::ManagePermissions)
            .await?;
        let user = self
            .services
            .identity
            .users
            .get_by_id(user_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("user {}", user_id)))?;
        let grants = grants
            .into_iter()
            .map(|mut grant| {
                grant.permissions = grant.permissions.normalized_for_storage();
                grant
            })
            .collect();
        self.services
            .catalog
            .libraries
            .set_grants_for_user(user_id, grants)
            .await?;
        self.evict_cached_jwt_signing_key(user_id).await;
        self.refresh_cached_jwt_signing_key(&user).await?;
        self.emit_configuration_changed_event(
            actor,
            "user_permissions",
            Some(user.id.clone()),
            ConfigurationChangeAction::Updated,
        )
        .await;
        Ok(user)
    }

    pub async fn set_user_login_enabled(
        &self,
        actor: &User,
        user_id: &str,
        enabled: bool,
        effective_form_login_enabled: bool,
    ) -> AppResult<User> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageUsers)
            .await?;

        let user = self
            .services
            .identity
            .users
            .get_by_id(user_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("user {user_id}")))?;
        if user.id == actor.id {
            return Err(AppError::Validation(
                "cannot change login status for the current user".into(),
            ));
        }
        if Self::is_reserved_recovery_username(&user.username) {
            return Err(AppError::Validation(
                "recovery-admin login is managed by the environment".into(),
            ));
        }
        let user = self.attach_user_authorization(user).await?;

        let status = if enabled {
            scryer_domain::UserLoginStatus::Enabled
        } else {
            scryer_domain::UserLoginStatus::Disabled
        };
        if user.login_status() == status {
            if !enabled {
                self.revoke_oauth_refresh_grants_for_user(user_id, "user_login_disabled")
                    .await?;
            }
            return Ok(user);
        }

        if !enabled
            && (effective_form_login_enabled
                || self.load_security_settings().await?.form_login_enabled)
        {
            let full_admin_permissions = Self::required_startup_admin_app_permissions();
            let target_is_usable_full_admin = user.login_status().is_enabled()
                && user.account_kind.allows_local_credentials()
                && user
                    .password_hash
                    .as_deref()
                    .is_some_and(|hash| self.validate_password_hash(hash).is_ok())
                && user.authorization.app.contains(full_admin_permissions);
            if target_is_usable_full_admin
                && !self
                    .usable_admin_login_exists_excluding(Some(user_id))
                    .await?
            {
                return Err(AppError::Validation(
                    "cannot disable the last usable full administrator".into(),
                ));
            }
        }

        let auth_session_version = Id::new().0;
        let mut updated = self
            .services
            .identity
            .users
            .update_login_status_and_rotate_session(user_id, status, &auth_session_version)
            .await?;
        updated.authorization = user.authorization;
        updated.set_login_status(status);
        if let Err(error) = self.refresh_cached_jwt_signing_key(&updated).await {
            tracing::warn!(user_id, %error, "failed to refresh JWT signing key after login status change");
        }
        if !enabled
            && let Err(error) = self
                .revoke_oauth_refresh_grants_for_user(user_id, "user_login_disabled")
                .await
        {
            tracing::warn!(user_id, %error, "failed to revoke OAuth refresh grants after disabling user login");
        }
        self.emit_configuration_changed_event(
            actor,
            "user_login_status",
            Some(updated.id.clone()),
            ConfigurationChangeAction::Updated,
        )
        .await;
        Ok(updated)
    }

    pub async fn delete_user(&self, actor: &User, user_id: &str) -> AppResult<()> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageUsers)
            .await?;

        let user = self
            .services
            .identity
            .users
            .get_by_id(user_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("user {}", user_id)))?;

        if user.id == actor.id {
            return Err(AppError::Validation("cannot delete current user".into()));
        }

        if Self::is_default_admin_username(&user.username) {
            return Err(AppError::Validation(
                "cannot delete the default admin; disable its login instead".into(),
            ));
        }

        let full_admin_permissions = Self::required_startup_admin_app_permissions();
        let target_is_full_admin = self
            .attach_user_authorization(user.clone())
            .await?
            .authorization
            .app
            .contains(full_admin_permissions);
        if target_is_full_admin {
            let mut replacement_full_admin_exists = false;
            for candidate in self.services.identity.users.list_all().await? {
                if candidate.id == user.id {
                    continue;
                }
                let candidate = self.attach_user_authorization(candidate).await?;
                if candidate.authorization.app.contains(full_admin_permissions) {
                    replacement_full_admin_exists = true;
                    break;
                }
            }
            if !replacement_full_admin_exists {
                return Err(AppError::Validation(
                    "cannot delete the last full administrator".into(),
                ));
            }
        }

        self.revoke_oauth_refresh_grants_for_user(user_id, "user_deleted")
            .await?;
        self.services.identity.users.delete(user_id).await?;
        self.evict_cached_jwt_signing_key(user_id).await;
        self.emit_configuration_changed_event(
            actor,
            "user",
            Some(user.id),
            ConfigurationChangeAction::Deleted,
        )
        .await;
        Ok(())
    }

    pub async fn reset_user_mfa(&self, actor: &User, user_id: &str) -> AppResult<User> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageUsers)
            .await?;

        let user = self
            .services
            .identity
            .users
            .get_by_id(user_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("user {}", user_id)))?;

        if user.id == actor.id {
            return Err(AppError::Validation(
                "cannot reset your own authentication factors".into(),
            ));
        }

        let auth_session_version = Id::new().0;
        self.services
            .identity
            .users
            .reset_authentication_factors_and_invalidate_sessions(user_id, &auth_session_version)
            .await?;
        self.revoke_oauth_refresh_grants_for_user(user_id, "auth_session_changed")
            .await?;
        self.evict_cached_jwt_signing_key(user_id).await;
        self.refresh_cached_jwt_signing_key(&user).await?;
        self.emit_configuration_changed_event(
            actor,
            "user_authentication_factors",
            Some(user.id.clone()),
            ConfigurationChangeAction::Updated,
        )
        .await;

        Ok(user)
    }
}

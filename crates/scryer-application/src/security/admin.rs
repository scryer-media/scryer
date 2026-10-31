use super::*;
use crate::event_views::history_event_from_domain_event;
use scryer_domain::ConfigurationChangeAction;

const DEFAULT_ADMIN_USERNAME: &str = "admin";
const RECOVERY_ADMIN_USERNAME: &str = "recovery-admin";
const ANONYMOUS_AUDIT_USERNAME: &str = "anonymous";

impl AppUseCase {
    pub async fn dashboard_summary(
        &self,
        actor: &User,
    ) -> AppResult<(crate::TitleCounts, Vec<crate::IndexerQueryStats>)> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        Ok((
            self.services.catalog.titles.title_counts().await?,
            self.services.integrations.indexer_stats.all_stats(),
        ))
    }

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
            AppError::validation_refused(
                crate::error_reasons::security::ANONYMOUS_RESERVED_AUTHLESS_AUDIT_ATTRIBUTION,
                "anonymous is reserved for authless audit attribution",
            )
        } else {
            AppError::validation_refused(
                crate::error_reasons::security::RESERVED_INSTANCE_RECOVERY,
                format!("{RECOVERY_ADMIN_USERNAME} is reserved for instance recovery"),
            )
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

        let counts = self.services.catalog.titles.title_counts().await?;
        let users = self.services.identity.users.list_all().await?;
        let recent_activity = self.recent_activity_page(12, 0).await?;

        let mut recent_event_preview = Vec::with_capacity(std::cmp::min(3, recent_activity.len()));

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
            total_titles: counts.total,
            monitored_titles: counts.monitored,
            total_users: users.len(),
            titles_movie: counts.movie,
            titles_series: counts.series,
            titles_anime: counts.anime,
            titles_other: 0,
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
        self.restore_coordinator().spawn_task(async move {
            let mut wake_rx = app.runtime.events.domain_event_broadcast.subscribe();
            let mut cursor = 0_i64;

            loop {
                if app.restore_coordinator().shutdown_workers.is_cancelled() {
                    break;
                }
                let events = match app
                    .services
                    .events
                    .domain_events
                    .list_after_sequence(cursor, 100)
                    .await
                {
                    Ok(events) if !events.is_empty() => events,
                    Ok(_) => match tokio::select! {
                        _ = app.restore_coordinator().shutdown_workers.cancelled() => break,
                        event = wake_rx.recv() => event,
                    } {
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
            return Err(AppError::validation_refused(
                crate::error_reasons::security::ADMIN_USERNAME_REQUIRED,
                "admin username is required",
            ));
        }
        if Self::is_reserved_local_username(username) {
            return Err(Self::reserved_local_username_error(username));
        }
        if password.is_empty() {
            return Err(AppError::validation_refused(
                crate::error_reasons::security::ADMIN_PASSWORD_REQUIRED,
                "admin password is required",
            ));
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
        if self
            .runtime
            .security
            .default_admin_disabled
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            return Err(AppError::validation_refused(
                crate::error_reasons::security::ADMIN_DISABLED_SCRYER_DISABLE_DEFAULT_ADMIN,
                "admin is disabled by SCRYER_DISABLE_DEFAULT_ADMIN; remove that setting and restart before using the default administrator",
            ));
        }
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

    pub async fn bootstrap_admin_password(&self, password: &str) -> AppResult<()> {
        self.configure_bootstrap_admin(None, Some(password), false, false)
            .await
    }

    async fn bootstrap_admin_eligible(
        &self,
        user: &User,
        require_password: bool,
    ) -> AppResult<bool> {
        if !user.account_kind.allows_local_credentials()
            || !user.login_status().is_enabled()
            || Self::is_reserved_local_username(&user.username)
        {
            return Ok(false);
        }
        if require_password
            && !user
                .password_hash
                .as_deref()
                .is_some_and(|hash| self.validate_password_hash(hash).is_ok())
        {
            return Ok(false);
        }
        let user = self.attach_user_authorization(user.clone()).await?;
        Ok(user
            .authorization
            .app
            .contains(Self::required_startup_admin_app_permissions()))
    }

    pub async fn configure_bootstrap_admin(
        &self,
        username: Option<&str>,
        password: Option<&str>,
        reset: bool,
        disable_default: bool,
    ) -> AppResult<()> {
        let select_account = username.is_some() || password.is_some() || reset;
        let username = Self::normalize_local_username(username.unwrap_or(DEFAULT_ADMIN_USERNAME));
        if username.is_empty() || Self::is_reserved_local_username(username) {
            return Err(AppError::validation_refused(crate::error_reasons::security::SCRYER_ADMIN_USERNAME_MUST_NAME_NON_RESERVED_LOCAL_ADMINISTRATOR, "SCRYER_ADMIN_USERNAME must name a non-reserved local administrator"));
        }
        if reset && password.is_none() {
            return Err(AppError::validation_refused(
                crate::error_reasons::security::ADMIN_RESET_PASSWORD_REQUIRED,
                "SCRYER_ADMIN_PASSWORD_RESET requires SCRYER_ADMIN_PASSWORD or SCRYER_ADMIN_PASSWORD_FILE",
            ));
        }
        if select_account && disable_default && Self::is_default_admin_username(username) {
            return Err(AppError::validation_refused(crate::error_reasons::security::CANNOT_PROVISION_SELECT_ADMIN_WHILE_SCRYER_DISABLE_DEFAULT_ADMIN_TRUE, "cannot provision or select admin while SCRYER_DISABLE_DEFAULT_ADMIN is true"
                    ));
        }
        let users = self.services.identity.users.list_all().await?;
        let matches: Vec<_> = users
            .iter()
            .filter(|user| {
                Self::normalize_local_username(&user.username).eq_ignore_ascii_case(username)
            })
            .collect();
        if select_account
            && (matches.len() > 1
                || matches
                    .first()
                    .is_some_and(|user| user.username != username))
        {
            return Err(AppError::validation_refused(
                crate::error_reasons::security::SCRYER_ADMIN_USERNAME_CONFLICTS_EXISTING_USERNAME,
                "SCRYER_ADMIN_USERNAME conflicts with an existing username; use its exact stored spelling",
            ));
        }
        let existing = matches.first().copied();
        let initialize_seed = if select_account && password.is_some() {
            if let Some(user) = existing {
                self.services
                    .identity
                    .users
                    .bootstrap_seed_is_uninitialized(&user.id)
                    .await?
            } else {
                false
            }
        } else {
            false
        };
        if select_account {
            if let Some(user) = existing {
                if !initialize_seed
                    && !self
                        .bootstrap_admin_eligible(user, password.is_none())
                        .await?
                {
                    return Err(AppError::validation_refused(crate::error_reasons::security::SELECTED_BOOTSTRAP_ACCOUNT_MUST_ALREADY_ENABLED_LOCAL_FULL_ADMINISTRATOR, "selected bootstrap account must already be an enabled local full administrator; use recovery for lockouts"));
                }
            } else if password.is_none() {
                return Err(AppError::validation_refused(
                    crate::error_reasons::security::BOOTSTRAP_ADMIN_PASSWORD_REQUIRED,
                    "new bootstrap administrator requires SCRYER_ADMIN_PASSWORD or SCRYER_ADMIN_PASSWORD_FILE",
                ));
            }
            if reset || existing.is_none_or(|user| user.password_hash.is_none()) {
                self.validate_new_local_password(password.unwrap_or_default())
                    .await?;
            }
        }
        // Validate disable-only requests before persisting anything, even if admin is already disabled.
        let mut alternate = None;
        if disable_default {
            for user in &users {
                if !Self::is_default_admin_username(&user.username)
                    && self.bootstrap_admin_eligible(user, true).await?
                {
                    alternate = Some(user.clone());
                    break;
                }
            }
            if alternate.is_none() && !select_account {
                return Err(AppError::validation_refused(
                    crate::error_reasons::security::ALTERNATIVE_LOCAL_ADMIN_REQUIRED,
                    "SCRYER_DISABLE_DEFAULT_ADMIN requires another enabled local full administrator with a usable password",
                ));
            }
        }
        let changes_credentials =
            select_account && (reset || existing.is_none_or(|user| user.password_hash.is_none()));
        let disables_enabled_admin = disable_default
            && users.iter().any(|user| {
                Self::is_default_admin_username(&user.username) && user.login_status().is_enabled()
            });
        // Persist the closed state before mutations, but preserve saved preferences on no-op boots.
        if changes_credentials || disables_enabled_admin {
            for (key, value) in [
                (crate::FORM_LOGIN_ENABLED_KEY, true),
                (crate::SKIP_LOGIN_FOR_LOCAL_IPS_KEY, false),
            ] {
                self.services
                    .config
                    .settings
                    .upsert_setting_json(
                        SETTINGS_SCOPE_SYSTEM,
                        key,
                        None,
                        value.to_string(),
                        "startup_bootstrap",
                        None,
                    )
                    .await?;
            }
        }
        if select_account {
            let user = if let Some(existing) = existing.filter(|_| !initialize_seed) {
                if reset || existing.password_hash.is_none() {
                    let updated = self
                        .services
                        .identity
                        .users
                        .update_own_password_and_invalidate_sessions(
                            &existing.id,
                            self.hash_password(password.unwrap_or_default())?,
                            true,
                            &Id::new().0,
                            existing.password_hash.as_deref(),
                        )
                        .await?;
                    self.refresh_cached_jwt_signing_key(&updated).await?;
                    self.revoke_oauth_refresh_grants_for_user(&updated.id, "password_changed")
                        .await?;
                    self.emit_configuration_changed_event(
                        None,
                        "temporary_password",
                        Some(updated.id.clone()),
                        ConfigurationChangeAction::Updated,
                    )
                    .await;
                    updated
                } else {
                    existing.clone()
                }
            } else {
                let user = User {
                    id: existing
                        .map(|user| user.id.clone())
                        .unwrap_or_else(|| Id::new().0),
                    username: username.to_string(),
                    password_hash: Some(self.hash_password(password.unwrap_or_default())?),
                    password_change_required: true,
                    account_kind: scryer_domain::UserAccountKind::Local,
                    authorization: scryer_domain::UserAuthorization::full_admin(),
                };
                let grants = self
                    .services
                    .catalog
                    .libraries
                    .list(None)
                    .await?
                    .into_iter()
                    .map(|library| scryer_domain::LibraryGrant {
                        user_id: user.id.clone(),
                        library_id: library.id,
                        permissions: user.authorization.default_library,
                    })
                    .collect();
                let user = self
                    .services
                    .identity
                    .users
                    .create_bootstrap_admin(user, grants, initialize_seed)
                    .await?;
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
            if !self.bootstrap_admin_eligible(&user, true).await? {
                return Err(AppError::validation_refused(
                    crate::error_reasons::security::BOOTSTRAP_ADMINISTRATOR_NO_USABLE_CREDENTIALS,
                    "bootstrap administrator has no usable credentials",
                ));
            }
            if !Self::is_default_admin_username(&user.username) {
                alternate = Some(user);
            }
        }
        if disable_default {
            alternate.ok_or_else(|| {
                AppError::validation_refused(crate::error_reasons::security::CANNOT_DISABLE_ADMIN_WITHOUT_ANOTHER_USABLE_ADMINISTRATOR, "cannot disable admin without another usable administrator")
            })?;
            if let Some(admin) = self.find_default_user().await? {
                self.set_user_login_enabled_internal(None, &admin.id, false, true)
                    .await?;
                self.revoke_oauth_refresh_grants_for_user(&admin.id, "user_login_disabled")
                    .await?;
            }
            self.runtime
                .security
                .default_admin_disabled
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
        Ok(())
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
                return Err(AppError::validation_refused(crate::error_reasons::security::RECOVERY_ADMIN_USER_EXTERNALLY_MANAGED_CANNOT_REPAIRED_LOCAL_LOGIN, "recovery admin user is externally managed and cannot be repaired for local login"
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
        if let Some(pending_admin) = self
            .services
            .identity
            .users
            .prepare_initial_authentication_recovery(&recovery_admin.id)
            .await?
        {
            recovery_admin = pending_admin;
            self.services.config.settings.invalidate_cache();
            self.emit_configuration_changed_event(
                None,
                "initial_authentication",
                Some(recovery_admin.id.clone()),
                ConfigurationChangeAction::Updated,
            )
            .await;
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
        if Self::is_default_admin_username(&username)
            && self
                .runtime
                .security
                .default_admin_disabled
                .load(std::sync::atomic::Ordering::SeqCst)
        {
            return Err(AppError::validation_refused(crate::error_reasons::security::CANNOT_CREATE_ADMIN_WHILE_SCRYER_DISABLE_DEFAULT_ADMIN_TRUE, "cannot create admin while SCRYER_DISABLE_DEFAULT_ADMIN is true"));
        }
        if username.is_empty() {
            return Err(AppError::validation_refused(
                crate::error_reasons::security::USERNAME_REQUIRED,
                "username is required".to_string(),
            ));
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
            return Err(AppError::validation_refused(
                crate::error_reasons::security::USER_ALREADY_EXISTS,
                format!("user {} already exists", username),
            ));
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
            return Err(AppError::validation_refused(
                crate::error_reasons::security::PASSWORD_REQUIRED,
                "password is required",
            ));
        }

        let existing = self
            .services
            .identity
            .users
            .get_by_id(&actor.id)
            .await?
            .ok_or_else(|| {
                AppError::not_found_reason(
                    crate::error_reasons::security::USER_NOT_FOUND,
                    format!("user {}", actor.id),
                )
            })?;

        if !existing.account_kind.allows_local_credentials() {
            return Err(AppError::validation_refused(
                crate::error_reasons::security::EXTERNALLY_MANAGED_USERS_CANNOT_SET_SCRYER_PASSWORD,
                "externally managed users cannot set a Scryer password",
            ));
        }

        let hash = existing.password_hash.as_deref().ok_or_else(|| {
            AppError::validation_refused(
                crate::error_reasons::security::ACCOUNT_NO_PASSWORD_SET,
                "account has no password set",
            )
        })?;
        if !self.validate_password(&current_password, hash)? {
            return Err(AppError::unauthorized_reason(
                crate::error_reasons::security::CURRENT_PASSWORD_INCORRECT,
                "current password is incorrect",
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
            .ok_or_else(|| {
                AppError::not_found_reason(
                    crate::error_reasons::security::USER_NOT_FOUND,
                    format!("user {}", actor.id),
                )
            })?;

        if !existing.account_kind.allows_local_credentials() {
            return Err(AppError::validation_refused(
                crate::error_reasons::security::EXTERNALLY_MANAGED_USERS_CANNOT_SET_SCRYER_PASSWORD,
                "externally managed users cannot set a Scryer password",
            ));
        }

        if existing.password_hash.is_some() {
            return Err(AppError::validation_refused(
                crate::error_reasons::security::CURRENT_PASSWORD_REQUIRED,
                "current password is required",
            ));
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
            return Err(AppError::validation_refused(
                crate::error_reasons::security::USE_CHANGE_OWN_PASSWORD_UPDATE_OWN_PASSWORD,
                "use change_own_password to update your own password",
            ));
        }

        if password.is_empty() {
            return Err(AppError::validation_refused(
                crate::error_reasons::security::PASSWORD_REQUIRED,
                "password is required",
            ));
        }
        let existing = self
            .services
            .identity
            .users
            .get_by_id(user_id)
            .await?
            .ok_or_else(|| {
                AppError::not_found_reason(
                    crate::error_reasons::security::USER_NOT_FOUND,
                    format!("user {user_id}"),
                )
            })?;
        if !existing.account_kind.allows_local_credentials() {
            return Err(AppError::validation_refused(
                crate::error_reasons::security::EXTERNALLY_MANAGED_USERS_CANNOT_SET_SCRYER_PASSWORD,
                "externally managed users cannot set a Scryer password",
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
            return Err(AppError::validation_refused(
                crate::error_reasons::security::PASSWORD_REQUIRED,
                "password is required",
            ));
        }
        let existing = self
            .services
            .identity
            .users
            .get_by_id(&actor.id)
            .await?
            .ok_or_else(|| {
                AppError::not_found_reason(
                    crate::error_reasons::security::USER_NOT_FOUND,
                    format!("user {}", actor.id),
                )
            })?;
        if !existing.account_kind.allows_local_credentials() {
            return Err(AppError::validation_refused(
                crate::error_reasons::security::EXTERNALLY_MANAGED_USERS_CANNOT_SET_SCRYER_PASSWORD,
                "externally managed users cannot set a Scryer password",
            ));
        }

        self.validate_new_local_password(&password).await?;
        if let Some(password_hash) = existing.password_hash.as_deref()
            && self.validate_password(&password, password_hash)?
        {
            return Err(AppError::validation_refused(
                crate::error_reasons::security::NEW_PASSWORD_MUST_DIFFER_TEMPORARY_PASSWORD,
                "new password must differ from the temporary password",
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
            return Err(AppError::validation_refused(
                crate::error_reasons::security::PASSWORD_REQUIRED,
                "password is required",
            ));
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
            return Err(AppError::validation_refused(
                crate::error_reasons::security::PASSWORD_REQUIRED,
                "password is required",
            ));
        }

        let existing = self
            .services
            .identity
            .users
            .get_by_id(user_id)
            .await?
            .ok_or_else(|| {
                AppError::not_found_reason(
                    crate::error_reasons::security::USER_NOT_FOUND,
                    format!("user {user_id}"),
                )
            })?;
        if !existing.account_kind.allows_local_credentials() {
            return Err(AppError::validation_refused(
                crate::error_reasons::security::EXTERNALLY_MANAGED_USERS_CANNOT_SET_SCRYER_PASSWORD,
                "externally managed users cannot set a Scryer password",
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
            .ok_or_else(|| {
                AppError::not_found_reason(
                    crate::error_reasons::security::USER_NOT_FOUND,
                    format!("user {}", user_id),
                )
            })?;

        if user.id == actor.id {
            self.reject_self_lockout(&user, permissions).await?;
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

    /// Administrators may edit their own app permissions, but never into a
    /// state they cannot recover from: the user- and permission-management
    /// pair that gates this mutation must survive, and the last full
    /// administrator cannot demote themselves.
    async fn reject_self_lockout(
        &self,
        actor: &User,
        next_permissions: scryer_domain::AppPermissionMask,
    ) -> AppResult<()> {
        let management_permissions = scryer_domain::AppPermissionMask::from_permissions([
            scryer_domain::AppPermission::ManageUsers,
            scryer_domain::AppPermission::ManagePermissions,
        ]);
        if !next_permissions.contains(management_permissions) {
            return Err(AppError::validation_refused(crate::error_reasons::security::CANNOT_REMOVE_OWN_USER_MANAGEMENT_PERMISSION_MANAGEMENT_ACCESS, "cannot remove your own user-management or permission-management access"));
        }

        let full_admin_permissions = Self::required_startup_admin_app_permissions();
        if next_permissions.contains(full_admin_permissions) {
            return Ok(());
        }
        let actor_is_full_admin = self
            .attach_user_authorization(actor.clone())
            .await?
            .authorization
            .app
            .contains(full_admin_permissions);
        if !actor_is_full_admin {
            return Ok(());
        }
        for candidate in self.services.identity.users.list_all().await? {
            if candidate.id == actor.id {
                continue;
            }
            let candidate = self.attach_user_authorization(candidate).await?;
            if candidate.authorization.app.contains(full_admin_permissions) {
                return Ok(());
            }
        }
        Err(AppError::validation_refused(
            crate::error_reasons::security::CANNOT_DEMOTE_LAST_FULL_ADMINISTRATOR,
            "cannot demote the last full administrator",
        ))
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
            .ok_or_else(|| {
                AppError::not_found_reason(
                    crate::error_reasons::security::USER_NOT_FOUND,
                    format!("user {}", user_id),
                )
            })?;
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

        self.set_user_login_enabled_internal(
            Some(actor),
            user_id,
            enabled,
            effective_form_login_enabled,
        )
        .await
    }

    async fn set_user_login_enabled_internal(
        &self,
        actor: Option<&User>,
        user_id: &str,
        enabled: bool,
        effective_form_login_enabled: bool,
    ) -> AppResult<User> {
        let user = self
            .services
            .identity
            .users
            .get_by_id(user_id)
            .await?
            .ok_or_else(|| {
                AppError::not_found_reason(
                    crate::error_reasons::security::USER_NOT_FOUND,
                    format!("user {user_id}"),
                )
            })?;
        if enabled
            && Self::is_default_admin_username(&user.username)
            && self
                .runtime
                .security
                .default_admin_disabled
                .load(std::sync::atomic::Ordering::SeqCst)
        {
            return Err(AppError::validation_refused(
                crate::error_reasons::security::ADMIN_DISABLED_SCRYER_DISABLE_DEFAULT_ADMIN,
                "admin is disabled by SCRYER_DISABLE_DEFAULT_ADMIN; remove that setting and restart before enabling it",
            ));
        }
        if actor.is_some_and(|actor| user.id == actor.id) {
            return Err(AppError::validation_refused(
                crate::error_reasons::security::CANNOT_CHANGE_LOGIN_STATUS_CURRENT_USER,
                "cannot change login status for the current user",
            ));
        }
        if Self::is_reserved_recovery_username(&user.username) {
            return Err(AppError::validation_refused(
                crate::error_reasons::security::RECOVERY_ADMIN_LOGIN_MANAGED_ENVIRONMENT,
                "recovery-admin login is managed by the environment",
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
                return Err(AppError::validation_refused(
                    crate::error_reasons::security::CANNOT_DISABLE_LAST_USABLE_FULL_ADMINISTRATOR,
                    "cannot disable the last usable full administrator",
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
            actor.map_or_else(
                crate::DomainEventActor::system,
                crate::DomainEventActor::user,
            ),
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
            .ok_or_else(|| {
                AppError::not_found_reason(
                    crate::error_reasons::security::USER_NOT_FOUND,
                    format!("user {}", user_id),
                )
            })?;

        if user.id == actor.id {
            return Err(AppError::validation_refused(
                crate::error_reasons::security::CANNOT_DELETE_CURRENT_USER,
                "cannot delete current user",
            ));
        }

        if Self::is_default_admin_username(&user.username) {
            return Err(AppError::validation_refused(
                crate::error_reasons::security::CANNOT_DELETE_DEFAULT_ADMIN,
                "cannot delete the default admin; disable its login instead",
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
                return Err(AppError::validation_refused(
                    crate::error_reasons::security::CANNOT_DELETE_LAST_FULL_ADMINISTRATOR,
                    "cannot delete the last full administrator",
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
            .ok_or_else(|| {
                AppError::not_found_reason(
                    crate::error_reasons::security::USER_NOT_FOUND,
                    format!("user {}", user_id),
                )
            })?;

        if user.id == actor.id {
            return Err(AppError::validation_refused(
                crate::error_reasons::security::CANNOT_RESET_OWN_AUTHENTICATION_FACTORS,
                "cannot reset your own authentication factors",
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

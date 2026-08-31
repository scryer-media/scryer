#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceSettings {
    pub tls_cert_path: String,
    pub tls_key_path: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecuritySettings {
    pub form_login_enabled: bool,
    pub password_min_length: i32,
    pub skip_login_for_local_ips: bool,
    pub api_keys_restrict_to_system_settings_users: bool,
    pub mfa_require_config_step_up: bool,
    pub mfa_require_password_login: bool,
    pub totp_require_jellyfin_login: bool,
    pub totp_require_emby_login: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateSecuritySettings {
    pub form_login_enabled: bool,
    pub password_min_length: i32,
    pub skip_login_for_local_ips: bool,
    /// When absent, preserve the current value without writing this protected setting.
    pub api_keys_restrict_to_system_settings_users: Option<bool>,
    pub mfa_require_config_step_up: bool,
    pub mfa_require_password_login: bool,
    pub totp_require_jellyfin_login: bool,
    pub totp_require_emby_login: Option<bool>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateServiceSettings {
    pub tls_cert_path: String,
    pub tls_key_path: String,
}
impl AppUseCase {
    pub(crate) async fn load_security_settings(&self) -> AppResult<SecuritySettings> {
        let form_login_enabled = self
            .read_setting_bool_value(FORM_LOGIN_ENABLED_KEY, None)
            .await?
            .unwrap_or(false);
        let password_min_length = self.password_min_length().await?;
        let skip_login_for_local_ips = self
            .read_setting_bool_value(SKIP_LOGIN_FOR_LOCAL_IPS_KEY, None)
            .await?
            .unwrap_or(false);
        let api_keys_restrict_to_system_settings_users = self
            .read_setting_bool_value(API_KEYS_RESTRICT_TO_SYSTEM_SETTINGS_USERS_KEY, None)
            .await?
            .unwrap_or(false);
        let mfa_require_config_step_up = self
            .load_mfa_setting_with_legacy_migration(
                MFA_REQUIRE_CONFIG_STEP_UP_KEY,
                LEGACY_TOTP_REQUIRE_CONFIG_STEP_UP_KEY,
            )
            .await?;
        let totp_require_jellyfin_login = self
            .read_setting_bool_value(TOTP_REQUIRE_JELLYFIN_LOGIN_KEY, None)
            .await?
            .unwrap_or(false);
        let totp_require_emby_login = self
            .read_setting_bool_value(TOTP_REQUIRE_EMBY_LOGIN_KEY, None)
            .await?
            .unwrap_or(false);
        let mfa_require_password_login = self
            .load_mfa_setting_with_legacy_migration(
                MFA_REQUIRE_PASSWORD_LOGIN_KEY,
                LEGACY_TOTP_REQUIRE_PASSWORD_LOGIN_KEY,
            )
            .await?;

        Ok(SecuritySettings {
            form_login_enabled,
            password_min_length,
            skip_login_for_local_ips,
            api_keys_restrict_to_system_settings_users,
            mfa_require_config_step_up,
            mfa_require_password_login,
            totp_require_jellyfin_login,
            totp_require_emby_login,
        })
    }
}

impl AppUseCase {
    async fn load_mfa_setting_with_legacy_migration(
        &self,
        key_name: &'static str,
        legacy_key_name: &'static str,
    ) -> AppResult<bool> {
        if let Some(value) = self.read_setting_bool_value(key_name, None).await? {
            return Ok(value);
        }

        let Some(legacy_value) = self.read_setting_bool_value(legacy_key_name, None).await? else {
            return Ok(false);
        };

        self.upsert_system_setting_json(key_name, &legacy_value, None)
            .await?;
        Ok(legacy_value)
    }
}
impl AppUseCase {
    pub async fn security_settings(&self) -> AppResult<SecuritySettings> {
        self.load_security_settings().await
    }
}
impl AppUseCase {
    pub async fn get_security_settings(&self, actor: &User) -> AppResult<SecuritySettings> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageUsers)
            .await?;
        self.load_security_settings().await
    }
}
impl AppUseCase {
    pub async fn setup_complete(&self) -> AppResult<bool> {
        Ok(self
            .read_setting_bool_value(SETUP_COMPLETE_KEY, None)
            .await?
            .unwrap_or(false))
    }
}
impl AppUseCase {
    pub async fn complete_setup(&self, actor: &User) -> AppResult<bool> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        self.services
            .config
            .settings
            .upsert_setting_json(
                SETTINGS_SCOPE_SYSTEM,
                SETUP_COMPLETE_KEY,
                None,
                encode_setting_json(&true)?,
                "setup-wizard",
                Some(actor.id.clone()),
            )
            .await?;

        Ok(true)
    }
}
impl AppUseCase {
    pub async fn get_service_settings(&self, actor: &User) -> AppResult<ServiceSettings> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        Ok(ServiceSettings {
            tls_cert_path: self
                .read_setting_string_value(TLS_CERT_PATH_KEY, None)
                .await?
                .unwrap_or_default(),
            tls_key_path: self
                .read_setting_string_value(TLS_KEY_PATH_KEY, None)
                .await?
                .unwrap_or_default(),
        })
    }
}
impl AppUseCase {
    pub async fn update_security_settings(
        &self,
        actor: &User,
        input: UpdateSecuritySettings,
    ) -> AppResult<SecuritySettings> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageUsers)
            .await?;
        if input.api_keys_restrict_to_system_settings_users.is_some() {
            self.require_app_permission(
                actor,
                scryer_domain::AppPermission::ManageSystemSettings,
            )
            .await?;
        }

        let current = self.load_security_settings().await?;
        let api_keys_restrict_to_system_settings_users = input
            .api_keys_restrict_to_system_settings_users
            .unwrap_or(current.api_keys_restrict_to_system_settings_users);
        let totp_require_emby_login = input
            .totp_require_emby_login
            .unwrap_or(current.totp_require_emby_login);

        if input.password_min_length < PASSWORD_MIN_LENGTH_MIN as i32 {
            return Err(AppError::Validation(format!(
                "password minimum length must be at least {PASSWORD_MIN_LENGTH_MIN}"
            )));
        }

        if input.mfa_require_config_step_up
            && self
                .services
                .identity
                .totp
                .get_credential_for_user(&actor.id)
                .await?
                .is_none()
        {
            return Err(AppError::TotpEnrollmentRequired(
                "enable TOTP for your account before requiring TOTP for system configuration"
                    .into(),
            ));
        }

        if !current.form_login_enabled && input.form_login_enabled {
            if self
                .existing_default_admin_uses_bootstrap_password()
                .await?
            {
                return Err(AppError::Validation(
                    "change the default admin password before enabling form login".into(),
                ));
            }
            if !self.usable_admin_login_exists().await? {
                return Err(AppError::Validation(
                    "configure an enabled full administrator login before enabling form login"
                        .into(),
                ));
            }
        }

        if current.form_login_enabled && !input.form_login_enabled {
            self.find_or_create_default_user().await?;
        }

        self.upsert_system_setting_json(
            FORM_LOGIN_ENABLED_KEY,
            &input.form_login_enabled,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            PASSWORD_MIN_LENGTH_KEY,
            &input.password_min_length,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            SKIP_LOGIN_FOR_LOCAL_IPS_KEY,
            &input.skip_login_for_local_ips,
            Some(actor.id.clone()),
        )
        .await?;
        if let Some(value) = input.api_keys_restrict_to_system_settings_users {
            self.upsert_system_setting_json(
                API_KEYS_RESTRICT_TO_SYSTEM_SETTINGS_USERS_KEY,
                &value,
                Some(actor.id.clone()),
            )
            .await?;
        }
        self.upsert_system_setting_json(
            MFA_REQUIRE_CONFIG_STEP_UP_KEY,
            &input.mfa_require_config_step_up,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            TOTP_REQUIRE_JELLYFIN_LOGIN_KEY,
            &input.totp_require_jellyfin_login,
            Some(actor.id.clone()),
        )
        .await?;
        if let Some(value) = input.totp_require_emby_login {
            self.upsert_system_setting_json(
                TOTP_REQUIRE_EMBY_LOGIN_KEY,
                &value,
                Some(actor.id.clone()),
            )
            .await?;
        }
        self.upsert_system_setting_json(
            MFA_REQUIRE_PASSWORD_LOGIN_KEY,
            &input.mfa_require_password_login,
            Some(actor.id.clone()),
        )
        .await?;

        if !current.form_login_enabled && input.form_login_enabled {
            self.revoke_authless_oauth_refresh_grants("form_login_enabled")
                .await?;
        }

        let mut saved_keys = vec![
            FORM_LOGIN_ENABLED_KEY.to_string(),
            PASSWORD_MIN_LENGTH_KEY.to_string(),
            SKIP_LOGIN_FOR_LOCAL_IPS_KEY.to_string(),
            MFA_REQUIRE_CONFIG_STEP_UP_KEY.to_string(),
            MFA_REQUIRE_PASSWORD_LOGIN_KEY.to_string(),
            TOTP_REQUIRE_JELLYFIN_LOGIN_KEY.to_string(),
        ];
        if input.api_keys_restrict_to_system_settings_users.is_some() {
            saved_keys.push(API_KEYS_RESTRICT_TO_SYSTEM_SETTINGS_USERS_KEY.to_string());
        }
        if input.totp_require_emby_login.is_some() {
            saved_keys.push(TOTP_REQUIRE_EMBY_LOGIN_KEY.to_string());
        }
        self.emit_settings_saved(actor, "security_settings", None, saved_keys)
            .await;

        Ok(SecuritySettings {
            form_login_enabled: input.form_login_enabled,
            password_min_length: input.password_min_length,
            skip_login_for_local_ips: input.skip_login_for_local_ips,
            api_keys_restrict_to_system_settings_users,
            mfa_require_config_step_up: input.mfa_require_config_step_up,
            mfa_require_password_login: input.mfa_require_password_login,
            totp_require_jellyfin_login: input.totp_require_jellyfin_login,
            totp_require_emby_login,
        })
    }
}
impl AppUseCase {
    pub async fn update_service_settings(
        &self,
        actor: &User,
        input: UpdateServiceSettings,
    ) -> AppResult<ServiceSettings> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        let tls_cert_path = input.tls_cert_path.trim().to_string();
        let tls_key_path = input.tls_key_path.trim().to_string();

        self.services
            .config
            .settings
            .upsert_setting_json(
                SETTINGS_SCOPE_SYSTEM,
                TLS_CERT_PATH_KEY,
                None,
                encode_setting_json(&tls_cert_path)?,
                SETTINGS_SOURCE_TYPED_GRAPHQL,
                Some(actor.id.clone()),
            )
            .await?;
        self.services
            .config
            .settings
            .upsert_setting_json(
                SETTINGS_SCOPE_SYSTEM,
                TLS_KEY_PATH_KEY,
                None,
                encode_setting_json(&tls_key_path)?,
                SETTINGS_SOURCE_TYPED_GRAPHQL,
                Some(actor.id.clone()),
            )
            .await?;

        self.emit_settings_saved(
            actor,
            "service_settings",
            None,
            vec![TLS_CERT_PATH_KEY.to_string(), TLS_KEY_PATH_KEY.to_string()],
        )
        .await;

        self.get_service_settings(actor).await
    }
}

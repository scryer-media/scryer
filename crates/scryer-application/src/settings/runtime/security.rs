#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceSettings {
    pub tls_cert_path: String,
    pub tls_key_path: String,
    pub trusted_proxy_ips: Vec<String>,
    pub trusted_proxy_override: Option<Vec<String>>,
    pub trusted_proxy_source: String,
    pub public_url: PublicUrlSettings,
}
/// The effective public URL and the read-only addressing facts beside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicUrlSettings {
    /// Effective, valid public URL; absent when unset or invalid.
    pub effective: Option<String>,
    pub source: crate::public_url::ConfigValueSource,
    /// Saved value, shown even when the environment overrides it.
    pub saved: Option<String>,
    /// Validation error for the winning value.
    pub error: Option<crate::public_url::PublicUrlError>,
    /// False while `SCRYER_PUBLIC_URL` is set.
    pub editable: bool,
    pub addressing: crate::public_url::InstanceAddressing,
    /// Passkey enrollment, absent when it could not be read.
    pub passkey_enrollment: Option<crate::PasskeyEnrollmentCounts>,
}
/// What saving a proposed public URL would do, computed with the same rules
/// the save and the next start apply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicUrlChangePreview {
    /// The stored form of the proposed value; absent for a reset or a
    /// rejected value.
    pub normalized: Option<String>,
    /// Why the proposed value would be refused.
    pub error: Option<crate::public_url::PublicUrlError>,
    pub passkey_impact: crate::public_url::PasskeyImpact,
    pub current_passkey_rp_id: Option<String>,
    pub next_passkey_rp_id: Option<String>,
    pub passkey_enrollment: Option<crate::PasskeyEnrollmentCounts>,
    /// The save must carry an explicit acknowledgement.
    pub acknowledgement_required: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecuritySettings {
    pub form_login_enabled: bool,
    pub session_duration_days: i32,
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
    /// Omission preserves the instance-wide session duration.
    pub session_duration_days: Option<i32>,
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
    pub tls_cert_path: Option<String>,
    pub tls_key_path: Option<String>,
    pub trusted_proxy_ips: Option<Vec<String>>,
    pub reset_trusted_proxy_ips: bool,
    /// Save a public URL. Omission preserves the saved value.
    pub public_url: Option<String>,
    /// Clear the saved public URL.
    pub reset_public_url: bool,
    /// The administrator confirmed that existing passkeys stop working.
    pub acknowledge_passkey_impact: bool,
}
impl AppUseCase {
    pub fn trusted_proxy_runtime(&self) -> crate::rate_limit_proxy_policy::TrustedProxyRuntime {
        self.runtime.security.trusted_proxies.clone()
    }

    /// The shared public URL that transport adapters read per request.
    pub fn public_url_runtime(&self) -> crate::public_url::PublicUrlRuntime {
        self.runtime.security.public_url.clone()
    }

    /// Install the startup public URL policy and addressing facts. The saved
    /// value is read by the composition root before passkeys are built.
    pub fn install_public_url(
        &self,
        policy: crate::public_url::PublicUrlPolicy,
        addressing: crate::public_url::InstanceAddressing,
    ) {
        self.runtime.security.public_url.install(policy, addressing);
    }

    /// Passkey enrollment counts in one query. A failed read is logged and
    /// reported as unknown so it never fails the settings page.
    async fn passkey_enrollment(&self) -> Option<crate::PasskeyEnrollmentCounts> {
        match self
            .services
            .identity
            .webauthn
            .passkey_enrollment_counts()
            .await
        {
            Ok(counts) => Some(counts),
            Err(error) => {
                tracing::warn!(error = %error, "could not count registered passkeys");
                None
            }
        }
    }

    async fn public_url_settings(&self) -> AppResult<PublicUrlSettings> {
        let policy = self.runtime.security.public_url.snapshot();
        Ok(PublicUrlSettings {
            effective: policy.url().map(|_| {
                policy
                    .configured_value()
                    .unwrap_or_default()
                    .trim_end_matches('/')
                    .to_string()
            }),
            source: policy.source(),
            saved: policy.saved_value().map(str::to_string),
            error: policy.error(),
            editable: policy.environment_value().is_none(),
            addressing: (*self.runtime.security.public_url.addressing()).clone(),
            passkey_enrollment: self.passkey_enrollment().await,
        })
    }

    /// Preview what saving `value` (or clearing it) would do, without saving.
    pub async fn preview_public_url_change(
        &self,
        actor: &User,
        value: Option<&str>,
        reset: bool,
    ) -> AppResult<PublicUrlChangePreview> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let addressing = self.runtime.security.public_url.addressing();
        let mut preview = PublicUrlChangePreview {
            normalized: None,
            error: None,
            passkey_impact: crate::public_url::PasskeyImpact::Unchanged,
            current_passkey_rp_id: addressing.passkey_rp_id.clone(),
            next_passkey_rp_id: addressing.passkey_rp_id.clone(),
            passkey_enrollment: None,
            acknowledgement_required: false,
        };
        let change = match self.validated_public_url_change(value, reset) {
            Ok(change) => change,
            Err(AppError::PublicUrlRejected { message, code }) => {
                preview.error = Some(crate::public_url::PublicUrlError::new(code, message));
                return Ok(preview);
            }
            Err(error) => return Err(error),
        };
        let Some(next) = change else {
            return Ok(preview);
        };
        let (impact, next_rp_id) =
            crate::public_url::passkey_impact_of_saved_value(&addressing, next.as_deref());
        preview.normalized = next;
        preview.passkey_impact = impact;
        if impact != crate::public_url::PasskeyImpact::Unaffected {
            preview.next_passkey_rp_id = next_rp_id;
        }
        preview.passkey_enrollment = self.passkey_enrollment().await;
        preview.acknowledgement_required =
            crate::public_url::passkey_acknowledgement_required(impact, preview.passkey_enrollment);
        Ok(preview)
    }

    /// `Some(Some(url))` saves, `Some(None)` clears, `None` leaves it alone.
    fn validated_public_url_change(
        &self,
        value: Option<&str>,
        reset: bool,
    ) -> AppResult<Option<Option<String>>> {
        if value.is_none() && !reset {
            return Ok(None);
        }
        use crate::public_url::PublicUrlErrorCode;
        let rejected = |code: PublicUrlErrorCode, message: String| AppError::PublicUrlRejected {
            message,
            code,
        };
        if value.is_some() && reset {
            return Err(rejected(
                PublicUrlErrorCode::SaveAndReset,
                "cannot save and reset the public URL together".into(),
            ));
        }
        if self
            .runtime
            .security
            .public_url
            .snapshot()
            .environment_value()
            .is_some()
        {
            return Err(rejected(
                PublicUrlErrorCode::EnvironmentLocked,
                format!(
                    "the public URL is set by {} and cannot be changed here",
                    crate::public_url::PUBLIC_URL_ENV
                ),
            ));
        }
        let Some(value) = value else {
            return Ok(Some(None));
        };
        let base_path = self.runtime.security.public_url.addressing().base_path.clone();
        crate::public_url::normalize_saved_public_url(value, &base_path)
            .map(|saved| Some(Some(saved)))
            .map_err(|error| rejected(error.code, error.message))
    }

    pub async fn initialize_trusted_proxy_policy(&self, environment: &str) -> AppResult<()> {
        use crate::rate_limit_proxy_policy::{IpMatcher, TRUSTED_PROXIES_KEY, TrustedProxyPolicy};
        let _guard = self.runtime.security.service_settings_lock.lock().await;
        let environment = environment
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .filter_map(|value| {
                if IpMatcher::parse(value).is_some() {
                    Some(value.to_string())
                } else {
                    tracing::warn!("ignoring invalid SCRYER_RATE_LIMIT_TRUSTED_PROXY_IPS entry");
                    None
                }
            })
            .collect();
        let saved = self
            .read_setting_json_value::<Option<Vec<String>>>(TRUSTED_PROXIES_KEY, None)
            .await?
            .flatten();
        let policy = TrustedProxyPolicy::new(environment, saved).map_err(AppError::Validation)?;
        self.runtime.security.trusted_proxies.replace(policy);
        Ok(())
    }
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
            session_duration_days: self.session_duration_days().await?,
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

        let policy = self.runtime.security.trusted_proxies.snapshot();
        Ok(ServiceSettings {
            trusted_proxy_ips: policy.addresses.clone(),
            trusted_proxy_override: policy.override_addresses.clone(),
            trusted_proxy_source: if policy.override_addresses.is_some() {
                "settings"
            } else {
                "environment"
            }
            .to_string(),
            tls_cert_path: self
                .read_setting_string_value(TLS_CERT_PATH_KEY, None)
                .await?
                .unwrap_or_default(),
            tls_key_path: self
                .read_setting_string_value(TLS_KEY_PATH_KEY, None)
                .await?
                .unwrap_or_default(),
            public_url: self.public_url_settings().await?,
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
            self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
                .await?;
        }

        let current = self.load_security_settings().await?;
        let session_duration_days = input
            .session_duration_days
            .unwrap_or(current.session_duration_days);
        if !(1..=365).contains(&session_duration_days) {
            return Err(AppError::Validation(
                "session duration must be between 1 and 365 days".into(),
            ));
        }
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
            if self
                .runtime
                .security
                .default_admin_disabled
                .load(std::sync::atomic::Ordering::SeqCst)
            {
                return Err(AppError::Validation("form login cannot be turned off while SCRYER_DISABLE_DEFAULT_ADMIN is true, because running without login uses the default admin account; remove that setting and restart first".into()));
            }
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
        if input.session_duration_days.is_some() {
            self.upsert_system_setting_json(
                settings::keys::SESSION_DURATION_DAYS_KEY,
                &session_duration_days,
                Some(actor.id.clone()),
            )
            .await?;
        }
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
        if input.session_duration_days.is_some() {
            saved_keys.push(settings::keys::SESSION_DURATION_DAYS_KEY.to_string());
        }
        self.emit_settings_saved(actor, "security_settings", None, saved_keys)
            .await;

        Ok(SecuritySettings {
            form_login_enabled: input.form_login_enabled,
            session_duration_days,
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

        use crate::rate_limit_proxy_policy::{TRUSTED_PROXIES_KEY, TrustedProxyPolicy};
        let _guard = self.runtime.security.service_settings_lock.lock().await;
        if input.reset_trusted_proxy_ips && input.trusted_proxy_ips.is_some() {
            return Err(AppError::Validation(
                "cannot save and reset trusted proxies together".into(),
            ));
        }
        let policy = if input.reset_trusted_proxy_ips || input.trusted_proxy_ips.is_some() {
            let saved = input.trusted_proxy_ips.map(|values| {
                values
                    .into_iter()
                    .map(|value| value.trim().to_string())
                    .collect()
            });
            Some(
                TrustedProxyPolicy::new(
                    self.runtime
                        .security
                        .trusted_proxies
                        .snapshot()
                        .environment_addresses
                        .clone(),
                    saved,
                )
                .map_err(AppError::Validation)?,
            )
        } else {
            None
        };
        let public_url = self.validated_public_url_change(
            input.public_url.as_deref(),
            input.reset_public_url,
        )?;
        if let Some(next) = &public_url {
            let addressing = self.runtime.security.public_url.addressing();
            let (impact, _) =
                crate::public_url::passkey_impact_of_saved_value(&addressing, next.as_deref());
            if impact.breaks_existing_passkeys()
                && !input.acknowledge_passkey_impact
                && crate::public_url::passkey_acknowledgement_required(
                    impact,
                    self.passkey_enrollment().await,
                )
            {
                return Err(AppError::PublicUrlRejected {
                    message: "this change stops registered passkeys from working after the next \
                              restart; confirm it explicitly to save"
                        .into(),
                    code: crate::public_url::PublicUrlErrorCode::PasskeyAcknowledgementRequired,
                });
            }
        }
        let mut saved_keys = Vec::new();
        for (key, value) in [
            (TLS_CERT_PATH_KEY, input.tls_cert_path),
            (TLS_KEY_PATH_KEY, input.tls_key_path),
        ] {
            if let Some(value) = value {
                self.upsert_system_setting_json(key, &value.trim(), Some(actor.id.clone()))
                    .await?;
                saved_keys.push(key.to_string());
            }
        }
        if let Some(policy) = policy {
            self.upsert_system_setting_json(
                TRUSTED_PROXIES_KEY,
                &policy.override_addresses,
                Some(actor.id.clone()),
            )
            .await?;
            self.runtime.security.trusted_proxies.replace(policy);
            saved_keys.push(TRUSTED_PROXIES_KEY.to_string());
        }
        if let Some(saved) = public_url {
            use crate::public_url::{PUBLIC_URL_KEY, PublicUrlPolicy};
            self.upsert_system_setting_json(PUBLIC_URL_KEY, &saved, Some(actor.id.clone()))
                .await?;
            let current = self.runtime.security.public_url.snapshot();
            self.runtime.security.public_url.replace(PublicUrlPolicy::new(
                current.environment_value(),
                saved.as_deref(),
            ));
            saved_keys.push(PUBLIC_URL_KEY.to_string());
        }
        self.emit_settings_saved(actor, "service_settings", None, saved_keys)
            .await;

        self.get_service_settings(actor).await
    }
}

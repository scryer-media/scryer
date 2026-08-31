use chrono::{DateTime, Duration, Utc};

use crate::*;

pub(crate) const LOGIN_VERIFICATION_CHALLENGE_TTL_MINUTES: i64 = 5;

impl AppUseCase {
    /// Resolves whether a successful primary credential must be followed by an
    /// enrolled factor. This is deliberately shared by local and media-server
    /// password login so voluntarily enrolled factors cannot be bypassed when
    /// instance-wide enforcement is disabled.
    pub async fn login_verification_requirement(
        &self,
        user: &scryer_domain::User,
        login_method: LoginVerificationMethod,
        policy_requires_mfa: bool,
        persist_session: bool,
        legacy_totp_code: Option<&str>,
        expected_auth_session_version: Option<&Option<String>>,
    ) -> AppResult<LoginVerificationRequirement> {
        self.cleanup_expired_login_verification_challenges().await?;
        let factors = self.user_auth_factor_status(&user.id).await?;
        let current_auth_session_version = self
            .services
            .identity
            .users
            .auth_session_version(&user.id)
            .await?;
        let auth_session_version =
            if let Some(expected_auth_session_version) = expected_auth_session_version {
                if current_auth_session_version != *expected_auth_session_version {
                    return Err(AppError::Unauthorized(
                        "authentication session was invalidated".into(),
                    ));
                }
                expected_auth_session_version.clone()
            } else {
                current_auth_session_version
            };

        if factors.has_mfa
            && let Some(code) = legacy_totp_code.filter(|code| !code.trim().is_empty())
        {
            return self
                .verify_totp_for_user(user, code)
                .await
                .map(|verified_until| {
                    LoginVerificationRequirement::Satisfied(LoginVerificationSatisfied {
                        mfa_verified_until: Some(verified_until),
                        auth_session_version,
                    })
                });
        }

        if !factors.has_mfa && !factors.has_passkey {
            return Ok(if policy_requires_mfa {
                LoginVerificationRequirement::EnrollmentRequired {
                    auth_session_version: auth_session_version.clone(),
                }
            } else {
                LoginVerificationRequirement::Satisfied(LoginVerificationSatisfied {
                    mfa_verified_until: None,
                    auth_session_version,
                })
            });
        }

        let now = Utc::now();
        let challenge = LoginVerificationChallengeRecord {
            id: Id::new().0,
            user_id: user.id.clone(),
            login_method,
            persist_session,
            allow_passkey: factors.has_passkey,
            allow_totp: factors.has_mfa,
            auth_session_version,
            created_at: now.to_rfc3339(),
            expires_at: (now + Duration::minutes(LOGIN_VERIFICATION_CHALLENGE_TTL_MINUTES))
                .to_rfc3339(),
        };
        let challenge = self
            .services
            .identity
            .webauthn
            .create_login_verification_challenge(challenge)
            .await?;
        Ok(LoginVerificationRequirement::Challenge(challenge))
    }

    pub(crate) async fn require_login_verification_factor(
        &self,
        challenge_id: &str,
        require_passkey: bool,
    ) -> AppResult<(LoginVerificationChallengeRecord, scryer_domain::User)> {
        self.cleanup_expired_login_verification_challenges().await?;
        let challenge = self
            .services
            .identity
            .webauthn
            .get_login_verification_challenge(challenge_id)
            .await?
            .ok_or_else(|| {
                AppError::Unauthorized("login verification challenge is unavailable".into())
            })?;
        if self.login_verification_challenge_expired(&challenge)? {
            let _ = self
                .services
                .identity
                .webauthn
                .take_login_verification_challenge(challenge_id)
                .await?;
            return Err(AppError::Validation(
                "login verification challenge has expired".into(),
            ));
        }
        if (require_passkey && !challenge.allow_passkey)
            || (!require_passkey && !challenge.allow_totp)
        {
            return Err(AppError::Unauthorized(
                "requested login verification factor is unavailable".into(),
            ));
        }
        let user = self
            .services
            .identity
            .users
            .get_by_id(&challenge.user_id)
            .await?
            .ok_or_else(|| {
                AppError::Unauthorized("login verification user is unavailable".into())
            })?;
        if !user.login_status().is_enabled() {
            return Err(AppError::Unauthorized(
                "login verification user is unavailable".into(),
            ));
        }
        let current_auth_session_version = self
            .services
            .identity
            .users
            .auth_session_version(&user.id)
            .await?;
        if current_auth_session_version != challenge.auth_session_version {
            return Err(AppError::Unauthorized(
                "login verification challenge was invalidated".into(),
            ));
        }
        Ok((challenge, user))
    }

    pub async fn complete_login_verification_totp(
        &self,
        challenge_id: &str,
        code: &str,
    ) -> AppResult<(
        scryer_domain::User,
        DateTime<Utc>,
        bool,
        Option<String>,
        bool,
    )> {
        let (challenge, user) = self
            .require_login_verification_factor(challenge_id, false)
            .await?;
        let verified_until = self.verify_totp_for_user(&user, code).await?;
        let consumed = self
            .services
            .identity
            .webauthn
            .take_login_verification_challenge(challenge_id)
            .await?
            .ok_or_else(|| {
                AppError::Unauthorized("login verification challenge was already used".into())
            })?;
        if consumed.user_id != user.id || consumed.id != challenge.id {
            return Err(AppError::Unauthorized(
                "login verification challenge was invalidated".into(),
            ));
        }
        let password_change_required = consumed.login_method
            == LoginVerificationMethod::LocalPassword
            && user.password_change_required;
        Ok((
            user,
            verified_until,
            consumed.persist_session,
            consumed.auth_session_version,
            password_change_required,
        ))
    }

    pub(crate) async fn consume_login_verification_challenge(
        &self,
        challenge_id: &str,
        expected_user_id: &str,
    ) -> AppResult<LoginVerificationChallengeRecord> {
        let challenge = self
            .services
            .identity
            .webauthn
            .take_login_verification_challenge(challenge_id)
            .await?
            .ok_or_else(|| {
                AppError::Unauthorized("login verification challenge was already used".into())
            })?;
        if challenge.user_id != expected_user_id
            || self.login_verification_challenge_expired(&challenge)?
        {
            return Err(AppError::Unauthorized(
                "login verification challenge was invalidated".into(),
            ));
        }
        Ok(challenge)
    }

    pub async fn cleanup_expired_login_verification_challenges(&self) -> AppResult<()> {
        self.services
            .identity
            .webauthn
            .delete_expired_login_verification_challenges(&Utc::now().to_rfc3339())
            .await?;
        Ok(())
    }

    fn login_verification_challenge_expired(
        &self,
        challenge: &LoginVerificationChallengeRecord,
    ) -> AppResult<bool> {
        let expires_at = DateTime::parse_from_rfc3339(&challenge.expires_at)
            .map_err(|error| {
                AppError::Repository(format!(
                    "invalid login verification challenge expiry {}: {error}",
                    challenge.expires_at
                ))
            })?
            .with_timezone(&Utc);
        Ok(expires_at <= Utc::now())
    }
}

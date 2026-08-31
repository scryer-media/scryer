use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use webauthn_rs::prelude::{
    DiscoverableAuthentication, DiscoverableKey, Passkey, PasskeyAuthentication,
    PasskeyRegistration, PublicKeyCredential, RegisterPublicKeyCredential,
};

use super::*;

const WEBAUTHN_CHALLENGE_TTL_MINUTES: i64 = 5;

#[derive(Debug, Serialize, Deserialize)]
enum StoredAuthenticationState {
    Passkey(PasskeyAuthentication),
    Discoverable(DiscoverableAuthentication),
}

#[derive(Debug, Serialize, Deserialize)]
enum StoredChallengeState {
    Registration(PasskeyRegistration),
    Authentication(StoredAuthenticationState),
}

impl AppUseCase {
    fn ensure_passkey_management_enabled(&self) -> AppResult<()> {
        if self.webauthn.available().is_none() {
            return Err(AppError::Validation(
                "passkey authentication is not configured".into(),
            ));
        }

        Ok(())
    }

    fn ensure_passkey_authentication_enabled(&self, form_login_enabled: bool) -> AppResult<()> {
        if !form_login_enabled {
            return Err(AppError::Validation(
                "passkey authentication is unavailable while form login is disabled".into(),
            ));
        }

        self.ensure_passkey_management_enabled()
    }

    fn webauthn_runtime(&self) -> AppResult<&webauthn_rs::Webauthn> {
        self.webauthn
            .available()
            .map(Arc::as_ref)
            .ok_or_else(|| AppError::Validation("passkey authentication is not configured".into()))
    }

    async fn load_passkey_user(&self, user_id: &str) -> AppResult<User> {
        let user = self
            .services
            .identity
            .users
            .get_by_id(user_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("user {user_id}")))?;

        if !user.login_status().is_enabled() {
            return Err(AppError::Unauthorized("credentials unavailable".into()));
        }

        Ok(user)
    }

    fn user_id_candidates_for_webauthn_uuid(user_uuid: Uuid) -> Vec<String> {
        let mut candidates = vec![user_uuid.to_string()];
        let compact = user_uuid.simple().to_string();
        if compact != candidates[0] {
            candidates.push(compact);
        }
        candidates
    }

    async fn load_passkey_user_by_webauthn_uuid(&self, user_uuid: Uuid) -> AppResult<User> {
        for candidate in Self::user_id_candidates_for_webauthn_uuid(user_uuid) {
            match self.load_passkey_user(&candidate).await {
                Ok(user) => return Ok(user),
                Err(AppError::NotFound(_)) => {}
                Err(error) => return Err(error),
            }
        }

        Err(AppError::NotFound(format!("user {user_uuid}")))
    }

    async fn cleanup_expired_webauthn_challenges(&self) -> AppResult<()> {
        self.services
            .identity
            .webauthn
            .delete_expired_challenges(&Utc::now().to_rfc3339())
            .await?;
        Ok(())
    }

    fn parse_user_uuid(&self, user_id: &str) -> AppResult<Uuid> {
        Uuid::parse_str(user_id).map_err(|error| {
            AppError::Repository(format!("user id {user_id} is not a valid UUID: {error}"))
        })
    }

    fn trim_friendly_name(value: Option<String>) -> Option<String> {
        value.and_then(|value| {
            let trimmed = value.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        })
    }

    fn challenge_expired(record: &WebauthnChallengeRecord) -> bool {
        chrono::DateTime::parse_from_rfc3339(&record.expires_at)
            .map(|value| value.with_timezone(&Utc) <= Utc::now())
            .unwrap_or(true)
    }

    fn encode_credential_id(bytes: &[u8]) -> String {
        URL_SAFE_NO_PAD.encode(bytes)
    }

    fn deserialize_passkey(record: &WebauthnCredentialRecord) -> AppResult<Passkey> {
        serde_json::from_str(&record.credential_json).map_err(|error| {
            AppError::Repository(format!(
                "failed to decode stored passkey {}: {error}",
                record.id
            ))
        })
    }

    fn passkey_summary(record: WebauthnCredentialRecord) -> PasskeySummary {
        PasskeySummary {
            id: record.id,
            friendly_name: record.friendly_name,
            created_at: record.created_at,
            last_used_at: record.last_used_at,
        }
    }

    pub fn passkey_enabled(&self) -> bool {
        self.webauthn.available().is_some()
    }

    pub async fn webauthn_register_start(
        &self,
        actor: &User,
        form_login_enabled: bool,
    ) -> AppResult<WebauthnChallengeStart> {
        self.require_actor_capability(actor, scryer_domain::ActorCapability::ManageOwnAccount)
            .await?;
        let user = self.load_passkey_user(&actor.id).await?;
        self.webauthn_register_start_for_user(
            user,
            WebauthnChallengePurpose::AccountRegistration,
            form_login_enabled,
        )
        .await
    }

    /// Starts passkey enrollment while the actor holds the restricted
    /// post-primary MFA-enrollment session.
    pub async fn webauthn_login_enrollment_start(
        &self,
        actor: &User,
        form_login_enabled: bool,
    ) -> AppResult<WebauthnChallengeStart> {
        let user = self.load_passkey_user(&actor.id).await?;
        self.webauthn_register_start_for_user(
            user,
            WebauthnChallengePurpose::LoginEnrollment,
            form_login_enabled,
        )
        .await
    }

    async fn webauthn_register_start_for_user(
        &self,
        user: User,
        purpose: WebauthnChallengePurpose,
        form_login_enabled: bool,
    ) -> AppResult<WebauthnChallengeStart> {
        self.ensure_passkey_authentication_enabled(form_login_enabled)?;
        self.cleanup_expired_webauthn_challenges().await?;
        let auth_session_version = self
            .services
            .identity
            .users
            .auth_session_version(&user.id)
            .await?;

        let existing_records = self
            .services
            .identity
            .webauthn
            .list_credentials_for_user(&user.id)
            .await?;
        let existing_passkeys = existing_records
            .iter()
            .map(Self::deserialize_passkey)
            .collect::<AppResult<Vec<_>>>()?;
        let exclude_credentials = (!existing_passkeys.is_empty()).then(|| {
            existing_passkeys
                .iter()
                .map(|passkey| passkey.cred_id().clone())
                .collect::<Vec<_>>()
        });

        let (options, state) = self
            .webauthn_runtime()?
            .start_passkey_registration(
                self.parse_user_uuid(&user.id)?,
                &user.username,
                &user.username,
                exclude_credentials,
            )
            .map_err(|error| {
                AppError::Validation(format!("failed to start passkey registration: {error}"))
            })?;

        let challenge = WebauthnChallengeRecord {
            id: Id::new().0,
            user_id: Some(user.id),
            challenge_type: WebauthnChallengeType::Registration,
            purpose,
            login_verification_challenge_id: None,
            auth_session_version,
            state_json: serde_json::to_string(&StoredChallengeState::Registration(state)).map_err(
                |error| {
                    AppError::Repository(format!(
                        "failed to persist passkey registration state: {error}"
                    ))
                },
            )?,
            created_at: Utc::now().to_rfc3339(),
            expires_at: (Utc::now() + Duration::minutes(WEBAUTHN_CHALLENGE_TTL_MINUTES))
                .to_rfc3339(),
        };

        self.services
            .identity
            .webauthn
            .create_challenge(challenge.clone())
            .await?;

        Ok(WebauthnChallengeStart {
            challenge_id: challenge.id,
            options_json: serde_json::to_string(&options).map_err(|error| {
                AppError::Repository(format!(
                    "failed to encode passkey registration options: {error}"
                ))
            })?,
            expires_at: challenge.expires_at,
        })
    }

    pub async fn webauthn_register_complete(
        &self,
        actor: &User,
        challenge_id: &str,
        response_json: &str,
        friendly_name: Option<String>,
        form_login_enabled: bool,
    ) -> AppResult<PasskeySummary> {
        self.require_actor_capability(actor, scryer_domain::ActorCapability::ManageOwnAccount)
            .await?;
        self.ensure_passkey_authentication_enabled(form_login_enabled)?;
        let user = self.load_passkey_user(&actor.id).await?;
        self.webauthn_register_complete_for_user(
            user,
            challenge_id,
            response_json,
            friendly_name,
            WebauthnChallengePurpose::AccountRegistration,
        )
        .await
    }

    /// Completes passkey enrollment for a restricted post-primary login and
    /// returns the registered credential summary for the final login payload.
    pub async fn webauthn_login_enrollment_complete(
        &self,
        actor: &User,
        challenge_id: &str,
        response_json: &str,
        friendly_name: Option<String>,
        form_login_enabled: bool,
    ) -> AppResult<PasskeySummary> {
        self.ensure_passkey_authentication_enabled(form_login_enabled)?;
        let user = self.load_passkey_user(&actor.id).await?;
        self.webauthn_register_complete_for_user(
            user,
            challenge_id,
            response_json,
            friendly_name,
            WebauthnChallengePurpose::LoginEnrollment,
        )
        .await
    }

    async fn webauthn_register_complete_for_user(
        &self,
        user: User,
        challenge_id: &str,
        response_json: &str,
        friendly_name: Option<String>,
        expected_purpose: WebauthnChallengePurpose,
    ) -> AppResult<PasskeySummary> {
        let challenge = self
            .services
            .identity
            .webauthn
            .take_challenge(challenge_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("passkey challenge {challenge_id}")))?;

        if challenge.user_id.as_deref() != Some(user.id.as_str()) {
            return Err(AppError::Unauthorized(
                "passkey challenge does not belong to the current user".into(),
            ));
        }
        if challenge.challenge_type != WebauthnChallengeType::Registration {
            return Err(AppError::Validation(
                "passkey challenge is not a registration ceremony".into(),
            ));
        }
        if challenge.purpose != expected_purpose {
            return Err(AppError::Validation(
                "passkey challenge has an unexpected registration purpose".into(),
            ));
        }
        if Self::challenge_expired(&challenge) {
            return Err(AppError::Validation("passkey challenge has expired".into()));
        }

        let registration = serde_json::from_str::<RegisterPublicKeyCredential>(response_json)
            .map_err(|error| {
                AppError::Validation(format!(
                    "invalid passkey registration response payload: {error}"
                ))
            })?;
        let state = match serde_json::from_str::<StoredChallengeState>(&challenge.state_json)
            .map_err(|error| {
                AppError::Repository(format!(
                    "failed to decode stored passkey registration state: {error}"
                ))
            })? {
            StoredChallengeState::Registration(state) => state,
            StoredChallengeState::Authentication(_) => {
                return Err(AppError::Validation(
                    "passkey challenge stored an authentication ceremony".into(),
                ));
            }
        };

        let passkey = self
            .webauthn_runtime()?
            .finish_passkey_registration(&registration, &state)
            .map_err(|error| {
                AppError::Validation(format!("failed to finish passkey registration: {error}"))
            })?;
        let credential_id = Self::encode_credential_id(passkey.cred_id().as_ref());

        if self
            .services
            .identity
            .webauthn
            .get_credential_by_credential_id(&credential_id)
            .await?
            .is_some()
        {
            return Err(AppError::Validation(
                "a passkey with this credential id is already registered".into(),
            ));
        }

        let created = self
            .services
            .identity
            .webauthn
            .create_credential_for_current_session(
                WebauthnCredentialRecord {
                    id: Id::new().0,
                    user_id: user.id,
                    credential_id,
                    credential_json: serde_json::to_string(&passkey).map_err(|error| {
                        AppError::Repository(format!(
                            "failed to persist registered passkey credential: {error}"
                        ))
                    })?,
                    friendly_name: Self::trim_friendly_name(friendly_name),
                    created_at: Utc::now().to_rfc3339(),
                    last_used_at: None,
                },
                challenge.auth_session_version.as_deref(),
            )
            .await?;

        Ok(Self::passkey_summary(created))
    }

    /// Starts an actor-bound assertion that establishes account-security freshness.
    pub async fn account_security_passkey_start(
        &self,
        actor: &User,
        form_login_enabled: bool,
    ) -> AppResult<WebauthnChallengeStart> {
        self.require_actor_capability(actor, scryer_domain::ActorCapability::ManageOwnAccount)
            .await?;
        self.ensure_passkey_authentication_enabled(form_login_enabled)?;
        self.cleanup_expired_webauthn_challenges().await?;

        let user = self.load_passkey_user(&actor.id).await?;
        let records = self
            .services
            .identity
            .webauthn
            .list_credentials_for_user(&user.id)
            .await?;
        let passkeys = records
            .iter()
            .map(Self::deserialize_passkey)
            .collect::<AppResult<Vec<_>>>()?;
        if passkeys.is_empty() {
            return Err(AppError::Unauthorized(
                "passkey reauthentication is unavailable for this account".into(),
            ));
        }

        let (options, state) = self
            .webauthn_runtime()?
            .start_passkey_authentication(&passkeys)
            .map_err(|error| {
                AppError::Validation(format!("failed to start passkey authentication: {error}"))
            })?;
        let now = Utc::now();
        let auth_session_version = self
            .services
            .identity
            .users
            .auth_session_version(&user.id)
            .await?;
        let challenge = WebauthnChallengeRecord {
            id: Id::new().0,
            user_id: Some(user.id),
            challenge_type: WebauthnChallengeType::Authentication,
            purpose: WebauthnChallengePurpose::AccountSecurityReauthentication,
            login_verification_challenge_id: None,
            auth_session_version,
            state_json: serde_json::to_string(&StoredChallengeState::Authentication(
                StoredAuthenticationState::Passkey(state),
            ))
            .map_err(|error| {
                AppError::Repository(format!(
                    "failed to persist passkey reauthentication state: {error}"
                ))
            })?,
            created_at: now.to_rfc3339(),
            expires_at: (now + Duration::minutes(WEBAUTHN_CHALLENGE_TTL_MINUTES)).to_rfc3339(),
        };
        self.services
            .identity
            .webauthn
            .create_challenge(challenge.clone())
            .await?;

        Ok(WebauthnChallengeStart {
            challenge_id: challenge.id,
            options_json: serde_json::to_string(&options).map_err(|error| {
                AppError::Repository(format!(
                    "failed to encode passkey reauthentication options: {error}"
                ))
            })?,
            expires_at: challenge.expires_at,
        })
    }

    pub async fn webauthn_authenticate_start(
        &self,
        _username: Option<&str>,
        form_login_enabled: bool,
    ) -> AppResult<WebauthnChallengeStart> {
        self.ensure_passkey_authentication_enabled(form_login_enabled)?;
        self.cleanup_expired_webauthn_challenges().await?;

        // Username remains accepted for GraphQL compatibility but is deliberately
        // not an account selector: every public start is discoverable.
        let (options, state) = self
            .webauthn_runtime()?
            .start_discoverable_authentication()
            .map_err(|error| {
                AppError::Validation(format!(
                    "failed to start discoverable passkey authentication: {error}"
                ))
            })?;
        let record = WebauthnChallengeRecord {
            id: Id::new().0,
            user_id: None,
            challenge_type: WebauthnChallengeType::Authentication,
            purpose: WebauthnChallengePurpose::StandaloneAuthentication,
            login_verification_challenge_id: None,
            auth_session_version: None,
            state_json: serde_json::to_string(&StoredChallengeState::Authentication(
                StoredAuthenticationState::Discoverable(state),
            ))
            .map_err(|error| {
                AppError::Repository(format!(
                    "failed to persist discoverable passkey authentication state: {error}"
                ))
            })?,
            created_at: Utc::now().to_rfc3339(),
            expires_at: (Utc::now() + Duration::minutes(WEBAUTHN_CHALLENGE_TTL_MINUTES))
                .to_rfc3339(),
        };
        let options_json = serde_json::to_string(&options).map_err(|error| {
            AppError::Repository(format!(
                "failed to encode discoverable passkey authentication options: {error}"
            ))
        })?;

        self.services
            .identity
            .webauthn
            .create_challenge(record.clone())
            .await?;

        Ok(WebauthnChallengeStart {
            challenge_id: record.id,
            options_json,
            expires_at: record.expires_at,
        })
    }

    pub async fn webauthn_authenticate_complete(
        &self,
        challenge_id: &str,
        response_json: &str,
        form_login_enabled: bool,
    ) -> AppResult<(User, Option<String>)> {
        self.webauthn_authenticate_complete_for_purpose(
            challenge_id,
            response_json,
            form_login_enabled,
            WebauthnChallengePurpose::StandaloneAuthentication,
            None,
        )
        .await
    }

    /// Completes the actor-bound assertion required to refresh account-security access.
    pub async fn account_security_passkey_complete(
        &self,
        actor: &User,
        challenge_id: &str,
        response_json: &str,
        form_login_enabled: bool,
    ) -> AppResult<()> {
        self.require_actor_capability(actor, scryer_domain::ActorCapability::ManageOwnAccount)
            .await?;
        let (user, _) = self
            .webauthn_authenticate_complete_for_purpose(
                challenge_id,
                response_json,
                form_login_enabled,
                WebauthnChallengePurpose::AccountSecurityReauthentication,
                None,
            )
            .await?;
        if user.id != actor.id {
            return Err(AppError::Unauthorized(
                "passkey reauthentication challenge belongs to another user".into(),
            ));
        }
        Ok(())
    }

    async fn webauthn_authenticate_complete_for_purpose(
        &self,
        challenge_id: &str,
        response_json: &str,
        form_login_enabled: bool,
        expected_purpose: WebauthnChallengePurpose,
        expected_login_verification_challenge_id: Option<&str>,
    ) -> AppResult<(User, Option<String>)> {
        self.ensure_passkey_authentication_enabled(form_login_enabled)?;

        let challenge = self
            .services
            .identity
            .webauthn
            .take_challenge(challenge_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("passkey challenge {challenge_id}")))?;

        if challenge.challenge_type != WebauthnChallengeType::Authentication {
            return Err(AppError::Validation(
                "passkey challenge is not an authentication ceremony".into(),
            ));
        }
        if challenge.purpose != expected_purpose {
            return Err(AppError::Validation(
                "passkey challenge has an unexpected authentication purpose".into(),
            ));
        }
        if challenge.login_verification_challenge_id.as_deref()
            != expected_login_verification_challenge_id
        {
            return Err(AppError::Unauthorized(
                "passkey challenge does not belong to this login verification".into(),
            ));
        }
        if Self::challenge_expired(&challenge) {
            return Err(AppError::Validation("passkey challenge has expired".into()));
        }
        if let Some(expected_auth_session_version) = &challenge.auth_session_version {
            let current_auth_session_version = self
                .services
                .identity
                .users
                .auth_session_version(challenge.user_id.as_deref().ok_or_else(|| {
                    AppError::Unauthorized("passkey challenge is not actor-bound".into())
                })?)
                .await?;
            if current_auth_session_version.as_deref() != Some(expected_auth_session_version) {
                return Err(AppError::Unauthorized(
                    "passkey challenge session has been invalidated".into(),
                ));
            }
        }

        let credential =
            serde_json::from_str::<PublicKeyCredential>(response_json).map_err(|error| {
                AppError::Validation(format!(
                    "invalid passkey authentication response payload: {error}"
                ))
            })?;
        let state = match serde_json::from_str::<StoredChallengeState>(&challenge.state_json)
            .map_err(|error| {
                AppError::Repository(format!(
                    "failed to decode stored passkey authentication state: {error}"
                ))
            })? {
            StoredChallengeState::Authentication(state) => state,
            StoredChallengeState::Registration(_) => {
                return Err(AppError::Validation(
                    "passkey challenge stored a registration ceremony".into(),
                ));
            }
        };

        match state {
            StoredAuthenticationState::Passkey(state) => {
                let user_id = challenge.user_id.as_deref().ok_or_else(|| {
                    AppError::Repository("authentication challenge missing user id".into())
                })?;
                let user = self.load_passkey_user(user_id).await?;
                let requested_credential_id = Self::encode_credential_id(&credential.raw_id);
                let mut record = self
                    .services
                    .identity
                    .webauthn
                    .get_credential_by_credential_id(&requested_credential_id)
                    .await?
                    .ok_or_else(|| AppError::Unauthorized("passkey credential not found".into()))?;
                if record.user_id != user.id {
                    return Err(AppError::Unauthorized(
                        "passkey credential does not belong to the challenge user".into(),
                    ));
                }
                let auth_session_version = self
                    .services
                    .identity
                    .users
                    .auth_session_version(&user.id)
                    .await?;
                let original_credential_json = record.credential_json.clone();
                let mut passkey = Self::deserialize_passkey(&record)?;
                let auth_result = self
                    .webauthn_runtime()?
                    .finish_passkey_authentication(&credential, &state, &passkey)
                    .map_err(|error| {
                        AppError::Unauthorized(format!(
                            "failed to finish passkey authentication: {error}"
                        ))
                    })?;
                passkey.update_credential(&auth_result);
                record.credential_json = serde_json::to_string(&passkey).map_err(|error| {
                    AppError::Repository(format!(
                        "failed to persist updated passkey credential state: {error}"
                    ))
                })?;
                record.last_used_at = Some(Utc::now().to_rfc3339());
                self.services
                    .identity
                    .webauthn
                    .update_credential_if_current(record, &original_credential_json)
                    .await?
                    .ok_or_else(|| {
                        AppError::Unauthorized(
                            "passkey credential changed during authentication".into(),
                        )
                    })?;
                Ok((user, auth_session_version))
            }
            StoredAuthenticationState::Discoverable(state) => {
                let (user_uuid, credential_id) = self
                    .webauthn_runtime()?
                    .identify_discoverable_authentication(&credential)
                    .map_err(|error| {
                        AppError::Unauthorized(format!(
                            "failed to identify discoverable passkey authentication: {error}"
                        ))
                    })?;
                let credential_id = Self::encode_credential_id(credential_id);
                let user = self.load_passkey_user_by_webauthn_uuid(user_uuid).await?;
                let mut record = self
                    .services
                    .identity
                    .webauthn
                    .get_credential_by_credential_id(&credential_id)
                    .await?
                    .ok_or_else(|| {
                        AppError::Unauthorized(
                            "discoverable passkey credential was not found".into(),
                        )
                    })?;
                if record.user_id != user.id {
                    return Err(AppError::Unauthorized(
                        "discoverable passkey credential does not belong to the resolved user"
                            .into(),
                    ));
                }
                let auth_session_version = self
                    .services
                    .identity
                    .users
                    .auth_session_version(&user.id)
                    .await?;
                let original_credential_json = record.credential_json.clone();
                let mut passkey = Self::deserialize_passkey(&record)?;
                let discoverable_key: DiscoverableKey = passkey.clone().into();
                let auth_result = self
                    .webauthn_runtime()?
                    .finish_discoverable_authentication(&credential, state, &[discoverable_key])
                    .map_err(|error| {
                        AppError::Unauthorized(format!(
                            "failed to finish discoverable passkey authentication: {error}"
                        ))
                    })?;
                passkey.update_credential(&auth_result);
                record.credential_json = serde_json::to_string(&passkey).map_err(|error| {
                    AppError::Repository(format!(
                        "failed to persist updated discoverable passkey state: {error}"
                    ))
                })?;
                record.last_used_at = Some(Utc::now().to_rfc3339());
                self.services
                    .identity
                    .webauthn
                    .update_credential_if_current(record, &original_credential_json)
                    .await?
                    .ok_or_else(|| {
                        AppError::Unauthorized(
                            "discoverable passkey credential changed during authentication".into(),
                        )
                    })?;
                Ok((user, auth_session_version))
            }
        }
    }

    pub async fn login_verification_passkey_start(
        &self,
        login_verification_challenge_id: &str,
        form_login_enabled: bool,
    ) -> AppResult<WebauthnChallengeStart> {
        self.ensure_passkey_authentication_enabled(form_login_enabled)?;
        self.cleanup_expired_webauthn_challenges().await?;
        let (verification, user) = self
            .require_login_verification_factor(login_verification_challenge_id, true)
            .await?;
        let auth_session_version = verification.auth_session_version.clone();
        let records = self
            .services
            .identity
            .webauthn
            .list_credentials_for_user(&user.id)
            .await?;
        let passkeys = records
            .iter()
            .map(Self::deserialize_passkey)
            .collect::<AppResult<Vec<_>>>()?;
        if passkeys.is_empty() {
            return Err(AppError::Unauthorized(
                "passkey factor is no longer available".into(),
            ));
        }
        let (options, state) = self
            .webauthn_runtime()?
            .start_passkey_authentication(&passkeys)
            .map_err(|error| {
                AppError::Validation(format!("failed to start passkey authentication: {error}"))
            })?;
        let now = Utc::now();
        let challenge = WebauthnChallengeRecord {
            id: Id::new().0,
            user_id: Some(user.id),
            challenge_type: WebauthnChallengeType::Authentication,
            purpose: WebauthnChallengePurpose::LoginVerification,
            login_verification_challenge_id: Some(verification.id),
            auth_session_version,
            state_json: serde_json::to_string(&StoredChallengeState::Authentication(
                StoredAuthenticationState::Passkey(state),
            ))
            .map_err(|error| {
                AppError::Repository(format!(
                    "failed to persist passkey verification state: {error}"
                ))
            })?,
            created_at: now.to_rfc3339(),
            expires_at: (now + Duration::minutes(WEBAUTHN_CHALLENGE_TTL_MINUTES)).to_rfc3339(),
        };
        self.services
            .identity
            .webauthn
            .create_challenge(challenge.clone())
            .await?;
        Ok(WebauthnChallengeStart {
            challenge_id: challenge.id,
            options_json: serde_json::to_string(&options).map_err(|error| {
                AppError::Repository(format!(
                    "failed to encode passkey verification options: {error}"
                ))
            })?,
            expires_at: challenge.expires_at,
        })
    }

    pub async fn login_verification_passkey_complete(
        &self,
        login_verification_challenge_id: &str,
        webauthn_challenge_id: &str,
        response_json: &str,
        form_login_enabled: bool,
    ) -> AppResult<(User, chrono::DateTime<Utc>, bool, Option<String>, bool)> {
        let (_expected_verification, expected_user) = self
            .require_login_verification_factor(login_verification_challenge_id, true)
            .await?;
        let (user, _) = self
            .webauthn_authenticate_complete_for_purpose(
                webauthn_challenge_id,
                response_json,
                form_login_enabled,
                WebauthnChallengePurpose::LoginVerification,
                Some(login_verification_challenge_id),
            )
            .await?;
        if user.id != expected_user.id {
            return Err(AppError::Unauthorized(
                "passkey credential does not belong to this login verification".into(),
            ));
        }
        let verification = self
            .consume_login_verification_challenge(login_verification_challenge_id, &user.id)
            .await?;
        let password_change_required = verification.login_method
            == LoginVerificationMethod::LocalPassword
            && user.password_change_required;
        Ok((
            user,
            self.mfa_freshness_verified_until(),
            verification.persist_session,
            verification.auth_session_version,
            password_change_required,
        ))
    }

    pub async fn list_my_passkeys(
        &self,
        actor: &User,
        form_login_enabled: bool,
    ) -> AppResult<Vec<PasskeySummary>> {
        self.require_actor_capability(actor, scryer_domain::ActorCapability::ManageOwnAccount)
            .await?;
        self.ensure_passkey_authentication_enabled(form_login_enabled)?;
        let records = self
            .services
            .identity
            .webauthn
            .list_credentials_for_user(&actor.id)
            .await?;
        Ok(records.into_iter().map(Self::passkey_summary).collect())
    }

    pub async fn delete_my_passkey(
        &self,
        actor: &User,
        credential_record_id: &str,
        form_login_enabled: bool,
        expected_auth_session_version: Option<&str>,
    ) -> AppResult<()> {
        self.require_actor_capability(actor, scryer_domain::ActorCapability::ManageOwnAccount)
            .await?;
        self.ensure_passkey_authentication_enabled(form_login_enabled)?;
        self.services
            .identity
            .webauthn
            .delete_credential_preserving_login_route_for_current_session(
                credential_record_id,
                &actor.id,
                expected_auth_session_version,
            )
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webauthn_uuid_lookup_candidates_include_legacy_compact_admin_id() {
        let user_uuid =
            Uuid::parse_str("00000000-0000-0000-0000-000000000001").expect("valid uuid");

        assert_eq!(
            AppUseCase::user_id_candidates_for_webauthn_uuid(user_uuid),
            vec![
                "00000000-0000-0000-0000-000000000001".to_string(),
                "00000000000000000000000000000001".to_string(),
            ],
        );
    }
}

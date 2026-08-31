use aws_lc_rs::hmac;
use aws_lc_rs::rand::{SecureRandom, SystemRandom};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{Duration, Utc};
use scryer_domain::{ActorCapability, AppPermissionMask, Id, User};
use std::collections::HashSet;

use crate::{ApiKeyProvisioningSource, ApiKeyRecord, AppError, AppResult, AppUseCase};

pub const API_KEY_PREFIX: &str = "ska";
const API_KEY_LOOKUP_BYTES: usize = 16;
const API_KEY_SECRET_BYTES: usize = 32;
const API_KEY_LABEL_MAX_LENGTH: usize = 120;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApiKeyExpiryPreset {
    Days30,
    Days90,
    Days365,
    Never,
}

impl ApiKeyExpiryPreset {
    pub fn expires_at(self) -> Option<chrono::DateTime<Utc>> {
        match self {
            Self::Days30 => Some(Utc::now() + Duration::days(30)),
            Self::Days90 => Some(Utc::now() + Duration::days(90)),
            Self::Days365 => Some(Utc::now() + Duration::days(365)),
            Self::Never => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateApiKey {
    pub label: String,
    pub expiry: ApiKeyExpiryPreset,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApiKeySummary {
    pub id: String,
    pub label: String,
    pub expires_at: Option<chrono::DateTime<Utc>>,
    pub revoked_at: Option<chrono::DateTime<Utc>>,
    pub last_used_at: Option<chrono::DateTime<Utc>>,
    pub created_at: chrono::DateTime<Utc>,
    pub provisioning_source: ApiKeyProvisioningSource,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreatedApiKey {
    pub api_key: ApiKeySummary,
    pub raw_key: String,
}

#[derive(Clone, Debug)]
pub struct ApiKeyAuthentication {
    pub user: User,
    pub key_id: String,
    pub key_label: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DevelopmentApiKeySeed {
    pub username: String,
    pub label: String,
    pub raw_key: String,
    pub expires_at: Option<chrono::DateTime<Utc>>,
}

impl AppUseCase {
    pub async fn list_api_keys(&self, actor: &User) -> AppResult<Vec<ApiKeySummary>> {
        self.require_actor_capability(actor, ActorCapability::ManageOwnAccount)
            .await?;
        self.services
            .identity
            .oauth
            .list_api_keys(&actor.id)
            .await?
            .into_iter()
            .map(api_key_summary)
            .collect()
    }

    pub async fn create_api_key(
        &self,
        actor: &User,
        input: CreateApiKey,
    ) -> AppResult<CreatedApiKey> {
        self.require_actor_capability(actor, ActorCapability::ManageOwnAccount)
            .await?;
        self.ensure_api_key_owner_is_eligible(actor).await?;
        let label = validate_api_key_label(input.label)?;
        let (lookup_id, secret) = generate_api_key_material()?;
        let now = Utc::now();
        let record = ApiKeyRecord {
            id: Id::new().0,
            user_id: actor.id.clone(),
            lookup_id: lookup_id.clone(),
            secret_hash: self.api_key_hash(&lookup_id, &secret),
            label,
            expires_at: input.expiry.expires_at(),
            revoked_at: None,
            last_used_at: None,
            created_at: now,
            provisioning_source: ApiKeyProvisioningSource::User,
        };
        let record = self.services.identity.oauth.create_api_key(record).await?;
        Ok(CreatedApiKey {
            api_key: api_key_summary(record)?,
            raw_key: format!("{API_KEY_PREFIX}_{lookup_id}.{secret}"),
        })
    }

    pub async fn revoke_api_key(&self, actor: &User, id: &str) -> AppResult<bool> {
        self.require_actor_capability(actor, ActorCapability::ManageOwnAccount)
            .await?;
        let keys = self
            .services
            .identity
            .oauth
            .list_api_keys(&actor.id)
            .await?;
        if keys.iter().any(|key| {
            key.id == id && key.provisioning_source == ApiKeyProvisioningSource::Environment
        }) {
            return Err(AppError::Validation(
                "environment-seeded API keys are managed by SCRYER_DEV_API_KEYS".into(),
            ));
        }
        self.services
            .identity
            .oauth
            .revoke_api_key(id, &actor.id, Utc::now())
            .await
    }

    pub async fn authenticate_api_key(&self, raw_key: &str) -> AppResult<ApiKeyAuthentication> {
        let (lookup_id, secret) = parse_api_key(raw_key)
            .ok_or_else(|| AppError::Unauthorized("invalid API key".into()))?;
        let record = self
            .services
            .identity
            .oauth
            .get_api_key_by_lookup_id(&lookup_id)
            .await?
            .ok_or_else(|| AppError::Unauthorized("invalid API key".into()))?;
        if record.revoked_at.is_some()
            || record
                .expires_at
                .is_some_and(|expires_at| expires_at <= Utc::now())
        {
            return Err(AppError::Unauthorized(
                "API key is expired or revoked".into(),
            ));
        }
        let expected_hash = self.api_key_hash(&lookup_id, &secret);
        let expected_tag = URL_SAFE_NO_PAD
            .decode(expected_hash)
            .map_err(|_| AppError::Unauthorized("invalid API key".into()))?;
        let actual_tag = URL_SAFE_NO_PAD
            .decode(&record.secret_hash)
            .map_err(|_| AppError::Unauthorized("invalid API key".into()))?;
        if expected_tag.len() != actual_tag.len()
            || hmac::verify(
                &hmac::Key::new(hmac::HMAC_SHA256, self.auth.jwt_signing_salt.as_bytes()),
                format!("api-key:{lookup_id}:{secret}").as_bytes(),
                &actual_tag,
            )
            .is_err()
        {
            return Err(AppError::Unauthorized("invalid API key".into()));
        }
        let user = self
            .services
            .identity
            .users
            .get_by_id(&record.user_id)
            .await?
            .ok_or_else(|| AppError::Unauthorized("API key owner no longer exists".into()))?;
        if !user.login_status().is_enabled() {
            return Err(AppError::Unauthorized("API key owner is disabled".into()));
        }
        let user = self.attach_user_authorization(user).await?;
        self.ensure_api_key_owner_is_eligible(&user).await?;
        let still_active = self
            .services
            .identity
            .oauth
            .touch_api_key_last_used(&record.id, Utc::now())
            .await?;
        if !still_active {
            return Err(AppError::Unauthorized(
                "API key is expired or revoked".into(),
            ));
        }
        Ok(ApiKeyAuthentication {
            user,
            key_id: record.id,
            key_label: record.label,
        })
    }

    pub async fn sync_development_api_keys(
        &self,
        seeds: Vec<DevelopmentApiKeySeed>,
    ) -> AppResult<()> {
        let now = Utc::now();
        let mut declared_lookup_ids = HashSet::with_capacity(seeds.len());
        let mut records = Vec::with_capacity(seeds.len());

        for seed in seeds {
            let (lookup_id, secret) = parse_api_key(&seed.raw_key).ok_or_else(|| {
                AppError::Validation("invalid development API key declaration".into())
            })?;
            if !declared_lookup_ids.insert(lookup_id.clone()) {
                return Err(AppError::Validation(
                    "development API key declarations must have unique lookup IDs".into(),
                ));
            }
            let label = validate_api_key_label(seed.label)?;
            let user = self
                .services
                .identity
                .users
                .get_by_username(&seed.username)
                .await?
                .ok_or_else(|| {
                    AppError::Validation("development API key owner does not exist".into())
                })?;
            let secret_hash = self.api_key_hash(&lookup_id, &secret);
            let record = match self
                .services
                .identity
                .oauth
                .get_api_key_by_lookup_id(&lookup_id)
                .await?
            {
                Some(existing)
                    if existing.provisioning_source != ApiKeyProvisioningSource::Environment =>
                {
                    return Err(AppError::Validation(
                        "development API key lookup ID collides with a user-managed key".into(),
                    ));
                }
                Some(existing) if existing.secret_hash != secret_hash => {
                    return Err(AppError::Validation(
                        "development API key lookup ID has a different secret".into(),
                    ));
                }
                Some(existing) => ApiKeyRecord {
                    id: existing.id,
                    user_id: user.id,
                    lookup_id,
                    secret_hash,
                    label,
                    expires_at: seed.expires_at,
                    revoked_at: None,
                    last_used_at: existing.last_used_at,
                    created_at: existing.created_at,
                    provisioning_source: ApiKeyProvisioningSource::Environment,
                },
                None => ApiKeyRecord {
                    id: Id::new().0,
                    user_id: user.id,
                    lookup_id,
                    secret_hash,
                    label,
                    expires_at: seed.expires_at,
                    revoked_at: None,
                    last_used_at: None,
                    created_at: now,
                    provisioning_source: ApiKeyProvisioningSource::Environment,
                },
            };
            records.push(record);
        }

        let existing = self
            .services
            .identity
            .oauth
            .list_environment_api_keys()
            .await?;
        for record in &records {
            self.services
                .identity
                .oauth
                .upsert_environment_api_key(record.clone())
                .await?;
        }
        for record in existing {
            if !declared_lookup_ids.contains(&record.lookup_id) {
                self.services
                    .identity
                    .oauth
                    .revoke_api_key(&record.id, &record.user_id, now)
                    .await?;
            }
        }
        Ok(())
    }

    pub async fn ensure_api_key_owner_is_eligible(&self, actor: &User) -> AppResult<()> {
        if self
            .security_settings()
            .await?
            .api_keys_restrict_to_system_settings_users
            && !actor
                .authorization
                .app
                .contains(AppPermissionMask::MANAGE_SYSTEM_SETTINGS)
        {
            return Err(AppError::Unauthorized(
                "API keys are restricted to users who manage system settings".into(),
            ));
        }
        Ok(())
    }

    pub async fn can_create_api_key(&self, actor: &User) -> AppResult<bool> {
        self.require_actor_capability(actor, ActorCapability::ManageOwnAccount)
            .await?;
        let settings = self.security_settings().await?;
        Ok(!settings.api_keys_restrict_to_system_settings_users
            || actor
                .authorization
                .app
                .contains(AppPermissionMask::MANAGE_SYSTEM_SETTINGS))
    }

    fn api_key_hash(&self, lookup_id: &str, secret: &str) -> String {
        let key = hmac::Key::new(hmac::HMAC_SHA256, self.auth.jwt_signing_salt.as_bytes());
        URL_SAFE_NO_PAD
            .encode(hmac::sign(&key, format!("api-key:{lookup_id}:{secret}").as_bytes()).as_ref())
    }
}

fn api_key_summary(record: ApiKeyRecord) -> AppResult<ApiKeySummary> {
    Ok(ApiKeySummary {
        id: record.id,
        label: record.label,
        expires_at: record.expires_at,
        revoked_at: record.revoked_at,
        last_used_at: record.last_used_at,
        created_at: record.created_at,
        provisioning_source: record.provisioning_source,
    })
}

pub fn parse_api_key(raw_key: &str) -> Option<(String, String)> {
    let value = raw_key.strip_prefix(&format!("{API_KEY_PREFIX}_"))?;
    let (lookup_id, secret) = value.split_once('.')?;
    if lookup_id.is_empty() || secret.is_empty() || secret.contains('.') {
        return None;
    }
    let lookup_bytes = URL_SAFE_NO_PAD.decode(lookup_id).ok()?;
    let secret_bytes = URL_SAFE_NO_PAD.decode(secret).ok()?;
    (lookup_bytes.len() == API_KEY_LOOKUP_BYTES && secret_bytes.len() == API_KEY_SECRET_BYTES)
        .then(|| (lookup_id.to_string(), secret.to_string()))
}

fn generate_api_key_material() -> AppResult<(String, String)> {
    let random = SystemRandom::new();
    let mut lookup = [0_u8; API_KEY_LOOKUP_BYTES];
    let mut secret = [0_u8; API_KEY_SECRET_BYTES];
    random
        .fill(&mut lookup)
        .map_err(|_| AppError::Repository("failed to generate API key lookup ID".into()))?;
    random
        .fill(&mut secret)
        .map_err(|_| AppError::Repository("failed to generate API key secret".into()))?;
    Ok((
        URL_SAFE_NO_PAD.encode(lookup),
        URL_SAFE_NO_PAD.encode(secret),
    ))
}

fn validate_api_key_label(label: String) -> AppResult<String> {
    let label = label.trim().to_string();
    if label.is_empty() {
        return Err(AppError::Validation("API key label is required".into()));
    }
    if label.len() > API_KEY_LABEL_MAX_LENGTH {
        return Err(AppError::Validation(format!(
            "API key label must not exceed {API_KEY_LABEL_MAX_LENGTH} characters"
        )));
    }
    if label.chars().any(char::is_control) {
        return Err(AppError::Validation(
            "API key label must not contain control characters".into(),
        ));
    }
    Ok(label)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use base64::Engine as _;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;

    use super::{
        API_KEY_LOOKUP_BYTES, API_KEY_PREFIX, API_KEY_SECRET_BYTES, generate_api_key_material,
        parse_api_key, validate_api_key_label,
    };

    #[test]
    fn generated_material_is_parseable_and_unique() {
        let mut lookup_ids = HashSet::new();
        let mut secrets = HashSet::new();

        for _ in 0..128 {
            let (lookup_id, secret) = generate_api_key_material().expect("generate API key");
            assert!(lookup_ids.insert(lookup_id.clone()));
            assert!(secrets.insert(secret.clone()));
            assert_eq!(
                parse_api_key(&format!("{API_KEY_PREFIX}_{lookup_id}.{secret}")),
                Some((lookup_id, secret))
            );
        }
    }

    #[test]
    fn parser_requires_exact_lookup_and_secret_lengths() {
        let lookup = URL_SAFE_NO_PAD.encode([0_u8; API_KEY_LOOKUP_BYTES]);
        let secret = URL_SAFE_NO_PAD.encode([0_u8; API_KEY_SECRET_BYTES]);
        assert!(parse_api_key(&format!("{API_KEY_PREFIX}_{lookup}.{secret}")).is_some());
        assert!(parse_api_key(&format!("{API_KEY_PREFIX}_{lookup}.short")).is_none());
        assert!(parse_api_key("ska_invalid").is_none());
    }

    #[test]
    fn labels_cannot_inject_control_characters_into_audit_identity() {
        assert!(validate_api_key_label("agent\nkey".into()).is_err());
        assert_eq!(
            validate_api_key_label("  local agent  ".into()).unwrap(),
            "local agent"
        );
    }
}

use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use ring::hmac;

use super::*;

impl AppUseCase {
    pub fn new(
        services: AppServices,
        auth: JwtAuthConfig,
        facet_registry: Arc<FacetRegistry>,
    ) -> Self {
        Self {
            services,
            auth,
            facet_registry,
        }
    }

    pub(super) fn hash_password(&self, password: &str) -> AppResult<String> {
        if password.trim().is_empty() {
            return Err(AppError::Validation("password is required".into()));
        }

        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::default();
        let phc_string = argon2
            .hash_password(password.as_bytes(), &salt)
            .map_err(|err| AppError::Repository(format!("password hashing failed: {err}")))?
            .to_string();
        Ok(format!("v2${phc_string}"))
    }

    pub(super) fn validate_password(&self, password: &str, password_hash: &str) -> AppResult<bool> {
        if let Some(phc_string) = password_hash.strip_prefix("v2$") {
            let parsed = PasswordHash::new(phc_string)
                .map_err(|err| AppError::Validation(format!("invalid v2 password hash: {err}")))?;
            Ok(Argon2::default()
                .verify_password(password.as_bytes(), &parsed)
                .is_ok())
        } else if password_hash.starts_with("v1$") {
            self.validate_password_v1(password, password_hash)
        } else {
            Err(AppError::Validation(
                "unsupported password hash version".into(),
            ))
        }
    }

    fn validate_password_v1(&self, password: &str, password_hash: &str) -> AppResult<bool> {
        let mut parts = password_hash.splitn(3, '$');
        let _ = parts.next(); // "v1"

        let salt = parts
            .next()
            .ok_or_else(|| AppError::Validation("invalid password hash: missing salt".into()))?;
        let stored_hash = parts
            .next()
            .ok_or_else(|| AppError::Validation("invalid password hash: missing hash".into()))?;

        let candidate = sha256_hex(format!("{salt}{}", password));
        Ok(candidate == stored_hash)
    }

    /// Derive a per-user JWT signing key: HMAC-SHA256(key=salt, msg=password_hash).
    ///
    /// The salt is the registration secret baked into the binary, so an offline
    /// DB dump alone cannot forge tokens.
    fn derive_jwt_key(&self, password_hash: &str) -> Vec<u8> {
        let hmac_key = hmac::Key::new(hmac::HMAC_SHA256, self.auth.jwt_signing_salt.as_bytes());
        hmac::sign(&hmac_key, password_hash.as_bytes())
            .as_ref()
            .to_vec()
    }

    pub fn token_lifetime(&self) -> i64 {
        i64::try_from(self.auth.access_ttl_seconds).unwrap_or(86_400)
    }

    pub fn issue_access_token(&self, actor: &User) -> AppResult<String> {
        let password_hash = actor
            .password_hash
            .as_deref()
            .ok_or_else(|| AppError::Unauthorized("cannot issue token: no password hash".into()))?;

        let now = Utc::now();
        let iat = now.timestamp();
        let exp = (now + Duration::seconds(self.token_lifetime())).timestamp();

        let entitlements: Vec<String> = actor
            .entitlements
            .iter()
            .filter_map(|e| serde_json::to_value(e).ok())
            .filter_map(|v| v.as_str().map(String::from))
            .collect();

        let claims = JwtClaims {
            sub: actor.id.clone(),
            exp,
            iat,
            iss: self.auth.issuer.clone(),
            username: actor.username.clone(),
            entitlements,
        };

        let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256);
        let signing_key = self.derive_jwt_key(password_hash);
        let key = jsonwebtoken::EncodingKey::from_secret(&signing_key);

        let token = jsonwebtoken::encode(&header, &claims, &key)
            .map_err(|err| AppError::Repository(format!("failed to issue token: {err}")))?;

        Ok(token)
    }

    pub async fn authenticate_token(&self, token: &str) -> AppResult<User> {
        // Decode claims without signature verification to extract the subject (user ID).
        let mut insecure = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::HS256);
        insecure.insecure_disable_signature_validation();
        insecure.validate_exp = false;

        let unverified = jsonwebtoken::decode::<JwtClaims>(
            token,
            &jsonwebtoken::DecodingKey::from_secret(&[]),
            &insecure,
        )
        .map_err(|err| AppError::Unauthorized(format!("malformed token: {err}")))?;

        let user_id = &unverified.claims.sub;

        // Fetch the user from DB to get the current password hash.
        let user = self
            .services
            .users
            .get_by_id(user_id)
            .await?
            .ok_or_else(|| AppError::Unauthorized("unknown token subject".into()))?;

        let password_hash = user
            .password_hash
            .as_deref()
            .ok_or_else(|| AppError::Unauthorized("user has no password hash".into()))?;

        // Now verify the signature with the per-user key.
        let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::HS256);
        validation.validate_exp = true;
        validation.set_issuer(&[self.auth.issuer.as_str()]);

        let signing_key = self.derive_jwt_key(password_hash);
        let key = jsonwebtoken::DecodingKey::from_secret(&signing_key);

        jsonwebtoken::decode::<JwtClaims>(token, &key, &validation)
            .map_err(|err| AppError::Unauthorized(format!("invalid token: {err}")))?;

        // Return the DB user — always has fresh entitlements/username.
        Ok(user)
    }

    pub async fn authenticate_credentials(
        &self,
        username: &str,
        password: &str,
    ) -> AppResult<User> {
        let username = username.trim();
        if username.is_empty() {
            return Err(AppError::Validation("username is required".into()));
        }
        let password = password.trim();
        if password.is_empty() {
            return Err(AppError::Validation("password is required".into()));
        }

        let user = self
            .services
            .users
            .get_by_username(username)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("user {username} not found")))?;

        let password_hash = user
            .password_hash
            .as_ref()
            .ok_or_else(|| AppError::Unauthorized("credentials unavailable".into()))?;

        if !self.validate_password(password, password_hash)? {
            return Err(AppError::Unauthorized("invalid credentials".into()));
        }

        // Online migration: re-hash v1 passwords with Argon2id on successful login.
        // Must return the updated user so the caller's JWT signing key matches the DB.
        if password_hash.starts_with("v1$")
            && let Ok(new_hash) = self.hash_password(password)
        {
            match self
                .services
                .users
                .update_password_hash(&user.id, new_hash)
                .await
            {
                Ok(updated) => {
                    tracing::info!(user_id = %user.id, "migrated password hash from v1 to v2");
                    return Ok(updated);
                }
                Err(err) => {
                    tracing::warn!(user_id = %user.id, error = %err, "failed to migrate password hash from v1 to v2");
                }
            }
        }

        Ok(user)
    }
}

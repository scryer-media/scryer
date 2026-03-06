use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};

use super::*;

impl AppUseCase {
    pub fn new(services: AppServices, auth: JwtAuthConfig, facet_registry: Arc<FacetRegistry>) -> Self {
        Self { services, auth, facet_registry }
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

    pub fn token_lifetime(&self) -> i64 {
        i64::try_from(self.auth.access_ttl_seconds).unwrap_or(86_400)
    }

    pub fn issue_access_token(&self, actor: &User) -> AppResult<String> {
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

        let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS512);
        let key = jsonwebtoken::EncodingKey::from_secret(self.auth.jwt_hmac_secret.as_bytes());

        let token = jsonwebtoken::encode(&header, &claims, &key)
            .map_err(|err| AppError::Repository(format!("failed to issue token: {err}")))?;

        Ok(token)
    }

    pub async fn authenticate_token(&self, token: &str) -> AppResult<User> {
        let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::HS512);
        validation.validate_exp = true;
        validation.set_issuer(&[self.auth.issuer.as_str()]);

        let key = jsonwebtoken::DecodingKey::from_secret(self.auth.jwt_hmac_secret.as_bytes());

        let token_data = jsonwebtoken::decode::<JwtClaims>(token, &key, &validation)
            .map_err(|err| AppError::Unauthorized(format!("invalid token: {err}")))?;

        let claims = token_data.claims;

        // Old tokens lack embedded entitlements/username — fall back to DB lookup
        if claims.entitlements.is_empty() || claims.username.is_empty() {
            let user = self
                .services
                .users
                .get_by_id(&claims.sub)
                .await?;
            return user.ok_or_else(|| AppError::Unauthorized("unknown token subject".into()));
        }

        let entitlements: Vec<Entitlement> = claims
            .entitlements
            .iter()
            .filter_map(|s| serde_json::from_value(serde_json::Value::String(s.clone())).ok())
            .collect();

        Ok(User {
            id: claims.sub,
            username: claims.username,
            password_hash: None,
            entitlements,
        })
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

        // Online migration: re-hash v1 passwords with Argon2id on successful login
        if password_hash.starts_with("v1$") {
            if let Ok(new_hash) = self.hash_password(password) {
                if let Err(err) = self
                    .services
                    .users
                    .update_password_hash(&user.id, new_hash)
                    .await
                {
                    tracing::warn!(user_id = %user.id, error = %err, "failed to migrate password hash from v1 to v2");
                } else {
                    tracing::info!(user_id = %user.id, "migrated password hash from v1 to v2");
                }
            }
        }

        Ok(user)
    }
}

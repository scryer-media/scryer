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
            jwt_signing_keys: Arc::new(RwLock::new(HashMap::new())),
            jwt_signing_keys_loaded: Arc::new(OnceCell::new()),
            jwt_signing_keys_seed_lock: Arc::new(Mutex::new(())),
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

    pub(crate) fn entitlement_claim_string(entitlement: &Entitlement) -> &'static str {
        match entitlement {
            Entitlement::ViewCatalog => "view_catalog",
            Entitlement::MonitorTitle => "monitor_title",
            Entitlement::ManageTitle => "manage_title",
            Entitlement::TriggerActions => "trigger_actions",
            Entitlement::ManageConfig => "manage_config",
            Entitlement::ViewHistory => "view_history",
        }
    }

    fn parse_entitlement_claim(raw: &str) -> Option<Entitlement> {
        match raw.trim().to_lowercase().replace('-', "_").as_str() {
            "viewcatalog" | "view_catalog" => Some(Entitlement::ViewCatalog),
            "monitortitle" | "monitor_title" => Some(Entitlement::MonitorTitle),
            "managetitle" | "manage_title" => Some(Entitlement::ManageTitle),
            "triggeractions" | "trigger_actions" => Some(Entitlement::TriggerActions),
            "manageconfig" | "manage_config" => Some(Entitlement::ManageConfig),
            "viewhistory" | "view_history" => Some(Entitlement::ViewHistory),
            _ => None,
        }
    }

    fn canonical_entitlement_claims(entitlements: &[Entitlement]) -> Vec<String> {
        let mut claims = entitlements
            .iter()
            .map(|entitlement| Self::entitlement_claim_string(entitlement).to_string())
            .collect::<Vec<_>>();
        claims.sort();
        claims.dedup();
        claims
    }

    fn parse_entitlement_claims(&self, raw_claims: &[String]) -> AppResult<Vec<Entitlement>> {
        let mut entitlements = Vec::with_capacity(raw_claims.len());
        let mut seen = std::collections::HashSet::new();

        for raw in raw_claims {
            let entitlement = Self::parse_entitlement_claim(raw)
                .ok_or_else(|| AppError::Validation(format!("unknown entitlement: {raw}")))?;
            if seen.insert(entitlement.clone()) {
                entitlements.push(entitlement);
            }
        }

        entitlements.sort_by_key(Self::entitlement_claim_string);
        Ok(entitlements)
    }

    /// Derive a per-user JWT signing key:
    /// HMAC-SHA256(key=salt, msg="{password_hash}\n{entitlements_fingerprint}").
    ///
    /// The salt is the registration secret baked into the binary, so an offline
    /// DB dump alone cannot forge tokens.
    pub(crate) fn derive_jwt_key(
        &self,
        password_hash: &str,
        entitlements: &[Entitlement],
    ) -> Vec<u8> {
        let entitlement_claims = Self::canonical_entitlement_claims(entitlements);
        let entitlement_fingerprint = sha256_hex(entitlement_claims.join("\n"));
        let signing_material = format!("{password_hash}\n{entitlement_fingerprint}");
        let hmac_key = hmac::Key::new(hmac::HMAC_SHA256, self.auth.jwt_signing_salt.as_bytes());
        hmac::sign(&hmac_key, signing_material.as_bytes())
            .as_ref()
            .to_vec()
    }

    fn derive_jwt_key_for_user(&self, user: &User) -> AppResult<Option<Vec<u8>>> {
        let Some(password_hash) = user.password_hash.as_deref() else {
            return Ok(None);
        };

        Ok(Some(self.derive_jwt_key(password_hash, &user.entitlements)))
    }

    async fn write_cached_jwt_signing_key(&self, user: &User, evict_first: bool) -> AppResult<()> {
        let _seed_guard = self.jwt_signing_keys_seed_lock.lock().await;
        let mut cache = self.jwt_signing_keys.write().await;

        if evict_first {
            cache.remove(&user.id);
        }

        match self.derive_jwt_key_for_user(user)? {
            Some(signing_key) => {
                cache.insert(user.id.clone(), signing_key);
            }
            None => {
                cache.remove(&user.id);
            }
        }

        Ok(())
    }

    pub(super) async fn cache_jwt_signing_key(&self, user: &User) -> AppResult<()> {
        self.write_cached_jwt_signing_key(user, false).await
    }

    pub(super) async fn refresh_cached_jwt_signing_key(&self, user: &User) -> AppResult<()> {
        self.write_cached_jwt_signing_key(user, true).await
    }

    pub(super) async fn evict_cached_jwt_signing_key(&self, user_id: &str) {
        let _seed_guard = self.jwt_signing_keys_seed_lock.lock().await;
        self.jwt_signing_keys.write().await.remove(user_id);
    }

    pub(crate) async fn ensure_jwt_signing_keys_loaded(&self) -> AppResult<()> {
        if self.jwt_signing_keys_loaded.get().is_some() {
            return Ok(());
        }

        let _seed_guard = self.jwt_signing_keys_seed_lock.lock().await;
        if self.jwt_signing_keys_loaded.get().is_some() {
            return Ok(());
        }

        let users = self.services.users.list_all().await?;
        let mut cache = self.jwt_signing_keys.write().await;
        cache.clear();
        for user in users {
            if let Some(signing_key) = self.derive_jwt_key_for_user(&user)? {
                cache.insert(user.id, signing_key);
            }
        }
        let _ = self.jwt_signing_keys_loaded.set(());
        Ok(())
    }

    pub fn token_lifetime(&self) -> i64 {
        self.auth.access_ttl_seconds as i64
    }

    pub fn issue_access_token(&self, actor: &User) -> AppResult<String> {
        let password_hash = actor
            .password_hash
            .as_deref()
            .ok_or_else(|| AppError::Unauthorized("cannot issue token: no password hash".into()))?;

        let now = Utc::now();
        let iat = now.timestamp();
        let exp = (now + Duration::seconds(self.token_lifetime())).timestamp();

        let entitlements = Self::canonical_entitlement_claims(&actor.entitlements);

        let claims = JwtClaims {
            sub: actor.id.clone(),
            exp,
            iat,
            iss: self.auth.issuer.clone(),
            username: actor.username.clone(),
            entitlements,
        };

        let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256);
        let signing_key = self.derive_jwt_key(password_hash, &actor.entitlements);
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
        self.ensure_jwt_signing_keys_loaded().await?;

        // Now verify the signature with the per-user key.
        let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::HS256);
        validation.validate_exp = true;
        validation.set_issuer(&[self.auth.issuer.as_str()]);

        let signing_key = self
            .jwt_signing_keys
            .read()
            .await
            .get(user_id)
            .cloned()
            .ok_or_else(|| AppError::Unauthorized("unknown token subject".into()))?;
        let key = jsonwebtoken::DecodingKey::from_secret(&signing_key);

        let verified = jsonwebtoken::decode::<JwtClaims>(token, &key, &validation)
            .map_err(|err| AppError::Unauthorized(format!("invalid token: {err}")))?;
        let claims = verified.claims;
        let entitlements = self
            .parse_entitlement_claims(&claims.entitlements)
            .map_err(|err| AppError::Unauthorized(format!("invalid token claims: {err}")))?;

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
                    self.cache_jwt_signing_key(&updated).await?;
                    tracing::info!(user_id = %user.id, "migrated password hash from v1 to v2");
                    return Ok(updated);
                }
                Err(err) => {
                    tracing::warn!(user_id = %user.id, error = %err, "failed to migrate password hash from v1 to v2");
                }
            }
        }

        self.cache_jwt_signing_key(&user).await?;
        Ok(user)
    }
}

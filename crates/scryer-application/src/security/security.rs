use std::time::{Duration as StdDuration, Instant};

use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use aws_lc_rs::hmac;
use aws_lc_rs::rand::{SecureRandom, SystemRandom};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

use super::*;
use crate::services::{AppAssembly, ReleaseCandidatePasswordTicket, RuntimeFeature};
use crate::types::{
    AuthenticatedTokenClaims, BackupDownloadTicket, BackupDownloadTokenClaims,
    JwtLibraryPermissionClaim, JwtSessionScope, LoginFailureTimingClass, OAuthAuthorizationSource,
    ReleaseCandidateTokenClaims,
};

const DUMMY_LOGIN_PASSWORD_HASH: &str = "v2$$argon2id$v=19$m=19456,t=2,p=1$zyGbHzPhFQTT8+t6oz3ZNw$CtJ2dcsWSe1CCV4O30Gm9zPD/03F7MfEIMDvBvjc/ig";

struct AccessTokenOptions {
    mfa_verified_until: Option<chrono::DateTime<Utc>>,
    mfa_step_up_verified_until: Option<chrono::DateTime<Utc>>,
    security_action_verified_until: Option<chrono::DateTime<Utc>>,
    auth_scope: JwtSessionScope,
    ttl_seconds: i64,
    persist_session: bool,
    password_change_required_after_enrollment: bool,
    oauth: Option<(String, String, OAuthAuthorizationSource)>,
}

impl AppUseCase {
    const BACKUP_DOWNLOAD_TOKEN_KIND: &'static str = "backup_download_v1";
    const BACKUP_DOWNLOAD_TOKEN_TTL_SECONDS: i64 = 5 * 60;
    const MFA_ENROLLMENT_TOKEN_TTL_SECONDS: i64 = 10 * 60;
    pub(crate) const OAUTH_ACCESS_TOKEN_TTL_SECONDS: i64 = 15 * 60;
    const RELEASE_CANDIDATE_TOKEN_KIND: &'static str = "release_candidate_v1";
    const RELEASE_CANDIDATE_TOKEN_TTL_SECONDS: i64 = 15 * 60;

    pub(crate) fn app_permission_claim_string(
        permission: scryer_domain::AppPermission,
    ) -> &'static str {
        match permission {
            scryer_domain::AppPermission::ManageUsers => "manageUsers",
            scryer_domain::AppPermission::ManagePermissions => "managePermissions",
            scryer_domain::AppPermission::ManageSystemSettings => "manageSystemSettings",
            scryer_domain::AppPermission::ManageCatalogSettings => "manageCatalogSettings",
        }
    }

    pub(crate) fn actor_capability_claim_string(
        capability: scryer_domain::ActorCapability,
    ) -> &'static str {
        match capability {
            scryer_domain::ActorCapability::ManageOwnAccount => "manageOwnAccount",
        }
    }

    pub(crate) fn library_permission_claim_string(
        permission: scryer_domain::LibraryPermission,
    ) -> &'static str {
        match permission {
            scryer_domain::LibraryPermission::View => "view",
            scryer_domain::LibraryPermission::ManageTitles => "manageTitles",
            scryer_domain::LibraryPermission::ResolveImports => "resolveImports",
            scryer_domain::LibraryPermission::ManageLibrary => "manageLibrary",
            scryer_domain::LibraryPermission::Request => "request",
            scryer_domain::LibraryPermission::AutoApproveRequests => "autoApproveRequests",
        }
    }

    pub fn new(
        assembly: AppAssembly,
        auth: JwtAuthConfig,
        facet_registry: Arc<FacetRegistry>,
    ) -> Self {
        Self::new_with_webauthn(assembly, auth, facet_registry, None)
    }

    pub fn new_with_webauthn(
        assembly: AppAssembly,
        auth: JwtAuthConfig,
        facet_registry: Arc<FacetRegistry>,
        webauthn: Option<Arc<webauthn_rs::Webauthn>>,
    ) -> Self {
        Self {
            services: assembly.services,
            runtime: assembly.runtime,
            auth,
            facet_registry,
            pending_import_resolution_locks: Arc::new(std::sync::Mutex::new(HashSet::new())),
            jwt_signing_keys: Arc::new(RwLock::new(HashMap::new())),
            jwt_signing_keys_loaded: Arc::new(OnceCell::new()),
            jwt_signing_keys_seed_lock: Arc::new(Mutex::new(())),
            webauthn: webauthn.map(RuntimeFeature::enabled).unwrap_or_default(),
        }
    }

    pub(crate) fn hash_password(&self, password: &str) -> AppResult<String> {
        if password.is_empty() {
            return Err(AppError::Validation("password is required".into()));
        }

        let argon2 = Argon2::default();
        let phc_string = argon2
            .hash_password(password.as_bytes())
            .map_err(|err| AppError::Repository(format!("password hashing failed: {err}")))?
            .to_string();
        Ok(format!("v2${phc_string}"))
    }

    pub(crate) fn normalize_local_username(username: &str) -> &str {
        username.trim()
    }

    fn dummy_login_password_hash() -> &'static str {
        DUMMY_LOGIN_PASSWORD_HASH
    }

    fn verify_dummy_login_password(&self, password: &str) {
        let _ = self.validate_password(password, Self::dummy_login_password_hash());
    }

    fn login_failure_delay_range_ms(class: LoginFailureTimingClass) -> (u64, u64) {
        match class {
            LoginFailureTimingClass::PasswordBackedLocal => (400, 700),
            LoginFailureTimingClass::FastMasked => (400, 700),
        }
    }

    pub fn login_failure_delay_target_for_random(
        class: LoginFailureTimingClass,
        random: u64,
    ) -> StdDuration {
        let (min_ms, max_ms) = Self::login_failure_delay_range_ms(class);
        let span_ms = max_ms - min_ms;
        StdDuration::from_millis(min_ms + (random % (span_ms + 1)))
    }

    pub fn login_failure_remaining_delay_for_elapsed(
        class: LoginFailureTimingClass,
        random: u64,
        elapsed: StdDuration,
    ) -> Option<StdDuration> {
        let target = Self::login_failure_delay_target_for_random(class, random);
        target
            .checked_sub(elapsed)
            .filter(|duration| !duration.is_zero())
    }

    fn login_failure_random() -> u64 {
        let rng = SystemRandom::new();
        let mut bytes = [0_u8; 8];
        if rng.fill(&mut bytes).is_err() {
            return 0;
        }
        u64::from_le_bytes(bytes)
    }

    fn release_candidate_password_ref() -> AppResult<String> {
        let rng = SystemRandom::new();
        let mut bytes = [0_u8; 32];
        rng.fill(&mut bytes).map_err(|error| {
            AppError::Repository(format!(
                "failed to generate release candidate password reference: {error}"
            ))
        })?;
        Ok(URL_SAFE_NO_PAD.encode(bytes))
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "release candidate password tickets bind several immutable claim fields"
    )]
    fn store_release_candidate_password_ticket(
        &self,
        actor: &User,
        title_id: &str,
        scope_kind: &str,
        scope_id: Option<&str>,
        source_hint: &str,
        source_title: &str,
        raw_password: Option<&str>,
        expires_at: DateTime<Utc>,
    ) -> AppResult<Option<String>> {
        let Some(password) = normalize_release_password(raw_password) else {
            return Ok(None);
        };
        let password_ref = Self::release_candidate_password_ref()?;
        let mut tickets = self
            .runtime
            .acquisition
            .release_candidate_passwords
            .lock()
            .map_err(|_| {
                AppError::Repository(
                    "release candidate password ticket store is unavailable".to_string(),
                )
            })?;
        let now = Utc::now();
        tickets.retain(|_, ticket| ticket.expires_at > now);
        tickets.insert(
            password_ref.clone(),
            ReleaseCandidatePasswordTicket {
                actor_id: actor.id.clone(),
                title_id: title_id.to_string(),
                scope_kind: scope_kind.to_string(),
                scope_id: scope_id.map(str::to_string),
                source_hint: source_hint.to_string(),
                source_title: source_title.to_string(),
                password,
                expires_at,
            },
        );
        Ok(Some(password_ref))
    }

    fn resolve_release_candidate_password_ticket(
        &self,
        actor: &User,
        title_id: &str,
        claims: &ReleaseCandidateTokenClaims,
        claimed_scope: &SubmissionScope,
    ) -> AppResult<Option<String>> {
        let Some(password_ref) = claims
            .password_ref
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return Ok(None);
        };
        let (scope_kind, scope_id) = Self::submission_scope_claims(claimed_scope);
        let mut tickets = self
            .runtime
            .acquisition
            .release_candidate_passwords
            .lock()
            .map_err(|_| {
                AppError::Repository(
                    "release candidate password ticket store is unavailable".to_string(),
                )
            })?;
        let now = Utc::now();
        tickets.retain(|_, ticket| ticket.expires_at > now);
        let Some(ticket) = tickets.get(password_ref) else {
            return Err(AppError::Unauthorized(
                "release candidate expired; search again".to_string(),
            ));
        };

        if ticket.actor_id != actor.id
            || ticket.title_id != title_id
            || ticket.scope_kind != scope_kind
            || ticket.scope_id != scope_id
            || ticket.source_hint != claims.source_hint
            || ticket.source_title != claims.source_title
        {
            return Err(AppError::Unauthorized(
                "release candidate expired; search again".to_string(),
            ));
        }

        Ok(Some(ticket.password.clone()))
    }

    pub async fn apply_login_failure_timing(class: LoginFailureTimingClass, started_at: Instant) {
        let random = Self::login_failure_random();
        if let Some(remaining) =
            Self::login_failure_remaining_delay_for_elapsed(class, random, started_at.elapsed())
        {
            tokio::time::sleep(remaining).await;
        }
    }

    pub(crate) async fn password_min_length(&self) -> AppResult<i32> {
        Ok(self
            .read_setting_i64_value(PASSWORD_MIN_LENGTH_KEY, None)
            .await?
            .unwrap_or(PASSWORD_MIN_LENGTH_MIN)
            .max(PASSWORD_MIN_LENGTH_MIN) as i32)
    }

    pub(crate) async fn validate_new_local_password(&self, password: &str) -> AppResult<()> {
        let min_length = self.password_min_length().await?;
        if password.chars().count() < min_length as usize {
            return Err(AppError::Validation(format!(
                "password must be at least {min_length} characters"
            )));
        }

        Ok(())
    }

    pub async fn existing_default_admin_uses_bootstrap_password(&self) -> AppResult<bool> {
        let Some(admin) = self.find_default_user().await? else {
            return Ok(false);
        };
        if !admin.login_status().is_enabled() {
            return Ok(false);
        }
        let Some(password_hash) = admin.password_hash.as_deref() else {
            return Ok(false);
        };

        self.validate_password("admin", password_hash)
    }

    pub(crate) fn validate_password(&self, password: &str, password_hash: &str) -> AppResult<bool> {
        self.validate_password_hash(password_hash)?;

        if let Some(phc_string) = password_hash.strip_prefix("v2$") {
            let parsed = PasswordHash::new(phc_string)
                .map_err(|err| AppError::Validation(format!("invalid v2 password hash: {err}")))?;
            Ok(Argon2::default()
                .verify_password(password.as_bytes(), &parsed)
                .is_ok())
        } else {
            Err(AppError::Validation(
                "unsupported password hash version".into(),
            ))
        }
    }

    /// Only Argon2id (`v2$`) hashes are accepted.
    ///
    /// The legacy `v1$<salt>$<sha256(salt+password)>` form is retired. Migration
    /// 0191 clears any surviving `v1$` row and flags it for password reset, so a
    /// stranded account fails closed with an explicit reason instead of being
    /// silently unable to authenticate.
    pub(crate) fn validate_password_hash(&self, password_hash: &str) -> AppResult<()> {
        if let Some(phc_string) = password_hash.strip_prefix("v2$") {
            PasswordHash::new(phc_string)
                .map(|_| ())
                .map_err(|err| AppError::Validation(format!("invalid v2 password hash: {err}")))
        } else {
            Err(AppError::Validation(
                "unsupported password hash version".into(),
            ))
        }
    }

    fn canonical_app_permission_claims(user: &User) -> Vec<String> {
        let mut claims = user
            .authorization
            .app
            .to_permissions()
            .into_iter()
            .map(Self::app_permission_claim_string)
            .map(str::to_string)
            .collect::<Vec<_>>();
        claims.sort();
        claims.dedup();
        claims
    }

    fn canonical_actor_capability_claims(user: &User) -> Vec<String> {
        let mut claims = user
            .authorization
            .actor_capabilities
            .to_capabilities()
            .into_iter()
            .map(Self::actor_capability_claim_string)
            .map(str::to_string)
            .collect::<Vec<_>>();
        claims.sort();
        claims.dedup();
        claims
    }

    fn actor_capability_claims_to_mask(
        claims: &[String],
    ) -> AppResult<scryer_domain::ActorCapabilityMask> {
        let mut mask = scryer_domain::ActorCapabilityMask::NONE;
        for claim in claims {
            let capability = match claim.as_str() {
                "manageOwnAccount" => scryer_domain::ActorCapability::ManageOwnAccount,
                value => scryer_domain::ActorCapability::parse(value).ok_or_else(|| {
                    AppError::Unauthorized(format!("unknown actor capability claim: {value}"))
                })?,
            };
            mask.insert(scryer_domain::ActorCapabilityMask::from_capability(
                capability,
            ));
        }
        Ok(mask)
    }

    fn canonical_library_permission_claims(user: &User) -> Vec<JwtLibraryPermissionClaim> {
        let mut claims = user
            .authorization
            .libraries
            .iter()
            .map(|(library_id, permissions)| {
                let mut permissions = permissions
                    .to_permissions()
                    .into_iter()
                    .map(Self::library_permission_claim_string)
                    .map(str::to_string)
                    .collect::<Vec<_>>();
                permissions.sort();
                permissions.dedup();
                JwtLibraryPermissionClaim {
                    library_id: library_id.clone(),
                    permissions,
                }
            })
            .collect::<Vec<_>>();
        claims.sort_by(|left, right| left.library_id.cmp(&right.library_id));
        claims
    }

    fn authorization_fingerprint(user: &User) -> String {
        let app_claims = Self::canonical_app_permission_claims(user).join("\n");
        let library_claims = Self::canonical_library_permission_claims(user)
            .into_iter()
            .map(|grant| format!("{}:{}", grant.library_id, grant.permissions.join(",")))
            .collect::<Vec<_>>()
            .join("\n");
        crate::helpers::blake3_identity_hex(
            crate::helpers::HashDomain::AuthorizationFingerprint,
            format!("app\n{app_claims}\nlibrary\n{library_claims}"),
        )
    }

    async fn auth_session_fingerprint(
        &self,
        user_id: &str,
        authorization_fingerprint: String,
    ) -> AppResult<String> {
        let auth_session_version = self
            .services
            .identity
            .users
            .auth_session_version(user_id)
            .await?;
        Ok(Self::auth_session_fingerprint_for_version(
            authorization_fingerprint,
            auth_session_version.as_deref(),
        ))
    }

    fn auth_session_fingerprint_for_version(
        authorization_fingerprint: String,
        auth_session_version: Option<&str>,
    ) -> String {
        match auth_session_version {
            Some(auth_session_version) => {
                format!("{authorization_fingerprint}\nauth_session:{auth_session_version}")
            }
            None => authorization_fingerprint,
        }
    }

    /// Derive a per-user JWT signing key:
    /// HMAC-SHA256(key=salt, msg="{password_hash}\n{authorization_and_session_fingerprint}").
    ///
    /// The salt is the registration secret baked into the binary, so an offline
    /// DB dump alone cannot forge tokens.
    pub(crate) fn derive_jwt_key(
        &self,
        password_hash: &str,
        authorization_fingerprint: &str,
    ) -> Vec<u8> {
        let signing_material = format!("{password_hash}\n{authorization_fingerprint}");
        let hmac_key = hmac::Key::new(hmac::HMAC_SHA256, self.auth.jwt_signing_salt.as_bytes());
        hmac::sign(&hmac_key, signing_material.as_bytes())
            .as_ref()
            .to_vec()
    }

    async fn user_with_authorization(&self, user: &User) -> AppResult<User> {
        if user.authorization.loaded {
            return Ok(user.clone());
        }
        let mut user = user.clone();
        let login_status = user.login_status();
        user.authorization = self.load_user_authorization(&user).await?;
        user.set_login_status(login_status);
        Ok(user)
    }

    pub async fn load_user_for_auth_payload(&self, user: &User) -> AppResult<User> {
        let mut user = self
            .services
            .identity
            .users
            .get_by_id(&user.id)
            .await?
            .ok_or_else(|| AppError::Unauthorized("token subject no longer exists".into()))?;
        let login_status = user.login_status();
        user.authorization = self.load_user_authorization(&user).await?;
        user.set_login_status(login_status);
        Ok(user)
    }

    async fn derive_jwt_key_for_user(&self, user: &User) -> AppResult<Option<Vec<u8>>> {
        let user = self.user_with_authorization(user).await?;
        let signing_seed = user
            .password_hash
            .clone()
            .unwrap_or_else(|| format!("federated:{}", user.id));

        let authorization_fingerprint = self
            .auth_session_fingerprint(&user.id, Self::authorization_fingerprint(&user))
            .await?;
        Ok(Some(
            self.derive_jwt_key(&signing_seed, &authorization_fingerprint),
        ))
    }

    async fn write_cached_jwt_signing_key(&self, user: &User, evict_first: bool) -> AppResult<()> {
        let _seed_guard = self.jwt_signing_keys_seed_lock.lock().await;
        let mut cache = self.jwt_signing_keys.write().await;

        if evict_first {
            cache.remove(&user.id);
        }

        match self.derive_jwt_key_for_user(user).await? {
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

        let users = self.services.identity.users.list_all().await?;
        let mut cache = self.jwt_signing_keys.write().await;
        cache.clear();
        for user in users {
            if let Some(signing_key) = self.derive_jwt_key_for_user(&user).await? {
                cache.insert(user.id, signing_key);
            }
        }
        let _ = self.jwt_signing_keys_loaded.set(());
        Ok(())
    }

    pub fn token_lifetime(&self) -> i64 {
        self.auth.access_ttl_seconds as i64
    }

    pub fn mfa_enrollment_token_lifetime(&self) -> i64 {
        Self::MFA_ENROLLMENT_TOKEN_TTL_SECONDS
    }

    pub fn mfa_freshness_verified_until(&self) -> chrono::DateTime<Utc> {
        Utc::now() + Duration::minutes(super::totp::MFA_FRESHNESS_TTL_MINUTES)
    }

    /// Freshness window required for self-service authentication-factor changes.
    pub fn security_action_verified_until(&self) -> chrono::DateTime<Utc> {
        Utc::now() + Duration::minutes(5)
    }

    pub async fn issue_access_token(&self, actor: &User) -> AppResult<String> {
        self.issue_access_token_with_mfa(actor, None, None).await
    }

    pub async fn issue_access_token_with_mfa(
        &self,
        actor: &User,
        mfa_verified_until: Option<chrono::DateTime<Utc>>,
        mfa_step_up_verified_until: Option<chrono::DateTime<Utc>>,
    ) -> AppResult<String> {
        self.issue_access_token_with_mfa_and_scope(
            actor,
            mfa_verified_until,
            mfa_step_up_verified_until,
            JwtSessionScope::Full,
            self.token_lifetime(),
            false,
        )
        .await
    }

    pub async fn issue_access_token_with_mfa_and_persistence(
        &self,
        actor: &User,
        mfa_verified_until: Option<chrono::DateTime<Utc>>,
        mfa_step_up_verified_until: Option<chrono::DateTime<Utc>>,
        persist_session: bool,
    ) -> AppResult<String> {
        self.issue_access_token_with_mfa_and_scope(
            actor,
            mfa_verified_until,
            mfa_step_up_verified_until,
            JwtSessionScope::Full,
            self.token_lifetime(),
            persist_session,
        )
        .await
    }

    /// Issues a full session only while the version observed during factor
    /// verification remains current. If it changes immediately afterwards,
    /// this token is deliberately signed with the old version and is rejected.
    pub async fn issue_access_token_with_mfa_and_persistence_at_auth_session_version(
        &self,
        actor: &User,
        mfa_verified_until: Option<chrono::DateTime<Utc>>,
        mfa_step_up_verified_until: Option<chrono::DateTime<Utc>>,
        persist_session: bool,
        expected_auth_session_version: &Option<String>,
    ) -> AppResult<String> {
        self.issue_access_token_with_mfa_scope_and_oauth(
            actor,
            AccessTokenOptions {
                mfa_verified_until,
                mfa_step_up_verified_until,
                security_action_verified_until: Some(self.security_action_verified_until()),
                auth_scope: JwtSessionScope::Full,
                ttl_seconds: self.token_lifetime(),
                persist_session,
                password_change_required_after_enrollment: false,
                oauth: None,
            },
            Some(expected_auth_session_version),
        )
        .await
    }

    pub async fn issue_mfa_enrollment_token(
        &self,
        actor: &User,
        persist_session: bool,
        password_change_required_after_enrollment: bool,
        expected_auth_session_version: Option<&Option<String>>,
    ) -> AppResult<String> {
        self.issue_access_token_with_mfa_scope_and_oauth(
            actor,
            AccessTokenOptions {
                mfa_verified_until: None,
                mfa_step_up_verified_until: None,
                security_action_verified_until: None,
                auth_scope: JwtSessionScope::MfaEnrollment,
                ttl_seconds: self.mfa_enrollment_token_lifetime(),
                persist_session,
                password_change_required_after_enrollment,
                oauth: None,
            },
            expected_auth_session_version,
        )
        .await
    }

    pub async fn issue_password_change_required_token(
        &self,
        actor: &User,
        mfa_verified_until: Option<chrono::DateTime<Utc>>,
        persist_session: bool,
        expected_auth_session_version: Option<&Option<String>>,
    ) -> AppResult<String> {
        self.issue_access_token_with_mfa_scope_and_oauth(
            actor,
            AccessTokenOptions {
                mfa_verified_until,
                mfa_step_up_verified_until: None,
                security_action_verified_until: None,
                auth_scope: JwtSessionScope::PasswordChangeRequired,
                ttl_seconds: self.mfa_enrollment_token_lifetime(),
                persist_session,
                password_change_required_after_enrollment: false,
                oauth: None,
            },
            expected_auth_session_version,
        )
        .await
    }

    pub async fn issue_oauth_access_token(
        &self,
        actor: &User,
        client_id: &str,
        grant_id: &str,
    ) -> AppResult<String> {
        self.issue_oauth_access_token_with_source(
            actor,
            client_id,
            grant_id,
            OAuthAuthorizationSource::Authenticated,
        )
        .await
    }

    pub async fn issue_oauth_access_token_with_source(
        &self,
        actor: &User,
        client_id: &str,
        grant_id: &str,
        authorization_source: OAuthAuthorizationSource,
    ) -> AppResult<String> {
        self.issue_access_token_with_mfa_scope_and_oauth(
            actor,
            AccessTokenOptions {
                mfa_verified_until: None,
                mfa_step_up_verified_until: None,
                security_action_verified_until: None,
                auth_scope: JwtSessionScope::Full,
                ttl_seconds: Self::OAUTH_ACCESS_TOKEN_TTL_SECONDS,
                persist_session: false,
                password_change_required_after_enrollment: false,
                oauth: Some((
                    client_id.to_string(),
                    grant_id.to_string(),
                    authorization_source,
                )),
            },
            None,
        )
        .await
    }

    async fn issue_access_token_with_mfa_and_scope(
        &self,
        actor: &User,
        mfa_verified_until: Option<chrono::DateTime<Utc>>,
        mfa_step_up_verified_until: Option<chrono::DateTime<Utc>>,
        auth_scope: JwtSessionScope,
        ttl_seconds: i64,
        persist_session: bool,
    ) -> AppResult<String> {
        self.issue_access_token_with_mfa_scope_and_oauth(
            actor,
            AccessTokenOptions {
                mfa_verified_until,
                mfa_step_up_verified_until,
                security_action_verified_until: (auth_scope == JwtSessionScope::Full)
                    .then(|| self.security_action_verified_until()),
                auth_scope,
                ttl_seconds,
                persist_session,
                password_change_required_after_enrollment: false,
                oauth: None,
            },
            None,
        )
        .await
    }

    async fn issue_access_token_with_mfa_scope_and_oauth(
        &self,
        actor: &User,
        options: AccessTokenOptions,
        expected_auth_session_version: Option<&Option<String>>,
    ) -> AppResult<String> {
        let AccessTokenOptions {
            mfa_verified_until,
            mfa_step_up_verified_until,
            security_action_verified_until,
            auth_scope,
            ttl_seconds,
            persist_session,
            password_change_required_after_enrollment,
            oauth,
        } = options;
        let actor = self.load_user_for_auth_payload(actor).await?;
        let is_authless_oauth = matches!(
            oauth.as_ref().map(|(_, _, source)| source),
            Some(OAuthAuthorizationSource::Authless)
        ) && Self::is_default_admin_username(&actor.username);
        if !actor.login_status().is_enabled() && !is_authless_oauth {
            return Err(AppError::Unauthorized("credentials unavailable".into()));
        }
        let signing_seed = actor
            .password_hash
            .clone()
            .unwrap_or_else(|| format!("federated:{}", actor.id));
        let auth_session_version = self
            .services
            .identity
            .users
            .auth_session_version(&actor.id)
            .await?;
        if let Some(expected_auth_session_version) = expected_auth_session_version
            && auth_session_version != *expected_auth_session_version
        {
            return Err(AppError::Unauthorized(
                "authentication session was invalidated".into(),
            ));
        }
        let authorization_fingerprint = Self::auth_session_fingerprint_for_version(
            Self::authorization_fingerprint(&actor),
            auth_session_version.as_deref(),
        );

        let now = Utc::now();
        let iat = now.timestamp();
        let exp = (now + Duration::seconds(ttl_seconds)).timestamp();

        let is_oauth = oauth.is_some();
        let app_permissions = if is_oauth || auth_scope != JwtSessionScope::Full {
            Vec::new()
        } else {
            Self::canonical_app_permission_claims(&actor)
        };
        let library_permissions = if auth_scope == JwtSessionScope::Full {
            Self::canonical_library_permission_claims(&actor)
        } else {
            Vec::new()
        };
        let actor_capabilities = if is_oauth
            || matches!(
                auth_scope,
                JwtSessionScope::MfaEnrollment | JwtSessionScope::PasswordChangeRequired
            ) {
            Vec::new()
        } else {
            Self::canonical_actor_capability_claims(&actor)
        };
        let (oauth_client_id, oauth_grant_id, oauth_authorization_source) = oauth
            .map(|(client_id, grant_id, authorization_source)| {
                (Some(client_id), Some(grant_id), authorization_source)
            })
            .unwrap_or((None, None, OAuthAuthorizationSource::Authenticated));

        let claims = JwtClaims {
            sub: actor.id.clone(),
            exp,
            iat,
            iss: self.auth.issuer.clone(),
            username: actor.username.clone(),
            app_permissions,
            library_permissions,
            mfa_verified_until: mfa_verified_until.map(|value| value.timestamp()),
            mfa_step_up_verified_until: mfa_step_up_verified_until.map(|value| value.timestamp()),
            security_action_verified_until: security_action_verified_until
                .map(|value| value.timestamp()),
            auth_scope,
            persist_session,
            auth_session_version,
            password_change_required_after_enrollment,
            oauth_client_id,
            oauth_grant_id,
            oauth_authorization_source,
            actor_capabilities,
        };

        let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256);
        let signing_key = self.derive_jwt_key(&signing_seed, &authorization_fingerprint);
        let key = jsonwebtoken::EncodingKey::from_secret(&signing_key);

        let token = jsonwebtoken::encode(&header, &claims, &key)
            .map_err(|err| AppError::Repository(format!("failed to issue token: {err}")))?;

        Ok(token)
    }

    fn derive_scoped_signing_key(&self, jwt_signing_key: &[u8], token_kind: &str) -> Vec<u8> {
        let hmac_key = hmac::Key::new(hmac::HMAC_SHA256, jwt_signing_key);
        hmac::sign(&hmac_key, token_kind.as_bytes())
            .as_ref()
            .to_vec()
    }

    fn derive_backup_download_signing_key(&self, jwt_signing_key: &[u8]) -> Vec<u8> {
        self.derive_scoped_signing_key(jwt_signing_key, Self::BACKUP_DOWNLOAD_TOKEN_KIND)
    }

    fn derive_release_candidate_signing_key(&self, jwt_signing_key: &[u8]) -> Vec<u8> {
        self.derive_scoped_signing_key(jwt_signing_key, Self::RELEASE_CANDIDATE_TOKEN_KIND)
    }

    pub(crate) async fn backup_download_signing_key_for_actor(
        &self,
        actor: &User,
    ) -> AppResult<Vec<u8>> {
        self.ensure_jwt_signing_keys_loaded().await?;
        let cache = self.jwt_signing_keys.read().await;
        let jwt_signing_key = cache.get(&actor.id).cloned().ok_or_else(|| {
            AppError::Unauthorized(format!(
                "cannot resolve backup download signing key for actor {}",
                actor.id
            ))
        })?;

        Ok(self.derive_backup_download_signing_key(&jwt_signing_key))
    }

    pub(crate) async fn release_candidate_signing_key_for_actor(
        &self,
        actor: &User,
    ) -> AppResult<Vec<u8>> {
        self.ensure_jwt_signing_keys_loaded().await?;
        let cache = self.jwt_signing_keys.read().await;
        let jwt_signing_key = cache.get(&actor.id).cloned().ok_or_else(|| {
            AppError::Unauthorized(format!(
                "cannot resolve release candidate signing key for actor {}",
                actor.id
            ))
        })?;

        Ok(self.derive_release_candidate_signing_key(&jwt_signing_key))
    }

    fn submission_scope_claims(scope: &SubmissionScope) -> (&'static str, Option<String>) {
        match scope {
            SubmissionScope::Episode { episode_id } => ("episode", Some(episode_id.clone())),
            SubmissionScope::EpisodeSet { episode_ids } => (
                "episode_set",
                Some(serde_json::to_string(episode_ids).unwrap_or_else(|_| String::new())),
            ),
            SubmissionScope::Collection { collection_id } => {
                ("collection", Some(collection_id.clone()))
            }
            SubmissionScope::Title => ("title", None),
            SubmissionScope::SeriesMovie {
                series_movie_link_id,
            } => ("series_movie", Some(series_movie_link_id.clone())),
            SubmissionScope::Orphan => ("orphan", None),
        }
    }

    fn submission_scope_from_claims(
        scope_kind: &str,
        scope_id: Option<String>,
    ) -> AppResult<SubmissionScope> {
        match scope_kind {
            "episode" => Ok(SubmissionScope::Episode {
                episode_id: scope_id.ok_or_else(|| {
                    AppError::Unauthorized(
                        "release candidate token missing episode scope id".into(),
                    )
                })?,
            }),
            "episode_set" => {
                let raw = scope_id.ok_or_else(|| {
                    AppError::Unauthorized(
                        "release candidate token missing episode-set scope id".into(),
                    )
                })?;
                let mut episode_ids =
                    serde_json::from_str::<Vec<String>>(&raw).unwrap_or_else(|_| {
                        raw.split(',')
                            .map(|value| value.trim().to_string())
                            .collect()
                    });
                episode_ids.retain(|episode_id| !episode_id.is_empty());
                episode_ids.sort();
                episode_ids.dedup();
                if episode_ids.is_empty() {
                    return Err(AppError::Unauthorized(
                        "release candidate token has empty episode-set scope".into(),
                    ));
                }
                Ok(SubmissionScope::EpisodeSet { episode_ids })
            }
            "collection" => Ok(SubmissionScope::Collection {
                collection_id: scope_id.ok_or_else(|| {
                    AppError::Unauthorized(
                        "release candidate token missing collection scope id".into(),
                    )
                })?,
            }),
            "series_movie" => Ok(SubmissionScope::SeriesMovie {
                series_movie_link_id: scope_id.ok_or_else(|| {
                    AppError::Unauthorized(
                        "release candidate token missing series movie scope id".into(),
                    )
                })?,
            }),
            "title" => Ok(SubmissionScope::Title),
            "orphan" => Ok(SubmissionScope::Orphan),
            _ => Err(AppError::Unauthorized(
                "release candidate token has unknown scope".into(),
            )),
        }
    }

    pub(crate) fn issue_release_candidate_token_with_signing_key(
        &self,
        actor: &User,
        title_id: &str,
        scope: &SubmissionScope,
        selection: &QueuedReleaseSelection,
        signing_key: &[u8],
    ) -> AppResult<String> {
        let source_hint = selection
            .source_hint
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                AppError::Validation("release candidate token requires a source hint".into())
            })?;
        let source_title = selection
            .source_title
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                AppError::Validation("release candidate token requires a source title".into())
            })?;

        let now = Utc::now();
        let iat = now.timestamp();
        let expires_at = now + Duration::seconds(Self::RELEASE_CANDIDATE_TOKEN_TTL_SECONDS);
        let exp = expires_at.timestamp();
        let (scope_kind, scope_id) = Self::submission_scope_claims(scope);
        let password_ref = self.store_release_candidate_password_ticket(
            actor,
            title_id,
            scope_kind,
            scope_id.as_deref(),
            source_hint,
            source_title,
            selection.source_password.as_deref(),
            expires_at,
        )?;
        let claims = ReleaseCandidateTokenClaims {
            sub: actor.id.clone(),
            exp,
            iat,
            iss: self.auth.issuer.clone(),
            kind: Self::RELEASE_CANDIDATE_TOKEN_KIND.to_string(),
            title_id: title_id.to_string(),
            scope_kind: scope_kind.to_string(),
            scope_id,
            indexer_id: selection.indexer_id.clone(),
            source_hint: source_hint.to_string(),
            source_kind: selection.source_kind,
            source_title: source_title.to_string(),
            password_ref,
            info_hash_hint: selection.info_hash_hint.clone(),
            size_bytes: selection.size_bytes,
            seeders: selection.seeders,
        };
        let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256);
        let key = jsonwebtoken::EncodingKey::from_secret(signing_key);

        jsonwebtoken::encode(&header, &claims, &key).map_err(|err| {
            AppError::Repository(format!("failed to issue release candidate token: {err}"))
        })
    }

    pub(crate) fn issue_backup_download_token_with_signing_key(
        &self,
        actor: &User,
        filename: &str,
        signing_key: &[u8],
    ) -> AppResult<BackupDownloadTicket> {
        let now = Utc::now();
        let iat = now.timestamp();
        let expires_at = now + Duration::seconds(Self::BACKUP_DOWNLOAD_TOKEN_TTL_SECONDS);
        let claims = BackupDownloadTokenClaims {
            sub: actor.id.clone(),
            exp: expires_at.timestamp(),
            iat,
            iss: self.auth.issuer.clone(),
            kind: Self::BACKUP_DOWNLOAD_TOKEN_KIND.to_string(),
            filename: filename.to_string(),
        };
        let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256);
        let key = jsonwebtoken::EncodingKey::from_secret(signing_key);
        let token = jsonwebtoken::encode(&header, &claims, &key).map_err(|err| {
            AppError::Repository(format!("failed to issue backup download token: {err}"))
        })?;

        Ok(BackupDownloadTicket {
            token,
            expires_at: expires_at.to_rfc3339(),
        })
    }

    pub async fn issue_backup_download_token(
        &self,
        actor: &User,
        filename: &str,
    ) -> AppResult<BackupDownloadTicket> {
        let signing_key = self.backup_download_signing_key_for_actor(actor).await?;
        self.issue_backup_download_token_with_signing_key(actor, filename, &signing_key)
    }

    pub async fn issue_release_candidate_token(
        &self,
        actor: &User,
        title_id: &str,
        scope: &SubmissionScope,
        selection: &QueuedReleaseSelection,
    ) -> AppResult<String> {
        let signing_key = self.release_candidate_signing_key_for_actor(actor).await?;
        self.issue_release_candidate_token_with_signing_key(
            actor,
            title_id,
            scope,
            selection,
            &signing_key,
        )
    }

    pub async fn verify_release_candidate_token(
        &self,
        actor: &User,
        title_id: &str,
        scope: &SubmissionScope,
        token: &str,
    ) -> AppResult<QueuedReleaseSelection> {
        let (selection, claimed_scope) = self
            .verify_release_candidate_token_for_signed_scope(actor, title_id, token)
            .await?;
        if &claimed_scope != scope {
            return Err(AppError::Unauthorized(
                "release candidate token scope does not match request".into(),
            ));
        }
        Ok(selection)
    }

    fn backup_download_token_subject(&self, token: &str) -> AppResult<String> {
        let unverified = jsonwebtoken::dangerous::insecure_decode::<BackupDownloadTokenClaims>(
            token,
        )
        .map_err(|err| AppError::Unauthorized(format!("malformed backup download token: {err}")))?;
        let subject = unverified.claims.sub.trim();
        if subject.is_empty() {
            return Err(AppError::Unauthorized(
                "backup download token subject is empty".into(),
            ));
        }
        Ok(subject.to_string())
    }

    pub async fn verify_backup_download_token(
        &self,
        actor: &User,
        filename: &str,
        token: &str,
    ) -> AppResult<()> {
        let signing_key = self.backup_download_signing_key_for_actor(actor).await?;
        let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::HS256);
        validation.validate_exp = true;
        validation.set_issuer(&[self.auth.issuer.as_str()]);
        let key = jsonwebtoken::DecodingKey::from_secret(&signing_key);
        let claims = jsonwebtoken::decode::<BackupDownloadTokenClaims>(token, &key, &validation)
            .map_err(|err| AppError::Unauthorized(format!("invalid backup download token: {err}")))?
            .claims;

        if claims.kind != Self::BACKUP_DOWNLOAD_TOKEN_KIND {
            return Err(AppError::Unauthorized(
                "invalid backup download token kind".into(),
            ));
        }
        if claims.sub != actor.id {
            return Err(AppError::Unauthorized(
                "backup download token subject does not match actor".into(),
            ));
        }
        if claims.filename != filename {
            return Err(AppError::Unauthorized(
                "backup download token filename does not match request".into(),
            ));
        }

        Ok(())
    }

    pub async fn authorize_backup_download_ticket(
        &self,
        filename: &str,
        token: &str,
    ) -> AppResult<User> {
        let subject = self.backup_download_token_subject(token)?;
        let actor = self
            .services
            .identity
            .users
            .get_by_id(&subject)
            .await?
            .ok_or_else(|| AppError::Unauthorized("unknown backup download subject".into()))?;
        let actor = self.attach_user_authorization(actor).await?;
        self.require_app_permission(&actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        self.verify_backup_download_token(&actor, filename, token)
            .await?;
        Ok(actor)
    }

    pub async fn verify_release_candidate_token_for_signed_scope(
        &self,
        actor: &User,
        title_id: &str,
        token: &str,
    ) -> AppResult<(QueuedReleaseSelection, SubmissionScope)> {
        let signing_key = self.release_candidate_signing_key_for_actor(actor).await?;
        let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::HS256);
        validation.validate_exp = true;
        validation.set_issuer(&[self.auth.issuer.as_str()]);
        let key = jsonwebtoken::DecodingKey::from_secret(&signing_key);
        let claims = jsonwebtoken::decode::<ReleaseCandidateTokenClaims>(token, &key, &validation)
            .map_err(|err| {
                AppError::Unauthorized(format!("invalid release candidate token: {err}"))
            })?
            .claims;

        if claims.kind != Self::RELEASE_CANDIDATE_TOKEN_KIND {
            return Err(AppError::Unauthorized(
                "invalid release candidate token kind".into(),
            ));
        }
        if claims.sub != actor.id {
            return Err(AppError::Unauthorized(
                "release candidate token subject does not match actor".into(),
            ));
        }
        if claims.title_id != title_id {
            return Err(AppError::Unauthorized(
                "release candidate token title does not match request".into(),
            ));
        }

        let claimed_scope =
            Self::submission_scope_from_claims(&claims.scope_kind, claims.scope_id.clone())?;
        // Re-judge admission here rather than trusting the verdict made when the
        // candidate was offered. This path exists so the API cannot be used to
        // queue something the UI refused, and the threshold may have changed
        // inside the token's lifetime. A token minted before this field existed
        // carries no count, which reads as unknown and stays eligible for the
        // rest of its short TTL — a bounded migration grace, not the same thing
        // as an indexer that genuinely reports nothing.
        let minimum_seeders = self
            .minimum_seeders_for_indexer(claims.indexer_id.as_deref())
            .await;
        if !crate::acquisition::seed_goals::meets_minimum_seeders(
            claims.source_kind,
            claims.indexer_id.as_deref(),
            claims.seeders,
            minimum_seeders,
        ) {
            return Err(AppError::Validation(format!(
                "release reports {} seeders, below the minimum of {minimum_seeders} for this indexer",
                claims
                    .seeders
                    .map_or_else(|| "unknown".to_string(), |count| count.to_string())
            )));
        }

        let source_password = self.resolve_release_candidate_password_ticket(
            actor,
            title_id,
            &claims,
            &claimed_scope,
        )?;

        Ok((
            QueuedReleaseSelection {
                indexer_id: claims.indexer_id,
                source_hint: Some(claims.source_hint),
                source_kind: claims.source_kind,
                source_title: Some(claims.source_title),
                source_password,
                info_hash_hint: claims.info_hash_hint,
                size_bytes: claims.size_bytes,
                seeders: claims.seeders,
            },
            claimed_scope,
        ))
    }

    pub async fn authenticate_token(&self, token: &str) -> AppResult<User> {
        self.authenticate_token_with_claims(token)
            .await
            .map(|(user, _)| user)
    }

    pub async fn authenticate_token_with_claims(
        &self,
        token: &str,
    ) -> AppResult<(User, AuthenticatedTokenClaims)> {
        // Decode claims without signature verification to extract the subject (user ID).
        let unverified = jsonwebtoken::dangerous::insecure_decode::<JwtClaims>(token)
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
        let oauth_client_present = claims.oauth_client_id.is_some();
        let oauth_grant_present = claims.oauth_grant_id.is_some();
        if oauth_client_present ^ oauth_grant_present {
            return Err(AppError::Unauthorized("invalid OAuth token claims".into()));
        }
        let is_oauth = oauth_client_present && oauth_grant_present;
        let is_authless_oauth = is_oauth
            && matches!(
                claims.oauth_authorization_source,
                OAuthAuthorizationSource::Authless
            );
        if is_oauth && !claims.app_permissions.is_empty() {
            return Err(AppError::Unauthorized(
                "OAuth tokens cannot carry app permissions".into(),
            ));
        }
        if claims.password_change_required_after_enrollment
            && claims.auth_scope != JwtSessionScope::MfaEnrollment
        {
            return Err(AppError::Unauthorized(
                "invalid password-change enrollment token claims".into(),
            ));
        }
        let actor_capabilities = if claims.actor_capabilities.is_empty() {
            if is_oauth
                || matches!(
                    claims.auth_scope,
                    JwtSessionScope::MfaEnrollment | JwtSessionScope::PasswordChangeRequired
                )
            {
                scryer_domain::ActorCapabilityMask::NONE
            } else {
                scryer_domain::ActorCapabilityMask::MANAGE_OWN_ACCOUNT
            }
        } else {
            let mask = Self::actor_capability_claims_to_mask(&claims.actor_capabilities)?;
            if is_oauth && !mask.is_empty() {
                return Err(AppError::Unauthorized(
                    "OAuth tokens cannot carry actor capabilities".into(),
                ));
            }
            if matches!(
                claims.auth_scope,
                JwtSessionScope::MfaEnrollment | JwtSessionScope::PasswordChangeRequired
            ) && !mask.is_empty()
            {
                return Err(AppError::Unauthorized(
                    "restricted authentication tokens cannot carry actor capabilities".into(),
                ));
            }
            mask
        };
        let subject = claims.sub.clone();
        let token_claims = AuthenticatedTokenClaims {
            mfa_verified_until: claims.mfa_verified_until,
            mfa_step_up_verified_until: claims.mfa_step_up_verified_until,
            security_action_verified_until: claims.security_action_verified_until,
            session_scope: claims.auth_scope,
            persist_session: claims.persist_session,
            auth_session_version: claims.auth_session_version,
            password_change_required_after_enrollment: claims
                .password_change_required_after_enrollment,
            oauth_client_id: claims.oauth_client_id,
            oauth_grant_id: claims.oauth_grant_id,
            oauth_authorization_source: claims.oauth_authorization_source,
            actor_capabilities,
        };
        let mut user = self
            .services
            .identity
            .users
            .get_by_id(&subject)
            .await?
            .ok_or_else(|| AppError::Unauthorized("token subject no longer exists".into()))?;
        let disabled_authless_grant_allowed =
            is_authless_oauth && Self::is_default_admin_username(&user.username);
        if !(user.login_status().is_enabled() || disabled_authless_grant_allowed) {
            return Err(AppError::Unauthorized("credentials unavailable".into()));
        }
        user.password_hash = None;
        Ok((user, token_claims))
    }

    pub async fn authenticate_credentials(
        &self,
        username: &str,
        password: &str,
    ) -> AppResult<User> {
        let started_at = Instant::now();
        let username = Self::normalize_local_username(username);
        if username.is_empty() {
            self.verify_dummy_login_password(password);
            Self::apply_login_failure_timing(LoginFailureTimingClass::FastMasked, started_at).await;
            return Err(AppError::Validation("username is required".into()));
        }
        if password.is_empty() {
            self.verify_dummy_login_password(password);
            Self::apply_login_failure_timing(LoginFailureTimingClass::FastMasked, started_at).await;
            return Err(AppError::Validation("password is required".into()));
        }
        if Self::is_reserved_recovery_username(username) && !self.recovery_admin_login_enabled() {
            self.verify_dummy_login_password(password);
            Self::apply_login_failure_timing(LoginFailureTimingClass::FastMasked, started_at).await;
            return Err(AppError::Unauthorized("credentials unavailable".into()));
        }

        let Some(user) = self
            .services
            .identity
            .users
            .get_by_username(username)
            .await?
        else {
            self.verify_dummy_login_password(password);
            Self::apply_login_failure_timing(LoginFailureTimingClass::FastMasked, started_at).await;
            return Err(AppError::NotFound(format!("user {username} not found")));
        };

        if !user.login_status().is_enabled() {
            if let Some(password_hash) = user.password_hash.as_deref() {
                let _ = self.validate_password(password, password_hash);
                Self::apply_login_failure_timing(
                    LoginFailureTimingClass::PasswordBackedLocal,
                    started_at,
                )
                .await;
            } else {
                self.verify_dummy_login_password(password);
                Self::apply_login_failure_timing(LoginFailureTimingClass::FastMasked, started_at)
                    .await;
            }
            return Err(AppError::Unauthorized("credentials unavailable".into()));
        }

        let Some(password_hash) = user.password_hash.as_ref() else {
            self.verify_dummy_login_password(password);
            Self::apply_login_failure_timing(LoginFailureTimingClass::FastMasked, started_at).await;
            return Err(AppError::Unauthorized("credentials unavailable".into()));
        };

        if !self.validate_password(password, password_hash)? {
            Self::apply_login_failure_timing(
                LoginFailureTimingClass::PasswordBackedLocal,
                started_at,
            )
            .await;
            return Err(AppError::Unauthorized("invalid credentials".into()));
        }

        // The former online v1 → v2 re-hash on login is gone with the v1 format
        // itself. Migration 0191 retires surviving v1 rows directly.

        self.cache_jwt_signing_key(&user).await?;
        Ok(user)
    }

    /// Verifies the current local password before issuing an account-security grant.
    pub async fn account_security_password_verify(
        &self,
        actor: &User,
        current_password: &str,
    ) -> AppResult<()> {
        self.require_actor_capability(actor, scryer_domain::ActorCapability::ManageOwnAccount)
            .await?;
        if current_password.is_empty() {
            return Err(AppError::Unauthorized(
                "current password is required for reauthentication".into(),
            ));
        }

        let user = self.load_user_for_auth_payload(actor).await?;
        let Some(password_hash) = user.password_hash.as_deref() else {
            return Err(AppError::Unauthorized(
                "password reauthentication is unavailable for this account".into(),
            ));
        };
        if !self.validate_password(current_password, password_hash)? {
            return Err(AppError::Unauthorized("invalid current password".into()));
        }
        Ok(())
    }

    /// Rotates every token-bearing session after a self-service factor change.
    pub async fn rotate_auth_session_after_factor_change(&self, actor: &User) -> AppResult<()> {
        let auth_session_version = Id::new().0;
        let updated_user = self
            .services
            .identity
            .users
            .rotate_auth_session_version(&actor.id, &auth_session_version)
            .await?;
        self.finalize_auth_session_rotation(&actor.id, &updated_user)
            .await
    }

    pub async fn finalize_auth_session_rotation(
        &self,
        user_id: &str,
        updated_user: &User,
    ) -> AppResult<()> {
        self.revoke_oauth_refresh_grants_for_user(user_id, "auth_session_changed")
            .await?;
        self.evict_cached_jwt_signing_key(user_id).await;
        self.refresh_cached_jwt_signing_key(updated_user).await?;
        Ok(())
    }
}

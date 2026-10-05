//! Owner-only provider accounts and short-lived, single-use linking sessions.
use super::account_transport::*;
use super::privacy::ensure_account_owner;
use crate::{AppError, AppResult, AppUseCase};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Duration, Utc};
use scryer_domain::{
    Id, ListAccountCredential, ListPolicy, User, UserListAccount, UserListAccountStatus,
};
use scryer_plugin_sdk::{
    ListCredential, ListPluginAccountResponse, ListProviderAuth, PluginResult,
};
use std::collections::{BTreeMap, HashMap};
use tokio::sync::Mutex;

const MAX_LINK_SESSIONS: usize = 1024;
const LINK_TTL_SECONDS: i64 = 600;
/// First pause after a failed renewal; each further failure doubles it.
const REFRESH_BACKOFF_FIRST_SECONDS: i64 = 5 * 60;
/// Longest pause between renewal attempts.
const REFRESH_BACKOFF_MAX_SECONDS: i64 = 6 * 60 * 60;
/// Accounts whose renewal pause is remembered. Beyond this the entries whose
/// pause has passed are dropped, then the oldest pauses.
const MAX_REFRESH_BACKOFFS: usize = 4096;
/// A credential that has failed to renew this many times in a row, over at
/// least `REFRESH_FAILURE_MIN_SPAN_HOURS`, is treated as gone even though no
/// provider said so. With the backoff above this takes about 40 hours.
const REFRESH_FAILURE_LIMIT: u32 = 12;
const REFRESH_FAILURE_MIN_SPAN_HOURS: i64 = 36;
/// How long an interactive read waits for the account lock, and then for a
/// renewal, before answering that the account is still refreshing.
const INTERACTIVE_LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(5);
const INTERACTIVE_REFRESH_WAIT: std::time::Duration = std::time::Duration::from_secs(15);
/// How long an unlink waits behind an in-flight renewal.
const UNLINK_LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(10);
/// The pause suggested to a caller told that the account is still refreshing.
const REFRESHING_RETRY_AFTER: std::time::Duration = std::time::Duration::from_secs(30);

pub struct ListAccountRuntime {
    sessions: Mutex<HashMap<String, LinkSession>>,
    /// Fixed stripes bound coordination memory while separating most accounts.
    operations: [std::sync::Arc<Mutex<()>>; 64],
    links: Mutex<()>,
    revocations: std::sync::Arc<tokio::sync::Semaphore>,
    /// In-memory renewal pauses by account id, cleared on success. A restart
    /// forgets them; the persisted failure count still bounds the retries.
    refresh_backoffs: std::sync::Mutex<HashMap<String, RefreshBackoff>>,
}
#[derive(Clone, Copy)]
struct RefreshBackoff {
    failures: u32,
    next_attempt_at: DateTime<Utc>,
    rate_limited: bool,
}
impl Default for ListAccountRuntime {
    fn default() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            operations: std::array::from_fn(|_| std::sync::Arc::new(Mutex::new(()))),
            links: Mutex::new(()),
            revocations: std::sync::Arc::new(tokio::sync::Semaphore::new(16)),
            refresh_backoffs: std::sync::Mutex::new(HashMap::new()),
        }
    }
}
impl ListAccountRuntime {
    fn stripe(&self, id: &str) -> std::sync::Arc<Mutex<()>> {
        use std::hash::{Hash, Hasher};
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        id.hash(&mut hash);
        self.operations[(hash.finish() as usize) % self.operations.len()].clone()
    }
    pub(crate) async fn lock_account(&self, id: &str) -> tokio::sync::OwnedMutexGuard<()> {
        self.stripe(id).lock_owned().await
    }
    /// The account lock, or `None` when it is still held after `wait`.
    async fn lock_account_within(
        &self,
        id: &str,
        wait: std::time::Duration,
    ) -> Option<tokio::sync::OwnedMutexGuard<()>> {
        tokio::time::timeout(wait, self.stripe(id).lock_owned())
            .await
            .ok()
    }
    /// The active renewal pause for `id`, if any.
    fn refresh_backoff(&self, id: &str, now: DateTime<Utc>) -> Option<RefreshBackoff> {
        let backoffs = self
            .refresh_backoffs
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        backoffs
            .get(id)
            .filter(|backoff| backoff.next_attempt_at > now)
            .copied()
    }
    /// Record a failed renewal and return the pause before the next attempt.
    fn record_refresh_failure(
        &self,
        id: &str,
        now: DateTime<Utc>,
        retry_after: Option<std::time::Duration>,
        rate_limited: bool,
    ) -> RefreshBackoff {
        let mut backoffs = self
            .refresh_backoffs
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if backoffs.len() >= MAX_REFRESH_BACKOFFS && !backoffs.contains_key(id) {
            backoffs.retain(|_, backoff| backoff.next_attempt_at > now);
            if backoffs.len() >= MAX_REFRESH_BACKOFFS
                && let Some(oldest) = backoffs
                    .iter()
                    .min_by_key(|(_, backoff)| backoff.next_attempt_at)
                    .map(|(key, _)| key.clone())
            {
                backoffs.remove(&oldest);
            }
        }
        let failures = backoffs
            .get(id)
            .map_or(1, |backoff| backoff.failures.saturating_add(1));
        let exponent = failures.saturating_sub(1).min(16);
        let pause = REFRESH_BACKOFF_FIRST_SECONDS
            .saturating_mul(1_i64 << exponent)
            .min(REFRESH_BACKOFF_MAX_SECONDS);
        let requested = retry_after
            .and_then(|delay| i64::try_from(delay.as_secs()).ok())
            .unwrap_or(0);
        let backoff = RefreshBackoff {
            failures,
            next_attempt_at: now + Duration::seconds(pause.max(requested)),
            rate_limited,
        };
        backoffs.insert(id.to_owned(), backoff);
        backoff
    }
    pub(crate) fn clear_refresh_backoff(&self, id: &str) {
        self.refresh_backoffs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
    }
}
/// The answer for a renewal that is paused or still running elsewhere.
fn refresh_pending(backoff: Option<RefreshBackoff>, now: DateTime<Utc>) -> AppError {
    let retry_after = backoff.map_or(Some(REFRESHING_RETRY_AFTER), |backoff| {
        (backoff.next_attempt_at - now).to_std().ok()
    });
    if backoff.is_some_and(|backoff| backoff.rate_limited) {
        return rate_limited_failure(retry_after);
    }
    AppError::temporary_unavailable(
        "list account refresh is temporarily unavailable",
        retry_after,
    )
}
fn reconnect_this_account() -> AppError {
    AppError::Validation("reconnect this list account".into())
}
fn needs_refresh(account: &UserListAccount, now: DateTime<Utc>) -> bool {
    account
        .credential
        .expires_at
        .is_some_and(|expires| expires <= now + Duration::seconds(60))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn session(now: DateTime<Utc>) -> LinkSession {
        LinkSession {
            owner: "owner".into(),
            provider: "trakt".into(),
            origin: "https://instance.invalid".into(),
            state: "expected-state".into(),
            verifier: "private-verifier".into(),
            expires_at: now + Duration::seconds(600),
            poll_token: None,
            app: None,
        }
    }
    #[tokio::test]
    async fn link_sessions_bind_owner_provider_and_state_without_admin_bypass() {
        let runtime = ListAccountRuntime::default();
        let now = Utc::now();
        runtime
            .sessions
            .lock()
            .await
            .insert("session".into(), session(now));
        for (owner, state, provider) in [
            ("administrator", "expected-state", "trakt"),
            ("owner", "tampered", "trakt"),
            ("owner", "expected-state", "simkl"),
        ] {
            assert!(
                runtime
                    .take("session", owner, Some(state), Some(provider), now)
                    .await
                    .is_err()
            );
        }
        assert!(
            runtime
                .take(
                    "session",
                    "owner",
                    Some("expected-state"),
                    Some("trakt"),
                    now
                )
                .await
                .is_ok()
        );
        assert!(
            runtime
                .take(
                    "session",
                    "owner",
                    Some("expected-state"),
                    Some("trakt"),
                    now
                )
                .await
                .is_err()
        );
    }
    #[tokio::test]
    async fn simultaneous_completion_consumes_exactly_one_session() {
        let runtime = ListAccountRuntime::default();
        let now = Utc::now();
        runtime
            .sessions
            .lock()
            .await
            .insert("session".into(), session(now));
        let (first, second) = tokio::join!(
            runtime.take(
                "session",
                "owner",
                Some("expected-state"),
                Some("trakt"),
                now
            ),
            runtime.take(
                "session",
                "owner",
                Some("expected-state"),
                Some("trakt"),
                now
            )
        );
        assert_ne!(first.is_ok(), second.is_ok());
    }
    #[tokio::test]
    async fn expiry_and_process_restart_require_a_new_link() {
        let runtime = ListAccountRuntime::default();
        let now = Utc::now();
        runtime
            .sessions
            .lock()
            .await
            .insert("session".into(), session(now));
        assert!(
            runtime
                .take("session", "owner", None, None, now + Duration::seconds(600))
                .await
                .is_err()
        );
        assert!(
            ListAccountRuntime::default()
                .take("session", "owner", None, None, now)
                .await
                .is_err()
        );
    }
    #[test]
    fn link_origins_are_origins_and_credentials_redact_every_secret() {
        assert!(origin("https://instance.invalid/prefix").is_err());
        assert!(origin("https://member:secret@instance.invalid").is_err());
        assert_eq!(
            origin("http://localhost:9000").unwrap(),
            "http://localhost:9000"
        );
        let token = ListAccountCredential {
            access_token: "access-secret".into(),
            refresh_token: Some("refresh-secret".into()),
            refresh_handle: Some("handle-secret".into()),
            app_config: Some(BTreeMap::from([(
                "client_secret".into(),
                "app-secret".into(),
            )])),
            ..Default::default()
        };
        let printed = format!("{token:?}");
        for secret in [
            "access-secret",
            "refresh-secret",
            "handle-secret",
            "app-secret",
        ] {
            assert!(!printed.contains(secret));
        }
    }
}
#[derive(Clone)]
struct LinkSession {
    owner: String,
    provider: String,
    origin: String,
    state: String,
    verifier: String,
    expires_at: DateTime<Utc>,
    poll_token: Option<String>,
    app: Option<ListProviderAppConfig>,
}
#[derive(Clone)]
pub struct ListAccountLink {
    pub session_id: String,
    pub state: String,
    pub authorize_url: String,
    pub authorization_origin: String,
    pub expires_at: DateTime<Utc>,
    pub poll_required: bool,
}
#[derive(Clone)]
pub struct ListAccountView {
    pub account: UserListAccount,
    pub sources: ListPluginAccountResponse,
}
pub struct ListAccountPoll {
    pub status: String,
    pub account: Option<ListAccountView>,
}
fn missing() -> AppError {
    AppError::NotFound("list account link not found or expired".into())
}
fn random_token() -> AppResult<String> {
    let mut bytes = [0u8; 32];
    aws_lc_rs::rand::fill(&mut bytes)
        .map_err(|_| AppError::Validation("could not start account link".into()))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}
fn origin(value: &str) -> AppResult<String> {
    if value.len() > 2048 {
        return Err(AppError::Validation("invalid account link origin".into()));
    }
    let url = url::Url::parse(value)
        .map_err(|_| AppError::Validation("an account link needs a valid origin".into()))?;
    if !matches!(url.scheme(), "https" | "http")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        return Err(AppError::Validation(
            "an account link needs a valid origin".into(),
        ));
    }
    Ok(url.origin().ascii_serialization())
}
fn credential(account: &UserListAccount) -> ListCredential {
    ListCredential {
        access_token: account.credential.access_token.clone(),
        token_type: account.credential.token_type.clone(),
        external_user_id: Some(account.external_user_id.clone()),
        username: Some(account.username.clone()),
    }
}
impl ListAccountRuntime {
    async fn take(
        &self,
        id: &str,
        owner: &str,
        state: Option<&str>,
        provider: Option<&str>,
        now: DateTime<Utc>,
    ) -> AppResult<LinkSession> {
        let mut sessions = self.sessions.lock().await;
        sessions.retain(|_, session| session.expires_at > now);
        let session = sessions.get(id).ok_or_else(missing)?;
        if session.owner != owner
            || state.is_some_and(|state| state != session.state)
            || provider.is_some_and(|provider| provider != session.provider)
        {
            return Err(missing());
        }
        sessions.remove(id).ok_or_else(missing)
    }
}
impl AppUseCase {
    pub(crate) async fn require_personal_lists_allowed(&self, actor: &User) -> AppResult<()> {
        self.require_lists_enabled().await?;
        if !actor.authorization.login_status.is_enabled() {
            return Err(AppError::Unauthorized("account is disabled".into()));
        }
        if self
            .services
            .lists
            .policies
            .get(&actor.id)
            .await?
            .is_some_and(|policy| policy.policy == ListPolicy::None)
        {
            return Err(AppError::Unauthorized(
                "personal lists are not allowed for this member".into(),
            ));
        }
        Ok(())
    }
    pub async fn my_list_accounts(&self, actor: &User) -> AppResult<Vec<ListAccountView>> {
        Ok(self
            .services
            .lists
            .accounts
            .list_by_user_id(&actor.id)
            .await?
            .into_iter()
            .filter(|account| account.user_id == actor.id)
            .map(|account| ListAccountView {
                account,
                sources: ListPluginAccountResponse::default(),
            })
            .collect())
    }
    pub async fn list_account(&self, actor: &User, id: &str) -> AppResult<ListAccountView> {
        self.require_personal_lists_allowed(actor).await?;
        let (_guard, account) = self
            .refresh_list_account_interactive(id, |account| ensure_account_owner(actor, account))
            .await?;
        let account = account.ok_or_else(|| AppError::NotFound("list account not found".into()))?;
        let sources = self
            .list_account_identity(&account.provider, &account.credential, Some(&account))
            .await?;
        Ok(ListAccountView { account, sources })
    }
    pub(crate) async fn owned_list_account(
        &self,
        actor: &User,
        id: &str,
    ) -> AppResult<UserListAccount> {
        let account = self
            .services
            .lists
            .accounts
            .get_by_id(id)
            .await?
            .ok_or_else(|| AppError::NotFound("list account not found".into()))?;
        ensure_account_owner(actor, &account)?;
        Ok(account)
    }
    pub async fn start_list_account_link(
        &self,
        actor: &User,
        provider: &str,
        browser_origin: &str,
    ) -> AppResult<ListAccountLink> {
        self.require_personal_lists_allowed(actor).await?;
        let origin = origin(browser_origin)?;
        let descriptor = self
            .services
            .lists
            .plugins
            .descriptors()
            .into_iter()
            .find(|descriptor| {
                descriptor.list_provider().is_some_and(|list| {
                    list.provider_type == provider
                        || list.provider_aliases.iter().any(|alias| alias == provider)
                })
            })
            .ok_or_else(|| AppError::NotFound("list provider not found".into()))?;
        let list = descriptor.list_provider().ok_or_else(missing)?;
        if !matches!(list.auth, ListProviderAuth::MemberAccount { .. })
            || !list.capabilities.account
        {
            return Err(AppError::Validation(
                "this provider does not support account linking".into(),
            ));
        }
        let provider = list.provider_type.clone();
        let protocol_provider = provider.trim_end_matches("-list").to_string();
        let app = if protocol_provider == "tmdb" {
            let values = self.list_provider_config_for("tmdb").await;
            let bearer = values.get("api_key").cloned().or_else(|| {
                descriptor
                    .config_fields()
                    .iter()
                    .find(|field| field.key == "api_key")
                    .and_then(|field| field.default_value.clone())
            });
            Some(ListProviderAppConfig {
                access_token: bearer,
                ..ListProviderAppConfig::default()
            })
        } else {
            self.list_provider_app_config(&protocol_provider).await?
        };
        let state = random_token()?;
        let verifier = random_token()?;
        let challenge = if protocol_provider == "mal" {
            verifier.clone()
        } else {
            URL_SAFE_NO_PAD.encode(
                aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, verifier.as_bytes()).as_ref(),
            )
        };
        let id = Id::new().0;
        let now = Utc::now();
        {
            let mut sessions = self.services.lists.account_runtime.sessions.lock().await;
            sessions.retain(|_, session| session.expires_at > now);
            if sessions.len() >= MAX_LINK_SESSIONS
                || sessions
                    .values()
                    .filter(|session| session.owner == actor.id)
                    .count()
                    >= 8
            {
                return Err(AppError::Validation(
                    "too many pending account links".into(),
                ));
            }
            sessions.insert(
                id.clone(),
                LinkSession {
                    owner: actor.id.clone(),
                    provider: protocol_provider.clone(),
                    origin: origin.clone(),
                    state: state.clone(),
                    verifier: verifier.clone(),
                    expires_at: now + Duration::seconds(LINK_TTL_SECONDS),
                    poll_token: None,
                    app: app.clone(),
                },
            );
        }
        let response = self
            .services
            .lists
            .auth
            .start(ListAccountStartRequest {
                provider: protocol_provider.clone(),
                origin: origin.clone(),
                state: state.clone(),
                code_challenge: challenge,
                app: app.clone(),
            })
            .await;
        let mut sessions = self.services.lists.account_runtime.sessions.lock().await;
        let response = match response {
            Ok(response) => response,
            Err(error) => {
                sessions.remove(&id);
                return Err(error);
            }
        };
        let session = sessions.get_mut(&id).ok_or_else(missing)?;
        session.expires_at = now
            + Duration::seconds(
                i64::try_from(response.expires_in)
                    .unwrap_or(LINK_TTL_SECONDS)
                    .clamp(1, LINK_TTL_SECONDS),
            );
        session.poll_token = response.poll_token;
        Ok(ListAccountLink {
            session_id: id,
            state,
            authorize_url: response.authorize_url,
            authorization_origin: if app.is_some()
                || !matches!(
                    protocol_provider.as_str(),
                    "trakt" | "anilist" | "mal" | "simkl"
                ) {
                origin
            } else {
                "https://smg.scryer.media".into()
            },
            expires_at: session.expires_at,
            poll_required: matches!(protocol_provider.as_str(), "plex" | "tmdb"),
        })
    }
    pub async fn complete_list_account_link(
        &self,
        actor: &User,
        id: &str,
        state: &str,
        provider: &str,
        code: &str,
        issuer: Option<&str>,
    ) -> AppResult<ListAccountView> {
        self.require_personal_lists_allowed(actor).await?;
        if code.trim().is_empty() || code.len() > 8192 {
            return Err(AppError::Validation("invalid account link result".into()));
        }
        let session = self
            .services
            .lists
            .account_runtime
            .take(
                id,
                &actor.id,
                Some(state),
                Some(provider.trim_end_matches("-list")),
                Utc::now(),
            )
            .await?;
        if session.provider == "simkl" && issuer != Some("https://simkl.com") {
            return Err(AppError::Validation("invalid account link issuer".into()));
        }
        let credential = self
            .services
            .lists
            .auth
            .complete(ListAccountCompleteRequest {
                provider: session.provider.clone(),
                code: code.into(),
                code_verifier: session.verifier,
                origin: session.origin,
                app: session.app.clone(),
                poll_token: session.poll_token,
            })
            .await?;
        if session.expires_at <= Utc::now() {
            return Err(missing());
        }
        self.finish_list_account_link(
            actor,
            &session.provider,
            credential,
            session.app,
            session.expires_at,
        )
        .await
    }
    pub async fn poll_list_account_link(
        &self,
        actor: &User,
        id: &str,
    ) -> AppResult<ListAccountPoll> {
        self.require_personal_lists_allowed(actor).await?;
        let session = self
            .services
            .lists
            .account_runtime
            .take(id, &actor.id, None, None, Utc::now())
            .await?;
        let token = session
            .poll_token
            .as_deref()
            .ok_or_else(|| AppError::Validation("this account link does not use polling".into()))?;
        match self
            .services
            .lists
            .auth
            .poll(&session.provider, token, session.app.as_ref())
            .await?
        {
            Some(credential) => {
                if session.expires_at <= Utc::now() {
                    return Err(missing());
                }
                Ok(ListAccountPoll {
                    status: "linked".into(),
                    account: Some(
                        self.finish_list_account_link(
                            actor,
                            &session.provider,
                            credential,
                            session.app,
                            session.expires_at,
                        )
                        .await?,
                    ),
                })
            }
            None => {
                if session.expires_at <= Utc::now() {
                    return Err(missing());
                }
                let mut sessions = self.services.lists.account_runtime.sessions.lock().await;
                if sessions.len() >= MAX_LINK_SESSIONS {
                    return Err(missing());
                }
                sessions.insert(id.into(), session);
                Ok(ListAccountPoll {
                    status: "pending".into(),
                    account: None,
                })
            }
        }
    }
    async fn finish_list_account_link(
        &self,
        actor: &User,
        provider: &str,
        mut token: ListAccountCredential,
        app: Option<ListProviderAppConfig>,
        expires_at: DateTime<Utc>,
    ) -> AppResult<ListAccountView> {
        self.require_personal_lists_allowed(actor).await?;
        let _link_guard = self.services.lists.account_runtime.links.lock().await;
        let sources = self.list_account_identity(provider, &token, None).await?;
        if expires_at <= Utc::now() {
            return Err(missing());
        }
        let current = self
            .services
            .identity
            .users
            .get_by_id(&actor.id)
            .await?
            .ok_or_else(missing)?;
        self.require_personal_lists_allowed(&current).await?;
        if sources.external_user_id.trim().is_empty() {
            return Err(AppError::Validation(
                "provider returned no account identity".into(),
            ));
        }
        token.app_config = app.as_ref().map(|app| {
            BTreeMap::from([
                ("client_id".into(), app.client_id.clone()),
                ("client_secret".into(), app.client_secret.clone()),
                ("redirect_uri".into(), app.redirect_uri.clone()),
                (
                    "access_token".into(),
                    app.access_token.clone().unwrap_or_default(),
                ),
            ])
        });
        let now = Utc::now();
        let existing = self
            .services
            .lists
            .accounts
            .list_by_user_id(&actor.id)
            .await?
            .into_iter()
            .find(|account| {
                account.provider == provider && account.external_user_id == sources.external_user_id
            });
        let _guard = if let Some(account) = &existing {
            Some(
                self.services
                    .lists
                    .account_runtime
                    .lock_account(&account.id)
                    .await,
            )
        } else {
            None
        };
        if expires_at <= Utc::now() {
            return Err(missing());
        }
        let current = self
            .services
            .identity
            .users
            .get_by_id(&actor.id)
            .await?
            .ok_or_else(missing)?;
        self.require_personal_lists_allowed(&current).await?;
        let account = UserListAccount {
            id: existing
                .as_ref()
                .map(|account| account.id.clone())
                .unwrap_or_else(|| Id::new().0),
            user_id: actor.id.clone(),
            provider: provider.into(),
            external_user_id: sources.external_user_id.clone(),
            username: sources.username.clone(),
            display_name: sources.display_name.clone(),
            credential: token,
            status: UserListAccountStatus::Active,
            error_message: None,
            linked_at: existing
                .as_ref()
                .map(|account| account.linked_at)
                .unwrap_or(now),
            last_used_at: None,
            last_refresh_at: None,
            updated_at: now,
        };
        let account = if existing.is_some() {
            self.services.lists.accounts.update(account).await?
        } else {
            self.services.lists.accounts.create(account).await?
        };
        self.append_domain_event(crate::events::domain_events::new_user_domain_event(
            actor,
            actor.id.clone(),
            scryer_domain::DomainEventPayload::ConfigurationChanged(
                scryer_domain::ConfigurationChangedEventData {
                    resource_type: "list_account".into(),
                    resource_id: Some(account.id.clone()),
                    action: if existing.is_some() {
                        scryer_domain::ConfigurationChangeAction::Updated
                    } else {
                        scryer_domain::ConfigurationChangeAction::Saved
                    },
                },
            ),
        ))
        .await?;
        Ok(ListAccountView { account, sources })
    }
    async fn list_account_identity(
        &self,
        provider: &str,
        token: &ListAccountCredential,
        account: Option<&UserListAccount>,
    ) -> AppResult<ListPluginAccountResponse> {
        let mut config = self.list_provider_config_for(provider).await;
        if let Some(id) = token.client_id.as_ref() {
            config.insert("client_id".into(), id.clone());
        }
        let client = self
            .services
            .lists
            .plugins
            .client_for_provider(provider, &config)
            .ok_or_else(|| AppError::Validation("list provider is unavailable".into()))?;
        let input = account.map(credential).unwrap_or_else(|| ListCredential {
            access_token: token.access_token.clone(),
            token_type: token.token_type.clone(),
            external_user_id: token.account_id.clone(),
            username: None,
        });
        match client.account(input).await {
            Ok(PluginResult::Ok(identity)) => Ok(identity),
            _ => Err(AppError::Validation(
                "list account identity could not be verified; reconnect the account".into(),
            )),
        }
    }
    /// Refresh account `id` for a caller that is waiting on the answer. The
    /// lock wait and the renewal are both bounded: a renewal still running
    /// after `INTERACTIVE_REFRESH_WAIT` finishes in the background, under the
    /// account lock, and the caller is told the account is still refreshing.
    /// `check` vets the stored row before any renewal. The returned guard keeps
    /// the account lock for the caller's remaining work.
    pub(crate) async fn refresh_list_account_interactive(
        &self,
        id: &str,
        check: impl FnOnce(&UserListAccount) -> AppResult<()>,
    ) -> AppResult<(tokio::sync::OwnedMutexGuard<()>, Option<UserListAccount>)> {
        let runtime = &self.services.lists.account_runtime;
        let guard = runtime
            .lock_account_within(id, INTERACTIVE_LOCK_WAIT)
            .await
            .ok_or_else(|| refresh_pending(None, Utc::now()))?;
        let Some(account) = self.services.lists.accounts.get_by_id(id).await? else {
            return Ok((guard, None));
        };
        check(&account)?;
        if account.status != UserListAccountStatus::Active || !needs_refresh(&account, Utc::now()) {
            let account = self.refresh_list_account_locked(account).await?;
            return Ok((guard, Some(account)));
        }
        let app = self.clone();
        // A renewal is never abandoned mid-flight: the provider may already
        // have spent the stored refresh credential.
        let renewal = tokio::spawn(async move {
            let result = app.refresh_list_account_locked(account).await;
            (guard, result)
        });
        match tokio::time::timeout(INTERACTIVE_REFRESH_WAIT, renewal).await {
            Ok(Ok((guard, result))) => Ok((guard, Some(result?))),
            Ok(Err(_)) => Err(AppError::Repository(
                "list account refresh task failed".into(),
            )),
            Err(_) => Err(refresh_pending(None, Utc::now())),
        }
    }
    pub(crate) async fn refresh_list_account_locked(
        &self,
        mut account: UserListAccount,
    ) -> AppResult<UserListAccount> {
        if account.status != UserListAccountStatus::Active {
            return Err(reconnect_this_account());
        }
        let now = Utc::now();
        if !needs_refresh(&account, now) {
            return Ok(account);
        }
        let runtime = &self.services.lists.account_runtime;
        // Callers hold the account lock, so concurrent refreshes cannot spend a
        // rotating refresh credential twice. Only an answer that the grant is
        // gone requires reconnect; any other failure leaves the account linked
        // and retries it after a growing pause, until the safety net below.
        if account
            .credential
            .refresh_expires_at
            .is_some_and(|deadline| deadline <= now)
        {
            self.mark_list_account_reconnect_required(account, now)
                .await?;
            return Err(reconnect_this_account());
        }
        if let Some(backoff) = runtime.refresh_backoff(&account.id, now) {
            return Err(refresh_pending(Some(backoff), now));
        }
        let app = account
            .credential
            .app_config
            .as_ref()
            .map(|values| ListProviderAppConfig {
                client_id: values.get("client_id").cloned().unwrap_or_default(),
                client_secret: values.get("client_secret").cloned().unwrap_or_default(),
                redirect_uri: values.get("redirect_uri").cloned().unwrap_or_default(),
                access_token: values.get("access_token").cloned(),
            });
        let previous = account.clone();
        let error = match self
            .services
            .lists
            .auth
            .renew(&account.provider, &account.credential, app.as_ref())
            .await
        {
            Ok(mut token) => {
                runtime.clear_refresh_backoff(&account.id);
                token.app_config = account.credential.app_config;
                account.credential = token;
                account.status = UserListAccountStatus::Active;
                account.error_message = None;
                account.last_refresh_at = Some(now);
                account.updated_at = now;
                return match self.services.lists.accounts.update(account).await {
                    Ok(account) => Ok(account),
                    Err(_) => {
                        // The provider has already spent the stored refresh
                        // credential; replaying it can never succeed.
                        let _ = self
                            .mark_list_account_reconnect_required(previous, now)
                            .await;
                        Err(reconnect_this_account())
                    }
                };
            }
            Err(error) => error,
        };
        let code = auth_failure_code(&error);
        if code.is_some_and(renew_requires_reconnect) {
            self.mark_list_account_reconnect_required(previous, now)
                .await?;
            return Err(reconnect_this_account());
        }
        let rate_limited = code == Some(RATE_LIMITED);
        let retry_after = match &error {
            AppError::TemporaryUnavailable { retry_after, .. } if rate_limited => *retry_after,
            _ => None,
        };
        let backoff = runtime.record_refresh_failure(&account.id, now, retry_after, rate_limited);
        let mut failures = account.credential.refresh_failures.unwrap_or(
            scryer_domain::ListAccountRefreshFailures {
                count: 0,
                since: now,
            },
        );
        failures.count = failures.count.saturating_add(1);
        if failures.count >= REFRESH_FAILURE_LIMIT
            && now - failures.since >= Duration::hours(REFRESH_FAILURE_MIN_SPAN_HOURS)
        {
            self.mark_list_account_reconnect_required(previous, now)
                .await?;
            return Err(reconnect_this_account());
        }
        account.credential.refresh_failures = Some(failures);
        account.updated_at = now;
        // Best effort: losing one count only delays the safety net.
        let _ = self.services.lists.accounts.update(account).await;
        if rate_limited {
            return Err(rate_limited_failure(retry_after));
        }
        Err(refresh_pending(Some(backoff), now))
    }
    async fn mark_list_account_reconnect_required(
        &self,
        mut account: UserListAccount,
        now: DateTime<Utc>,
    ) -> AppResult<UserListAccount> {
        account.status = UserListAccountStatus::Expired;
        account.error_message = Some("Reconnect this list account to resume syncing.".into());
        account.updated_at = now;
        self.services.lists.accounts.update(account).await
    }
    pub async fn unlink_list_account(&self, actor: &User, id: &str) -> AppResult<String> {
        // A renewal in flight holds the lock for up to the relay deadline. Waiting
        // for it, rather than unlinking beside it, means the revoke below reads
        // the credential that renewal saved instead of one it already spent.
        let guard = self
            .services
            .lists
            .account_runtime
            .lock_account_within(id, UNLINK_LOCK_WAIT)
            .await
            .ok_or_else(|| {
                AppError::temporary_unavailable(
                    "list account is refreshing; try unlinking again shortly",
                    Some(REFRESHING_RETRY_AFTER),
                )
            })?;
        let account = self.owned_list_account(actor, id).await?;
        // Reserve bounded task capacity before removing the only durable handle.
        let permit = self
            .services
            .lists
            .account_runtime
            .revocations
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| AppError::Repository("list account unlink unavailable".into()))?;
        let event = crate::events::domain_events::new_user_domain_event(
            actor,
            actor.id.clone(),
            scryer_domain::DomainEventPayload::ConfigurationChanged(
                scryer_domain::ConfigurationChangedEventData {
                    resource_type: "list_account".into(),
                    resource_id: Some(id.into()),
                    action: scryer_domain::ConfigurationChangeAction::Deleted,
                },
            ),
        );
        let app = self.clone();
        let gateway = self.services.lists.auth.clone();
        let id = id.to_owned();
        let owner = actor.id.clone();
        // Detaching the caller must not cancel a committed unlink's revoke attempt.
        tokio::spawn(async move {
            let _guard = guard;
            let _permit = permit;
            app.services.lists.accounts.unlink(&id, &owner).await?;
            // Never retry an ambiguous revoke or restore the erased local credential.
            let _ = tokio::time::timeout(
                std::time::Duration::from_secs(25),
                gateway.revoke(&account.provider, &account.credential, None),
            )
            .await;
            app.append_domain_event(event).await?;
            Ok::<String, AppError>(id)
        })
        .await
        .map_err(|_| AppError::Repository("list account unlink task failed".into()))?
    }
}

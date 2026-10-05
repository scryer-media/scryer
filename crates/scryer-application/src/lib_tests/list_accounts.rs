use super::*;
use crate::lists::account_transport::*;
use crate::lists::test_support::{MemoryListStore, subscription};
use crate::lists::{ListPluginProvider, ListProviderClient, UserListAccountRepository};
use scryer_domain::{ListAccountCredential, ListScope, UserListAccount, UserListAccountStatus};
use scryer_plugin_sdk::{
    ListCredential, ListPluginAccountResponse, ListPluginFetchRequest, ListPluginFetchResponse,
    ListPluginHealthResponse, PluginDescriptor, PluginResult,
};
use std::collections::BTreeMap;

#[derive(Default)]
struct Auth {
    exchanges: AtomicUsize,
    renewals: AtomicUsize,
    revocations: AtomicUsize,
    revocations_completed: AtomicUsize,
    block_revoke: bool,
    revoke_entered: Notify,
    revoke_release: Notify,
    revoke_finished: Notify,
    block_renew: bool,
    entered: Notify,
    release: Notify,
    renew_client: Mutex<Option<String>>,
    /// Failure codes answered by successive renewals before they succeed.
    renew_failures: std::sync::Mutex<Vec<&'static str>>,
    identity: Option<String>,
}
/// The pause a rate-limited fixture renewal asks for: longer than the
/// subscription interval, so a sync pause is observable.
const RATE_LIMIT_PAUSE: Duration = Duration::from_secs(12 * 3600);
fn token() -> ListAccountCredential {
    ListAccountCredential {
        access_token: "fixture-access".into(),
        refresh_handle: Some("fixture-rotated-handle".into()),
        expires_at: Some(chrono::Utc::now() + chrono::Duration::days(7)),
        ..Default::default()
    }
}
#[async_trait]
impl ListAccountAuthGateway for Auth {
    async fn start(&self, input: ListAccountStartRequest) -> AppResult<ListAccountStartResponse> {
        Ok(ListAccountStartResponse {
            authorize_url: format!("https://provider.invalid/authorize?state={}", input.state),
            poll_token: matches!(input.provider.as_str(), "plex" | "tmdb")
                .then(|| "fixture-pin".into()),
            expires_in: 600,
        })
    }
    async fn poll(
        &self,
        _: &str,
        _: &str,
        _: Option<&ListProviderAppConfig>,
    ) -> AppResult<Option<ListAccountCredential>> {
        self.exchanges.fetch_add(1, Ordering::SeqCst);
        Ok(Some(token()))
    }
    async fn complete(
        &self,
        input: ListAccountCompleteRequest,
    ) -> AppResult<ListAccountCredential> {
        self.exchanges.fetch_add(1, Ordering::SeqCst);
        let mut token = token();
        token.direct = input.app.is_some();
        token.account_id = self.identity.clone();
        Ok(token)
    }
    async fn renew(
        &self,
        _: &str,
        _: &ListAccountCredential,
        app: Option<&ListProviderAppConfig>,
    ) -> AppResult<ListAccountCredential> {
        self.renewals.fetch_add(1, Ordering::SeqCst);
        *self.renew_client.lock().await = app.map(|app| app.client_id.clone());
        self.entered.notify_one();
        if self.block_renew {
            timeout(Duration::from_secs(30), self.release.notified())
                .await
                .expect("renew released");
        }
        let failure = {
            let mut failures = self.renew_failures.lock().unwrap();
            (!failures.is_empty()).then(|| failures.remove(0))
        };
        match failure {
            Some(RATE_LIMITED) => Err(rate_limited_failure(Some(RATE_LIMIT_PAUSE))),
            Some(code) => Err(auth_failure(code)),
            None => Ok(token()),
        }
    }
    async fn revoke(
        &self,
        _: &str,
        _: &ListAccountCredential,
        _: Option<&ListProviderAppConfig>,
    ) -> AppResult<()> {
        self.revocations.fetch_add(1, Ordering::SeqCst);
        self.revoke_entered.notify_one();
        if self.block_revoke {
            timeout(Duration::from_secs(30), self.revoke_release.notified())
                .await
                .expect("revoke released");
        }
        self.revocations_completed.fetch_add(1, Ordering::SeqCst);
        self.revoke_finished.notify_one();
        Ok(())
    }
}
struct Client {
    descriptor: PluginDescriptor,
    fetch_gate: Option<Arc<FetchGate>>,
}
#[derive(Default)]
struct FetchGate {
    entered: Notify,
    release: Notify,
    fetches: AtomicUsize,
}
#[async_trait]
impl ListProviderClient for Client {
    fn descriptor(&self) -> &PluginDescriptor {
        &self.descriptor
    }
    async fn fetch(
        &self,
        _: ListPluginFetchRequest,
    ) -> AppResult<PluginResult<ListPluginFetchResponse>> {
        if let Some(gate) = &self.fetch_gate {
            if gate.fetches.fetch_add(1, Ordering::SeqCst) == 0 {
                gate.entered.notify_one();
                timeout(Duration::from_secs(30), gate.release.notified())
                    .await
                    .expect("fetch released");
            }
            let mut item = crate::lists::test_support::plugin_item("blocked-item");
            item.external_ids = vec![scryer_plugin_sdk::ListExternalId {
                source: "tvdb".into(),
                kind: None,
                id: "9055".into(),
            }];
            return Ok(PluginResult::Ok(ListPluginFetchResponse {
                items: vec![item],
                ..Default::default()
            }));
        }
        Ok(PluginResult::Ok(Default::default()))
    }
    async fn account(
        &self,
        input: ListCredential,
    ) -> AppResult<PluginResult<ListPluginAccountResponse>> {
        Ok(PluginResult::Ok(ListPluginAccountResponse {
            external_user_id: input
                .external_user_id
                .unwrap_or_else(|| "external-one".into()),
            username: "fixture-member".into(),
            ..Default::default()
        }))
    }
    async fn health(&self) -> AppResult<PluginResult<ListPluginHealthResponse>> {
        Ok(PluginResult::Ok(Default::default()))
    }
}
struct Plugins {
    clients: Vec<Arc<Client>>,
}
impl Plugins {
    fn new() -> Self {
        let clients=["trakt","plex","tmdb","anilist","mal","simkl"].into_iter().map(|provider|Arc::new(Client {fetch_gate:None,descriptor:serde_json::from_value(serde_json::json!({"id":format!("{provider}-list"),"name":provider,"version":"1.0.0","sdk_version":scryer_plugin_sdk::SDK_VERSION,"sdk_constraint":scryer_plugin_sdk::current_sdk_constraint(),"provider":{"kind":"list_provider","provider_type":provider,"auth":{"type":"member_account","flow":{"type":"authorization_code","pkce":true},"exchange":"smg_relay"},"capabilities":{"account":true},"groups":[{"label":"Personal","auth_badge":"member_account","items":[{"id":"watchlist","name":"Watchlist","kinds":["movie"],"source_type":"watchlist","personal":true,"default_interval_seconds":3600}]}]}})).unwrap()})).collect();
        Self { clients }
    }
}
impl ListPluginProvider for Plugins {
    fn client_for_provider(
        &self,
        provider: &str,
        _: &BTreeMap<String, String>,
    ) -> Option<Arc<dyn ListProviderClient>> {
        self.clients
            .iter()
            .find(|client| client.descriptor.provider_type() == provider)
            .map(|client| client.clone() as Arc<dyn ListProviderClient>)
    }
    fn descriptors(&self) -> Vec<PluginDescriptor> {
        self.clients
            .iter()
            .map(|client| client.descriptor.clone())
            .collect()
    }
    fn available_provider_types(&self) -> Vec<String> {
        self.clients
            .iter()
            .map(|client| client.descriptor.provider_type().into())
            .collect()
    }
}
async fn harness(auth: Arc<Auth>) -> MediaRequestTestHarness {
    let mut harness = bootstrap_media_request_app_with_list_plugins(Arc::new(Plugins::new()));
    harness.app.services.lists.auth = auth;
    harness.users.create(harness.user.clone()).await.unwrap();
    harness.users.create(harness.manager.clone()).await.unwrap();
    super::list_experimental_gate::set_experimental_features(&harness, true).await;
    harness
}
async fn seed_expired(harness: &MediaRequestTestHarness) -> UserListAccount {
    let now = chrono::Utc::now();
    let account = UserListAccount {
        id: "linked-account".into(),
        user_id: harness.user.id.clone(),
        provider: "trakt".into(),
        external_user_id: "external-one".into(),
        username: "fixture-member".into(),
        display_name: None,
        credential: ListAccountCredential {
            access_token: "fixture-old-access".into(),
            refresh_handle: Some("fixture-old-handle".into()),
            expires_at: Some(now - chrono::Duration::seconds(1)),
            app_config: Some(BTreeMap::from([
                ("client_id".into(), "issuing-client".into()),
                ("client_secret".into(), "issuing-secret".into()),
                (
                    "redirect_uri".into(),
                    "https://instance.invalid/lists/oauth/callback".into(),
                ),
            ])),
            direct: true,
            ..Default::default()
        },
        status: UserListAccountStatus::Active,
        error_message: None,
        linked_at: now,
        last_used_at: None,
        last_refresh_at: None,
        updated_at: now,
    };
    UserListAccountRepository::create(harness.lists.as_ref(), account.clone())
        .await
        .unwrap();
    account
}
#[tokio::test]
async fn private_account_start_complete_and_poll_store_verified_owner_accounts() {
    let auth = Arc::new(Auth::default());
    let harness = harness(auth.clone()).await;
    for provider in ["trakt", "anilist", "mal", "plex", "tmdb"] {
        let start = harness
            .app
            .start_list_account_link(&harness.user, provider, "https://instance.invalid")
            .await
            .unwrap();
        let view = if start.poll_required {
            harness
                .app
                .poll_list_account_link(&harness.user, &start.session_id)
                .await
                .unwrap()
                .account
                .unwrap()
        } else {
            harness
                .app
                .complete_list_account_link(
                    &harness.user,
                    &start.session_id,
                    &start.state,
                    provider,
                    "fixture-code",
                    None,
                )
                .await
                .unwrap()
        };
        assert_eq!(view.account.user_id, harness.user.id);
        assert_eq!(view.account.external_user_id, "external-one");
        assert_eq!(
            UserListAccountRepository::get_by_id(harness.lists.as_ref(), &view.account.id)
                .await
                .unwrap()
                .unwrap()
                .credential
                .refresh_handle
                .as_deref(),
            Some("fixture-rotated-handle")
        );
    }
    assert_eq!(auth.exchanges.load(Ordering::SeqCst), 5);
    assert!(
        harness
            .app
            .my_list_accounts(&harness.manager)
            .await
            .unwrap()
            .is_empty()
    );
}
#[tokio::test]
async fn private_account_completion_is_single_use_and_gate_rechecked() {
    let auth = Arc::new(Auth::default());
    let harness = harness(auth.clone()).await;
    let start = harness
        .app
        .start_list_account_link(&harness.user, "trakt", "https://instance.invalid")
        .await
        .unwrap();
    assert!(
        harness
            .app
            .complete_list_account_link(
                &harness.manager,
                &start.session_id,
                &start.state,
                "trakt",
                "fixture-code",
                None
            )
            .await
            .is_err()
    );
    let (first, second) = tokio::join!(
        harness.app.complete_list_account_link(
            &harness.user,
            &start.session_id,
            &start.state,
            "trakt",
            "fixture-code",
            None
        ),
        harness.app.complete_list_account_link(
            &harness.user,
            &start.session_id,
            &start.state,
            "trakt",
            "fixture-code",
            None
        )
    );
    assert_ne!(first.is_ok(), second.is_ok());
    assert_eq!(auth.exchanges.load(Ordering::SeqCst), 1);
    let pending = harness
        .app
        .start_list_account_link(&harness.user, "trakt", "https://instance.invalid")
        .await
        .unwrap();
    super::list_experimental_gate::set_experimental_features(&harness, false).await;
    assert!(
        harness
            .app
            .complete_list_account_link(
                &harness.user,
                &pending.session_id,
                &pending.state,
                "trakt",
                "fixture-code",
                None
            )
            .await
            .is_err()
    );
    assert_eq!(auth.exchanges.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn private_account_concurrent_ui_and_sync_refresh_once_and_keep_issuing_app() {
    let auth = Arc::new(Auth::default());
    let harness = harness(auth.clone()).await;
    let account = seed_expired(&harness).await;
    let mut list = subscription("private-follow");
    list.scope = ListScope::Personal;
    list.owner_user_id = harness.user.id.clone();
    list.credential_id = Some(account.id.clone());
    list.source.provider = "trakt".into();
    list.sync.next_at = Some(chrono::Utc::now());
    crate::lists::ListSubscriptionRepository::create(harness.lists.as_ref(), list)
        .await
        .unwrap();
    harness
        .app
        .update_list_provider_app(
            &harness.manager,
            "trakt",
            Some("replacement-client".into()),
            Some("replacement-secret".into()),
            Some("https://instance.invalid/lists/oauth/callback".into()),
            true,
        )
        .await
        .unwrap();
    let (view, report) = timeout(Duration::from_secs(30), async {
        tokio::join!(
            harness.app.list_account(&harness.user, &account.id),
            harness.app.run_list_sync_job(None)
        )
    })
    .await
    .expect("bounded concurrent operations");
    view.unwrap();
    report.unwrap();
    assert_eq!(auth.renewals.load(Ordering::SeqCst), 1);
    assert_eq!(
        auth.renew_client.lock().await.as_deref(),
        Some("issuing-client")
    );
}
struct FailRotatedWrite {
    store: Arc<MemoryListStore>,
    /// Writes of an Active account that fail before writes succeed again.
    failing_active_writes: AtomicUsize,
}
#[async_trait]
impl UserListAccountRepository for FailRotatedWrite {
    async fn create(&self, account: UserListAccount) -> AppResult<UserListAccount> {
        UserListAccountRepository::create(self.store.as_ref(), account).await
    }
    async fn update(&self, account: UserListAccount) -> AppResult<UserListAccount> {
        if account.status == UserListAccountStatus::Active
            && self
                .failing_active_writes
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
                    left.checked_sub(1)
                })
                .is_ok()
        {
            Err(AppError::Repository(
                "injected rotated write failure".into(),
            ))
        } else {
            UserListAccountRepository::update(self.store.as_ref(), account).await
        }
    }
    async fn get_by_id(&self, id: &str) -> AppResult<Option<UserListAccount>> {
        UserListAccountRepository::get_by_id(self.store.as_ref(), id).await
    }
    async fn list_by_user_id(&self, id: &str) -> AppResult<Vec<UserListAccount>> {
        self.store.list_by_user_id(id).await
    }
    async fn delete(&self, id: &str) -> AppResult<()> {
        UserListAccountRepository::delete(self.store.as_ref(), id).await
    }
}
#[tokio::test]
async fn private_account_failed_rotated_write_never_replays_old_handle() {
    let auth = Arc::new(Auth::default());
    let mut harness = harness(auth.clone()).await;
    let account = seed_expired(&harness).await;
    harness.app.services.lists.accounts = Arc::new(FailRotatedWrite {
        store: harness.lists.clone(),
        failing_active_writes: AtomicUsize::new(usize::MAX),
    });
    // The caller learns the account needs reconnecting, not that storage failed.
    match harness.app.list_account(&harness.user, &account.id).await {
        Err(AppError::Validation(message)) => assert_eq!(message, "reconnect this list account"),
        Err(other) => panic!("expected the reconnect outcome, got {other:?}"),
        Ok(_) => panic!("expected the reconnect outcome"),
    }
    assert_eq!(
        UserListAccountRepository::get_by_id(harness.lists.as_ref(), &account.id)
            .await
            .unwrap()
            .unwrap()
            .status,
        UserListAccountStatus::Expired
    );
    assert!(
        harness
            .app
            .list_account(&harness.user, &account.id)
            .await
            .is_err()
    );
    assert_eq!(auth.renewals.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn private_account_renewed_credential_survives_a_failed_save() {
    let auth = Arc::new(Auth::default());
    let mut harness = harness(auth.clone()).await;
    let account = seed_expired(&harness).await;
    harness.app.services.lists.accounts = Arc::new(FailRotatedWrite {
        store: harness.lists.clone(),
        failing_active_writes: AtomicUsize::new(2),
    });
    let view = harness
        .app
        .list_account(&harness.user, &account.id)
        .await
        .unwrap();
    assert_eq!(view.account.status, UserListAccountStatus::Active);
    let stored = UserListAccountRepository::get_by_id(harness.lists.as_ref(), &account.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.status, UserListAccountStatus::Active);
    assert_eq!(
        stored.credential.refresh_handle.as_deref(),
        Some("fixture-rotated-handle")
    );
    assert_eq!(auth.renewals.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn private_account_cancelled_sync_renewal_never_replays_the_old_handle() {
    let auth = Arc::new(Auth {
        block_renew: true,
        ..Default::default()
    });
    let harness = harness(auth.clone()).await;
    let account = seed_expired(&harness).await;
    follow_private_list(&harness, &account).await;
    let app = harness.app.clone();
    let sync = tokio::spawn(async move { app.run_list_sync_job(None).await });
    timeout(Duration::from_secs(30), auth.entered.notified())
        .await
        .expect("sync renewal entered");
    sync.abort();
    assert!(
        timeout(Duration::from_secs(30), sync)
            .await
            .expect("cancelled sync finished")
            .unwrap_err()
            .is_cancelled()
    );
    // The inline renewal was abandoned with the sync: the account keeps its
    // stored grant, nothing rotated was lost, and the lock is free again.
    drop(
        timeout(
            Duration::from_secs(30),
            harness
                .app
                .services
                .lists
                .account_runtime
                .lock_account(&account.id),
        )
        .await
        .expect("cancelled sync released the lock"),
    );
    let stored = UserListAccountRepository::get_by_id(harness.lists.as_ref(), &account.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.status, UserListAccountStatus::Active);
    assert_eq!(
        stored.credential.refresh_handle.as_deref(),
        Some("fixture-old-handle")
    );
    assert_eq!(auth.renewals.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn private_account_cancelled_view_finishes_its_renewal_then_releases_the_lock() {
    let auth = Arc::new(Auth {
        block_renew: true,
        ..Default::default()
    });
    let harness = harness(auth.clone()).await;
    let account = seed_expired(&harness).await;
    {
        let mut pending = Box::pin(harness.app.list_account(&harness.user, &account.id));
        timeout(Duration::from_secs(30),async {tokio::select! {result=&mut pending=>panic!("renewal completed before cancellation: {}",result.is_ok()),_=auth.entered.notified()=>{}}}).await.expect("renewal entered");
    }
    // Dropping the caller does not abandon the renewal mid-flight.
    let stored = UserListAccountRepository::get_by_id(harness.lists.as_ref(), &account.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.status, UserListAccountStatus::Active);
    assert_eq!(
        stored.credential.refresh_handle.as_deref(),
        Some("fixture-old-handle")
    );
    auth.release.notify_one();
    drop(
        timeout(
            Duration::from_secs(30),
            harness
                .app
                .services
                .lists
                .account_runtime
                .lock_account(&account.id),
        )
        .await
        .expect("background renewal released the lock"),
    );
    let stored = UserListAccountRepository::get_by_id(harness.lists.as_ref(), &account.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.status, UserListAccountStatus::Active);
    assert_eq!(
        stored.credential.refresh_handle.as_deref(),
        Some("fixture-rotated-handle")
    );
    assert_eq!(auth.renewals.load(Ordering::SeqCst), 1);
}

async fn follow_private_list(
    harness: &MediaRequestTestHarness,
    account: &UserListAccount,
) -> String {
    let mut list = subscription("private-follow");
    list.scope = ListScope::Personal;
    list.owner_user_id = harness.user.id.clone();
    list.credential_id = Some(account.id.clone());
    list.source.provider = "trakt".into();
    list.sync.next_at = Some(chrono::Utc::now());
    crate::lists::ListSubscriptionRepository::create(harness.lists.as_ref(), list)
        .await
        .unwrap()
        .id
}

#[tokio::test]
async fn private_account_transient_refresh_failures_keep_the_account_linked_and_retry() {
    for code in [
        "provider_unavailable",
        "rate_limited",
        "busy",
        "transport_unavailable",
        "relay_unavailable",
    ] {
        assert_ne!(auth_failure_class(code), AuthFailureClass::Final, "{code}");
        let auth = Arc::new(Auth {
            renew_failures: std::sync::Mutex::new(vec![code]),
            ..Default::default()
        });
        let harness = harness(auth.clone()).await;
        let account = seed_expired(&harness).await;
        let follow = follow_private_list(&harness, &account).await;

        let report = harness.app.run_list_sync_job(None).await.unwrap();
        assert_eq!(report.failed, 1, "{code}");
        let stored = UserListAccountRepository::get_by_id(harness.lists.as_ref(), &account.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.status, UserListAccountStatus::Active, "{code}");
        assert_eq!(stored.error_message, None, "{code}");
        assert_eq!(
            stored.credential.refresh_handle.as_deref(),
            Some("fixture-old-handle"),
            "{code}"
        );
        let sync =
            crate::lists::ListSubscriptionRepository::get_by_id(harness.lists.as_ref(), &follow)
                .await
                .unwrap()
                .unwrap()
                .sync;
        assert_eq!(sync.state, scryer_domain::ListSyncState::Fail, "{code}");
        assert!(sync.next_at.is_some(), "{code}");
        assert_eq!(
            sync.paused_until.is_some(),
            code == RATE_LIMITED,
            "{code}: only a rate limit pauses the subscription"
        );
        // Only the provider's own answer counts towards the safety net; the
        // local request slots, a connection with no answer and the relay's
        // own trouble do not.
        let counted = matches!(code, "provider_unavailable" | RATE_LIMITED);
        assert_eq!(
            stored
                .credential
                .refresh_failures
                .map(|failures| failures.count),
            counted.then_some(1),
            "{code}"
        );

        // Within the backoff the account answers without renewing again.
        let error = harness
            .app
            .list_account(&harness.user, &account.id)
            .await
            .err()
            .expect("the backoff answers with an error");
        assert_eq!(
            auth_failure_code(&error) == Some(RATE_LIMITED),
            code == RATE_LIMITED,
            "{code}"
        );
        match error {
            AppError::TemporaryUnavailable { retry_after, .. } => {
                let retry_after = retry_after.expect("a retry hint");
                assert!(retry_after > Duration::ZERO, "{code}");
                if code == RATE_LIMITED {
                    assert!(retry_after > Duration::from_secs(5 * 60), "{code}");
                }
            }
            other => panic!("{code}: expected a temporary failure, got {other:?}"),
        }
        assert_eq!(auth.renewals.load(Ordering::SeqCst), 1, "{code}");

        // Once the pause passes, the next attempt renews again and recovers.
        harness
            .app
            .services
            .lists
            .account_runtime
            .clear_refresh_backoff(&account.id);
        let view = harness
            .app
            .list_account(&harness.user, &account.id)
            .await
            .unwrap();
        assert_eq!(view.account.status, UserListAccountStatus::Active, "{code}");
        assert_eq!(
            view.account.credential.refresh_handle.as_deref(),
            Some("fixture-rotated-handle"),
            "{code}"
        );
        assert_eq!(view.account.credential.refresh_failures, None, "{code}");
        assert_eq!(auth.renewals.load(Ordering::SeqCst), 2, "{code}");
    }
}

#[test]
fn every_auth_failure_code_has_a_pinned_class() {
    for &code in AUTH_FAILURE_CODES {
        let expected = match code {
            "provider_unavailable" | "relay_unavailable" | "busy" | "transport_unavailable" => {
                AuthFailureClass::Transient
            }
            RATE_LIMITED => AuthFailureClass::RateLimited,
            _ => AuthFailureClass::Final,
        };
        assert_eq!(auth_failure_class(code), expected, "{code}");
    }
    assert!(AUTH_FAILURE_CODES.contains(&"relay_unavailable"));
    assert!(!renew_requires_reconnect("relay_unavailable"));
}

#[tokio::test]
async fn private_account_refusals_on_renew_require_reconnect() {
    for code in [RECONNECT_REQUIRED, PROVIDER_REJECTED, ACCESS_DENIED] {
        let auth = Arc::new(Auth {
            renew_failures: std::sync::Mutex::new(vec![code]),
            ..Default::default()
        });
        let harness = harness(auth.clone()).await;
        let account = seed_expired(&harness).await;
        assert!(
            harness
                .app
                .list_account(&harness.user, &account.id)
                .await
                .is_err()
        );
        let stored = UserListAccountRepository::get_by_id(harness.lists.as_ref(), &account.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.status, UserListAccountStatus::Expired, "{code}");
    }
    // Relay and configuration faults leave the account linked.
    for code in [
        "provider_not_configured",
        "instance_auth_required",
        "invalid_request",
        "unsupported_provider",
        "invalid_provider_response",
    ] {
        let auth = Arc::new(Auth {
            renew_failures: std::sync::Mutex::new(vec![code]),
            ..Default::default()
        });
        let harness = harness(auth.clone()).await;
        let account = seed_expired(&harness).await;
        assert!(
            harness
                .app
                .list_account(&harness.user, &account.id)
                .await
                .is_err()
        );
        let stored = UserListAccountRepository::get_by_id(harness.lists.as_ref(), &account.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.status, UserListAccountStatus::Active, "{code}");
    }
}

#[tokio::test]
async fn private_account_safety_net_expires_a_credential_that_keeps_failing_for_long_enough() {
    let now = chrono::Utc::now();
    // (prior failures, first failure age, expected status)
    // (prior failures, first failure age, renew answer, expected status, count after)
    let cases = [
        (
            11,
            chrono::Duration::hours(37),
            "provider_unavailable",
            UserListAccountStatus::Expired,
            0,
        ),
        (
            11,
            chrono::Duration::hours(1),
            "provider_unavailable",
            UserListAccountStatus::Active,
            12,
        ),
        (
            3,
            chrono::Duration::hours(72),
            "provider_unavailable",
            UserListAccountStatus::Active,
            4,
        ),
        // Scryer-side trouble leaves the count alone, however long it lasts.
        (
            11,
            chrono::Duration::hours(37),
            "transport_unavailable",
            UserListAccountStatus::Active,
            11,
        ),
        (
            11,
            chrono::Duration::hours(37),
            "busy",
            UserListAccountStatus::Active,
            11,
        ),
        (
            11,
            chrono::Duration::hours(37),
            "instance_auth_required",
            UserListAccountStatus::Active,
            11,
        ),
        (
            11,
            chrono::Duration::hours(37),
            "relay_unavailable",
            UserListAccountStatus::Active,
            11,
        ),
    ];
    for (count, age, answer, expected, count_after) in cases {
        let auth = Arc::new(Auth {
            renew_failures: std::sync::Mutex::new(vec![answer]),
            ..Default::default()
        });
        let harness = harness(auth.clone()).await;
        let mut account = seed_expired(&harness).await;
        account.credential.refresh_failures = Some(scryer_domain::ListAccountRefreshFailures {
            count,
            since: now - age,
        });
        UserListAccountRepository::update(harness.lists.as_ref(), account.clone())
            .await
            .unwrap();
        assert!(
            harness
                .app
                .list_account(&harness.user, &account.id)
                .await
                .is_err()
        );
        let stored = UserListAccountRepository::get_by_id(harness.lists.as_ref(), &account.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            stored.status, expected,
            "{count} failures over {age}, {answer}"
        );
        if expected == UserListAccountStatus::Active {
            assert_eq!(
                stored
                    .credential
                    .refresh_failures
                    .map(|failures| failures.count),
                Some(count_after),
                "{answer}"
            );
        }
        assert_eq!(auth.renewals.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn private_account_past_local_refresh_deadline_still_asks_the_provider() {
    let auth = Arc::new(Auth::default());
    let harness = harness(auth.clone()).await;
    let mut account = seed_expired(&harness).await;
    account.credential.refresh_expires_at = Some(chrono::Utc::now() - chrono::Duration::minutes(1));
    UserListAccountRepository::update(harness.lists.as_ref(), account.clone())
        .await
        .unwrap();
    assert!(
        harness
            .app
            .list_account(&harness.user, &account.id)
            .await
            .is_ok()
    );
    let stored = UserListAccountRepository::get_by_id(harness.lists.as_ref(), &account.id)
        .await
        .unwrap()
        .unwrap();
    // A fast host clock must not expire a good grant: the provider decides.
    assert_eq!(stored.status, UserListAccountStatus::Active);
    assert_eq!(
        stored.credential.refresh_handle.as_deref(),
        Some("fixture-rotated-handle")
    );
    assert_eq!(auth.renewals.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn private_account_view_and_unlink_do_not_wait_out_a_slow_renewal() {
    let auth = Arc::new(Auth {
        block_renew: true,
        ..Default::default()
    });
    let harness = harness(auth.clone()).await;
    let account = seed_expired(&harness).await;

    // The view answers that the account is still refreshing instead of
    // waiting on the provider.
    let error = harness
        .app
        .list_account(&harness.user, &account.id)
        .await
        .err()
        .expect("the view does not wait out the renewal");
    assert!(
        matches!(
            error,
            AppError::TemporaryUnavailable {
                retry_after: Some(_),
                ..
            }
        ),
        "{error:?}"
    );
    // An unlink gives up waiting for the lock the renewal holds, rather than
    // revoking beside it; the account stays linked.
    assert!(matches!(
        harness
            .app
            .unlink_list_account(&harness.user, &account.id)
            .await,
        Err(AppError::TemporaryUnavailable { .. })
    ));
    assert_eq!(
        harness
            .app
            .my_list_accounts(&harness.user)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(auth.renewals.load(Ordering::SeqCst), 1);

    // Once the renewal finishes and saves, unlink proceeds and revokes the
    // credential it saved.
    auth.release.notify_one();
    harness
        .app
        .unlink_list_account(&harness.user, &account.id)
        .await
        .unwrap();
    assert!(
        harness
            .app
            .my_list_accounts(&harness.user)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(auth.renewals.load(Ordering::SeqCst), 1);
    assert_eq!(auth.revocations.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn private_account_refused_grant_requires_reconnect_without_replay() {
    let auth = Arc::new(Auth {
        renew_failures: std::sync::Mutex::new(vec![RECONNECT_REQUIRED]),
        ..Default::default()
    });
    let harness = harness(auth.clone()).await;
    let account = seed_expired(&harness).await;
    let follow = follow_private_list(&harness, &account).await;

    let report = harness.app.run_list_sync_job(None).await.unwrap();
    assert_eq!(report.failed, 1);
    let stored = UserListAccountRepository::get_by_id(harness.lists.as_ref(), &account.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.status, UserListAccountStatus::Expired);
    assert!(stored.error_message.is_some());
    let sync = crate::lists::ListSubscriptionRepository::get_by_id(harness.lists.as_ref(), &follow)
        .await
        .unwrap()
        .unwrap()
        .sync;
    assert_eq!(sync.state, scryer_domain::ListSyncState::Fail);

    assert!(
        harness
            .app
            .list_account(&harness.user, &account.id)
            .await
            .is_err()
    );
    assert_eq!(auth.renewals.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn private_account_unlink_revokes_even_when_event_write_fails_and_gate_is_off() {
    let auth = Arc::new(Auth::default());
    let harness = harness(auth.clone()).await;
    let account = seed_expired(&harness).await;
    let mut list = subscription("private-follow");
    list.scope = ListScope::Personal;
    list.owner_user_id = harness.user.id.clone();
    list.credential_id = Some(account.id.clone());
    crate::lists::ListSubscriptionRepository::create(harness.lists.as_ref(), list)
        .await
        .unwrap();
    super::list_experimental_gate::set_experimental_features(&harness, false).await;
    assert_eq!(
        harness
            .app
            .my_list_accounts(&harness.user)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        harness
            .app
            .my_list_subscriptions(&harness.user)
            .await
            .unwrap()
            .len(),
        1
    );
    harness
        .domain_events
        .fail_append
        .store(true, Ordering::SeqCst);
    assert!(
        harness
            .app
            .unlink_list_account(&harness.user, &account.id)
            .await
            .is_err()
    );
    assert_eq!(auth.revocations.load(Ordering::SeqCst), 1);
    assert!(
        harness
            .app
            .my_list_accounts(&harness.user)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        harness
            .app
            .my_list_subscriptions(&harness.user)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn private_account_disabled_owner_sync_skips_refresh_and_provider_fetch() {
    let auth = Arc::new(Auth::default());
    let harness = harness(auth.clone()).await;
    let account = seed_expired(&harness).await;
    let mut list = subscription("private-follow");
    list.scope = ListScope::Personal;
    list.owner_user_id = harness.user.id.clone();
    list.credential_id = Some(account.id.clone());
    list.source.provider = "trakt".into();
    list.sync.next_at = Some(chrono::Utc::now());
    crate::lists::ListSubscriptionRepository::create(harness.lists.as_ref(), list)
        .await
        .unwrap();
    harness
        .users
        .store
        .lock()
        .await
        .iter_mut()
        .find(|user| user.id == harness.user.id)
        .unwrap()
        .set_login_status(scryer_domain::UserLoginStatus::Disabled);
    let report = harness.app.run_list_sync_job(None).await.unwrap();
    assert_eq!(report.off, 1);
    assert_eq!(auth.renewals.load(Ordering::SeqCst), 0);
    assert!(harness.media_requests.requests.lock().await.is_empty());
}

#[tokio::test]
async fn private_account_byo_links_snapshot_registration_and_rejects_simkl_override() {
    let auth = Arc::new(Auth::default());
    let harness = harness(auth).await;
    for provider in ["trakt", "anilist", "mal"] {
        harness
            .app
            .update_list_provider_app(
                &harness.manager,
                provider,
                Some("issuing-client".into()),
                Some("issuing-secret".into()),
                Some("https://instance.invalid/lists/oauth/callback".into()),
                true,
            )
            .await
            .unwrap();
        let start = harness
            .app
            .start_list_account_link(&harness.user, provider, "https://instance.invalid")
            .await
            .unwrap();
        assert_eq!(start.authorization_origin, "https://instance.invalid");
        let linked = harness
            .app
            .complete_list_account_link(
                &harness.user,
                &start.session_id,
                &start.state,
                provider,
                "fixture-code",
                None,
            )
            .await
            .unwrap();
        assert!(linked.account.credential.direct);
        assert_eq!(
            linked.account.credential.app_config.as_ref().unwrap()["client_id"],
            "issuing-client"
        );
    }
    assert!(
        harness
            .app
            .update_list_provider_app(
                &harness.manager,
                "simkl",
                Some("client".into()),
                Some("secret".into()),
                Some("https://instance.invalid/lists/oauth/callback".into()),
                true
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn private_account_reconnect_different_identity_does_not_rebind_existing_follow() {
    let auth = Arc::new(Auth {
        identity: Some("external-two".into()),
        ..Default::default()
    });
    let harness = harness(auth).await;
    let account = seed_expired(&harness).await;
    let mut list = subscription("private-follow");
    list.scope = ListScope::Personal;
    list.owner_user_id = harness.user.id.clone();
    list.credential_id = Some(account.id.clone());
    list.source.provider = "trakt".into();
    crate::lists::ListSubscriptionRepository::create(harness.lists.as_ref(), list)
        .await
        .unwrap();
    let start = harness
        .app
        .start_list_account_link(&harness.user, "trakt", "https://instance.invalid")
        .await
        .unwrap();
    let linked = harness
        .app
        .complete_list_account_link(
            &harness.user,
            &start.session_id,
            &start.state,
            "trakt",
            "fixture-code",
            None,
        )
        .await
        .unwrap();
    assert_ne!(linked.account.id, account.id);
    assert_eq!(linked.account.external_user_id, "external-two");
    assert_eq!(
        harness
            .app
            .my_list_subscriptions(&harness.user)
            .await
            .unwrap()[0]
            .credential_id
            .as_deref(),
        Some(account.id.as_str())
    );
}

#[tokio::test]
async fn private_follow_lifecycle_waits_for_fetch_then_stale_sync_snapshot_is_skipped() {
    use crate::lists::ListSubscriptionRepository;
    use crate::lists::sync::{ListSyncContext, SubscriptionSyncOutcome, sync_subscription};
    use crate::lists::test_support::{FixtureResolver, ScriptedCharts, route};
    use std::future::Future;
    use std::task::Poll;
    for unfollow in [false, true] {
        let mut harness = harness(Arc::new(Auth::default())).await;
        let account = seed_expired(&harness).await;
        let gate = Arc::new(FetchGate::default());
        let mut plugins = Plugins::new();
        for client in &mut plugins.clients {
            Arc::get_mut(client).unwrap().fetch_gate = Some(gate.clone());
        }
        harness.app.services.lists.plugins = Arc::new(plugins);
        let mut snapshot = subscription("private-follow");
        snapshot.scope = ListScope::Personal;
        snapshot.owner_user_id = harness.user.id.clone();
        snapshot.credential_id = Some(account.id.clone());
        snapshot.source.provider = "trakt".into();
        snapshot.mode = scryer_domain::ListMode::Request;
        snapshot.routes = vec![route(
            MediaFacet::Movie,
            &scryer_domain::default_library_id_for_facet(&MediaFacet::Movie),
        )];
        snapshot.sync.next_at = Some(chrono::Utc::now());
        ListSubscriptionRepository::create(harness.lists.as_ref(), snapshot.clone())
            .await
            .unwrap();
        let resolver = FixtureResolver::default();
        let charts = ScriptedCharts::default();
        let configs = crate::lists::provider_settings::ListProviderConfigs::default();
        let actions = crate::lists::AppListActions::new(&harness.app);
        let context = ListSyncContext {
            subscriptions: harness.lists.as_ref(),
            memberships: harness.lists.as_ref(),
            exclusions: harness.lists.as_ref(),
            accounts: harness.lists.as_ref(),
            policies: harness.lists.as_ref(),
            plugins: harness.app.services.lists.plugins.as_ref(),
            charts: &charts,
            resolver: &resolver,
            actions: &actions,
            provider_configs: &configs,
        };
        let mut syncing = Box::pin(sync_subscription(
            &context,
            &snapshot,
            chrono::Utc::now(),
            None,
        ));
        timeout(Duration::from_secs(30),async {tokio::select! { result=&mut syncing=>panic!("sync completed before controlled fetch: {}",result.is_ok()),_=gate.entered.notified()=>{} }}).await.expect("fetch entered");
        let mut lifecycle = Box::pin(async {
            if unfollow {
                harness
                    .app
                    .unsubscribe_visible_list(&harness.user, &snapshot.id)
                    .await
                    .map(|_| ())
            } else {
                harness
                    .app
                    .set_visible_list_enabled(&harness.user, &snapshot.id, false)
                    .await
                    .map(|_| ())
            }
        });
        std::future::poll_fn(|cx| {
            assert!(
                matches!(lifecycle.as_mut().poll(cx), Poll::Pending),
                "lifecycle must wait behind the credentialed fetch"
            );
            Poll::Ready(())
        })
        .await;
        gate.release.notify_one();
        let (report, changed) = timeout(Duration::from_secs(30), async {
            tokio::join!(&mut syncing, &mut lifecycle)
        })
        .await
        .expect("sync and lifecycle complete");
        changed.unwrap();
        let SubscriptionSyncOutcome::Synced { acted, .. } = report.unwrap() else {
            panic!("in-flight sync must finish before lifecycle succeeds");
        };
        assert_eq!(
            acted.requested, 1,
            "the in-flight request must finish before lifecycle succeeds"
        );
        let before = harness.media_requests.requests.lock().await.len();
        let outcome = timeout(
            Duration::from_secs(30),
            sync_subscription(&context, &snapshot, chrono::Utc::now(), None),
        )
        .await
        .expect("stale sync bounded")
        .unwrap();
        assert!(matches!(outcome, SubscriptionSyncOutcome::Off));
        assert_eq!(
            gate.fetches.load(Ordering::SeqCst),
            1,
            "stale queued snapshot must not fetch after disable or unfollow"
        );
        assert_eq!(
            harness.media_requests.requests.lock().await.len(),
            before,
            "no request after successful lifecycle call"
        );
        assert!(harness.titles.store.lock().await.is_empty());
    }
}

#[tokio::test]
async fn private_sync_failure_is_durable_and_visible_only_to_owner() {
    let harness = harness(Arc::new(Auth::default())).await;
    let mut list = subscription("private-follow");
    list.scope = ListScope::Personal;
    list.owner_user_id = harness.user.id.clone();
    list.credential_id = Some("missing-account".into());
    list.sync.next_at = Some(chrono::Utc::now());
    crate::lists::ListSubscriptionRepository::create(harness.lists.as_ref(), list)
        .await
        .unwrap();
    assert_eq!(harness.app.run_list_sync_job(None).await.unwrap().failed, 1);
    let filter = DomainEventFilter {
        event_types: Some(vec![DomainEventType::ListSyncFailed]),
        limit: 10,
        ..Default::default()
    };
    let events = harness
        .app
        .list_domain_events(&harness.user, &filter)
        .await
        .unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].stream,
        scryer_domain::DomainEventStream::User {
            user_id: harness.user.id.clone()
        }
    );
    assert!(
        harness
            .app
            .list_domain_events(&harness.manager, &filter)
            .await
            .unwrap()
            .is_empty()
    );
    let mut other = harness.user.clone();
    other.id = "other-member".into();
    assert!(
        harness
            .app
            .list_domain_events(&other, &filter)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn private_account_cancelled_unlink_finishes_remote_revoke() {
    let auth = Arc::new(Auth {
        block_revoke: true,
        ..Default::default()
    });
    let harness = harness(auth.clone()).await;
    let mut account = seed_expired(&harness).await;
    account.provider = "simkl".into();
    account.credential.direct = false;
    account.credential.app_config = None;
    UserListAccountRepository::update(harness.lists.as_ref(), account.clone())
        .await
        .unwrap();
    let mut list = subscription("private-follow");
    list.scope = ListScope::Personal;
    list.owner_user_id = harness.user.id.clone();
    list.credential_id = Some(account.id.clone());
    list.source.provider = "simkl".into();
    crate::lists::ListSubscriptionRepository::create(harness.lists.as_ref(), list)
        .await
        .unwrap();

    let app = harness.app.clone();
    let actor = harness.user.clone();
    let account_id = account.id.clone();
    let unlink = tokio::spawn(async move { app.unlink_list_account(&actor, &account_id).await });
    timeout(Duration::from_secs(30), auth.revoke_entered.notified())
        .await
        .expect("unlink committed and revoke entered");
    assert!(
        harness
            .app
            .my_list_accounts(&harness.user)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        harness
            .app
            .my_list_subscriptions(&harness.user)
            .await
            .unwrap()
            .is_empty()
    );
    unlink.abort();
    assert!(
        timeout(Duration::from_secs(30), unlink)
            .await
            .expect("caller cancellation finished")
            .unwrap_err()
            .is_cancelled()
    );
    auth.revoke_release.notify_one();
    timeout(Duration::from_secs(30), auth.revoke_finished.notified())
        .await
        .expect("detached revoke completed");
    drop(
        timeout(
            Duration::from_secs(30),
            harness
                .app
                .services
                .lists
                .account_runtime
                .lock_account(&account.id),
        )
        .await
        .expect("detached unlink finished"),
    );
    assert_eq!(auth.revocations.load(Ordering::SeqCst), 1);
    assert_eq!(auth.revocations_completed.load(Ordering::SeqCst), 1);
    assert!(
        harness
            .app
            .my_list_accounts(&harness.user)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        harness
            .app
            .my_list_subscriptions(&harness.user)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn private_account_simkl_issuer_is_required_before_single_use_exchange() {
    let auth = Arc::new(Auth::default());
    let harness = harness(auth.clone()).await;
    for issuer in [
        None,
        Some("https://attacker.invalid"),
        Some("https://smg.scryer.media"),
    ] {
        let start = harness
            .app
            .start_list_account_link(&harness.user, "simkl", "https://instance.invalid")
            .await
            .unwrap();
        assert!(matches!(
            harness
                .app
                .complete_list_account_link(
                    &harness.user,
                    &start.session_id,
                    &start.state,
                    "simkl",
                    "fixture-code",
                    issuer
                )
                .await,
            Err(AppError::Validation(_))
        ));
        assert_eq!(auth.exchanges.load(Ordering::SeqCst), 0);
        // A rejected issuer consumes the receipt; correcting it cannot replay it.
        assert!(matches!(
            harness
                .app
                .complete_list_account_link(
                    &harness.user,
                    &start.session_id,
                    &start.state,
                    "simkl",
                    "fixture-code",
                    Some("https://simkl.com")
                )
                .await,
            Err(AppError::NotFound(_))
        ));
        assert_eq!(auth.exchanges.load(Ordering::SeqCst), 0);
    }
    let start = harness
        .app
        .start_list_account_link(&harness.user, "simkl", "https://instance.invalid")
        .await
        .unwrap();
    let account = harness
        .app
        .complete_list_account_link(
            &harness.user,
            &start.session_id,
            &start.state,
            "simkl",
            "fixture-code",
            Some("https://simkl.com"),
        )
        .await
        .unwrap();
    assert_eq!(account.account.provider, "simkl");
    assert_eq!(account.account.user_id, harness.user.id);
    assert_eq!(auth.exchanges.load(Ordering::SeqCst), 1);
    assert!(matches!(
        harness
            .app
            .complete_list_account_link(
                &harness.user,
                &start.session_id,
                &start.state,
                "simkl",
                "fixture-code",
                Some("https://simkl.com")
            )
            .await,
        Err(AppError::NotFound(_))
    ));
    assert_eq!(auth.exchanges.load(Ordering::SeqCst), 1);
}

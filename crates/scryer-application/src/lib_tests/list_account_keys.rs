//! Accounts linked with a key the member supplies: the key is that member's
//! credential or stands in for the server's application, and it is never shown,
//! logged or kept anywhere but on the account.

use super::security_auth::CapturedEvents;
use super::*;
use crate::lists::account_transport::*;
use crate::lists::accounts::{ListAccountKeyLink, ListAccountView};
use crate::lists::refusal;
use crate::lists::{ListPluginProvider, ListProviderClient, UserListAccountRepository};
use scryer_domain::ListAccountCredential;
use scryer_plugin_sdk::{
    LIST_CREDENTIAL_TYPE_API_KEY, ListCredential, ListPluginAccountResponse,
    ListPluginFetchRequest, ListPluginFetchResponse, ListPluginHealthResponse, PluginDescriptor,
    PluginError, PluginErrorCode, PluginResult,
};
use std::collections::BTreeMap;

/// A provider whose member key is the credential.
const KEYED: &str = "keylists";
const GOOD_KEY: &str = "synthetic-member-key-7Q";
const NEXT_KEY: &str = "synthetic-member-key-9Z";
const ORIGIN: &str = "https://instance.invalid";
/// Refused keys the fixture provider answers with a shared open code instead
/// of `auth_failed`.
const OPEN_CODE_KEYS: &[(&str, &str)] = &[
    ("synthetic-open-api-key-2K", "invalid_api_key"),
    ("synthetic-open-cookie-3J", "invalid_cookie"),
];

struct KeyClient {
    descriptor: PluginDescriptor,
    /// Keys the provider accepts; any other is answered `auth_failed`.
    accepted: Vec<&'static str>,
    /// Every credential the account operation was asked about.
    seen: std::sync::Mutex<Vec<ListCredential>>,
}
#[async_trait]
impl ListProviderClient for KeyClient {
    fn descriptor(&self) -> &PluginDescriptor {
        &self.descriptor
    }
    async fn fetch(
        &self,
        _: ListPluginFetchRequest,
    ) -> AppResult<PluginResult<ListPluginFetchResponse>> {
        Ok(PluginResult::Ok(Default::default()))
    }
    async fn account(
        &self,
        input: ListCredential,
    ) -> AppResult<PluginResult<ListPluginAccountResponse>> {
        self.seen.lock().unwrap().push(input.clone());
        if !self.accepted.is_empty() && !self.accepted.contains(&input.access_token.as_str()) {
            let code = OPEN_CODE_KEYS
                .iter()
                .find(|(key, _)| *key == input.access_token)
                .map_or(PluginErrorCode::AuthFailed, |(_, code)| {
                    PluginErrorCode::from(*code)
                });
            return Ok(PluginResult::Err(PluginError {
                code,
                public_message: "the provider refused the key".into(),
                debug_message: None,
                retry_after_seconds: None,
                details: None,
            }));
        }
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

fn descriptor(provider: &str, provider_json: serde_json::Value) -> PluginDescriptor {
    let mut provider_json = provider_json;
    provider_json["kind"] = "list_provider".into();
    provider_json["provider_type"] = provider.into();
    provider_json["groups"] = serde_json::json!([{
        "label": "Personal",
        "auth_badge": "member_account",
        "items": [{
            "id": "watchlist", "name": "Watchlist", "kinds": ["movie"],
            "source_type": "watchlist", "personal": true, "default_interval_seconds": 3600
        }]
    }]);
    serde_json::from_value(serde_json::json!({
        "id": format!("{provider}-list"),
        "name": provider,
        "version": "1.0.0",
        "sdk_version": scryer_plugin_sdk::SDK_VERSION,
        "sdk_constraint": scryer_plugin_sdk::current_sdk_constraint(),
        "provider": provider_json,
    }))
    .expect("list provider descriptor")
}

struct KeyPlugins(Vec<Arc<KeyClient>>);
impl KeyPlugins {
    fn new() -> Self {
        let client = |descriptor, accepted: &[&'static str]| {
            Arc::new(KeyClient {
                descriptor,
                accepted: accepted.to_vec(),
                seen: Default::default(),
            })
        };
        Self(vec![
            client(
                descriptor(
                    KEYED,
                    serde_json::json!({
                        "capabilities": {"account": true},
                        "member_key": {
                            "label": "Fixture API key",
                            "help_text": "Copy it from your fixture account.",
                            "help_url": "https://lists.fixture.invalid/preferences"
                        }
                    }),
                ),
                &[GOOD_KEY, NEXT_KEY],
            ),
            client(
                descriptor(
                    "tmdb",
                    serde_json::json!({
                        "auth": {
                            "type": "member_account",
                            "flow": {"type": "tmdb_approval"},
                            "exchange": "direct"
                        },
                        "capabilities": {"account": true},
                        "member_key": {"label": "Fixture read token", "use_as": "account_app"}
                    }),
                ),
                &[],
            ),
            // A provider that predates member keys.
            client(
                descriptor(
                    "trakt",
                    serde_json::json!({
                        "auth": {
                            "type": "member_account",
                            "flow": {"type": "authorization_code", "pkce": true},
                            "exchange": "smg_relay"
                        },
                        "capabilities": {"account": true}
                    }),
                ),
                &[],
            ),
        ])
    }
}
impl ListPluginProvider for KeyPlugins {
    fn client_for_provider(
        &self,
        provider: &str,
        _: &BTreeMap<String, String>,
    ) -> Option<Arc<dyn ListProviderClient>> {
        self.0
            .iter()
            .find(|client| client.descriptor.provider_type() == provider)
            .map(|client| client.clone() as Arc<dyn ListProviderClient>)
    }
    fn descriptors(&self) -> Vec<PluginDescriptor> {
        self.0
            .iter()
            .map(|client| client.descriptor.clone())
            .collect()
    }
    fn available_provider_types(&self) -> Vec<String> {
        self.0
            .iter()
            .map(|client| client.descriptor.provider_type().into())
            .collect()
    }
}

/// The approval transport: it refuses to start with any application token but
/// `accepted_app`, and records what it was started and revoked with.
#[derive(Default)]
struct KeyAuth {
    accepted_app: Option<&'static str>,
    start_apps: std::sync::Mutex<Vec<Option<String>>>,
    revoked: std::sync::Mutex<Vec<String>>,
}
#[async_trait]
impl ListAccountAuthGateway for KeyAuth {
    async fn start(&self, input: ListAccountStartRequest) -> AppResult<ListAccountStartResponse> {
        let token = input.app.and_then(|app| app.access_token);
        self.start_apps.lock().unwrap().push(token.clone());
        if self.accepted_app.is_some() && token.as_deref() != self.accepted_app {
            return Err(auth_failure(PROVIDER_REJECTED));
        }
        Ok(ListAccountStartResponse {
            authorize_url: "https://provider.invalid/approve".into(),
            poll_token: Some("fixture-request-token".into()),
            expires_in: 600,
        })
    }
    async fn poll(
        &self,
        _: &str,
        _: &str,
        _: Option<&ListProviderAppConfig>,
    ) -> AppResult<Option<ListAccountCredential>> {
        Ok(Some(ListAccountCredential {
            access_token: "fixture-member-token".into(),
            account_id: Some("external-one".into()),
            direct: true,
            token_type: Some("bearer".into()),
            ..Default::default()
        }))
    }
    async fn complete(&self, _: ListAccountCompleteRequest) -> AppResult<ListAccountCredential> {
        Err(auth_failure("unsupported_exchange"))
    }
    async fn renew(
        &self,
        _: &str,
        _: &ListAccountCredential,
        _: Option<&ListProviderAppConfig>,
    ) -> AppResult<ListAccountCredential> {
        Err(auth_failure("unsupported_exchange"))
    }
    async fn revoke(
        &self,
        provider: &str,
        _: &ListAccountCredential,
        _: Option<&ListProviderAppConfig>,
    ) -> AppResult<()> {
        self.revoked.lock().unwrap().push(provider.into());
        Ok(())
    }
}

struct KeyHarness {
    harness: MediaRequestTestHarness,
    plugins: Arc<KeyPlugins>,
    auth: Arc<KeyAuth>,
}
impl KeyHarness {
    fn client(&self, provider: &str) -> &KeyClient {
        self.plugins
            .0
            .iter()
            .find(|client| client.descriptor.provider_type() == provider)
            .expect("fixture provider")
    }
    async fn link(&self, provider: &str, key: &str) -> AppResult<ListAccountKeyLink> {
        self.harness
            .app
            .link_list_account_with_key(&self.harness.user, provider, key, Some(ORIGIN))
            .await
    }
    async fn linked(&self, provider: &str, key: &str) -> ListAccountView {
        match self.link(provider, key).await.expect("link with key") {
            ListAccountKeyLink::Linked(view) => *view,
            ListAccountKeyLink::Approval(_) => panic!("the key alone should link {provider}"),
        }
    }
    async fn accounts(&self) -> Vec<scryer_domain::UserListAccount> {
        UserListAccountRepository::list_by_user_id(
            self.harness.lists.as_ref(),
            &self.harness.user.id,
        )
        .await
        .unwrap()
    }
}
async fn harness_with(auth: KeyAuth) -> KeyHarness {
    let plugins = Arc::new(KeyPlugins::new());
    let auth = Arc::new(auth);
    let mut harness = bootstrap_media_request_app_with_list_plugins(plugins.clone());
    harness.app.services.lists.auth = auth.clone();
    harness.users.create(harness.user.clone()).await.unwrap();
    harness.users.create(harness.manager.clone()).await.unwrap();
    super::list_experimental_gate::set_experimental_features(&harness, true).await;
    KeyHarness {
        harness,
        plugins,
        auth,
    }
}
async fn harness() -> KeyHarness {
    harness_with(KeyAuth::default()).await
}
fn refusal_reason(result: AppResult<ListAccountKeyLink>) -> &'static str {
    match result {
        Err(AppError::ValidationRefused { reason, .. }) => reason,
        Err(error) => panic!("expected a named refusal, got {error}"),
        Ok(_) => panic!("expected a named refusal, got a link"),
    }
}

#[tokio::test]
async fn a_member_key_links_the_account_as_that_members_credential() {
    let fixture = harness().await;
    let view = fixture.linked(KEYED, &format!("  {GOOD_KEY}\n")).await;

    assert_eq!(view.account.user_id, fixture.harness.user.id);
    assert_eq!(view.account.provider, KEYED);
    assert_eq!(view.account.external_user_id, "external-one");
    let stored = fixture.accounts().await;
    assert_eq!(stored.len(), 1);
    let credential = &stored[0].credential;
    assert_eq!(credential.access_token, GOOD_KEY);
    assert_eq!(
        credential.token_type.as_deref(),
        Some(LIST_CREDENTIAL_TYPE_API_KEY)
    );
    // Nothing to renew and no server-side application snapshot.
    assert_eq!(credential.expires_at, None);
    assert_eq!(credential.refresh_handle, None);
    assert_eq!(credential.app_config, None);

    let seen = fixture.client(KEYED).seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].access_token, GOOD_KEY);
    assert_eq!(
        seen[0].token_type.as_deref(),
        Some(LIST_CREDENTIAL_TYPE_API_KEY)
    );
    assert!(
        fixture.auth.start_apps.lock().unwrap().is_empty(),
        "a key that is the credential starts no sign-in"
    );

    // A personal fetch carries the same key as this member's credential.
    let mut row = crate::lists::test_support::subscription("member-list");
    row.scope = scryer_domain::ListScope::Personal;
    row.owner_user_id = fixture.harness.user.id.clone();
    row.source.provider = KEYED.into();
    let handed = crate::lists::privacy::credential_for(&row, Some(&stored[0]))
        .unwrap_or_else(|_| panic!("the owner's credential"));
    assert_eq!(handed.access_token, GOOD_KEY);
    row.owner_user_id = fixture.harness.manager.id.clone();
    assert!(crate::lists::privacy::credential_for(&row, Some(&stored[0])).is_err());
}

#[tokio::test]
async fn a_member_key_is_never_printed_recorded_or_logged() {
    let fixture = harness().await;
    let captured = CapturedEvents::default();
    // The test runtime is single-threaded, so the thread-local default
    // subscriber sees every event the link emits.
    let guard = tracing::subscriber::set_default(captured.clone());
    let view = fixture.linked(KEYED, GOOD_KEY).await;
    let refused = fixture.link(KEYED, "synthetic-wrong-key-4M").await;
    drop(guard);

    let Err(refused) = refused else {
        panic!("the provider refuses an unknown key");
    };
    assert!(!refused.to_string().contains("synthetic-wrong-key-4M"));
    assert!(!format!("{refused:?}").contains("synthetic-wrong-key-4M"));
    let printed = format!("{:?} {:?}", view.account, view.sources);
    assert!(!printed.contains(GOOD_KEY), "{printed}");
    let recorded = format!("{:?}", fixture.harness.domain_events.events.lock().await);
    assert!(recorded.contains("list_account"), "{recorded}");
    assert!(!recorded.contains(GOOD_KEY), "{recorded}");
    for fields in captured.0.lock().unwrap().iter() {
        for value in fields.values() {
            assert!(!value.contains(GOOD_KEY), "key leaked: {fields:?}");
            assert!(
                !value.contains("synthetic-wrong-key-4M"),
                "refused key leaked: {fields:?}"
            );
        }
    }
    // The server-wide provider settings never receive a member's key.
    assert!(
        fixture
            .harness
            .app
            .list_provider_config_for(KEYED)
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn a_key_the_provider_refuses_names_the_key_and_stores_nothing() {
    let fixture = harness().await;
    let reason = refusal_reason(fixture.link(KEYED, "synthetic-wrong-key-4M").await);
    assert_eq!(reason, refusal::ACCOUNT_KEY_REJECTED);
    assert!(fixture.accounts().await.is_empty());
}

#[tokio::test]
async fn an_invalid_api_key_code_names_the_key_and_stores_nothing() {
    let fixture = harness().await;
    let reason = refusal_reason(fixture.link(KEYED, "synthetic-open-api-key-2K").await);
    assert_eq!(reason, refusal::ACCOUNT_KEY_REJECTED);
    assert!(fixture.accounts().await.is_empty());
}

#[tokio::test]
async fn a_credential_code_for_another_credential_type_is_not_a_key_refusal() {
    let fixture = harness().await;
    let reason = refusal_reason(fixture.link(KEYED, "synthetic-open-cookie-3J").await);
    assert_eq!(reason, refusal::ACCOUNT_IDENTITY_UNVERIFIED);
    assert!(fixture.accounts().await.is_empty());
}

#[tokio::test]
async fn a_malformed_key_is_refused_before_the_provider_is_asked() {
    let fixture = harness().await;
    let oversized = "k".repeat(4097);
    for key in [
        "",
        "   ",
        "two words",
        "line\nbreak",
        "tab\tbed",
        "caf\u{e9}",
        &oversized,
    ] {
        let reason = refusal_reason(fixture.link(KEYED, key).await);
        assert_eq!(reason, refusal::ACCOUNT_KEY_INVALID, "{key:?}");
    }
    assert!(fixture.client(KEYED).seen.lock().unwrap().is_empty());
    assert!(fixture.accounts().await.is_empty());
}

#[tokio::test]
async fn a_provider_that_declares_no_member_key_takes_none() {
    let fixture = harness().await;
    let reason = refusal_reason(fixture.link("trakt", GOOD_KEY).await);
    assert_eq!(reason, refusal::ACCOUNT_KEY_UNSUPPORTED);
    assert!(fixture.client("trakt").seen.lock().unwrap().is_empty());
    // Its own sign-in still starts as before.
    fixture
        .harness
        .app
        .start_list_account_link(&fixture.harness.user, "trakt", ORIGIN)
        .await
        .expect("a provider without a member key still links by signing in");
}

#[tokio::test]
async fn linking_again_with_a_new_key_replaces_the_stored_one() {
    let fixture = harness().await;
    let first = fixture.linked(KEYED, GOOD_KEY).await;
    let second = fixture.linked(KEYED, NEXT_KEY).await;
    assert_eq!(second.account.id, first.account.id);
    let stored = fixture.accounts().await;
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].credential.access_token, NEXT_KEY);
}

#[tokio::test]
async fn unlinking_removes_the_stored_key() {
    let fixture = harness().await;
    let view = fixture.linked(KEYED, GOOD_KEY).await;
    fixture
        .harness
        .app
        .unlink_list_account(&fixture.harness.user, &view.account.id)
        .await
        .expect("unlink");
    assert!(fixture.accounts().await.is_empty());
    assert!(
        UserListAccountRepository::get_by_id(fixture.harness.lists.as_ref(), &view.account.id)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn another_member_cannot_use_or_remove_a_key_account() {
    let fixture = harness().await;
    let view = fixture.linked(KEYED, GOOD_KEY).await;
    let other = &fixture.harness.manager;
    assert!(
        fixture
            .harness
            .app
            .list_account(other, &view.account.id)
            .await
            .is_err()
    );
    assert!(
        fixture
            .harness
            .app
            .unlink_list_account(other, &view.account.id)
            .await
            .is_err()
    );
    assert_eq!(fixture.accounts().await.len(), 1);
}

#[tokio::test]
async fn a_key_standing_in_for_the_servers_application_runs_the_approval() {
    let fixture = harness().await;
    let link = match fixture.link("tmdb", GOOD_KEY).await.expect("start") {
        ListAccountKeyLink::Approval(link) => link,
        ListAccountKeyLink::Linked(_) => panic!("the member still approves at the provider"),
    };
    assert!(link.poll_required);
    assert_eq!(
        fixture.auth.start_apps.lock().unwrap().as_slice(),
        [Some(GOOD_KEY.to_string())]
    );
    assert!(
        fixture.accounts().await.is_empty(),
        "not linked until approved"
    );

    let polled = fixture
        .harness
        .app
        .poll_list_account_link(&fixture.harness.user, &link.session_id)
        .await
        .expect("poll");
    let account = polled.account.expect("linked account").account;
    // The token the approval issued is the credential; the member's key is
    // kept beside it on their own account and nowhere else.
    assert_eq!(account.credential.access_token, "fixture-member-token");
    assert_eq!(
        account
            .credential
            .app_config
            .as_ref()
            .and_then(|app| app.get("access_token"))
            .map(String::as_str),
        Some(GOOD_KEY)
    );
    assert!(!format!("{account:?}").contains(GOOD_KEY));
    assert!(
        fixture
            .harness
            .app
            .list_provider_config_for("tmdb")
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn a_key_refused_when_the_approval_starts_names_the_key() {
    let fixture = harness_with(KeyAuth {
        accepted_app: Some(GOOD_KEY),
        ..Default::default()
    })
    .await;
    let reason = refusal_reason(fixture.link("tmdb", "synthetic-wrong-key-4M").await);
    assert_eq!(reason, refusal::ACCOUNT_KEY_REJECTED);

    // Started without a key there is no application to run the approval on,
    // and the provider is never asked.
    let error = fixture
        .harness
        .app
        .start_list_account_link(&fixture.harness.user, "tmdb", ORIGIN)
        .await
        .err()
        .expect("the server holds no token of its own");
    assert!(matches!(
        error,
        AppError::ValidationRefused {
            reason: refusal::ACCOUNT_KEY_REQUIRED,
            ..
        }
    ));
    assert!(matches!(
        fixture.link("tmdb", GOOD_KEY).await,
        Ok(ListAccountKeyLink::Approval(_))
    ));
}

#[tokio::test]
async fn an_approval_started_with_a_key_needs_an_origin() {
    let fixture = harness().await;
    let result = fixture
        .harness
        .app
        .link_list_account_with_key(&fixture.harness.user, "tmdb", GOOD_KEY, None)
        .await;
    assert_eq!(refusal_reason(result), refusal::ACCOUNT_LINK_ORIGIN_INVALID);
    assert!(fixture.auth.start_apps.lock().unwrap().is_empty());
}

#[tokio::test]
async fn the_catalog_offers_a_member_key_where_the_provider_declares_one() {
    let fixture = harness().await;
    let key_of = |manifests: &[crate::lists::catalog::ListProviderManifest], provider: &str| {
        manifests
            .iter()
            .find(|manifest| manifest.provider_type == provider)
            .expect("manifest")
            .member_key
            .clone()
    };
    let catalog = fixture
        .harness
        .app
        .list_provider_catalog(&fixture.harness.user)
        .await
        .expect("catalog");
    let keyed = key_of(&catalog, KEYED).expect("declared key");
    assert_eq!(keyed.label, "Fixture API key");
    assert_eq!(
        keyed.help_url.as_deref(),
        Some("https://lists.fixture.invalid/preferences")
    );
    assert!(!keyed.approval);
    assert!(key_of(&catalog, "tmdb").expect("declared key").approval);
    assert_eq!(key_of(&catalog, "trakt"), None);
}

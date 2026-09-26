//! In-memory fakes for the list engine's ports, shared by the engine tests.
//!
//! The store follows the port contracts literally (upsert keeps
//! `first_seen_at` and clears departure; `mark_left` touches only rows older
//! than the watermark), so the tests exercise the engine against the same
//! semantics the SQL stores promise. Time is always passed in explicitly.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use scryer_domain::{
    ExternalId, ListCounts, ListExclusion, ListMembership, ListMode, ListOnLeave, ListRoute,
    ListScope, ListSource, ListSourceOrigin, ListSubscription, ListSyncRun, ListSyncStatus,
    MediaFacet, UserListAccount, UserListPolicy,
};
use scryer_plugin_sdk::{
    ListCredential, ListExternalId, ListMediaKind, ListPluginAccountResponse,
    ListPluginFetchRequest, ListPluginFetchResponse, ListPluginHealthResponse, ListPluginItem,
    ListProviderDescriptor, PluginDescriptor, PluginError, PluginResult, ProviderDescriptor,
};

use super::act::{AddedTitle, ListActions};
use super::plugin::{ListPluginProvider, ListProviderClient};
use super::ports::{
    ListExclusionRepository, ListMembershipRepository, ListSubscriptionQuery,
    ListSubscriptionRepository, UserListAccountRepository, UserListPolicyRepository,
};
use super::resolve::{ListItemResolver, ResolveInput, ResolveOutput, ResolvedItem};
use crate::{AppError, AppResult};

pub(crate) const PROVIDER: &str = "fixture-lists";
pub(crate) const LIBRARY: &str = "library-movies";

pub(crate) fn at(minutes: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2030, 1, 1, 0, 0, 0).unwrap() + chrono::Duration::minutes(minutes)
}

pub(crate) fn route(kind: MediaFacet, library_id: &str) -> ListRoute {
    ListRoute {
        kind,
        library_id: library_id.to_string(),
        quality_profile_id: None,
        root_folder_id: None,
        monitor_type: "monitored".to_string(),
        min_availability: None,
        use_season_folders: None,
        release_numbering: None,
        tags: Vec::new(),
    }
}

pub(crate) fn subscription(id: &str) -> ListSubscription {
    ListSubscription {
        id: id.to_string(),
        scope: ListScope::Public,
        owner_user_id: "owner-one".to_string(),
        source: ListSource {
            provider: PROVIDER.to_string(),
            source_type: "user_list".to_string(),
            params: BTreeMap::from([("list_id".to_string(), format!("{id}-source"))]),
            origin: ListSourceOrigin::ProviderFetch,
        },
        name: format!("Fixture list {id}"),
        provider_url: None,
        kinds: vec![MediaFacet::Movie],
        enabled: true,
        mode: ListMode::Add,
        routes: vec![route(MediaFacet::Movie, LIBRARY)],
        filters: Vec::new(),
        max_per_sync: None,
        on_leave: ListOnLeave::Keep,
        interval_seconds: 6 * 3600,
        sync: ListSyncStatus::default(),
        counts: ListCounts::default(),
        credential_id: None,
        created_at: at(0),
        updated_at: at(0),
    }
}

pub(crate) fn tmdb(value: &str) -> ExternalId {
    ExternalId {
        source: "tmdb".to_string(),
        kind: None,
        value: value.to_string(),
    }
}

pub(crate) fn plugin_item(key: &str) -> ListPluginItem {
    ListPluginItem {
        item_key: key.to_string(),
        kind_hint: Some(ListMediaKind::Movie),
        title: Some(format!("Fixture Title {key}")),
        external_ids: vec![ListExternalId {
            source: "tmdb".to_string(),
            kind: None,
            id: format!("{key}-id"),
        }],
        ..ListPluginItem::default()
    }
}

pub(crate) fn resolved_item(key: &str) -> ResolvedItem {
    let item = plugin_item(key);
    ResolvedItem {
        external_ids: vec![tmdb(&format!("{key}-id"))],
        item,
        kind: Some(MediaFacet::Movie),
        resolved: true,
        smg_title_id: None,
        library_title_id: None,
    }
}

pub(crate) fn membership(
    subscription_id: &str,
    key: &str,
    state: scryer_domain::ListMembershipState,
) -> ListMembership {
    ListMembership {
        subscription_id: subscription_id.to_string(),
        item_key: key.to_string(),
        rank: None,
        season: None,
        display_title: None,
        year: None,
        external_ids: vec![tmdb(&format!("{key}-id"))],
        smg_title_id: None,
        title_id: None,
        request_id: None,
        kind: MediaFacet::Movie,
        state,
        state_reason: None,
        added_by_list: false,
        first_seen_at: at(0),
        last_seen_at: at(0),
        left_at: None,
        left_handled: false,
    }
}

// ── Store ──────────────────────────────────────────────────────────────────

#[derive(Default)]
pub(crate) struct MemoryListStore {
    pub subscriptions: Mutex<Vec<ListSubscription>>,
    pub memberships: Mutex<Vec<ListMembership>>,
    pub exclusions: Mutex<Vec<ListExclusion>>,
    pub accounts: Mutex<Vec<UserListAccount>>,
    pub policies: Mutex<Vec<UserListPolicy>>,
    pub runs: Mutex<Vec<ListSyncRun>>,
}

impl MemoryListStore {
    pub(crate) fn with_subscriptions(subscriptions: Vec<ListSubscription>) -> Self {
        let store = Self::default();
        *store.subscriptions.lock().unwrap() = subscriptions;
        store
    }

    pub(crate) fn subscription(&self, id: &str) -> ListSubscription {
        self.subscriptions
            .lock()
            .unwrap()
            .iter()
            .find(|subscription| subscription.id == id)
            .cloned()
            .expect("fixture subscription exists")
    }

    pub(crate) fn rows(&self, subscription_id: &str) -> Vec<ListMembership> {
        self.memberships
            .lock()
            .unwrap()
            .iter()
            .filter(|row| row.subscription_id == subscription_id)
            .cloned()
            .collect()
    }

    pub(crate) fn row(&self, subscription_id: &str, key: &str) -> ListMembership {
        self.rows(subscription_id)
            .into_iter()
            .find(|row| row.item_key == key)
            .expect("fixture membership exists")
    }

    pub(crate) fn insert_rows(&self, rows: Vec<ListMembership>) {
        self.memberships.lock().unwrap().extend(rows);
    }
}

#[async_trait]
impl ListSubscriptionRepository for MemoryListStore {
    async fn create(&self, subscription: ListSubscription) -> AppResult<ListSubscription> {
        self.subscriptions
            .lock()
            .unwrap()
            .push(subscription.clone());
        Ok(subscription)
    }

    async fn update(&self, subscription: ListSubscription) -> AppResult<ListSubscription> {
        let mut rows = self.subscriptions.lock().unwrap();
        let row = rows
            .iter_mut()
            .find(|row| row.id == subscription.id)
            .ok_or_else(|| AppError::NotFound("list subscription".into()))?;
        *row = subscription.clone();
        Ok(subscription)
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<ListSubscription>> {
        Ok(self
            .subscriptions
            .lock()
            .unwrap()
            .iter()
            .find(|row| row.id == id)
            .cloned())
    }

    async fn list(&self, _query: ListSubscriptionQuery) -> AppResult<Vec<ListSubscription>> {
        Ok(self.subscriptions.lock().unwrap().clone())
    }

    async fn list_due(&self, now: DateTime<Utc>, limit: usize) -> AppResult<Vec<ListSubscription>> {
        Ok(self
            .subscriptions
            .lock()
            .unwrap()
            .iter()
            .filter(|row| row.enabled)
            .filter(|row| row.sync.next_at.is_none_or(|next| next <= now))
            .filter(|row| row.sync.paused_until.is_none_or(|until| until <= now))
            .take(limit)
            .cloned()
            .collect())
    }

    async fn record_sync(
        &self,
        id: &str,
        sync: &ListSyncStatus,
        counts: &ListCounts,
    ) -> AppResult<()> {
        let mut rows = self.subscriptions.lock().unwrap();
        if let Some(row) = rows.iter_mut().find(|row| row.id == id) {
            row.sync = sync.clone();
            row.counts = *counts;
        }
        Ok(())
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        self.subscriptions
            .lock()
            .unwrap()
            .retain(|row| row.id != id);
        Ok(())
    }

    async fn record_sync_run(&self, run: ListSyncRun) -> AppResult<ListSyncRun> {
        self.runs.lock().unwrap().push(run.clone());
        Ok(run)
    }

    async fn list_sync_runs(
        &self,
        subscription_id: &str,
        limit: usize,
    ) -> AppResult<Vec<ListSyncRun>> {
        Ok(self
            .runs
            .lock()
            .unwrap()
            .iter()
            .rev()
            .filter(|run| run.subscription_id == subscription_id)
            .take(limit)
            .cloned()
            .collect())
    }
}

#[async_trait]
impl ListMembershipRepository for MemoryListStore {
    async fn upsert_many(&self, memberships: &[ListMembership]) -> AppResult<u64> {
        let mut rows = self.memberships.lock().unwrap();
        for incoming in memberships {
            match rows.iter_mut().find(|row| {
                row.subscription_id == incoming.subscription_id && row.item_key == incoming.item_key
            }) {
                Some(row) => {
                    let first_seen_at = row.first_seen_at;
                    *row = incoming.clone();
                    row.first_seen_at = first_seen_at;
                    row.left_at = None;
                    row.left_handled = false;
                }
                None => rows.push(incoming.clone()),
            }
        }
        Ok(memberships.len() as u64)
    }

    async fn list_by_subscription(&self, subscription_id: &str) -> AppResult<Vec<ListMembership>> {
        Ok(self.rows(subscription_id))
    }

    async fn list_by_titles(&self, title_ids: &[String]) -> AppResult<Vec<ListMembership>> {
        Ok(self
            .memberships
            .lock()
            .unwrap()
            .iter()
            .filter(|row| {
                row.title_id
                    .as_ref()
                    .is_some_and(|title_id| title_ids.contains(title_id))
            })
            .cloned()
            .collect())
    }

    async fn list_by_title(&self, title_id: &str) -> AppResult<Vec<ListMembership>> {
        Ok(self
            .memberships
            .lock()
            .unwrap()
            .iter()
            .filter(|row| row.title_id.as_deref() == Some(title_id))
            .cloned()
            .collect())
    }

    async fn list_by_request(&self, request_id: &str) -> AppResult<Vec<ListMembership>> {
        Ok(self
            .memberships
            .lock()
            .unwrap()
            .iter()
            .filter(|row| row.request_id.as_deref() == Some(request_id))
            .cloned()
            .collect())
    }

    async fn mark_left(
        &self,
        subscription_id: &str,
        seen_before: DateTime<Utc>,
        left_at: DateTime<Utc>,
    ) -> AppResult<Vec<ListMembership>> {
        let mut marked = Vec::new();
        for row in self.memberships.lock().unwrap().iter_mut() {
            if row.subscription_id == subscription_id
                && row.last_seen_at < seen_before
                && row.left_at.is_none()
            {
                row.left_at = Some(left_at);
                marked.push(row.clone());
            }
        }
        Ok(marked)
    }

    async fn set_left_handled(
        &self,
        subscription_id: &str,
        item_keys: &[String],
    ) -> AppResult<u64> {
        let mut count = 0;
        for row in self.memberships.lock().unwrap().iter_mut() {
            if row.subscription_id == subscription_id && item_keys.contains(&row.item_key) {
                row.left_handled = true;
                count += 1;
            }
        }
        Ok(count)
    }
}

#[async_trait]
impl ListExclusionRepository for MemoryListStore {
    async fn create(&self, exclusion: ListExclusion) -> AppResult<ListExclusion> {
        self.exclusions.lock().unwrap().push(exclusion.clone());
        Ok(exclusion)
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<ListExclusion>> {
        Ok(self
            .exclusions
            .lock()
            .unwrap()
            .iter()
            .find(|row| row.id == id)
            .cloned())
    }

    async fn list(&self) -> AppResult<Vec<ListExclusion>> {
        Ok(self.exclusions.lock().unwrap().clone())
    }

    async fn find_matching(
        &self,
        kind: MediaFacet,
        external_ids: &[ExternalId],
        subscription_id: Option<&str>,
    ) -> AppResult<Vec<ListExclusion>> {
        Ok(self
            .exclusions
            .lock()
            .unwrap()
            .iter()
            .filter(|row| {
                row.matches(
                    kind.clone(),
                    external_ids,
                    subscription_id.unwrap_or_default(),
                )
            })
            .cloned()
            .collect())
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        self.exclusions.lock().unwrap().retain(|row| row.id != id);
        Ok(())
    }
}

#[async_trait]
impl UserListAccountRepository for MemoryListStore {
    async fn create(&self, account: UserListAccount) -> AppResult<UserListAccount> {
        self.accounts.lock().unwrap().push(account.clone());
        Ok(account)
    }

    async fn update(&self, account: UserListAccount) -> AppResult<UserListAccount> {
        let mut rows = self.accounts.lock().unwrap();
        rows.retain(|row| row.id != account.id);
        rows.push(account.clone());
        Ok(account)
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<UserListAccount>> {
        Ok(self
            .accounts
            .lock()
            .unwrap()
            .iter()
            .find(|row| row.id == id)
            .cloned())
    }

    async fn list_by_user_id(&self, user_id: &str) -> AppResult<Vec<UserListAccount>> {
        Ok(self
            .accounts
            .lock()
            .unwrap()
            .iter()
            .filter(|row| row.user_id == user_id)
            .cloned()
            .collect())
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        self.accounts.lock().unwrap().retain(|row| row.id != id);
        Ok(())
    }
}

#[async_trait]
impl UserListPolicyRepository for MemoryListStore {
    async fn get(&self, user_id: &str) -> AppResult<Option<UserListPolicy>> {
        Ok(self
            .policies
            .lock()
            .unwrap()
            .iter()
            .find(|row| row.user_id == user_id)
            .cloned())
    }

    async fn set(&self, policy: UserListPolicy) -> AppResult<UserListPolicy> {
        let mut rows = self.policies.lock().unwrap();
        rows.retain(|row| row.user_id != policy.user_id);
        rows.push(policy.clone());
        Ok(policy)
    }

    async fn list(&self) -> AppResult<Vec<UserListPolicy>> {
        Ok(self.policies.lock().unwrap().clone())
    }
}

// ── Actions ────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RecordedAction {
    Add {
        item_key: String,
        search: bool,
    },
    Request {
        item_key: String,
        hold: bool,
    },
    SetMonitored {
        title_id: String,
        monitored: bool,
    },
    Tag {
        title_id: String,
        tag: String,
    },
    Departure {
        title_id: String,
        action: ListOnLeave,
    },
    SyncFailure {
        subscription_id: String,
        class: String,
    },
}

/// Records every call. `fail_departures` makes the on-leave calls fail that
/// many times before succeeding; `reuse_titles` makes adds report an existing
/// title.
#[derive(Default)]
pub(crate) struct RecordingActions {
    pub calls: Mutex<Vec<RecordedAction>>,
    pub fail_departures: Mutex<u32>,
    pub reuse_titles: bool,
    pub refuse_adds: Option<fn() -> AppError>,
}

impl RecordingActions {
    pub(crate) fn calls(&self) -> Vec<RecordedAction> {
        self.calls.lock().unwrap().clone()
    }

    fn departure_call(&self, call: RecordedAction) -> AppResult<()> {
        self.calls.lock().unwrap().push(call);
        let mut remaining = self.fail_departures.lock().unwrap();
        if *remaining > 0 {
            *remaining -= 1;
            return Err(AppError::Repository("fixture failure".into()));
        }
        Ok(())
    }
}

#[async_trait]
impl ListActions for RecordingActions {
    async fn add_title(
        &self,
        _subscription: &ListSubscription,
        _route: &ListRoute,
        item: &ResolvedItem,
        search: bool,
    ) -> AppResult<AddedTitle> {
        self.calls.lock().unwrap().push(RecordedAction::Add {
            item_key: item.item.item_key.clone(),
            search,
        });
        if let Some(refuse) = self.refuse_adds {
            return Err(refuse());
        }
        Ok(AddedTitle {
            title_id: format!("title-{}", item.item.item_key),
            created: !self.reuse_titles,
        })
    }

    async fn submit_request(
        &self,
        _subscription: &ListSubscription,
        _route: &ListRoute,
        item: &ResolvedItem,
        hold: bool,
    ) -> AppResult<String> {
        self.calls.lock().unwrap().push(RecordedAction::Request {
            item_key: item.item.item_key.clone(),
            hold,
        });
        Ok(format!("request-{}", item.item.item_key))
    }

    async fn set_title_monitored(&self, title_id: &str, monitored: bool) -> AppResult<()> {
        self.departure_call(RecordedAction::SetMonitored {
            title_id: title_id.to_string(),
            monitored,
        })
    }

    async fn tag_title(&self, title_id: &str, tag: &str) -> AppResult<()> {
        self.departure_call(RecordedAction::Tag {
            title_id: title_id.to_string(),
            tag: tag.to_string(),
        })
    }

    async fn record_departure(
        &self,
        _subscription: &ListSubscription,
        title_id: &str,
        action: ListOnLeave,
    ) -> AppResult<()> {
        self.departure_call(RecordedAction::Departure {
            title_id: title_id.to_string(),
            action,
        })
    }

    async fn record_sync_failure(
        &self,
        subscription: &ListSubscription,
        failure: &super::fetch::ListFailure,
    ) -> AppResult<()> {
        self.calls
            .lock()
            .unwrap()
            .push(RecordedAction::SyncFailure {
                subscription_id: subscription.id.clone(),
                class: failure.class.as_str().to_string(),
            });
        Ok(())
    }
}

// ── Resolver ───────────────────────────────────────────────────────────────

/// Resolves every input; ids listed in `in_library` map to that title id.
#[derive(Default)]
pub(crate) struct FixtureResolver {
    pub in_library: HashMap<String, String>,
    pub fail: bool,
}

#[async_trait]
impl ListItemResolver for FixtureResolver {
    async fn resolve(&self, inputs: &[ResolveInput]) -> AppResult<Vec<ResolveOutput>> {
        if self.fail {
            return Err(AppError::Repository("fixture gateway down".into()));
        }
        Ok(inputs
            .iter()
            .map(|input| ResolveOutput {
                resolved: true,
                smg_title_id: None,
                external_ids: Vec::new(),
                library_title_id: input
                    .external_ids
                    .iter()
                    .find_map(|id| self.in_library.get(&id.value).cloned()),
            })
            .collect())
    }
}

// ── Plugin ─────────────────────────────────────────────────────────────────

fn fixture_descriptor() -> PluginDescriptor {
    let provider: ListProviderDescriptor =
        serde_json::from_value(serde_json::json!({ "provider_type": PROVIDER }))
            .expect("minimal list descriptor");
    PluginDescriptor {
        id: PROVIDER.to_string(),
        name: "Fixture Lists".to_string(),
        version: "1.0.0".to_string(),
        sdk_version: scryer_plugin_sdk::SDK_VERSION.to_string(),
        sdk_constraint: scryer_plugin_sdk::current_sdk_constraint(),
        socket_permissions: Vec::new(),
        provider: ProviderDescriptor::ListProvider(provider),
    }
}

/// Answers every fetch of one list id with a scripted result.
pub(crate) struct ScriptedLists {
    descriptor: PluginDescriptor,
    pub pages: Mutex<HashMap<String, PluginResult<ListPluginFetchResponse>>>,
    pub fetched: Mutex<Vec<ListPluginFetchRequest>>,
    /// The server-wide values each client was built with, in order.
    pub configs: Mutex<Vec<BTreeMap<String, String>>>,
}

impl ScriptedLists {
    pub(crate) fn new() -> Arc<Self> {
        Self::with_config_fields(Vec::new())
    }

    /// A provider that declares `fields` as its config fields.
    pub(crate) fn with_config_fields(fields: Vec<scryer_plugin_sdk::ConfigFieldDef>) -> Arc<Self> {
        let mut descriptor = fixture_descriptor();
        *descriptor.config_fields_mut() = fields;
        Arc::new(Self {
            descriptor,
            pages: Mutex::new(HashMap::new()),
            fetched: Mutex::new(Vec::new()),
            configs: Mutex::new(Vec::new()),
        })
    }

    /// Serve `keys` for `subscription_id`'s list.
    pub(crate) fn serve(&self, subscription_id: &str, keys: &[&str]) {
        self.pages.lock().unwrap().insert(
            format!("{subscription_id}-source"),
            PluginResult::Ok(ListPluginFetchResponse {
                items: keys.iter().map(|key| plugin_item(key)).collect(),
                fingerprint: Some(keys.join(",")),
                ..ListPluginFetchResponse::default()
            }),
        );
    }

    pub(crate) fn fail(&self, subscription_id: &str, error: PluginError) {
        self.pages.lock().unwrap().insert(
            format!("{subscription_id}-source"),
            PluginResult::Err(error),
        );
    }
}

#[async_trait]
impl ListProviderClient for ScriptedLists {
    fn descriptor(&self) -> &PluginDescriptor {
        &self.descriptor
    }

    async fn fetch(
        &self,
        request: ListPluginFetchRequest,
    ) -> AppResult<PluginResult<ListPluginFetchResponse>> {
        self.fetched.lock().unwrap().push(request.clone());
        let list_id = request.params.get("list_id").cloned().unwrap_or_default();
        let scripted = self
            .pages
            .lock()
            .unwrap()
            .get(&list_id)
            .cloned()
            .ok_or_else(|| AppError::NotFound("fixture list".into()))?;
        // Honor the fingerprint short-circuit the way a provider would.
        if let PluginResult::Ok(response) = &scripted
            && request.since_fingerprint.is_some()
            && request.since_fingerprint == response.fingerprint
        {
            return Ok(PluginResult::Ok(ListPluginFetchResponse {
                unchanged: true,
                fingerprint: response.fingerprint.clone(),
                ..ListPluginFetchResponse::default()
            }));
        }
        Ok(scripted)
    }

    async fn account(
        &self,
        _credential: ListCredential,
    ) -> AppResult<PluginResult<ListPluginAccountResponse>> {
        Err(AppError::Validation("not scripted".into()))
    }

    async fn health(&self) -> AppResult<PluginResult<ListPluginHealthResponse>> {
        Err(AppError::Validation("not scripted".into()))
    }
}

pub(crate) struct ScriptedProvider(pub Arc<ScriptedLists>);

impl ListPluginProvider for ScriptedProvider {
    fn client_for_provider(
        &self,
        provider_type: &str,
        config: &BTreeMap<String, String>,
    ) -> Option<Arc<dyn ListProviderClient>> {
        if provider_type != PROVIDER {
            return None;
        }
        self.0.configs.lock().unwrap().push(config.clone());
        Some(self.0.clone() as Arc<dyn ListProviderClient>)
    }

    fn descriptors(&self) -> Vec<PluginDescriptor> {
        vec![self.0.descriptor.clone()]
    }

    fn available_provider_types(&self) -> Vec<String> {
        vec![PROVIDER.to_string()]
    }
}

pub(crate) fn keys_of(rows: &[ListMembership]) -> HashSet<String> {
    rows.iter().map(|row| row.item_key.clone()).collect()
}

// ── Gateway charts ─────────────────────────────────────────────────────────

/// Serves scripted chart and IMDb list entries; anything not scripted is
/// unavailable, as a gateway outage would be.
#[derive(Default)]
pub(crate) struct ScriptedCharts {
    pub charts: Mutex<HashMap<String, Vec<super::gateway::ListChartItem>>>,
    pub imdb_lists: Mutex<HashMap<String, Vec<super::gateway::ListChartItem>>>,
}

#[async_trait]
impl super::fetch::ListChartSource for ScriptedCharts {
    async fn chart_items(
        &self,
        provider: &str,
        chart_key: &str,
        scope: &str,
    ) -> Result<Vec<super::gateway::ListChartItem>, super::fetch::ListFailure> {
        self.charts
            .lock()
            .unwrap()
            .get(&format!("{provider}/{chart_key}/{scope}"))
            .cloned()
            .ok_or_else(|| {
                super::fetch::ListFailure::new(
                    super::fetch::ListFailureClass::Unavailable,
                    "The metadata service",
                )
            })
    }

    async fn imdb_user_list(
        &self,
        list_id: &str,
    ) -> Result<Vec<super::gateway::ListChartItem>, super::fetch::ListFailure> {
        self.imdb_lists
            .lock()
            .unwrap()
            .get(list_id)
            .cloned()
            .ok_or_else(|| {
                super::fetch::ListFailure::new(super::fetch::ListFailureClass::NotFound, "IMDb")
            })
    }
}

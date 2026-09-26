//! Public list use cases: the provider catalog, following and editing public
//! lists, previews, sync-now, exclusions, and members' list policies.
//!
//! Anyone signed in may read public subscriptions, their memberships and
//! their sync history: they belong to the instance, not to a member. Changing
//! any of it needs the `manage_lists` app permission, and a route may target
//! only a library the actor may manage titles in. Personal subscriptions never
//! pass through here; a personal id answers "not found", the same as a
//! missing one.

use std::collections::{BTreeMap, HashMap};

use chrono::{Duration, Utc};
use scryer_domain::{
    AppPermission, ExternalId, Id, LibraryPermission, ListCounts, ListExclusion,
    ListExclusionScope, ListFilter, ListMembership, ListMembershipState, ListMode, ListOnLeave,
    ListPolicy, ListRoute, ListScope, ListSubscription, ListSyncRun, ListSyncState, ListSyncStatus,
    MediaFacet, MediaRequestOrigin, User, UserListPolicy,
};

use super::catalog::{
    ClassifiedSource, ListProviderManifest, RecognizedListUrl, classify_public_source,
    member_only_providers, merge_provider_catalog, recognize_url,
};
use super::evaluate::{ItemDecision, evaluate};
use super::fetch::{FetchedList, fetch_list};
use super::gateway::{
    GatewayListChartSource, GatewayListItemResolver, LIST_CHART_LANGUAGE, ListChartCatalogEntry,
};
use super::ports::ListSubscriptionQuery;
use super::resolve::resolve_items;
use super::runtime::AppListLibraryLookup;
use crate::jobs::JobKey;
use crate::{AppError, AppResult, AppUseCase, MediaRequestQuery};

/// The largest page of memberships one read returns.
pub const LIST_MEMBERSHIP_PAGE_MAX: usize = 500;
/// The most sync runs one read returns.
pub const LIST_SYNC_RUNS_MAX: usize = 100;
/// How many titles a preview lists as would-be adds.
pub const LIST_PREVIEW_WOULD_ADD_MAX: usize = 100;
/// The window `list_requests_last_30d` counts over.
const MEMBER_POLICY_WINDOW_DAYS: i64 = 30;

/// What a new public follow says.
#[derive(Clone, Debug, Default)]
pub struct PublicListInput {
    pub provider: Option<String>,
    pub source_type: Option<String>,
    pub params: BTreeMap<String, String>,
    /// A pasted link; used to name the source when provider and source type
    /// are not given, and kept as the subscription's link back.
    pub url: Option<String>,
    pub name: Option<String>,
    /// Narrows the kinds the source declares. `None` keeps them all.
    pub kinds: Option<Vec<MediaFacet>>,
    pub mode: ListMode,
    pub routes: Vec<ListRoute>,
    pub filters: Vec<ListFilter>,
    pub max_per_sync: Option<u32>,
    pub on_leave: ListOnLeave,
}

/// An edit of a public follow. `None` leaves a field alone; `max_per_sync`
/// is `Some(None)` to lift the cap.
#[derive(Clone, Debug, Default)]
pub struct PublicListPatch {
    pub name: Option<String>,
    pub kinds: Option<Vec<MediaFacet>>,
    pub mode: Option<ListMode>,
    pub routes: Option<Vec<ListRoute>>,
    pub filters: Option<Vec<ListFilter>>,
    pub max_per_sync: Option<Option<u32>>,
    pub on_leave: Option<ListOnLeave>,
}

/// A source to preview before following it.
#[derive(Clone, Debug, Default)]
pub struct ListSourceDraft {
    pub provider: Option<String>,
    pub source_type: Option<String>,
    pub params: BTreeMap<String, String>,
    pub url: Option<String>,
}

/// One title the next sync would act on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListPreviewItem {
    pub item_key: String,
    pub display_title: Option<String>,
    pub year: Option<i32>,
    pub kind: Option<MediaFacet>,
    pub poster_url: Option<String>,
}

/// What following a source would do right now. Nothing is written to make it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ListPreview {
    pub recognized: bool,
    pub provider: Option<String>,
    pub source_type: Option<String>,
    pub params: BTreeMap<String, String>,
    pub name: Option<String>,
    pub kinds: Vec<MediaFacet>,
    pub total: u64,
    pub in_library: u64,
    pub filtered: u64,
    pub excluded: u64,
    pub unresolved: u64,
    pub would_add: Vec<ListPreviewItem>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ListMembershipPage {
    pub total_count: u64,
    pub items: Vec<ListMembership>,
}

/// A new exclusion, as a manager enters it.
#[derive(Clone, Debug)]
pub struct NewListExclusionInput {
    pub kind: MediaFacet,
    pub external_ids: Vec<ExternalId>,
    pub display_title: String,
    pub year: Option<i32>,
    pub scope: ListExclusionScope,
}

/// An exclusion with the name of the public list it is scoped to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListExclusionView {
    pub exclusion: ListExclusion,
    pub subscription_name: Option<String>,
}

/// One member's list policy and how much their lists have asked for lately.
#[derive(Clone, Debug)]
pub struct MemberListPolicy {
    pub user: User,
    pub policy: ListPolicy,
    pub list_requests_last_30d: u64,
}

fn not_found() -> AppError {
    AppError::NotFound("list subscription not found".into())
}

fn trimmed(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// What a viewer without `manage_lists` sees of a public subscription: the
/// state, not the provider's failure text or the change fingerprint.
pub fn redact_for_viewer(mut subscription: ListSubscription, can_manage: bool) -> ListSubscription {
    subscription.sync.fetch_fingerprint = None;
    subscription.credential_id = None;
    if !can_manage {
        subscription.sync.error_message = None;
    }
    subscription
}

/// A page of memberships in list order: ranked items first by rank, then
/// the rest in the order they were first seen.
pub fn membership_page(
    mut rows: Vec<ListMembership>,
    limit: usize,
    offset: usize,
) -> ListMembershipPage {
    rows.sort_by(|left, right| {
        let rank = |row: &ListMembership| row.rank.unwrap_or(i64::MAX);
        rank(left)
            .cmp(&rank(right))
            .then(left.first_seen_at.cmp(&right.first_seen_at))
            .then(left.item_key.cmp(&right.item_key))
    });
    let total_count = rows.len() as u64;
    let limit = limit.clamp(1, LIST_MEMBERSHIP_PAGE_MAX);
    ListMembershipPage {
        total_count,
        items: rows.into_iter().skip(offset).take(limit).collect(),
    }
}

/// Check the settings every public follow must satisfy, apart from routes'
/// library checks. Returns the kinds the follow keeps.
pub fn validate_public_settings(
    declared_kinds: &[MediaFacet],
    kinds: Option<&[MediaFacet]>,
    mode: ListMode,
    routes: &[ListRoute],
    max_per_sync: Option<u32>,
) -> AppResult<Vec<MediaFacet>> {
    if matches!(mode, ListMode::Request | ListMode::Discover) {
        return Err(AppError::Validation(
            "a public list can search, add, or hold for review".to_string(),
        ));
    }
    let kinds = match kinds {
        Some(kinds) if !kinds.is_empty() => {
            let mut kept = Vec::new();
            for kind in kinds {
                if !declared_kinds.is_empty() && !declared_kinds.contains(kind) {
                    return Err(AppError::Validation(format!(
                        "this list does not contain {} titles",
                        kind.as_str()
                    )));
                }
                if !kept.contains(kind) {
                    kept.push(kind.clone());
                }
            }
            kept
        }
        _ => declared_kinds.to_vec(),
    };
    if kinds.is_empty() {
        return Err(AppError::Validation(
            "choose at least one kind of title".to_string(),
        ));
    }
    let mut seen = Vec::new();
    for route in routes {
        if !kinds.contains(&route.kind) {
            return Err(AppError::Validation(format!(
                "a route targets {} titles, which this list does not keep",
                route.kind.as_str()
            )));
        }
        if seen.contains(&route.kind) {
            return Err(AppError::Validation(format!(
                "only one route per kind; {} has two",
                route.kind.as_str()
            )));
        }
        if route.library_id.trim().is_empty() {
            return Err(AppError::Validation("a route needs a library".to_string()));
        }
        seen.push(route.kind.clone());
    }
    if max_per_sync == Some(0) {
        return Err(AppError::Validation(
            "the per-sync cap must be at least 1".to_string(),
        ));
    }
    Ok(kinds)
}

/// Summarise one evaluated list into a preview. Would-be adds are the items
/// the next sync acts on: candidates within the cap, in list order.
pub fn summarize_preview(
    evaluated: &[super::evaluate::EvaluatedItem],
    posters: &HashMap<String, String>,
) -> ListPreview {
    let mut preview = ListPreview {
        total: evaluated.len() as u64,
        ..ListPreview::default()
    };
    for item in evaluated {
        match &item.decision {
            ItemDecision::Excluded => preview.excluded += 1,
            ItemDecision::Filtered { .. } => preview.filtered += 1,
            ItemDecision::InLibrary { .. } => preview.in_library += 1,
            ItemDecision::Keep {
                state: ListMembershipState::Added,
            } => preview.in_library += 1,
            ItemDecision::Unresolved => preview.unresolved += 1,
            ItemDecision::Candidate => {
                if preview.would_add.len() < LIST_PREVIEW_WOULD_ADD_MAX {
                    let key = &item.item.item.item_key;
                    preview.would_add.push(ListPreviewItem {
                        item_key: key.clone(),
                        display_title: trimmed(item.item.item.title.as_deref()),
                        year: item.item.item.year,
                        kind: item.item.kind.clone(),
                        poster_url: posters.get(key).cloned(),
                    });
                }
            }
            ItemDecision::Keep { .. } | ItemDecision::Deferred => {}
        }
    }
    preview
}

/// How many list-originated requests each member submitted since `since`.
pub fn list_request_counts(
    requests: &[scryer_domain::MediaRequest],
    since: chrono::DateTime<Utc>,
) -> HashMap<String, u64> {
    let mut counts = HashMap::new();
    for request in requests {
        if request.origin != MediaRequestOrigin::Manual && request.created_at >= since {
            *counts
                .entry(request.created_by_user_id.clone())
                .or_default() += 1;
        }
    }
    counts
}

fn draft_subscription(
    actor: &User,
    classified: &ClassifiedSource,
    kinds: Vec<MediaFacet>,
    routes: Vec<ListRoute>,
) -> ListSubscription {
    let now = Utc::now();
    ListSubscription {
        id: Id::new().0,
        scope: ListScope::Public,
        owner_user_id: actor.id.clone(),
        source: classified.source.clone(),
        name: classified.name.clone(),
        provider_url: None,
        kinds,
        enabled: true,
        mode: ListMode::Search,
        routes,
        filters: Vec::new(),
        max_per_sync: None,
        on_leave: ListOnLeave::Keep,
        interval_seconds: i64::try_from(classified.interval_seconds).unwrap_or(i64::MAX),
        sync: ListSyncStatus::default(),
        counts: ListCounts::default(),
        credential_id: None,
        created_at: now,
        updated_at: now,
    }
}

/// A route for every kind, so a draft preview filters nothing for want of a
/// route.
fn preview_routes(kinds: &[MediaFacet]) -> Vec<ListRoute> {
    kinds
        .iter()
        .map(|kind| ListRoute {
            kind: kind.clone(),
            library_id: String::new(),
            quality_profile_id: None,
            root_folder_id: None,
            monitor_type: String::new(),
            min_availability: None,
            use_season_folders: None,
            release_numbering: None,
            tags: Vec::new(),
        })
        .collect()
}

fn same_source(left: &ListSubscription, right: &ClassifiedSource) -> bool {
    left.source.provider == right.source.provider
        && left.source.source_type == right.source.source_type
        && left.source.params == right.source.params
}

impl AppUseCase {
    async fn can_manage_lists(&self, actor: &User) -> AppResult<bool> {
        match self
            .require_app_permission(actor, AppPermission::ManageLists)
            .await
        {
            Ok(()) => Ok(true),
            Err(AppError::Unauthorized(_)) => Ok(false),
            Err(error) => Err(error),
        }
    }

    async fn list_chart_catalog(&self) -> Vec<ListChartCatalogEntry> {
        match self
            .services
            .library
            .metadata_gateway
            .list_chart_catalog(LIST_CHART_LANGUAGE)
            .await
        {
            Ok(charts) => charts,
            Err(error) => {
                // Installed providers stay followable while the gateway is
                // unreachable; its charts come back with it.
                tracing::warn!(error = %error, "list chart catalog is unavailable");
                Vec::new()
            }
        }
    }

    /// Every provider a public list can be followed from. Whether each
    /// server-wide setting is stored is shown only to callers who manage
    /// lists.
    pub async fn list_provider_catalog(
        &self,
        actor: &User,
    ) -> AppResult<Vec<ListProviderManifest>> {
        let plugins = self.services.lists.plugins.descriptors();
        let charts = self.list_chart_catalog().await;
        let mut manifests = merge_provider_catalog(&plugins, &charts);
        if self.can_manage_lists(actor).await? {
            for manifest in &mut manifests {
                if manifest.config_fields.is_empty() {
                    continue;
                }
                let stored = self
                    .stored_list_provider_config(&manifest.provider_type)
                    .await?;
                for field in &mut manifest.config_fields {
                    field.is_set = stored.contains_key(&field.key);
                }
            }
        }
        Ok(manifests)
    }

    async fn classify_source(
        &self,
        provider: Option<&str>,
        source_type: Option<&str>,
        params: &BTreeMap<String, String>,
        url: Option<&str>,
    ) -> AppResult<Option<(ClassifiedSource, Option<RecognizedListUrl>)>> {
        let plugins = self.services.lists.plugins.descriptors();
        let charts = self.list_chart_catalog().await;
        let manifests = merge_provider_catalog(&plugins, &charts);
        let member_only = member_only_providers(&plugins);
        let (provider, source_type, params, recognized) =
            match (trimmed(provider), trimmed(source_type)) {
                (Some(provider), Some(source_type)) => {
                    (provider, source_type, params.clone(), None)
                }
                _ => {
                    let Some(url) = trimmed(url) else {
                        return Err(AppError::Validation(
                            "name a provider and a list, or paste a link".to_string(),
                        ));
                    };
                    let Some(recognized) = recognize_url(&manifests, &url) else {
                        return Ok(None);
                    };
                    (
                        recognized.provider.clone(),
                        recognized.source_type.clone(),
                        recognized.params.clone(),
                        Some(recognized),
                    )
                }
            };
        let classified = classify_public_source(
            &manifests,
            &charts,
            &member_only,
            &provider,
            &source_type,
            &params,
        )?;
        Ok(Some((classified, recognized)))
    }

    async fn validate_route_libraries(&self, actor: &User, routes: &[ListRoute]) -> AppResult<()> {
        for route in routes {
            let library = self
                .services
                .catalog
                .libraries
                .get_by_id(&route.library_id)
                .await?
                .ok_or_else(|| AppError::Validation("a routed library does not exist".into()))?;
            if library.facet != route.kind {
                return Err(AppError::Validation(format!(
                    "{} titles cannot go to the {} library",
                    route.kind.as_str(),
                    library.name
                )));
            }
            self.require_library_permission(actor, &library.id, LibraryPermission::ManageTitles)
                .await?;
        }
        Ok(())
    }

    async fn public_subscription(&self, id: &str) -> AppResult<ListSubscription> {
        self.services
            .lists
            .subscriptions
            .get_by_id(id)
            .await?
            .filter(|subscription| subscription.scope == ListScope::Public)
            .ok_or_else(not_found)
    }

    /// Every public subscription, by name.
    pub async fn public_list_subscriptions(
        &self,
        actor: &User,
    ) -> AppResult<Vec<ListSubscription>> {
        let can_manage = self.can_manage_lists(actor).await?;
        let mut subscriptions = self
            .services
            .lists
            .subscriptions
            .list(ListSubscriptionQuery::public())
            .await?;
        subscriptions.sort_by(|left, right| {
            left.name
                .to_lowercase()
                .cmp(&right.name.to_lowercase())
                .then(left.id.cmp(&right.id))
        });
        Ok(subscriptions
            .into_iter()
            .map(|subscription| redact_for_viewer(subscription, can_manage))
            .collect())
    }

    pub async fn public_list_subscription(
        &self,
        actor: &User,
        id: &str,
    ) -> AppResult<Option<ListSubscription>> {
        let can_manage = self.can_manage_lists(actor).await?;
        match self.public_subscription(id).await {
            Ok(subscription) => Ok(Some(redact_for_viewer(subscription, can_manage))),
            Err(AppError::NotFound(_)) => Ok(None),
            Err(error) => Err(error),
        }
    }

    pub async fn public_list_memberships(
        &self,
        _actor: &User,
        id: &str,
        limit: usize,
        offset: usize,
    ) -> AppResult<ListMembershipPage> {
        let subscription = self.public_subscription(id).await?;
        let rows = self
            .services
            .lists
            .memberships
            .list_by_subscription(&subscription.id)
            .await?
            .into_iter()
            .filter(|row| row.left_at.is_none())
            .collect();
        Ok(membership_page(rows, limit, offset))
    }

    pub async fn public_list_sync_runs(
        &self,
        actor: &User,
        id: &str,
        limit: usize,
    ) -> AppResult<Vec<ListSyncRun>> {
        let subscription = self.public_subscription(id).await?;
        let can_manage = self.can_manage_lists(actor).await?;
        let runs = self
            .services
            .lists
            .subscriptions
            .list_sync_runs(&subscription.id, limit.clamp(1, LIST_SYNC_RUNS_MAX))
            .await?;
        Ok(runs
            .into_iter()
            .map(|mut run| {
                if !can_manage {
                    run.error_message = None;
                }
                run
            })
            .collect())
    }

    /// Fetch, resolve and evaluate `subscription` without writing anything.
    async fn run_preview(
        &self,
        subscription: &ListSubscription,
        existing: HashMap<String, ListMembership>,
    ) -> AppResult<ListPreview> {
        let lists = &self.services.lists;
        let gateway = self.services.library.metadata_gateway.clone();
        let charts = GatewayListChartSource::new(gateway.clone());
        let resolver = GatewayListItemResolver::new(gateway, AppListLibraryLookup::new(self));
        // A preview always reads the whole list: an "unchanged" answer would
        // leave it nothing to show.
        let mut subscription = subscription.clone();
        subscription.sync.fetch_fingerprint = None;
        let config = self
            .list_provider_config_for(&subscription.source.provider)
            .await;
        let mut fetched: FetchedList = fetch_list(
            &subscription,
            lists.plugins.as_ref(),
            &charts,
            None,
            &config,
        )
        .await
        .map_err(|failure| AppError::Validation(failure.message))?;
        fetched.dedupe();
        let resolved = resolve_items(&subscription, fetched.items, &resolver).await?;
        let exclusions = lists.exclusions.list().await?;
        let evaluated = evaluate(&subscription, resolved, &exclusions, &existing);
        let mut preview = summarize_preview(&evaluated, &fetched.posters);
        preview.recognized = true;
        preview.provider = Some(subscription.source.provider.clone());
        preview.source_type = Some(subscription.source.source_type.clone());
        preview.params = subscription.source.params.clone();
        preview.name = Some(subscription.name.clone());
        preview.kinds = subscription.kinds.clone();
        Ok(preview)
    }

    /// What the next sync of a followed public list would do.
    pub async fn preview_public_list(&self, actor: &User, id: &str) -> AppResult<ListPreview> {
        self.require_app_permission(actor, AppPermission::ManageLists)
            .await?;
        let subscription = self.public_subscription(id).await?;
        let existing = self
            .services
            .lists
            .memberships
            .list_by_subscription(&subscription.id)
            .await?
            .into_iter()
            .map(|row| (row.item_key.clone(), row))
            .collect();
        self.run_preview(&subscription, existing).await
    }

    /// What following a source would do, before following it. An unknown
    /// link answers `recognized: false` rather than an error.
    pub async fn preview_list_source(
        &self,
        actor: &User,
        draft: ListSourceDraft,
    ) -> AppResult<ListPreview> {
        self.require_app_permission(actor, AppPermission::ManageLists)
            .await?;
        let Some((classified, _)) = self
            .classify_source(
                draft.provider.as_deref(),
                draft.source_type.as_deref(),
                &draft.params,
                draft.url.as_deref(),
            )
            .await?
        else {
            return Ok(ListPreview::default());
        };
        let kinds = classified.kinds.clone();
        let subscription =
            draft_subscription(actor, &classified, kinds.clone(), preview_routes(&kinds));
        self.run_preview(&subscription, HashMap::new()).await
    }

    pub async fn preview_list_url(&self, actor: &User, url: &str) -> AppResult<ListPreview> {
        self.preview_list_source(
            actor,
            ListSourceDraft {
                url: Some(url.to_string()),
                ..ListSourceDraft::default()
            },
        )
        .await
    }

    /// Follow a public list. Its first sync is due at once.
    pub async fn subscribe_public_list(
        &self,
        actor: &User,
        input: PublicListInput,
    ) -> AppResult<ListSubscription> {
        self.require_app_permission(actor, AppPermission::ManageLists)
            .await?;
        let (classified, _) = self
            .classify_source(
                input.provider.as_deref(),
                input.source_type.as_deref(),
                &input.params,
                input.url.as_deref(),
            )
            .await?
            .ok_or_else(|| {
                AppError::Validation("that link is not a list Scryer can follow".into())
            })?;
        let kinds = validate_public_settings(
            &classified.kinds,
            input.kinds.as_deref(),
            input.mode,
            &input.routes,
            input.max_per_sync,
        )?;
        self.validate_route_libraries(actor, &input.routes).await?;

        let lists = &self.services.lists;
        let existing = lists
            .subscriptions
            .list(ListSubscriptionQuery::public())
            .await?;
        if existing
            .iter()
            .any(|subscription| same_source(subscription, &classified))
        {
            return Err(AppError::Validation("this list is already followed".into()));
        }

        let mut subscription = draft_subscription(actor, &classified, kinds, input.routes);
        subscription.name = trimmed(input.name.as_deref()).unwrap_or(subscription.name);
        subscription.provider_url = trimmed(input.url.as_deref());
        subscription.mode = input.mode;
        subscription.filters = input.filters;
        subscription.max_per_sync = input.max_per_sync;
        subscription.on_leave = input.on_leave;
        subscription.sync.next_at = Some(subscription.created_at);
        let created = lists.subscriptions.create(subscription).await?;
        Ok(redact_for_viewer(created, true))
    }

    pub async fn update_public_list(
        &self,
        actor: &User,
        id: &str,
        patch: PublicListPatch,
    ) -> AppResult<ListSubscription> {
        self.require_app_permission(actor, AppPermission::ManageLists)
            .await?;
        let mut subscription = self.public_subscription(id).await?;
        // The kinds a source declares are the ceiling; a follow may narrow
        // them and widen them back.
        let declared = match &subscription.source.origin {
            scryer_domain::ListSourceOrigin::SmgImdbList => {
                vec![MediaFacet::Movie, MediaFacet::Series]
            }
            _ => match self
                .classify_source(
                    Some(&subscription.source.provider),
                    Some(&subscription.source.source_type),
                    &subscription.source.params,
                    None,
                )
                .await
            {
                Ok(Some((classified, _))) => classified.kinds,
                _ => subscription.kinds.clone(),
            },
        };
        let mode = patch.mode.unwrap_or(subscription.mode);
        let routes = patch.routes.unwrap_or_else(|| subscription.routes.clone());
        let max_per_sync = patch.max_per_sync.unwrap_or(subscription.max_per_sync);
        let requested_kinds = patch.kinds.unwrap_or_else(|| subscription.kinds.clone());
        let kinds = validate_public_settings(
            &declared,
            Some(&requested_kinds),
            mode,
            &routes,
            max_per_sync,
        )?;
        self.validate_route_libraries(actor, &routes).await?;

        if let Some(name) = trimmed(patch.name.as_deref()) {
            subscription.name = name;
        }
        subscription.kinds = kinds;
        subscription.mode = mode;
        subscription.routes = routes;
        if let Some(filters) = patch.filters {
            subscription.filters = filters;
        }
        subscription.max_per_sync = max_per_sync;
        if let Some(on_leave) = patch.on_leave {
            subscription.on_leave = on_leave;
        }
        subscription.updated_at = Utc::now();
        let updated = self
            .services
            .lists
            .subscriptions
            .update(subscription)
            .await?;
        Ok(redact_for_viewer(updated, true))
    }

    /// Turn a public list off or back on. Turning it on makes it due at once.
    pub async fn set_public_list_enabled(
        &self,
        actor: &User,
        id: &str,
        enabled: bool,
    ) -> AppResult<ListSubscription> {
        self.require_app_permission(actor, AppPermission::ManageLists)
            .await?;
        let mut subscription = self.public_subscription(id).await?;
        let now = Utc::now();
        subscription.enabled = enabled;
        subscription.updated_at = now;
        let subscriptions = &self.services.lists.subscriptions;
        let mut updated = subscriptions.update(subscription).await?;
        updated.sync = if enabled {
            ListSyncStatus {
                state: ListSyncState::New,
                next_at: Some(now),
                error_message: None,
                error_at: None,
                paused_until: None,
                ..updated.sync.clone()
            }
        } else {
            ListSyncStatus {
                state: ListSyncState::Off,
                ..updated.sync.clone()
            }
        };
        subscriptions
            .record_sync(&updated.id, &updated.sync, &updated.counts)
            .await?;
        Ok(redact_for_viewer(updated, true))
    }

    /// Stop following a public list. Its memberships, list-scoped exclusions
    /// and history go with it; every title and request it made stays.
    pub async fn unsubscribe_public_list(&self, actor: &User, id: &str) -> AppResult<String> {
        self.require_app_permission(actor, AppPermission::ManageLists)
            .await?;
        let subscription = self.public_subscription(id).await?;
        self.services
            .lists
            .subscriptions
            .delete(&subscription.id)
            .await?;
        self.emit_list_unfollowed_event(actor, &subscription).await;
        Ok(subscription.id)
    }

    async fn queue_list_syncs(
        &self,
        actor: &User,
        subscriptions: Vec<ListSubscription>,
    ) -> AppResult<Vec<String>> {
        let now = Utc::now();
        let mut queued = Vec::new();
        for subscription in subscriptions.into_iter().filter(|row| row.enabled) {
            let status = ListSyncStatus {
                next_at: Some(now),
                ..subscription.sync.clone()
            };
            self.services
                .lists
                .subscriptions
                .record_sync(&subscription.id, &status, &subscription.counts)
                .await?;
            queued.push(subscription.id);
        }
        if !queued.is_empty() {
            match self.start_manual_job_run(actor, JobKey::ListSync).await {
                Ok(_) => {}
                // A sync already running picks due lists up on its next pass.
                Err(AppError::Validation(message)) => {
                    tracing::debug!(reason = %message, "list sync not started now");
                }
                Err(error) => return Err(error),
            }
        }
        Ok(queued)
    }

    /// Make one public list due now and start a sync. A list rate-limited by
    /// its provider stays paused until the provider allows it.
    pub async fn sync_public_list_now(&self, actor: &User, id: &str) -> AppResult<Vec<String>> {
        self.require_app_permission(actor, AppPermission::ManageLists)
            .await?;
        let subscription = self.public_subscription(id).await?;
        if !subscription.enabled {
            return Err(AppError::Validation("this list is turned off".into()));
        }
        self.queue_list_syncs(actor, vec![subscription]).await
    }

    pub async fn sync_all_public_lists(&self, actor: &User) -> AppResult<Vec<String>> {
        self.require_app_permission(actor, AppPermission::ManageLists)
            .await?;
        let subscriptions = self
            .services
            .lists
            .subscriptions
            .list(ListSubscriptionQuery::public())
            .await?;
        self.queue_list_syncs(actor, subscriptions).await
    }

    /// Every exclusion, with the public list a scoped one belongs to.
    pub async fn list_exclusions(&self, _actor: &User) -> AppResult<Vec<ListExclusionView>> {
        let lists = &self.services.lists;
        let names = lists
            .subscriptions
            .list(ListSubscriptionQuery::public())
            .await?
            .into_iter()
            .map(|subscription| (subscription.id, subscription.name))
            .collect::<HashMap<_, _>>();
        Ok(lists
            .exclusions
            .list()
            .await?
            .into_iter()
            .map(|exclusion| ListExclusionView {
                subscription_name: exclusion
                    .scope
                    .subscription_id()
                    .and_then(|id| names.get(id).cloned()),
                exclusion,
            })
            .collect())
    }

    pub async fn add_list_exclusion(
        &self,
        actor: &User,
        input: NewListExclusionInput,
    ) -> AppResult<ListExclusionView> {
        self.require_app_permission(actor, AppPermission::ManageLists)
            .await?;
        let external_ids = input
            .external_ids
            .into_iter()
            .filter_map(|id| {
                let source = id.source.trim().to_ascii_lowercase();
                let value = id.value.trim().to_string();
                (!source.is_empty() && !value.is_empty()).then_some(ExternalId {
                    source,
                    kind: id.kind,
                    value,
                })
            })
            .collect::<Vec<_>>();
        if external_ids.is_empty() {
            return Err(AppError::Validation(
                "an exclusion needs at least one id".into(),
            ));
        }
        let display_title = trimmed(Some(&input.display_title))
            .ok_or_else(|| AppError::Validation("an exclusion needs a title".into()))?;
        let subscription_name = match &input.scope {
            ListExclusionScope::AllLists => None,
            ListExclusionScope::List { subscription_id } => {
                Some(self.public_subscription(subscription_id).await?.name)
            }
        };
        let exclusion = self
            .services
            .lists
            .exclusions
            .create(ListExclusion {
                id: Id::new().0,
                kind: input.kind,
                external_ids,
                display_title,
                year: input.year,
                scope: input.scope,
                created_by_user_id: Some(actor.id.clone()),
                created_at: Utc::now(),
            })
            .await?;
        Ok(ListExclusionView {
            exclusion,
            subscription_name,
        })
    }

    pub async fn remove_list_exclusion(&self, actor: &User, id: &str) -> AppResult<String> {
        self.require_app_permission(actor, AppPermission::ManageLists)
            .await?;
        let exclusions = &self.services.lists.exclusions;
        let exclusion = exclusions
            .get_by_id(id)
            .await?
            .ok_or_else(|| AppError::NotFound("list exclusion not found".into()))?;
        exclusions.delete(&exclusion.id).await?;
        Ok(exclusion.id)
    }

    /// Every member's list policy, with how many list requests they made in
    /// the last thirty days.
    pub async fn member_list_policies(&self, actor: &User) -> AppResult<Vec<MemberListPolicy>> {
        self.require_app_permission(actor, AppPermission::ManageLists)
            .await?;
        let users = self.services.identity.users.list_all().await?;
        let policies = self
            .services
            .lists
            .policies
            .list()
            .await?
            .into_iter()
            .map(|policy| (policy.user_id.clone(), policy.policy))
            .collect::<HashMap<_, _>>();
        let since = Utc::now() - Duration::days(MEMBER_POLICY_WINDOW_DAYS);
        let requests = self
            .services
            .catalog
            .media_requests
            .list(MediaRequestQuery::default())
            .await?;
        let counts = list_request_counts(&requests, since);
        let mut members = users
            .into_iter()
            .filter(|user| !user.is_system_execution_actor())
            .map(|user| MemberListPolicy {
                policy: policies.get(&user.id).copied().unwrap_or_default(),
                list_requests_last_30d: counts.get(&user.id).copied().unwrap_or(0),
                user,
            })
            .collect::<Vec<_>>();
        members.sort_by(|left, right| {
            left.user
                .username
                .to_lowercase()
                .cmp(&right.user.username.to_lowercase())
        });
        Ok(members)
    }

    pub async fn set_member_list_policy(
        &self,
        actor: &User,
        user_id: &str,
        policy: ListPolicy,
    ) -> AppResult<MemberListPolicy> {
        self.require_app_permission(actor, AppPermission::ManageLists)
            .await?;
        let user = self
            .services
            .identity
            .users
            .get_by_id(user_id)
            .await?
            .ok_or_else(|| AppError::NotFound("user not found".into()))?;
        let stored = self
            .services
            .lists
            .policies
            .set(UserListPolicy {
                user_id: user.id.clone(),
                policy,
                updated_by_user_id: Some(actor.id.clone()),
                updated_at: Utc::now(),
            })
            .await?;
        let since = Utc::now() - Duration::days(MEMBER_POLICY_WINDOW_DAYS);
        let requests = self
            .services
            .catalog
            .media_requests
            .list(MediaRequestQuery {
                requester_user_id: Some(user.id.clone()),
                ..MediaRequestQuery::default()
            })
            .await?;
        let list_requests_last_30d = list_request_counts(&requests, since)
            .get(&user.id)
            .copied()
            .unwrap_or(0);
        Ok(MemberListPolicy {
            user,
            policy: stored.policy,
            list_requests_last_30d,
        })
    }
}

#[cfg(test)]
#[path = "public_tests.rs"]
mod tests;

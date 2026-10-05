//! Personal subscription policy and owner-scoped lifecycle operations.
use super::ListSubscriptionQuery;
use super::catalog::{ClassifiedSource, facet_of};
use super::privacy::ensure_subscription_visible;
use super::public::{
    ListMembershipPage, ListPreview, ListSourceDraft, PublicListInput, PublicListPatch,
    membership_page, validate_public_settings,
};
use crate::{AppError, AppResult, AppUseCase};
use chrono::Utc;
use scryer_domain::{
    Id, LibraryPermission, ListCounts, ListMode, ListScope, ListSource, ListSourceOrigin,
    ListSubscription, ListSyncRun, ListSyncState, ListSyncStatus, User, UserListAccountStatus,
};
use std::collections::HashMap;

impl AppUseCase {
    async fn personal_subscription(&self, actor: &User, id: &str) -> AppResult<ListSubscription> {
        let row = self
            .services
            .lists
            .subscriptions
            .get_by_id(id)
            .await?
            .ok_or_else(|| AppError::NotFound("list subscription not found".into()))?;
        ensure_subscription_visible(actor, &row)?;
        Ok(row)
    }
    async fn is_personal_list(&self, actor: &User, id: &str) -> AppResult<bool> {
        Ok(self.personal_subscription(actor, id).await?.is_personal())
    }
    async fn lock_personal_subscription(
        &self,
        actor: &User,
        id: &str,
    ) -> AppResult<(tokio::sync::OwnedMutexGuard<()>, ListSubscription)> {
        let row = self.personal_subscription(actor, id).await?;
        let guard = self
            .services
            .lists
            .account_runtime
            .lock_account(row.credential_id.as_deref().unwrap_or_default())
            .await;
        let current = self.personal_subscription(actor, id).await?;
        if !current.is_personal() || current.credential_id != row.credential_id {
            return Err(AppError::NotFound("list subscription changed".into()));
        }
        Ok((guard, current))
    }
    pub async fn my_list_subscriptions(&self, actor: &User) -> AppResult<Vec<ListSubscription>> {
        let mut rows = self
            .services
            .lists
            .subscriptions
            .list(ListSubscriptionQuery::personal_for(&actor.id))
            .await?;
        rows.retain(|row| row.is_personal() && row.owner_user_id == actor.id);
        for row in &mut rows {
            row.sync.fetch_fingerprint = None;
        }
        Ok(rows)
    }
    pub async fn visible_list_subscription(
        &self,
        actor: &User,
        id: &str,
    ) -> AppResult<Option<ListSubscription>> {
        match self.personal_subscription(actor, id).await {
            Ok(mut row) if row.is_personal() => {
                row.sync.fetch_fingerprint = None;
                Ok(Some(row))
            }
            Ok(_) => self.public_list_subscription(actor, id).await,
            Err(AppError::NotFound(_)) => Ok(None),
            Err(error) => Err(error),
        }
    }
    pub(crate) async fn classify_personal_source(
        &self,
        actor: &User,
        account_id: &str,
        provider: Option<&str>,
        source_type: Option<&str>,
        params: &std::collections::BTreeMap<String, String>,
    ) -> AppResult<ClassifiedSource> {
        let account = self.owned_list_account(actor, account_id).await?;
        if account.status != UserListAccountStatus::Active {
            return Err(AppError::Validation("reconnect this list account".into()));
        }
        if provider.is_some_and(|provider| provider != account.provider) {
            return Err(AppError::Validation(
                "the account belongs to another provider".into(),
            ));
        }
        let descriptor = self
            .services
            .lists
            .plugins
            .descriptors()
            .into_iter()
            .find(|descriptor| {
                descriptor
                    .list_provider()
                    .is_some_and(|list| list.provider_type == account.provider)
            })
            .ok_or_else(|| AppError::NotFound("list provider not found".into()))?;
        let list = descriptor
            .list_provider()
            .ok_or_else(|| AppError::NotFound("list provider not found".into()))?;
        let item = list
            .groups
            .iter()
            .flat_map(|group| &group.items)
            .find(|item| item.personal && Some(item.source_type.as_str()) == source_type)
            .ok_or_else(|| {
                AppError::Validation("choose a personal list offered by this provider".into())
            })?;
        for (key, value) in params {
            let field = item
                .params
                .iter()
                .find(|field| &field.key == key)
                .ok_or_else(|| AppError::Validation("unknown personal list parameter".into()))?;
            if value.len() > 2048 || (!field.options.is_empty() && !field.options.contains(value)) {
                return Err(AppError::Validation(
                    "invalid personal list parameter".into(),
                ));
            }
        }
        if item.params.iter().any(|field| {
            field.required
                && params
                    .get(&field.key)
                    .is_none_or(|value| value.trim().is_empty())
        }) {
            return Err(AppError::Validation(
                "a required personal list parameter is missing".into(),
            ));
        }
        Ok(ClassifiedSource {
            source: ListSource {
                provider: account.provider,
                source_type: item.source_type.clone(),
                params: params.clone(),
                origin: ListSourceOrigin::ProviderFetch,
            },
            name: item.name.clone(),
            kinds: item.kinds.iter().copied().map(facet_of).collect(),
            interval_seconds: item.default_interval_seconds,
        })
    }
    async fn validate_personal_routes(
        &self,
        actor: &User,
        input: &PublicListInput,
    ) -> AppResult<Vec<scryer_domain::MediaFacet>> {
        let kinds = validate_public_settings(
            input.kinds.as_deref().unwrap_or_default(),
            input.kinds.as_deref(),
            ListMode::Add,
            &input.routes,
            input.max_per_sync,
        )?;
        if !matches!(
            input.mode,
            ListMode::Add | ListMode::Search | ListMode::Request
        ) {
            return Err(AppError::Validation(
                "a personal list can add, search, or request".into(),
            ));
        }
        if kinds
            .iter()
            .any(|kind| !input.routes.iter().any(|route| &route.kind == kind))
        {
            return Err(AppError::Validation(
                "choose a library for every title kind".into(),
            ));
        }
        for route in &input.routes {
            let library = self
                .services
                .catalog
                .libraries
                .get_by_id(&route.library_id)
                .await?
                .ok_or_else(|| AppError::Validation("a routed library does not exist".into()))?;
            if library.facet != route.kind {
                return Err(AppError::Validation(
                    "the routed library has another title kind".into(),
                ));
            }
            if !self
                .has_library_permission(actor, &library.id, LibraryPermission::ManageTitles)
                .await?
            {
                self.require_library_permission(actor, &library.id, LibraryPermission::Request)
                    .await?;
                if input.mode != ListMode::Request {
                    return Err(AppError::Unauthorized(
                        "add and search need title management permission".into(),
                    ));
                }
                if !matches!(
                    input.on_leave,
                    scryer_domain::ListOnLeave::Keep | scryer_domain::ListOnLeave::Log
                ) {
                    return Err(AppError::Unauthorized(
                        "departure changes need title management permission".into(),
                    ));
                }
                self.request_quality_profile_snapshot_for_submission(
                    &library,
                    route.quality_profile_id.clone(),
                )
                .await?;
            }
        }
        Ok(kinds)
    }
    pub async fn subscribe_personal_list(
        &self,
        actor: &User,
        account_id: &str,
        mut input: PublicListInput,
    ) -> AppResult<ListSubscription> {
        self.require_personal_lists_allowed(actor).await?;
        let classified = self
            .classify_personal_source(
                actor,
                account_id,
                input.provider.as_deref(),
                input.source_type.as_deref(),
                &input.params,
            )
            .await?;
        let kinds = validate_public_settings(
            &classified.kinds,
            input.kinds.as_deref(),
            ListMode::Add,
            &input.routes,
            input.max_per_sync,
        )?;
        input.kinds = Some(kinds.clone());
        self.validate_personal_routes(actor, &input).await?;
        let _guard = self
            .services
            .lists
            .account_runtime
            .lock_account(account_id)
            .await;
        let account = self.owned_list_account(actor, account_id).await?;
        if account.status != UserListAccountStatus::Active {
            return Err(AppError::Validation("reconnect this list account".into()));
        }
        let existing = self
            .services
            .lists
            .subscriptions
            .list(ListSubscriptionQuery::personal_for(&actor.id))
            .await?;
        if existing.iter().any(|row| {
            row.credential_id.as_deref() == Some(account_id) && row.source == classified.source
        }) {
            return Err(AppError::Validation(
                "this personal list is already followed".into(),
            ));
        }
        let now = Utc::now();
        let row = ListSubscription {
            id: Id::new().0,
            scope: ListScope::Personal,
            owner_user_id: actor.id.clone(),
            source: classified.source,
            name: input
                .name
                .filter(|name| !name.trim().is_empty())
                .unwrap_or(classified.name),
            provider_url: None,
            kinds,
            enabled: true,
            mode: input.mode,
            routes: input.routes,
            filters: input.filters,
            max_per_sync: input.max_per_sync,
            on_leave: input.on_leave,
            interval_seconds: i64::try_from(classified.interval_seconds).unwrap_or(i64::MAX),
            sync: ListSyncStatus {
                next_at: Some(now),
                ..Default::default()
            },
            counts: ListCounts::default(),
            credential_id: Some(account_id.into()),
            created_at: now,
            updated_at: now,
        };
        let created = self.services.lists.subscriptions.create(row).await?;
        self.queue_list_syncs(actor, vec![created.clone()]).await?;
        Ok(created)
    }
    pub async fn preview_personal_source(
        &self,
        actor: &User,
        draft: ListSourceDraft,
    ) -> AppResult<ListPreview> {
        self.require_personal_lists_allowed(actor).await?;
        let id = draft
            .credential_id
            .as_deref()
            .ok_or_else(|| AppError::Validation("choose a linked account".into()))?;
        let classified = self
            .classify_personal_source(
                actor,
                id,
                draft.provider.as_deref(),
                draft.source_type.as_deref(),
                &draft.params,
            )
            .await?;
        let now = Utc::now();
        let row = ListSubscription {
            id: Id::new().0,
            scope: ListScope::Personal,
            owner_user_id: actor.id.clone(),
            source: classified.source,
            name: classified.name,
            provider_url: None,
            kinds: classified.kinds.clone(),
            enabled: true,
            mode: ListMode::Request,
            routes: classified
                .kinds
                .into_iter()
                .map(|kind| scryer_domain::ListRoute {
                    kind,
                    library_id: String::new(),
                    quality_profile_id: None,
                    root_folder_id: None,
                    monitor_type: String::new(),
                    min_availability: None,
                    use_season_folders: None,
                    release_numbering: None,
                    tags: Vec::new(),
                })
                .collect(),
            filters: Vec::new(),
            max_per_sync: None,
            on_leave: Default::default(),
            interval_seconds: i64::try_from(classified.interval_seconds).unwrap_or(i64::MAX),
            sync: Default::default(),
            counts: Default::default(),
            credential_id: Some(id.into()),
            created_at: now,
            updated_at: now,
        };
        self.run_preview(&row, HashMap::new()).await
    }
    pub async fn update_visible_list(
        &self,
        actor: &User,
        id: &str,
        patch: PublicListPatch,
    ) -> AppResult<ListSubscription> {
        if !self.is_personal_list(actor, id).await? {
            return self.update_public_list(actor, id, patch).await;
        }
        self.require_personal_lists_allowed(actor).await?;
        let (_guard, mut row) = self.lock_personal_subscription(actor, id).await?;
        if let Some(name) = patch.name {
            if !name.trim().is_empty() {
                row.name = name;
            }
        }
        if let Some(kinds) = patch.kinds {
            row.kinds = kinds;
        }
        if let Some(mode) = patch.mode {
            row.mode = mode;
        }
        if let Some(routes) = patch.routes {
            row.routes = routes;
        }
        if let Some(filters) = patch.filters {
            row.filters = filters;
        }
        if let Some(cap) = patch.max_per_sync {
            row.max_per_sync = cap;
        }
        if let Some(on_leave) = patch.on_leave {
            row.on_leave = on_leave;
        }
        let classified = self
            .classify_personal_source(
                actor,
                row.credential_id.as_deref().unwrap_or_default(),
                Some(&row.source.provider),
                Some(&row.source.source_type),
                &row.source.params,
            )
            .await?;
        validate_public_settings(
            &classified.kinds,
            Some(&row.kinds),
            ListMode::Add,
            &row.routes,
            row.max_per_sync,
        )?;
        self.validate_personal_routes(
            actor,
            &PublicListInput {
                kinds: Some(row.kinds.clone()),
                mode: row.mode,
                routes: row.routes.clone(),
                max_per_sync: row.max_per_sync,
                on_leave: row.on_leave,
                ..Default::default()
            },
        )
        .await?;
        row.updated_at = Utc::now();
        self.services.lists.subscriptions.update(row).await
    }
    pub async fn set_visible_list_enabled(
        &self,
        actor: &User,
        id: &str,
        enabled: bool,
    ) -> AppResult<ListSubscription> {
        if !self.is_personal_list(actor, id).await? {
            return self.set_public_list_enabled(actor, id, enabled).await;
        }
        if enabled {
            self.require_personal_lists_allowed(actor).await?;
        }
        let (_guard, mut row) = self.lock_personal_subscription(actor, id).await?;
        if enabled {
            self.owned_list_account(actor, row.credential_id.as_deref().unwrap_or_default())
                .await?;
        }
        row.enabled = enabled;
        row.updated_at = Utc::now();
        let mut row = self.services.lists.subscriptions.update(row).await?;
        row.sync.state = if enabled {
            ListSyncState::New
        } else {
            ListSyncState::Off
        };
        if enabled {
            row.sync.next_at = Some(Utc::now());
        }
        self.services
            .lists
            .subscriptions
            .record_sync(id, &row.sync, &row.counts)
            .await?;
        Ok(row)
    }
    pub async fn unsubscribe_visible_list(&self, actor: &User, id: &str) -> AppResult<String> {
        if !self.is_personal_list(actor, id).await? {
            return self.unsubscribe_public_list(actor, id).await;
        }
        let (_guard, row) = self.lock_personal_subscription(actor, id).await?;
        self.services.lists.subscriptions.delete(id).await?;
        self.emit_list_unfollowed_event(actor, &row).await;
        Ok(id.into())
    }
    pub async fn sync_visible_list_now(&self, actor: &User, id: &str) -> AppResult<Vec<String>> {
        if !self.is_personal_list(actor, id).await? {
            return self.sync_public_list_now(actor, id).await;
        }
        self.require_personal_lists_allowed(actor).await?;
        self.queue_list_syncs(actor, vec![self.personal_subscription(actor, id).await?])
            .await
    }
    pub async fn sync_my_lists(&self, actor: &User) -> AppResult<Vec<String>> {
        self.require_personal_lists_allowed(actor).await?;
        self.queue_list_syncs(actor, self.my_list_subscriptions(actor).await?)
            .await
    }
    pub async fn visible_list_memberships(
        &self,
        actor: &User,
        id: &str,
        limit: usize,
        offset: usize,
    ) -> AppResult<ListMembershipPage> {
        if !self.is_personal_list(actor, id).await? {
            return self.public_list_memberships(actor, id, limit, offset).await;
        }
        Ok(membership_page(
            self.services
                .lists
                .memberships
                .list_by_subscription(id)
                .await?
                .into_iter()
                .filter(|row| row.left_at.is_none())
                .collect(),
            limit,
            offset,
        ))
    }
    pub async fn visible_list_sync_runs(
        &self,
        actor: &User,
        id: &str,
        limit: usize,
    ) -> AppResult<Vec<ListSyncRun>> {
        if !self.is_personal_list(actor, id).await? {
            return self.public_list_sync_runs(actor, id, limit).await;
        }
        self.services
            .lists
            .subscriptions
            .list_sync_runs(id, limit.clamp(1, super::public::LIST_SYNC_RUNS_MAX))
            .await
    }
    pub async fn preview_visible_list(&self, actor: &User, id: &str) -> AppResult<ListPreview> {
        if !self.is_personal_list(actor, id).await? {
            return self.preview_public_list(actor, id).await;
        }
        self.require_personal_lists_allowed(actor).await?;
        let row = self.personal_subscription(actor, id).await?;
        let existing = self
            .services
            .lists
            .memberships
            .list_by_subscription(id)
            .await?
            .into_iter()
            .map(|row| (row.item_key.clone(), row))
            .collect();
        self.run_preview(&row, existing).await
    }
}

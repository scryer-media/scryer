//! Binds the sync engine's ports to the application's use cases, and the
//! scheduled job body.
//!
//! Library adds run as the system actor, exactly as a request approval does;
//! requests are submitted as the subscription's owner so their grants and
//! request rules apply. Nothing here deletes a title.

use async_trait::async_trait;
use chrono::Utc;
use scryer_domain::{
    DomainEventPayload, ExternalId, ListEventSubject, ListOnLeave, ListRequestSubmittedEventData,
    ListRoute, ListSubscription, ListSyncFailedEventData, ListTitleAddedEventData,
    ListTitleLeftEventData, MediaFacet, MediaRequest, MediaRequestOrigin, NewDomainEvent, NewTitle,
    User,
};

use super::act::{AddedTitle, ListActions};
use super::fetch::ListFailure;
use super::gateway::{GatewayListChartSource, GatewayListItemResolver, ListLibraryLookup};
use super::rejection::remember_rejected_request;
use super::resolve::ResolvedItem;
use super::sync::{ListSyncContext, ListSyncReport, sync_due_subscriptions};
use crate::events::domain_events::{
    new_global_domain_event, new_title_domain_event, title_context_snapshot,
};
use crate::{
    AppError, AppResult, AppUseCase, MediaRequestAdmission, SubmissionConflictPolicy,
    SubmitMediaRequestInput, TitleOptionsPatch,
};

/// The name a list item is created or requested under: its own title when the
/// provider sent one, otherwise its first id. Metadata hydration replaces it.
fn item_name(item: &ResolvedItem) -> String {
    item.item
        .title
        .as_deref()
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(str::to_string)
        .or_else(|| {
            item.external_ids
                .first()
                .map(|id| format!("{}:{}", id.source, id.value))
        })
        .unwrap_or_else(|| item.item.item_key.clone())
}

/// The registry description of the tag the `Tag` on-leave action applies.
const LEFT_LIST_TAG_DESCRIPTION: &str = "Added by a list that no longer includes it.";

fn non_empty(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

pub(crate) struct AppListActions<'a> {
    app: &'a AppUseCase,
}

impl<'a> AppListActions<'a> {
    pub(crate) fn new(app: &'a AppUseCase) -> Self {
        Self { app }
    }
}

#[async_trait]
impl ListActions for AppListActions<'_> {
    async fn add_title(
        &self,
        subscription: &ListSubscription,
        route: &ListRoute,
        item: &ResolvedItem,
        search: bool,
    ) -> AppResult<AddedTitle> {
        let actor = User::system_execution_actor();
        let request = NewTitle {
            name: item_name(item),
            facet: route.kind.clone(),
            monitored: true,
            tags: route.tags.clone(),
            external_ids: item.external_ids.clone(),
            root_folder_id: route.root_folder_id.clone(),
            min_availability: route.min_availability.clone(),
            year: item.item.year,
            ..NewTitle::default()
        };
        let patch = TitleOptionsPatch {
            quality_profile_id: route.quality_profile_id.clone().map(Some),
            monitor_type: non_empty(&route.monitor_type).map(Some),
            use_season_folders: route.use_season_folders.map(Some),
            release_numbering: route.release_numbering.clone().map(Some),
            ..TitleOptionsPatch::default()
        };
        let outcome = self
            .app
            .add_title_with_options_patch_outcome_in_library(
                &actor,
                request,
                route.library_id.clone(),
                patch,
            )
            .await?;
        let created = !outcome.reused_existing_title;
        if search && created {
            // The title exists either way; a search that fails to start is
            // picked up by the ordinary wanted-search schedule.
            if let Err(error) = self
                .app
                .trigger_title_wanted_search(
                    &actor,
                    &outcome.title.id,
                    SubmissionConflictPolicy::from_replace_flag(false),
                )
                .await
            {
                tracing::warn!(
                    title_id = %outcome.title.id,
                    error = %error,
                    "list add could not start its wanted search"
                );
            }
        }
        if created && !subscription.is_personal() {
            self.app
                .append_list_event(new_title_domain_event(
                    &actor,
                    &outcome.title,
                    DomainEventPayload::ListTitleAdded(ListTitleAddedEventData {
                        list: ListEventSubject::of(subscription),
                        title: title_context_snapshot(&outcome.title),
                        library_id: route.library_id.clone(),
                        searched: search,
                    }),
                ))
                .await;
        }
        Ok(AddedTitle {
            title_id: outcome.title.id,
            created,
        })
    }

    async fn submit_request(
        &self,
        subscription: &ListSubscription,
        route: &ListRoute,
        item: &ResolvedItem,
        hold: bool,
    ) -> AppResult<String> {
        let owner = self
            .app
            .services
            .identity
            .users
            .get_by_id(&subscription.owner_user_id)
            .await?
            .ok_or_else(|| AppError::NotFound("list owner".to_string()))?;
        let outcome = self
            .app
            .submit_media_request(
                &owner,
                SubmitMediaRequestInput {
                    library_id: route.library_id.clone(),
                    facet: route.kind.clone(),
                    title: item_name(item),
                    sort_title: None,
                    slug: None,
                    year: item.item.year,
                    overview: None,
                    runtime_minutes: None,
                    language: None,
                    content_status: None,
                    rating_summary: Default::default(),
                    requested_quality_profile_id: route.quality_profile_id.clone(),
                    requested_monitor_type: non_empty(&route.monitor_type),
                    requested_monitor_selection: None,
                    requested_lease_days: None,
                    external_ids: item.external_ids.clone(),
                    origin: MediaRequestOrigin::for_subscription(subscription),
                    // Hold waits for review whatever the owner's grants and
                    // the request rules would allow.
                    admission: if hold {
                        MediaRequestAdmission::HoldForReview
                    } else {
                        MediaRequestAdmission::Evaluate
                    },
                },
            )
            .await?;
        if !subscription.is_personal() {
            self.app
                .append_list_event(new_global_domain_event(
                    &User::system_execution_actor(),
                    DomainEventPayload::ListRequestSubmitted(ListRequestSubmittedEventData {
                        list: ListEventSubject::of(subscription),
                        request_id: outcome.request_id.clone(),
                        library_id: route.library_id.clone(),
                        facet: route.kind.clone(),
                        title_name: item_name(item),
                        year: item.item.year,
                        held: hold,
                    }),
                ))
                .await;
        }
        Ok(outcome.request_id)
    }

    async fn set_title_monitored(&self, title_id: &str, monitored: bool) -> AppResult<()> {
        self.app
            .set_title_monitored(&User::system_execution_actor(), title_id, monitored)
            .await
            .map(|_| ())
    }

    async fn tag_title(&self, title_id: &str, tag: &str) -> AppResult<()> {
        self.app
            .ensure_title_tag_registered(tag, LEFT_LIST_TAG_DESCRIPTION)
            .await?;
        self.app
            .update_title_tags(
                &User::system_execution_actor(),
                &[title_id.to_string()],
                &[tag.to_string()],
                &[],
            )
            .await
            .map(|_| ())
    }

    async fn record_departure(
        &self,
        subscription: &ListSubscription,
        title_id: &str,
        action: ListOnLeave,
    ) -> AppResult<()> {
        if subscription.is_personal() {
            // A personal list's name and contents are private to its owner:
            // no feed event, and the log line carries ids only.
            tracing::debug!(
                subscription_id = %subscription.id,
                owner_user_id = %subscription.owner_user_id,
                provider = %subscription.source.provider,
                title_id = %title_id,
                action = action.as_str(),
                "a title a personal list added has left it"
            );
            return Ok(());
        }
        let Some(title) = self.app.services.catalog.titles.get_by_id(title_id).await? else {
            // The title is gone; there is nothing left to record against.
            return Ok(());
        };
        self.app
            .append_domain_event(new_title_domain_event(
                &User::system_execution_actor(),
                &title,
                DomainEventPayload::ListTitleLeft(ListTitleLeftEventData {
                    list: ListEventSubject::of(subscription),
                    title: title_context_snapshot(&title),
                    action,
                }),
            ))
            .await
            .map(|_| ())
    }

    async fn record_sync_failure(
        &self,
        subscription: &ListSubscription,
        failure: &ListFailure,
    ) -> AppResult<()> {
        if subscription.is_personal() {
            return Ok(());
        }
        self.app
            .append_domain_event(new_global_domain_event(
                &User::system_execution_actor(),
                DomainEventPayload::ListSyncFailed(ListSyncFailedEventData {
                    list: ListEventSubject::of(subscription),
                    reason: failure.message.clone(),
                    failure_class: failure.class.as_str().to_string(),
                }),
            ))
            .await
            .map(|_| ())
    }
}

/// Library lookup for list items, across every library of the kind.
pub(crate) struct AppListLibraryLookup<'a> {
    app: &'a AppUseCase,
}

impl<'a> AppListLibraryLookup<'a> {
    pub(crate) fn new(app: &'a AppUseCase) -> Self {
        Self { app }
    }
}

#[async_trait]
impl ListLibraryLookup for AppListLibraryLookup<'_> {
    async fn find_title(&self, kind: &MediaFacet, ids: &[ExternalId]) -> AppResult<Option<String>> {
        for id in ids {
            if let Some(title) = self
                .app
                .services
                .catalog
                .titles
                .find_by_external_id_in_facet(kind.clone(), &id.source, &id.value)
                .await?
            {
                return Ok(Some(title.id));
            }
        }
        Ok(None)
    }
}

impl AppUseCase {
    /// Append a list event. Best effort: the add or request it describes
    /// already happened, and a missing feed entry must not undo it or make
    /// the sync try it again.
    async fn append_list_event(&self, event: NewDomainEvent) {
        if let Err(error) = self.append_domain_event(event).await {
            tracing::warn!(error = %error, "could not record a list event");
        }
    }

    /// Emit the event for an unfollowed public list. Personal lists are
    /// never announced.
    pub(crate) async fn emit_list_unfollowed_event(
        &self,
        actor: &User,
        subscription: &ListSubscription,
    ) {
        if subscription.is_personal() {
            return;
        }
        self.append_list_event(new_global_domain_event(
            actor,
            DomainEventPayload::ListUnfollowed(scryer_domain::ListUnfollowedEventData {
                list: ListEventSubject::of(subscription),
            }),
        ))
        .await;
    }

    /// Remember a rejected list request so its list does not submit it again.
    /// Best effort: the rejection already stands, and a failure here is logged
    /// rather than undoing it.
    pub(crate) async fn remember_rejected_list_request(
        &self,
        actor: &User,
        request: &MediaRequest,
    ) {
        let lists = &self.services.lists;
        if let Err(error) = remember_rejected_request(
            lists.subscriptions.as_ref(),
            lists.memberships.as_ref(),
            lists.exclusions.as_ref(),
            request,
            &actor.id,
            Utc::now(),
        )
        .await
        {
            tracing::warn!(
                request_id = %request.id,
                error = %error,
                "could not remember a rejected list request"
            );
        }
    }

    /// Lists ship behind the instance-wide experimental switch for now:
    /// following and syncing are refused, and the sync job idles, until the
    /// operator opts in.
    pub(crate) async fn require_lists_enabled(&self) -> AppResult<()> {
        if self.experimental_features_enabled().await? {
            Ok(())
        } else {
            Err(AppError::Validation(
                "lists are an experimental feature; turn on experimental features in Settings first"
                    .into(),
            ))
        }
    }

    /// Job body for [`crate::jobs::JobKey::ListSync`].
    pub(crate) async fn run_list_sync_job(
        &self,
        job_run_id: Option<String>,
    ) -> AppResult<ListSyncReport> {
        if !self.experimental_features_enabled().await? {
            return Ok(ListSyncReport::default());
        }
        let lists = &self.services.lists;
        let actions = AppListActions::new(self);
        let gateway = self.services.library.metadata_gateway.clone();
        let resolver =
            GatewayListItemResolver::new(gateway.clone(), AppListLibraryLookup::new(self));
        let charts = GatewayListChartSource::new(gateway);
        let provider_configs = self.load_list_provider_configs().await;
        let context = ListSyncContext {
            subscriptions: lists.subscriptions.as_ref(),
            memberships: lists.memberships.as_ref(),
            exclusions: lists.exclusions.as_ref(),
            accounts: lists.accounts.as_ref(),
            policies: lists.policies.as_ref(),
            plugins: lists.plugins.as_ref(),
            charts: &charts,
            resolver: &resolver,
            actions: &actions,
            provider_configs: &provider_configs,
        };
        sync_due_subscriptions(&context, Utc::now(), job_run_id).await
    }
}

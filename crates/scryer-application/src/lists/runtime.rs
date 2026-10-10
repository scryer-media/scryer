//! Binds the sync engine's ports to the application's use cases, and the
//! scheduled job body.
//!
//! Library adds run as the system actor, exactly as a request approval does;
//! requests are submitted as the subscription's owner so their grants and
//! request rules apply. A Request list whose owner manages titles in the
//! routed library adds instead of requesting; a Hold list always requests.
//! Nothing here deletes a title.

use async_trait::async_trait;
use chrono::Utc;
use scryer_domain::{
    DomainEventPayload, ExternalId, LibraryPermission, ListEventSubject, ListOnLeave,
    ListRequestSubmittedEventData, ListRoute, ListSubscription, ListSyncFailedEventData,
    ListTitleAddedEventData, ListTitleLeftEventData, MediaFacet, MediaRequest, MediaRequestOrigin,
    NewDomainEvent, NewTitle, ReleaseNumbering, User,
};

use super::act::{AddedTitle, ListActions};
use super::fetch::ListFailure;
use super::gateway::{GatewayListChartSource, GatewayListItemResolver, ListLibraryLookup};
use super::rejection::remember_rejected_request;
use super::resolve::ResolvedItem;
use super::route_options::{route_monitor_type, route_option_tags};
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
fn series_movie_parent_ids(target: &scryer_domain::ListSeriesMovieTarget) -> Vec<ExternalId> {
    vec![
        ExternalId {
            source: "smg".into(),
            kind: Some("series".into()),
            value: target.parent_smg_id.to_string(),
        },
        ExternalId {
            source: "tvdb".into(),
            kind: Some("series".into()),
            value: target.parent_tvdb_id.to_string(),
        },
    ]
}

fn series_movie_selection(item: &ResolvedItem) -> scryer_domain::MonitorSelection {
    scryer_domain::MonitorSelection {
        seasons: vec![],
        series_movies: vec![scryer_domain::MonitorSelectionMovie {
            name: item_name(item),
            external_ids: item.external_ids.clone(),
        }],
    }
}

fn series_movie_matches(movie: &scryer_domain::MovieEntity, ids: &[ExternalId]) -> bool {
    ids.iter().any(|id| {
        let value = match id.source.as_str() {
            "tvdb" => &movie.tvdb_id,
            "tmdb" => &movie.tmdb_id,
            "imdb" => &movie.imdb_id,
            "mal" => &movie.mal_id,
            "anidb" => &movie.anidb_id,
            _ => return false,
        };
        value.as_deref() == Some(id.value.as_str())
    })
}

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

    async fn list_owner(&self, subscription: &ListSubscription) -> AppResult<User> {
        self.app
            .services
            .identity
            .users
            .get_by_id(&subscription.owner_user_id)
            .await?
            .ok_or_else(|| AppError::NotFound("list owner".to_string()))
    }
}

#[async_trait]
impl ListActions for AppListActions<'_> {
    async fn owner_is_enabled(&self, subscription: &ListSubscription) -> AppResult<bool> {
        Ok(self
            .app
            .services
            .identity
            .users
            .get_by_id(&subscription.owner_user_id)
            .await?
            .is_some_and(|owner| owner.authorization.login_status.is_enabled()))
    }
    async fn lock_account(
        &self,
        subscription: &ListSubscription,
    ) -> Option<tokio::sync::OwnedMutexGuard<()>> {
        if subscription.is_personal() {
            Some(
                self.app
                    .services
                    .lists
                    .account_runtime
                    .lock_account(subscription.credential_id.as_deref().unwrap_or_default())
                    .await,
            )
        } else {
            None
        }
    }
    async fn prepare_account(
        &self,
        account: scryer_domain::UserListAccount,
        guard: &mut Option<tokio::sync::OwnedMutexGuard<()>>,
    ) -> AppResult<scryer_domain::UserListAccount> {
        let app = self.app.clone();
        let held = guard.take();
        // A renewal is never abandoned mid-flight: the provider may already
        // have spent the stored refresh credential. The task keeps the account
        // lock until the renewed credential is saved, even if this sync is
        // cancelled first, and hands the lock back when it finishes.
        let renewal = tokio::spawn(async move {
            let result = async {
                let mut account = app.refresh_list_account_locked(account).await?;
                account.last_used_at = Some(Utc::now());
                account.updated_at = Utc::now();
                // Recording the use is best effort: a renewal that could not
                // be saved is kept by the account runtime, and this sync
                // still uses it.
                Ok(app
                    .services
                    .lists
                    .accounts
                    .update(account.clone())
                    .await
                    .unwrap_or(account))
            }
            .await;
            (held, result)
        });
        let (held, result) = renewal
            .await
            .map_err(|_| AppError::Repository("list account refresh task failed".into()))?;
        *guard = held;
        result
    }
    async fn add_title(
        &self,
        subscription: &ListSubscription,
        route: &ListRoute,
        item: &ResolvedItem,
        search: bool,
    ) -> AppResult<AddedTitle> {
        if subscription.is_personal() {
            let owner = self.list_owner(subscription).await?;
            self.app
                .require_library_permission(
                    &owner,
                    &route.library_id,
                    LibraryPermission::ManageTitles,
                )
                .await?;
        }
        let actor = User::system_execution_actor();
        if let Some(target) = &item.series_movie {
            let mut parent_route = route.clone();
            parent_route.monitor_type = "advanced".into();
            let request = NewTitle {
                name: target.parent_name.clone(),
                facet: route.kind.clone(),
                monitored: true,
                tags: route_option_tags(&parent_route),
                external_ids: series_movie_parent_ids(target),
                root_folder_id: route.root_folder_id.clone(),
                ..NewTitle::default()
            };
            let _profile_guard = self
                .app
                .runtime
                .catalog
                .quality_profile_reference_lock
                .lock()
                .await;
            let title = self
                .app
                .new_title_for_library(&actor, request, route.library_id.clone())
                .await?;
            let created = self
                .app
                .services
                .catalog
                .titles
                .create_or_get_existing_preserving_options(title, series_movie_selection(item))
                .await?;
            let reused = created.reused_existing;
            if !reused {
                self.app.invalidate_monitored_title_matcher().await;
                self.app
                    .append_list_event(new_title_domain_event(
                        &actor,
                        &created.title,
                        DomainEventPayload::TitleAdded(scryer_domain::TitleAddedEventData {
                            title: title_context_snapshot(&created.title),
                        }),
                    ))
                    .await;
            }
            let outcome = self.app.finish_add_title_with_outcome(created).await?;
            if reused {
                let links = self
                    .app
                    .services
                    .catalog
                    .shows
                    .list_series_movie_links_for_title(&outcome.title.id)
                    .await?;
                if !links
                    .iter()
                    .any(|link| series_movie_matches(&link.movie, &item.external_ids))
                {
                    self.app
                        .services
                        .catalog
                        .titles
                        .mark_title_metadata_hydration_due_now(&outcome.title.id)
                        .await?;
                    self.app.runtime.catalog.title_hydration_wake.notify_one();
                    return Err(AppError::Repository(
                        "series movie is waiting for metadata hydration".into(),
                    ));
                }
            }
            // Hydration and ordinary wanted scheduling see only this movie in
            // the advanced selection. Never queue a parent-wide search here.
            return Ok(AddedTitle {
                title_id: outcome.title.id,
                created: !reused,
            });
        }
        let monitor_type = route_monitor_type(route);
        // A new title takes its options from its tags, as the add dialog's
        // titles do; the patch below only reaches a title that already exists.
        let mut tags = route_option_tags(route);
        tags.extend(route.tags.iter().cloned());
        let request = NewTitle {
            name: item_name(item),
            facet: route.kind.clone(),
            monitored: monitor_type
                .as_deref()
                .is_none_or(crate::media_requests::monitor_type_to_monitored),
            tags,
            external_ids: item.external_ids.clone(),
            root_folder_id: route.root_folder_id.clone(),
            min_availability: route.min_availability.clone(),
            year: item.item.year,
            ..NewTitle::default()
        };
        let patch = TitleOptionsPatch {
            quality_profile_id: route.quality_profile_id.clone().map(Some),
            monitor_type: monitor_type.map(Some),
            use_season_folders: route.use_season_folders.map(Some),
            release_numbering: route.release_numbering.as_deref().map(|value| {
                let numbering = ReleaseNumbering::from_str_or_default(value);
                (numbering != ReleaseNumbering::Auto).then(|| numbering.as_str().to_string())
            }),
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
        if created {
            self.app
                .append_list_event(if subscription.is_personal() {
                    crate::events::domain_events::new_user_domain_event(
                        &actor,
                        subscription.owner_user_id.clone(),
                        DomainEventPayload::ListTitleAdded(ListTitleAddedEventData {
                            list: ListEventSubject::of(subscription),
                            title: title_context_snapshot(&outcome.title),
                            library_id: route.library_id.clone(),
                            searched: search,
                        }),
                    )
                } else {
                    new_title_domain_event(
                        &actor,
                        &outcome.title,
                        DomainEventPayload::ListTitleAdded(ListTitleAddedEventData {
                            list: ListEventSubject::of(subscription),
                            title: title_context_snapshot(&outcome.title),
                            library_id: route.library_id.clone(),
                            searched: search,
                        }),
                    )
                })
                .await;
        }
        Ok(AddedTitle {
            title_id: outcome.title.id,
            created,
        })
    }

    async fn owner_manages_titles(
        &self,
        subscription: &ListSubscription,
        route: &ListRoute,
    ) -> AppResult<bool> {
        let owner = self.list_owner(subscription).await?;
        if !owner.authorization.login_status.is_enabled() {
            return Ok(false);
        }
        self.app
            .has_library_permission(&owner, &route.library_id, LibraryPermission::ManageTitles)
            .await
    }

    async fn submit_request(
        &self,
        subscription: &ListSubscription,
        route: &ListRoute,
        item: &ResolvedItem,
        hold: bool,
    ) -> AppResult<String> {
        let owner = self.list_owner(subscription).await?;
        if !owner.authorization.login_status.is_enabled() {
            return Err(AppError::Unauthorized("list owner is disabled".into()));
        }
        let committed = self
            .app
            .submit_media_request_committed(
                &owner,
                SubmitMediaRequestInput {
                    library_id: route.library_id.clone(),
                    facet: route.kind.clone(),
                    title: item
                        .series_movie
                        .as_ref()
                        .map(|target| target.parent_name.clone())
                        .unwrap_or_else(|| item_name(item)),
                    sort_title: None,
                    slug: None,
                    year: item.item.year,
                    overview: None,
                    runtime_minutes: None,
                    language: None,
                    content_status: None,
                    rating_summary: Default::default(),
                    requested_quality_profile_id: route.quality_profile_id.clone(),
                    requested_monitor_type: if item.series_movie.is_some() {
                        Some("advanced".into())
                    } else {
                        non_empty(&route.monitor_type)
                    },
                    requested_monitor_selection: item
                        .series_movie
                        .as_ref()
                        .map(|_| series_movie_selection(item)),
                    requested_lease_days: None,
                    external_ids: item
                        .series_movie
                        .as_ref()
                        .map(series_movie_parent_ids)
                        .unwrap_or_else(|| item.external_ids.clone()),
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
        // Keep a committed request's id even when acting on its verdict fails.
        if !subscription.is_personal()
            && let Err(error) = &committed.decision
        {
            tracing::warn!(
                subscription_id = %subscription.id,
                request_id = %committed.request_id,
                error = %error,
                "a list request was filed but acting on its verdict failed"
            );
        }
        {
            self.app
                .append_list_event(if subscription.is_personal() {
                    crate::events::domain_events::new_user_domain_event(
                        &owner,
                        subscription.owner_user_id.clone(),
                        DomainEventPayload::ListRequestSubmitted(ListRequestSubmittedEventData {
                            list: ListEventSubject::of(subscription),
                            request_id: committed.request_id.clone(),
                            library_id: route.library_id.clone(),
                            facet: route.kind.clone(),
                            title_name: item_name(item),
                            year: item.item.year,
                            held: hold,
                        }),
                    )
                } else {
                    new_global_domain_event(
                        &User::system_execution_actor(),
                        DomainEventPayload::ListRequestSubmitted(ListRequestSubmittedEventData {
                            list: ListEventSubject::of(subscription),
                            request_id: committed.request_id.clone(),
                            library_id: route.library_id.clone(),
                            facet: route.kind.clone(),
                            title_name: item_name(item),
                            year: item.item.year,
                            held: hold,
                        }),
                    )
                })
                .await;
        }
        Ok(committed.request_id)
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

    async fn set_departed_title_monitored(
        &self,
        subscription: &ListSubscription,
        title_id: &str,
        monitored: bool,
    ) -> AppResult<()> {
        if !subscription.is_personal() {
            return self.set_title_monitored(title_id, monitored).await;
        }
        let owner = self.list_owner(subscription).await?;
        if !owner.authorization.login_status.is_enabled() {
            return Err(AppError::Unauthorized("list owner is disabled".into()));
        }
        self.app
            .set_title_monitored(&owner, title_id, monitored)
            .await
            .map(|_| ())
    }

    async fn tag_departed_title(
        &self,
        subscription: &ListSubscription,
        title_id: &str,
        tag: &str,
    ) -> AppResult<()> {
        if !subscription.is_personal() {
            return self.tag_title(title_id, tag).await;
        }
        let owner = self.list_owner(subscription).await?;
        if !owner.authorization.login_status.is_enabled() {
            return Err(AppError::Unauthorized("list owner is disabled".into()));
        }
        let title = self
            .app
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound("title".into()))?;
        self.app
            .require_library_permission(&owner, &title.library_id, LibraryPermission::ManageTitles)
            .await?;
        self.app
            .ensure_title_tag_registered(tag, LEFT_LIST_TAG_DESCRIPTION)
            .await?;
        self.app
            .update_title_tags(&owner, &[title_id.into()], &[tag.into()], &[])
            .await
            .map(|_| ())
    }

    async fn title_exists(&self, title_id: &str) -> AppResult<bool> {
        Ok(self
            .app
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .is_some())
    }

    async fn record_departure(
        &self,
        subscription: &ListSubscription,
        title_id: &str,
        action: ListOnLeave,
    ) -> AppResult<()> {
        if subscription.is_personal() {
            if let Some(title) = self.app.services.catalog.titles.get_by_id(title_id).await? {
                self.app
                    .append_domain_event(crate::events::domain_events::new_user_domain_event(
                        &User::system_execution_actor(),
                        subscription.owner_user_id.clone(),
                        DomainEventPayload::ListTitleLeft(ListTitleLeftEventData {
                            list: ListEventSubject::of(subscription),
                            title: title_context_snapshot(&title),
                            action,
                        }),
                    ))
                    .await?;
            }
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
            return self
                .app
                .append_domain_event(crate::events::domain_events::new_user_domain_event(
                    &User::system_execution_actor(),
                    subscription.owner_user_id.clone(),
                    DomainEventPayload::ListSyncFailed(ListSyncFailedEventData {
                        list: ListEventSubject::of(subscription),
                        reason: failure.message.clone(),
                        failure_class: failure.class.as_str().into(),
                    }),
                ))
                .await
                .map(|_| ());
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
    async fn find_series_movies(
        &self,
        targets: &[(scryer_domain::ListSeriesMovieTarget, Vec<ExternalId>)],
    ) -> AppResult<Vec<Option<(String, String)>>> {
        let mut result = vec![None; targets.len()];
        for (batch, targets) in targets
            .chunks(super::gateway::RESOLVE_TITLES_BATCH)
            .enumerate()
        {
            let lookups = targets
                .iter()
                .enumerate()
                .map(|(index, (target, _))| crate::TitleExternalIdLookup {
                    lookup_index: index,
                    source: "tvdb".into(),
                    external_id: target.parent_tvdb_id.to_string(),
                })
                .collect::<Vec<_>>();
            let parents = self
                .app
                .services
                .catalog
                .titles
                .list_by_external_id_lookups(&lookups)
                .await?;
            let parent_ids = parents
                .iter()
                .filter(|entry| entry.title.facet != MediaFacet::Movie)
                .map(|entry| entry.title.id.clone())
                .collect::<Vec<_>>();
            let links = self
                .app
                .services
                .catalog
                .shows
                .list_series_movie_links_for_titles(&parent_ids)
                .await?;
            for parent in &parents {
                if parent.title.facet == MediaFacet::Movie {
                    continue;
                }
                let Some((_, ids)) = targets.get(parent.lookup_index) else {
                    continue;
                };
                if let Some(link) = links.iter().find(|link| {
                    link.series_title_id == parent.title.id
                        && series_movie_matches(&link.movie, ids)
                }) {
                    result[batch * super::gateway::RESOLVE_TITLES_BATCH + parent.lookup_index] =
                        Some((parent.title.id.clone(), link.id.clone()));
                }
            }
        }
        Ok(result)
    }

    async fn find_title(&self, kind: &MediaFacet, ids: &[ExternalId]) -> AppResult<Option<String>> {
        Ok(self
            .find_titles(&[(kind.clone(), ids.to_vec())])
            .await?
            .remove(0))
    }

    async fn find_titles(
        &self,
        targets: &[(MediaFacet, Vec<ExternalId>)],
    ) -> AppResult<Vec<Option<String>>> {
        let mut found = vec![None; targets.len()];
        for (batch, targets) in targets
            .chunks(super::gateway::RESOLVE_TITLES_BATCH)
            .enumerate()
        {
            let lookups = targets
                .iter()
                .enumerate()
                .flat_map(|(index, (_, ids))| {
                    ids.iter().map(move |id| crate::TitleExternalIdLookup {
                        lookup_index: index,
                        source: id.source.clone(),
                        external_id: id.value.clone(),
                    })
                })
                .collect::<Vec<_>>();
            for entry in self
                .app
                .services
                .catalog
                .titles
                .list_by_external_id_lookups(&lookups)
                .await?
            {
                let Some((kind, _)) = targets.get(entry.lookup_index) else {
                    return Err(AppError::Repository("invalid library lookup index".into()));
                };
                if &entry.title.facet == kind {
                    let slot = &mut found
                        [batch * super::gateway::RESOLVE_TITLES_BATCH + entry.lookup_index];
                    if slot.as_ref().is_none_or(|id: &String| id > &entry.title.id) {
                        *slot = Some(entry.title.id);
                    }
                }
            }
        }
        Ok(found)
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
            self.append_list_event(crate::events::domain_events::new_user_domain_event(
                actor,
                subscription.owner_user_id.clone(),
                DomainEventPayload::ListUnfollowed(scryer_domain::ListUnfollowedEventData {
                    list: ListEventSubject::of(subscription),
                }),
            ))
            .await;
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
        // Expired account links, and any provider grant one still held, go
        // even when nobody starts or polls another link.
        self.services
            .lists
            .account_runtime
            .prune_expired_links(Utc::now());
        if !self.experimental_features_enabled().await? {
            return Ok(ListSyncReport::default());
        }
        let lists = &self.services.lists;
        let actions = AppListActions::new(self);
        let gateway = self.services.library.metadata_gateway.clone();
        let resolver =
            GatewayListItemResolver::new(gateway.clone(), AppListLibraryLookup::new(self))
                .with_vocabulary(lists.vocabulary.clone(), lists.subscriptions.clone());
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

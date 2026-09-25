pub(crate) const HYDRATION_BULK_BATCH_SIZE: usize = 20;
const TITLE_MORE_LIKE_THIS_HYDRATION_LIMIT: usize = 24;
const TITLE_MORE_LIKE_THIS_BACKGROUND_REFRESH_HOURS: i64 = 24;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct HydrationCompletionOptions {
    sync_wanted_after_completion: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HydrationSource {
    BackgroundDue,
    LibraryScanFull,
    LibraryScanAdditive,
    Interactive,
    Maintenance,
}
impl HydrationSource {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::BackgroundDue => "background_due",
            Self::LibraryScanFull => "library_scan_full",
            Self::LibraryScanAdditive => "library_scan_additive",
            Self::Interactive => "interactive",
            Self::Maintenance => "maintenance",
        }
    }

    fn refresh_recommendations_inline(&self) -> bool {
        matches!(self, Self::Maintenance)
    }
}

const TITLE_RECOMMENDATION_REFRESH_WORKER_COUNT: usize = 2;

fn title_more_like_this_refresh_due(
    now: chrono::DateTime<chrono::Utc>,
    refreshed_at: Option<chrono::DateTime<chrono::Utc>>,
) -> bool {
    refreshed_at.is_none_or(|refreshed_at| {
        now.signed_duration_since(refreshed_at)
            >= chrono::Duration::hours(TITLE_MORE_LIKE_THIS_BACKGROUND_REFRESH_HOURS)
    })
}

pub(crate) struct TitleRecommendationRefreshJob {
    title: Title,
    external_ids: Vec<scryer_domain::ExternalId>,
    seeded_more_like_this: Vec<crate::DiscoveryTitle>,
    source: HydrationSource,
    queued_at: Instant,
}

impl TitleRecommendationRefreshJob {
    fn new(
        title: Title,
        external_ids: Vec<scryer_domain::ExternalId>,
        seeded_more_like_this: Vec<crate::DiscoveryTitle>,
        source: HydrationSource,
    ) -> Self {
        Self {
            title,
            external_ids,
            seeded_more_like_this,
            source,
            queued_at: Instant::now(),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum TitleRecommendationRefreshEnqueueOutcome {
    Queued,
    ReplacedPending,
    QueuedAfterInFlight,
}

#[derive(Default)]
pub(crate) struct TitleRecommendationRefreshQueue {
    pending_order: VecDeque<String>,
    pending: HashMap<String, TitleRecommendationRefreshJob>,
    in_flight: HashSet<String>,
    workers_started: bool,
}

impl TitleRecommendationRefreshQueue {
    fn mark_workers_started(&mut self) -> bool {
        if self.workers_started {
            false
        } else {
            self.workers_started = true;
            true
        }
    }

    fn enqueue(
        &mut self,
        job: TitleRecommendationRefreshJob,
    ) -> TitleRecommendationRefreshEnqueueOutcome {
        let title_id = job.title.id.clone();
        let was_pending = self.pending.insert(title_id.clone(), job).is_some();
        if !was_pending {
            self.pending_order.push_back(title_id.clone());
        }

        if was_pending {
            TitleRecommendationRefreshEnqueueOutcome::ReplacedPending
        } else if self.in_flight.contains(&title_id) {
            TitleRecommendationRefreshEnqueueOutcome::QueuedAfterInFlight
        } else {
            TitleRecommendationRefreshEnqueueOutcome::Queued
        }
    }

    fn take_next(&mut self) -> Option<TitleRecommendationRefreshJob> {
        let queued = self.pending_order.len();
        for _ in 0..queued {
            let title_id = self.pending_order.pop_front()?;
            if self.in_flight.contains(&title_id) {
                self.pending_order.push_back(title_id);
                continue;
            }
            if let Some(job) = self.pending.remove(&title_id) {
                self.in_flight.insert(title_id);
                return Some(job);
            }
        }
        None
    }

    fn complete(&mut self, title_id: &str) {
        self.in_flight.remove(title_id);
    }

    fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }
}

#[derive(Clone)]
pub(crate) struct HydrationTarget {
    pub(crate) title: Title,
    pub(crate) requested_tvdb_id: Option<i64>,
    pub(crate) requested_movie_ref: Option<MovieTitleRef>,
    pub(crate) sync_wanted_after_completion: bool,
    pub(crate) source: HydrationSource,
}
#[derive(Default)]
pub(crate) struct HydrationBatchOutcome {
    pub(crate) hydrated_titles: HashMap<String, Title>,
    pub(crate) failed_titles: HashMap<String, String>,
    pub(crate) deferred_titles: HashSet<String>,
}

/// Does this hydration failure reproduce identically on every retry?
///
/// A title external id that another title in the same library already owns is
/// deterministic: the same write replays into the same unique index, so
/// backing off burns the whole retry budget without ever succeeding. The store
/// now skips such an id instead of failing the write, so this is a backstop
/// for the paths that still surface the raw constraint violation.
pub(crate) fn is_non_retryable_hydration_failure(reason: &str) -> bool {
    (reason.contains("UNIQUE constraint failed")
        || reason.contains("duplicate key value violates unique constraint"))
        && reason.contains("title_external_ids")
}

/// The selections the SMG title-id surface introduced, lowercased and quoted the
/// way a GraphQL validation error names an unknown field. A gateway that
/// predates the surface answers any of them with `Cannot query field "<name>"`,
/// which is the same capability signal as the mapped gateway error -- and is
/// what a caller sees when the raw validation error reaches this layer.
const TITLE_ID_UNKNOWN_FIELD_MARKERS: [&str; 5] = [
    "\"titles\"",
    "\"resolvetitles\"",
    "\"searchtitles\"",
    "\"searchtitlesbatch\"",
    "\"title_id\"",
];

/// Derive a numbering bridge from the episode orders TVDB publishes.
///
/// The catalog follows the `official` order, so that is always the left side of
/// the join. The right side is whichever alternate order the title's setting
/// pins; on `auto` it is `alternate` if TVDB has one and `dvd` otherwise, which
/// is the order release groups are far more likely to have numbered by.
/// Returns `None` whenever no alternate order disagrees with the official one.
fn numbering_bridge_from_orders(
    orders: &[scryer_domain::EpisodeOrderSet],
    preference: scryer_domain::ReleaseNumbering,
) -> Option<scryer_domain::AnimeNumberingBridge> {
    use scryer_domain::NumberingBridgeSource;

    let entries_for = |season_type: &str| {
        orders
            .iter()
            .find(|order| order.season_type == season_type)
            .map(|order| order.entries.as_slice())
    };
    let official = entries_for("official")?;
    let candidates: &[(&str, NumberingBridgeSource)] = match preference.forced_season_type() {
        Some("dvd") => &[("dvd", NumberingBridgeSource::TvdbDvd)],
        Some(_) => &[("alternate", NumberingBridgeSource::TvdbAlternate)],
        None => &[
            ("alternate", NumberingBridgeSource::TvdbAlternate),
            ("dvd", NumberingBridgeSource::TvdbDvd),
        ],
    };
    candidates.iter().find_map(|(season_type, source)| {
        scryer_domain::numbering_bridge_from_episode_orders(
            official,
            entries_for(season_type)?,
            *source,
        )
    })
}

pub(crate) fn movie_title_queries_not_supported(error: &AppError) -> bool {
    let AppError::Repository(message) = error else {
        return false;
    };
    let message = message.to_ascii_lowercase();
    if message.contains("title-id")
        && (message.contains("does not support")
            || message.contains("not supported")
            || message.contains("unsupported"))
    {
        return true;
    }

    message.contains("cannot query field")
        && TITLE_ID_UNKNOWN_FIELD_MARKERS
            .iter()
            .any(|marker| message.contains(marker))
}
impl AppUseCase {
    async fn emit_hydration_started(&self, title: &Title) {
        self.emit_metadata_hydration_updated_event(title, MetadataHydrationState::Started, None)
            .await;
    }
}

#[cfg(test)]
mod location_lock_tests {
    use super::*;

    #[tokio::test]
    async fn location_locked_hydration_propagates_deferral_without_marking_metadata_complete() {
        let (app, user) = crate::lib_tests::bootstrap();
        let title = app
            .add_title(
                &user,
                NewTitle {
                    name: "Locked Hydration".into(),
                    facet: MediaFacet::Series,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let before = title.metadata_fetched_at;
        app.runtime.library.location_ownership.claim_all(
            "move",
            &[crate::location::ownership_guard::OwnedEntity::Title(
                title.id.clone(),
            )],
        );
        let result = app
            .apply_hydration_result(
                title.clone(),
                super::super::HydrationResult::default(),
                HydrationSource::BackgroundDue,
            )
            .await;
        assert!(matches!(result, Err(AppError::LocationOperationBusy(_))));
        let current = app.get_title(&user, &title.id).await.unwrap().unwrap();
        assert_eq!(current.metadata_fetched_at, before);
    }
}

#[cfg(test)]
mod title_id_capability_tests {
    use super::*;

    #[test]
    fn the_mapped_gateway_error_is_a_capability_error() {
        assert!(movie_title_queries_not_supported(&AppError::Repository(
            "metadata gateway does not support title-id queries".into()
        )));
    }

    /// Every title-id operation names its own root field when an old gateway
    /// rejects it, so the raw validation error must be read as the same
    /// capability signal no matter which operation hit the gateway first.
    #[test]
    fn a_raw_unknown_field_error_is_a_capability_error_for_every_operation() {
        for field in [
            "titles",
            "resolveTitles",
            "searchTitles",
            "searchTitlesBatch",
            "title_id",
        ] {
            let error =
                AppError::Repository(format!("Cannot query field \"{field}\" on type \"Query\"."));
            assert!(
                movie_title_queries_not_supported(&error),
                "unknown field {field} should be read as a capability error"
            );
        }
    }

    #[test]
    fn unrelated_gateway_failures_are_not_capability_errors() {
        assert!(!movie_title_queries_not_supported(&AppError::Repository(
            "metadata gateway request failed (503): upstream unavailable".into()
        )));
        assert!(!movie_title_queries_not_supported(&AppError::Repository(
            "Cannot query field \"seedMinimums\" on type \"Query\".".into()
        )));
        assert!(!movie_title_queries_not_supported(&AppError::Validation(
            "metadata gateway does not support title-id queries".into()
        )));
    }
}

#[cfg(test)]
mod title_recommendation_refresh_queue_tests {
    use super::*;
    use chrono::Utc;

    fn queue_title(id: &str, name: &str) -> Title {
        Title {
            id: id.to_string(),
            library_id: scryer_domain::default_library_id_for_facet(&MediaFacet::Movie),
            name: name.to_string(),
            facet: MediaFacet::Movie,
            monitored: true,
            tags: Vec::new(),
            canonical_tags: vec![],
            external_ids: Vec::new(),
            root_folder_id: "root".to_string(),
            created_by: None,
            created_at: Utc::now(),
            year: None,
            overview: None,
            poster_url: None,
            poster_source_url: None,
            background_url: None,
            background_source_url: None,
            sort_title: None,
            catalog_sort_key: name.to_string(),
            slug: None,
            imdb_id: None,
            runtime_minutes: None,
            popularity: None,
            content_status: None,
            language: None,
            first_aired: None,
            network: None,
            studio: None,
            country: None,
            aliases: Vec::new(),
            tagged_aliases: Vec::new(),
            metadata_language: None,
            metadata_fetched_at: None,
            min_availability: None,
            digital_release_date: None,
            folder_path: None,
        }
    }

    fn job(id: &str, name: &str) -> TitleRecommendationRefreshJob {
        TitleRecommendationRefreshJob::new(
            queue_title(id, name),
            Vec::new(),
            Vec::new(),
            HydrationSource::BackgroundDue,
        )
    }

    #[test]
    fn queue_replaces_pending_refresh_with_newest_payload() {
        let mut queue = TitleRecommendationRefreshQueue::default();

        assert_eq!(
            queue.enqueue(job("title-1", "Old")),
            TitleRecommendationRefreshEnqueueOutcome::Queued
        );
        assert_eq!(
            queue.enqueue(job("title-1", "New")),
            TitleRecommendationRefreshEnqueueOutcome::ReplacedPending
        );

        let next = queue.take_next().expect("queued job");
        assert_eq!(next.title.name, "New");
        assert!(queue.take_next().is_none());
    }

    #[test]
    fn queue_preserves_follow_up_refresh_for_in_flight_title() {
        let mut queue = TitleRecommendationRefreshQueue::default();
        queue.enqueue(job("title-1", "Initial"));
        let first = queue.take_next().expect("initial job");
        assert_eq!(first.title.name, "Initial");

        assert_eq!(
            queue.enqueue(job("title-1", "Follow Up")),
            TitleRecommendationRefreshEnqueueOutcome::QueuedAfterInFlight
        );
        assert!(
            queue.take_next().is_none(),
            "same title must not run concurrently"
        );

        queue.complete("title-1");
        let follow_up = queue.take_next().expect("follow-up job");
        assert_eq!(follow_up.title.name, "Follow Up");
    }

    #[test]
    fn title_more_like_this_refresh_due_after_twenty_four_hours() {
        let now = Utc::now();

        assert!(title_more_like_this_refresh_due(now, None));
        assert!(!title_more_like_this_refresh_due(
            now,
            Some(now - chrono::Duration::hours(23))
        ));
        assert!(title_more_like_this_refresh_due(
            now,
            Some(now - chrono::Duration::hours(24))
        ));
    }
}
impl AppUseCase {
    async fn emit_hydration_completed(&self, title: &Title) {
        self.emit_metadata_hydration_updated_event(title, MetadataHydrationState::Completed, None)
            .await;
    }
}
impl AppUseCase {
    async fn emit_hydration_failed(&self, title: &Title, reason: &str) {
        self.emit_metadata_hydration_updated_event(
            title,
            MetadataHydrationState::Failed,
            Some(reason.to_string()),
        )
        .await;
    }
}
impl AppUseCase {
    #[cfg(test)]
    pub(crate) async fn create_title_without_hydration(
        &self,
        actor: &User,
        request: NewTitle,
    ) -> AppResult<CreateTitleOutcome> {
        let library_id = scryer_domain::default_library_id_for_facet(&request.facet);
        self.create_title_without_hydration_with_options_patch_in_library(
            actor,
            request,
            library_id,
            TitleOptionsPatch::default(),
        )
        .await
    }

    #[cfg(test)]
    pub(crate) async fn create_title_without_hydration_in_library(
        &self,
        actor: &User,
        request: NewTitle,
        library_id: String,
    ) -> AppResult<CreateTitleOutcome> {
        self.create_title_without_hydration_with_options_patch_in_library(
            actor,
            request,
            library_id,
            TitleOptionsPatch::default(),
        )
        .await
    }

    pub(crate) async fn create_title_without_hydration_with_options_patch_in_library(
        &self,
        actor: &User,
        request: NewTitle,
        library_id: String,
        options_patch: TitleOptionsPatch,
    ) -> AppResult<CreateTitleOutcome> {
        self.require_library_permission(
            actor,
            &library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;
        self.create_title_without_hydration_with_options_patch_after_library_authorization(
            actor,
            request,
            library_id,
            options_patch,
        )
        .await
    }
}
impl AppUseCase {
    pub(crate) async fn new_title_for_library(
        &self,
        actor: &User,
        request: NewTitle,
        library_id: String,
    ) -> AppResult<Title> {
        if request.name.trim().is_empty() {
            return Err(AppError::Validation("title name is required".into()));
        }
        let root_folder_id = self
            .resolve_title_root_folder_id_for_library(
                &library_id,
                request.root_folder_id.as_deref(),
            )
            .await?;

        let name = request.name.trim().to_string();
        let mut tags = normalize_tags(&request.tags);
        self.canonicalize_title_quality_profile_tags(&mut tags)
            .await?;
        Ok(Title {
            id: Id::new().0,
            library_id: library_id.clone(),
            name,
            facet: request.facet,
            monitored: request.monitored,
            tags,
            canonical_tags: vec![],
            external_ids: sanitize_ids(request.external_ids),
            root_folder_id,
            created_by: Some(actor.id.clone()),
            created_at: Utc::now(),
            year: request.year,
            overview: request.overview,
            poster_url: request.poster_url,
            poster_source_url: None,
            background_url: None,
            background_source_url: None,
            sort_title: request.sort_title,
            // Recomputed by the title store on every write from (name, metadata_language); left
            // empty here because metadata_language is not yet known at creation, and the store —
            // not this struct field — is the source of truth for the persisted key.
            catalog_sort_key: String::new(),
            slug: request.slug,
            imdb_id: None,
            runtime_minutes: request.runtime_minutes,
            popularity: None,
            content_status: request.content_status,
            language: request.language,
            first_aired: None,
            network: None,
            studio: None,
            country: None,
            aliases: vec![],
            tagged_aliases: vec![],
            metadata_language: None,
            metadata_fetched_at: None,
            min_availability: request.min_availability,
            digital_release_date: None,
            folder_path: None,
        })
    }

    pub(crate) async fn create_title_without_hydration_after_library_authorization(
        &self,
        actor: &User,
        request: NewTitle,
        library_id: String,
    ) -> AppResult<CreateTitleOutcome> {
        self.create_title_without_hydration_with_options_patch_after_library_authorization(
            actor,
            request,
            library_id,
            TitleOptionsPatch::default(),
        )
        .await
    }

    pub(crate) async fn create_title_without_hydration_with_options_patch_after_library_authorization(
        &self,
        actor: &User,
        request: NewTitle,
        library_id: String,
        options_patch: TitleOptionsPatch,
    ) -> AppResult<CreateTitleOutcome> {
        let _profile_reference_guard = self
            .runtime
            .catalog
            .quality_profile_reference_lock
            .lock()
            .await;
        self.create_title_without_hydration_with_options_patch_after_library_authorization_lock_held(
            actor,
            request,
            library_id,
            options_patch,
        )
        .await
    }

    pub(crate) async fn create_title_without_hydration_with_options_patch_after_library_authorization_lock_held(
        &self,
        actor: &User,
        request: NewTitle,
        library_id: String,
        options_patch: TitleOptionsPatch,
    ) -> AppResult<CreateTitleOutcome> {
        let title = self
            .new_title_for_library(actor, request, library_id)
            .await?;

        let created = self
            .services
            .catalog
            .titles
            .create_or_get_existing_with_options_patch(title, options_patch.clone())
            .await?;
        // Must land before the caller notifies the hydration worker: season and
        // episode monitoring is decided from this selection.
        self.apply_title_monitor_selection_patch(&created.title, &options_patch)
            .await?;
        if !created.reused_existing {
            // A brand new title is a new matcher identity. Done here as well
            // as through `TitleAdded` so the guarantee does not depend on the
            // event append succeeding. A reused row changed no matcher input.
            self.invalidate_monitored_title_matcher().await;
            self.append_domain_event(new_title_domain_event(
                actor,
                &created.title,
                DomainEventPayload::TitleAdded(TitleAddedEventData {
                    title: title_context_snapshot(&created.title),
                }),
            ))
            .await?;
        }

        Ok(created)
    }

    pub(crate) async fn create_title_without_hydration_and_bind_pending_import_in_library(
        &self,
        actor: &User,
        request: NewTitle,
        library_id: String,
        pending_import_id: &str,
    ) -> AppResult<CreateTitleOutcome> {
        self.require_library_permission(
            actor,
            &library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;
        let _profile_reference_guard = self
            .runtime
            .catalog
            .quality_profile_reference_lock
            .lock()
            .await;
        let title = self
            .new_title_for_library(actor, request, library_id)
            .await?;
        let created = self
            .services
            .catalog
            .titles
            .create_or_get_existing_and_bind_pending_import(title, pending_import_id)
            .await?;
        if !created.reused_existing {
            // A brand new title is a new matcher identity. Done here as well
            // as through `TitleAdded` so the guarantee does not depend on the
            // event append succeeding. A reused row changed no matcher input.
            self.invalidate_monitored_title_matcher().await;
            self.append_domain_event(new_title_domain_event(
                actor,
                &created.title,
                DomainEventPayload::TitleAdded(TitleAddedEventData {
                    title: title_context_snapshot(&created.title),
                }),
            ))
            .await?;
        }

        Ok(created)
    }
}
impl AppUseCase {
    async fn complete_title_hydration(&self, title: &Title, options: HydrationCompletionOptions) {
        debug!(
            title_id = %title.id,
            title_name = %title.name,
            facet = %title.facet.as_str(),
            metadata_fetched = title.metadata_fetched_at.is_some(),
            sync_wanted_after_completion = options.sync_wanted_after_completion,
            "complete_title_hydration invoked"
        );

        if title.metadata_fetched_at.is_some() {
            self.notify_title_image_wakes(title);
            self.emit_hydration_completed(title).await;
            self.emit_title_updated_activity(None, title).await;
            if options.sync_wanted_after_completion {
                sync_wanted_after_hydration(self, title).await;
            }
        } else {
            debug!(
                title_id = %title.id,
                title_name = %title.name,
                "complete_title_hydration missing persisted metadata"
            );
            self.emit_hydration_failed(title, "metadata could not be persisted")
                .await;
        }
    }
}

/// One lane's claim on a title's hydration persistence.
enum TitleHydrationClaim {
    /// This lane owns the title until the guard drops. `title` is the row as it
    /// stands right now, re-read under the lock, so the result is applied to
    /// current state rather than to the struct the lane queued.
    Claimed {
        _guard: tokio::sync::OwnedMutexGuard<()>,
        title: Title,
    },
    /// Another lane hydrated this title while we waited. The caller only wanted
    /// an unhydrated title hydrated, and it now is; its work is done.
    AlreadyHydrated(Title),
}

impl AppUseCase {
    /// Take the title's hydration lock and decide whether there is still work.
    ///
    /// Only a lane that queued an *unhydrated* title short-circuits: a refresh
    /// or a language change deliberately re-hydrates a title that already has
    /// `metadata_fetched_at`, and must not be skipped.
    async fn claim_title_hydration(&self, title: &Title) -> TitleHydrationClaim {
        let wanted_first_hydration = title.metadata_fetched_at.is_none();
        let guard = self
            .runtime
            .catalog
            .title_hydration_locks
            .acquire(&title.id)
            .await;
        let current = self
            .services
            .catalog
            .titles
            .get_by_id(&title.id)
            .await
            .ok()
            .flatten()
            .unwrap_or_else(|| title.clone());
        if wanted_first_hydration && current.metadata_fetched_at.is_some() {
            debug!(
                title_id = %current.id,
                "title hydration: another lane hydrated this title while we waited"
            );
            return TitleHydrationClaim::AlreadyHydrated(current);
        }
        TitleHydrationClaim::Claimed {
            _guard: guard,
            title: current,
        }
    }

    /// Persist a series hydration result and read the row back, serialized
    /// against every other hydration of the same title.
    async fn persist_series_hydration(
        &self,
        target: HydrationTarget,
        result: super::HydrationResult,
    ) -> AppResult<Title> {
        let (_guard, title) = match self.claim_title_hydration(&target.title).await {
            TitleHydrationClaim::Claimed { _guard, title } => (_guard, title),
            TitleHydrationClaim::AlreadyHydrated(title) => return Ok(title),
        };
        let hydrated = self
            .apply_hydration_result(title, result, target.source)
            .await?;
        self.complete_title_hydration(
            &hydrated,
            HydrationCompletionOptions {
                sync_wanted_after_completion: target.sync_wanted_after_completion,
            },
        )
        .await;
        Ok(self
            .services
            .catalog
            .titles
            .get_by_id(&hydrated.id)
            .await?
            .unwrap_or(hydrated))
    }

    async fn complete_movie_hydration(
        &self,
        target: &HydrationTarget,
        movie: MovieMetadata,
        language: &str,
        redirects: &[(i64, i64)],
    ) -> AppResult<Title> {
        let (_guard, current_title) = match self.claim_title_hydration(&target.title).await {
            TitleHydrationClaim::Claimed { _guard, title } => (_guard, title),
            TitleHydrationClaim::AlreadyHydrated(title) => return Ok(title),
        };
        let target = &HydrationTarget {
            title: current_title,
            ..target.clone()
        };
        let _location_guard = self
            .acquire_location_title_mutation(
                &crate::location::ownership_guard::TITLE_HYDRATION_ENTRY,
                &target.title.id,
            )
            .await?;
        let movie_smg_id = movie.smg_id;
        let redirected_from = extract_smg_id(&target.title).filter(|stored_smg_id| {
            movie_smg_id.is_some_and(|movie_smg_id| movie_smg_id != *stored_smg_id)
                && redirects
                    .iter()
                    .any(|(from, to)| *from == *stored_smg_id && Some(*to) == movie_smg_id)
        });
        let result = super::movie_to_hydration_result(movie, language);
        let hydrated = self
            .apply_hydration_result(target.title.clone(), result, target.source)
            .await?;

        if let (Some(movie_smg_id), Some(redirected_from)) = (movie_smg_id, redirected_from) {
            match self
                .services
                .catalog
                .titles
                .persist_smg_id(&hydrated.id, movie_smg_id, Some(redirected_from))
                .await
            {
                // An SMG id is an external id the matcher indexes.
                Ok(()) => self.invalidate_monitored_title_matcher().await,
                Err(error) => {
                    warn!(
                        title_id = %hydrated.id,
                        smg_id = movie_smg_id,
                        redirected_from,
                        error = %error,
                        "failed to persist redirected movie SMG title id"
                    );
                }
            }
        }

        self.complete_title_hydration(
            &hydrated,
            HydrationCompletionOptions {
                sync_wanted_after_completion: target.sync_wanted_after_completion,
            },
        )
        .await;
        self.services
            .catalog
            .titles
            .get_by_id(&hydrated.id)
            .await
            .map(|title| title.unwrap_or(hydrated))
    }
}
impl AppUseCase {
    pub(crate) async fn hydrate_titles_bulk_cancellable(
        &self,
        targets: Vec<HydrationTarget>,
        cancel_token: Option<&tokio_util::sync::CancellationToken>,
    ) -> AppResult<HydrationBatchOutcome> {
        let mut targets_by_language = HashMap::<String, Vec<HydrationTarget>>::new();
        let titles = targets
            .iter()
            .map(|target| target.title.clone())
            .collect::<Vec<_>>();
        let effective_languages = self.resolve_metadata_languages_for_titles(&titles).await;
        for target in targets {
            let language = effective_languages
                .get(&target.title.id)
                .cloned()
                .unwrap_or_else(|| "eng".to_string());
            targets_by_language
                .entry(language)
                .or_default()
                .push(target);
        }
        let mut outcome = HydrationBatchOutcome::default();
        let hydration_started_at = Instant::now();

        'languages: for (language, targets) in targets_by_language {
            for chunk in targets.chunks(HYDRATION_BULK_BATCH_SIZE) {
                if crate::library::library::library_scan_cancel_requested(cancel_token) {
                    break 'languages;
                }
                let chunk_started_at = Instant::now();
                let chunk_len = chunk.len();
                let hydrated_before = outcome.hydrated_titles.len();
                let failed_before = outcome.failed_titles.len();
                let mut movie_targets = Vec::new();
                let mut series_targets = Vec::new();

                for target in chunk.iter().cloned() {
                    self.emit_hydration_started(&target.title).await;
                    match target.title.facet {
                        MediaFacet::Movie => {
                            let movie_ref = target
                                .requested_movie_ref
                                .clone()
                                .or_else(|| movie_title_ref(&target.title));
                            let Some(movie_ref) = movie_ref else {
                                self.emit_hydration_failed(
                                    &target.title,
                                    "no movie external id found",
                                )
                                .await;
                                outcome.failed_titles.insert(
                                    target.title.id.clone(),
                                    "no movie external id found".to_string(),
                                );
                                continue;
                            };
                            movie_targets.push((target, movie_ref));
                        }
                        MediaFacet::Series | MediaFacet::Anime => {
                            let Some(tvdb_id) = target
                                .requested_tvdb_id
                                .or_else(|| extract_tvdb_id(&target.title))
                            else {
                                self.emit_hydration_failed(
                                    &target.title,
                                    "no tvdb external id found",
                                )
                                .await;
                                outcome.failed_titles.insert(
                                    target.title.id.clone(),
                                    "no tvdb external id found".to_string(),
                                );
                                continue;
                            };
                            series_targets.push((target, tvdb_id));
                        }
                    }
                }

                let movie_count = movie_targets.len();
                let series_count = series_targets.len();

                if !movie_targets.is_empty() {
                    let refs = movie_targets
                        .iter()
                        .map(|(_, movie_ref)| movie_ref.clone())
                        .collect::<Vec<_>>();
                    let movie_result = await_cancellable(
                        cancel_token,
                        self.services
                            .library
                            .metadata_gateway
                            .get_movie_titles(&refs, &language),
                    )
                    .await;
                    let Some(movie_result) = movie_result else {
                        break 'languages;
                    };

                    match movie_result {
                        Ok(movie_result) => {
                            for (ref_index, (target, _)) in movie_targets.iter().enumerate() {
                                if crate::library::library::library_scan_cancel_requested(
                                    cancel_token,
                                ) {
                                    break 'languages;
                                }
                                let title_id = target.title.id.clone();
                                let Some(movie) = movie_result.by_ref_index.get(&ref_index) else {
                                    self.emit_hydration_failed(
                                        &target.title,
                                        "bulk metadata response missing title",
                                    )
                                    .await;
                                    outcome.failed_titles.insert(
                                        title_id,
                                        "bulk metadata response missing title".to_string(),
                                    );
                                    continue;
                                };
                                let refreshed = self
                                    .complete_movie_hydration(
                                        target,
                                        movie.clone(),
                                        &language,
                                        &movie_result.redirects,
                                    )
                                    .await?;
                                if refreshed.metadata_fetched_at.is_some() {
                                    outcome
                                        .hydrated_titles
                                        .insert(refreshed.id.clone(), refreshed);
                                } else {
                                    outcome.failed_titles.insert(
                                        title_id,
                                        "metadata could not be persisted".to_string(),
                                    );
                                }
                            }
                        }
                        Err(error) if movie_title_queries_not_supported(&error) => {
                            let fallback_targets = movie_targets
                                .iter()
                                .filter(|(_, movie_ref)| movie_ref.tvdb_id.is_some())
                                .collect::<Vec<_>>();
                            let fallback_ids = fallback_targets
                                .iter()
                                .filter_map(|(_, movie_ref)| movie_ref.tvdb_id)
                                .collect::<Vec<_>>();
                            for (target, movie_ref) in &movie_targets {
                                if movie_ref.tvdb_id.is_none() {
                                    outcome.deferred_titles.insert(target.title.id.clone());
                                }
                            }
                            if !fallback_ids.is_empty() {
                                let legacy_result = await_cancellable(
                                    cancel_token,
                                    self.services.library.metadata_gateway.get_metadata_bulk(
                                        &fallback_ids,
                                        &[],
                                        &language,
                                    ),
                                )
                                .await;
                                let Some(legacy_result) = legacy_result else {
                                    break 'languages;
                                };
                                match legacy_result {
                                    Ok(legacy_result) => {
                                        for (target, movie_ref) in fallback_targets {
                                            let title_id = target.title.id.clone();
                                            let tvdb_id =
                                                movie_ref.tvdb_id.expect("filtered above");
                                            let Some(movie) = legacy_result.movies.get(&tvdb_id)
                                            else {
                                                self.emit_hydration_failed(
                                                    &target.title,
                                                    "bulk metadata response missing title",
                                                )
                                                .await;
                                                outcome.failed_titles.insert(
                                                    title_id,
                                                    "bulk metadata response missing title"
                                                        .to_string(),
                                                );
                                                continue;
                                            };
                                            let refreshed = self
                                                .complete_movie_hydration(
                                                    target,
                                                    movie.clone(),
                                                    &language,
                                                    &[],
                                                )
                                                .await?;
                                            if refreshed.metadata_fetched_at.is_some() {
                                                outcome
                                                    .hydrated_titles
                                                    .insert(refreshed.id.clone(), refreshed);
                                            } else {
                                                outcome.failed_titles.insert(
                                                    title_id,
                                                    "metadata could not be persisted".to_string(),
                                                );
                                            }
                                        }
                                    }
                                    Err(error) => {
                                        let reason = error.to_string();
                                        for (target, _) in fallback_targets {
                                            self.emit_hydration_failed(&target.title, &reason)
                                                .await;
                                            outcome
                                                .failed_titles
                                                .insert(target.title.id.clone(), reason.clone());
                                        }
                                    }
                                }
                            }
                        }
                        Err(error) => {
                            let reason = error.to_string();
                            for (target, _) in &movie_targets {
                                self.emit_hydration_failed(&target.title, &reason).await;
                                outcome
                                    .failed_titles
                                    .insert(target.title.id.clone(), reason.clone());
                            }
                        }
                    }
                }

                if !series_targets.is_empty() {
                    let series_ids = series_targets
                        .iter()
                        .map(|(_, tvdb_id)| *tvdb_id)
                        .collect::<Vec<_>>();
                    let series_result = await_cancellable(
                        cancel_token,
                        self.services.library.metadata_gateway.get_metadata_bulk(
                            &[],
                            &series_ids,
                            &language,
                        ),
                    )
                    .await;
                    let Some(series_result) = series_result else {
                        break 'languages;
                    };
                    match series_result {
                        Ok(series_result) => {
                            let series_items = series_result.series.values().collect::<Vec<_>>();
                            let movie_metadata = await_cancellable(
                                cancel_token,
                                crate::catalog::facets::handler::hydrate_referenced_movie_metadata(
                                    self.services.library.metadata_gateway.as_ref(),
                                    &series_items,
                                    &language,
                                ),
                            )
                            .await;
                            let Some(movie_metadata) = movie_metadata else {
                                break 'languages;
                            };
                            let movie_metadata = movie_metadata
                                .inspect_err(|error| {
                                    warn!(error = %error, "linked movie metadata hydration failed");
                                })
                                .unwrap_or_default();
                            for (target, tvdb_id) in series_targets {
                                if crate::library::library::library_scan_cancel_requested(
                                    cancel_token,
                                ) {
                                    break 'languages;
                                }
                                let title_id = target.title.id.clone();
                                let title_facet = target.title.facet.clone();
                                let title_source = target.source;
                                if let Some(series) = series_result.series.get(&tvdb_id) {
                                    let mut result = super::series_to_hydration_result(
                                        series.clone(),
                                        &language,
                                    );
                                    result.movie_metadata = movie_metadata.clone();
                                    let refreshed =
                                        match self.persist_series_hydration(target, result).await {
                                            Ok(refreshed) => refreshed,
                                            Err(error) => {
                                                outcome
                                                    .failed_titles
                                                    .insert(title_id, error.to_string());
                                                continue;
                                            }
                                        };
                                    if refreshed.metadata_fetched_at.is_some() {
                                        outcome
                                            .hydrated_titles
                                            .insert(refreshed.id.clone(), refreshed);
                                    } else {
                                        outcome.failed_titles.insert(
                                            title_id,
                                            "metadata could not be persisted".to_string(),
                                        );
                                    }
                                } else {
                                    warn!(
                                        hydration_source = title_source.as_str(),
                                        facet = title_facet.as_str(),
                                        title_id = %title_id,
                                        "title hydration failed: bulk metadata response missing series title"
                                    );
                                    self.emit_hydration_failed(
                                        &target.title,
                                        "bulk metadata response missing title",
                                    )
                                    .await;
                                    outcome.failed_titles.insert(
                                        title_id,
                                        "bulk metadata response missing title".to_string(),
                                    );
                                }
                            }
                        }
                        Err(error) => {
                            let reason = error.to_string();
                            for (target, _) in series_targets {
                                self.emit_hydration_failed(&target.title, &reason).await;
                                outcome
                                    .failed_titles
                                    .insert(target.title.id.clone(), reason.clone());
                            }
                        }
                    }
                }

                info!(
                    target_count = chunk_len,
                    movie_count,
                    series_count,
                    hydrated_delta = outcome.hydrated_titles.len() - hydrated_before,
                    failed_delta = outcome.failed_titles.len() - failed_before,
                    elapsed_ms = chunk_started_at.elapsed().as_millis(),
                    total_elapsed_ms = hydration_started_at.elapsed().as_millis(),
                    "metadata hydration chunk complete"
                );
            }
        }

        Ok(outcome)
    }
}
impl AppUseCase {
    pub(crate) async fn hydrate_title_single_apq(
        &self,
        target: HydrationTarget,
    ) -> AppResult<Title> {
        let language = self
            .resolve_metadata_language_for_title(&target.title)
            .await;
        self.hydrate_title_single_apq_with_language(target, &language)
            .await
    }

    pub(crate) async fn hydrate_title_single_apq_with_language(
        &self,
        target: HydrationTarget,
        language: &str,
    ) -> AppResult<Title> {
        self.emit_hydration_started(&target.title).await;
        match target.title.facet {
            MediaFacet::Movie => {
                let movie_ref = target
                    .requested_movie_ref
                    .clone()
                    .or_else(|| movie_title_ref(&target.title))
                    .ok_or_else(|| {
                        AppError::Repository("no movie external id found".to_string())
                    })?;
                let (movie, redirects) = match self
                    .services
                    .library
                    .metadata_gateway
                    .get_movie_titles(std::slice::from_ref(&movie_ref), language)
                    .await
                {
                    Ok(result) => (
                        result.by_ref_index.get(&0).cloned().ok_or_else(|| {
                            AppError::NotFound("movie metadata response missing title".to_string())
                        })?,
                        result.redirects,
                    ),
                    Err(error) if movie_title_queries_not_supported(&error) => {
                        let tvdb_id = movie_ref.tvdb_id.ok_or_else(|| {
                            AppError::Repository("no tvdb external id found".to_string())
                        })?;
                        (
                            self.services
                                .library
                                .metadata_gateway
                                .get_movie(tvdb_id, language)
                                .await?,
                            Vec::new(),
                        )
                    }
                    Err(error) => return Err(error),
                };
                let refreshed = self
                    .complete_movie_hydration(&target, movie, language, &redirects)
                    .await?;
                if refreshed.metadata_fetched_at.is_some() {
                    Ok(refreshed)
                } else {
                    Err(AppError::Repository(
                        "metadata could not be persisted".to_string(),
                    ))
                }
            }
            MediaFacet::Series | MediaFacet::Anime => {
                let tvdb_id = target
                    .requested_tvdb_id
                    .or_else(|| extract_tvdb_id(&target.title))
                    .ok_or_else(|| AppError::Repository("no tvdb external id found".to_string()))?;
                let series = self
                    .services
                    .library
                    .metadata_gateway
                    .get_series(tvdb_id, language)
                    .await?;
                let result = super::series_to_hydration_result(series, language);
                let refreshed = self.persist_series_hydration(target, result).await?;
                if refreshed.metadata_fetched_at.is_some() {
                    Ok(refreshed)
                } else {
                    Err(AppError::Repository(
                        "metadata could not be persisted".to_string(),
                    ))
                }
            }
        }
    }

    pub(crate) async fn hydrate_titles_bulk(
        &self,
        targets: Vec<HydrationTarget>,
    ) -> AppResult<HydrationBatchOutcome> {
        self.hydrate_titles_bulk_cancellable(targets, None).await
    }
}
impl AppUseCase {
    /// Store (or clear) the release-numbering bridge for a title.
    ///
    /// Unconditional: the bridge is a snapshot of an upstream dataset, so every
    /// hydration replaces it wholesale rather than merging — a season that the
    /// upstream order dropped must disappear here too, and `None` clears the row.
    ///
    /// Two datasets can supply one. Under `auto`, SMG's AniBridge community
    /// layout wins where it exists (anime, unchanged from before); otherwise a
    /// bridge is derived from the alternate — or failing that the DVD — episode
    /// order TVDB publishes, whenever that order numbers the series differently
    /// from the official one the catalog follows. The title's
    /// `release_numbering` setting overrides both: `official` stores nothing at
    /// all, and `alternate`/`dvd` store only the TVDB order they name, even for
    /// an anime title SMG supplies a community bridge for.
    ///
    /// One exception to "unconditional". `metadataBulk` has no argument for
    /// episode orders, so bulk hydration always arrives here with none — which
    /// is indistinguishable, from inside this function, from a single-title
    /// hydration that asked for orders and found no differing alternate one.
    /// Clearing on the first reading would delete a TVDB-derived bridge on
    /// every bulk sweep and restore it on the next single-title hydration, so
    /// no orders at all plus a stored TVDB-sourced row means "orders were not
    /// fetched": leave the row alone. The same holds for a pinned order even when
    /// SMG supplied a community bridge, since the pin never reads that one. An
    /// `Official` pin still clears it, an anime community bridge is still
    /// cleared when SMG stops supplying one, and a hydration that did carry
    /// orders still clears a row whose alternate order no longer differs.
    pub(crate) async fn replace_numbering_bridge_after_hydration(
        &self,
        title: &Title,
        bridge: Option<&scryer_domain::AnimeNumberingBridge>,
        episode_orders: &[scryer_domain::EpisodeOrderSet],
    ) {
        use scryer_domain::ReleaseNumbering;

        let preference = ReleaseNumbering::from_title_tags(&title.tags);
        let pinned = preference.forced_season_type().is_some();
        if preference != ReleaseNumbering::Official
            && (bridge.is_none() || pinned)
            && episode_orders.is_empty()
            && self
                .stored_bridge_is_admitted_tvdb_order(&title.id, preference)
                .await
        {
            debug!(
                title_id = %title.id,
                "hydration carried no episode orders; keeping the stored TVDB numbering bridge"
            );
            return;
        }
        let derived;
        let bridge = match preference {
            ReleaseNumbering::Official => None,
            ReleaseNumbering::Auto if bridge.is_some() => bridge,
            ReleaseNumbering::Auto | ReleaseNumbering::Alternate | ReleaseNumbering::Dvd => {
                derived = numbering_bridge_from_orders(episode_orders, preference);
                derived.as_ref()
            }
        };
        if let Err(error) = self
            .services
            .catalog
            .shows
            .replace_anime_numbering_bridge(&title.id, bridge)
            .await
        {
            warn!(
                title_id = %title.id,
                error = %error,
                "failed to persist release numbering bridge"
            );
            return;
        }
        // A cour's own name reaches the matcher only through the bridge
        // (`title_with_bridge_cour_titles`), so replacing the bridge changes
        // the names the cached matcher was built over.
        self.invalidate_monitored_title_matcher().await;
        if let Some(bridge) = bridge {
            debug!(
                title_id = %title.id,
                generated_on = %bridge.generated_on,
                source = bridge.source.as_str(),
                bridge_seasons = bridge.seasons.len(),
                corroborating_order = bridge.corroborating_order.as_deref().unwrap_or("none"),
                "stored release numbering bridge"
            );
        }
    }

    /// Whether the catalog already holds a bridge this instance derived from a
    /// TVDB episode order that `preference` still reads. A read failure answers
    /// `false`, which falls through to the ordinary replace path rather than
    /// preserving a row we cannot see.
    async fn stored_bridge_is_admitted_tvdb_order(
        &self,
        title_id: &str,
        preference: scryer_domain::ReleaseNumbering,
    ) -> bool {
        self.services
            .catalog
            .shows
            .get_anime_numbering_bridge(title_id)
            .await
            .unwrap_or_default()
            .is_some_and(|stored| {
                stored.source.is_tvdb_alternate_order()
                    && preference.admits_bridge_source(stored.source)
            })
    }

    /// Bring the stored bridge in line with a `release_numbering` setting the
    /// operator just changed. A row the new setting does not read is cleared at
    /// once, so nothing trusts it while the rebuild runs; then, unless the
    /// title is now pinned to `official`, a single-title hydration — the only
    /// kind that fetches TVDB episode orders — rebuilds the bridge in the
    /// background. A failed rebuild is logged and left to the next hydration.
    pub(crate) async fn reconcile_numbering_bridge_after_setting_change(&self, title: &Title) {
        let preference = scryer_domain::ReleaseNumbering::from_title_tags(&title.tags);
        match self
            .services
            .catalog
            .shows
            .get_anime_numbering_bridge(&title.id)
            .await
        {
            Ok(Some(stored)) if !preference.admits_bridge_source(stored.source) => {
                if let Err(error) = self
                    .services
                    .catalog
                    .shows
                    .replace_anime_numbering_bridge(&title.id, None)
                    .await
                {
                    warn!(
                        title_id = %title.id,
                        error = %error,
                        "failed to clear a numbering bridge the new release numbering setting does not read"
                    );
                } else {
                    // Cour names just left the catalog; see
                    // `replace_numbering_bridge_after_hydration`.
                    self.invalidate_monitored_title_matcher().await;
                }
            }
            Ok(_) => {}
            Err(error) => {
                warn!(
                    title_id = %title.id,
                    error = %error,
                    "failed to read the numbering bridge after a release numbering change"
                );
            }
        }
        if preference == scryer_domain::ReleaseNumbering::Official
            || extract_tvdb_id(title).is_none()
        {
            return;
        }
        let app = self.clone();
        let target = HydrationTarget {
            title: title.clone(),
            requested_tvdb_id: None,
            requested_movie_ref: None,
            sync_wanted_after_completion: false,
            source: HydrationSource::Interactive,
        };
        tokio::spawn(async move {
            let title_id = target.title.id.clone();
            if let Err(error) = app.hydrate_title_single_apq(target).await {
                warn!(
                    title_id = %title_id,
                    error = %error,
                    "failed to rebuild the numbering bridge after a release numbering change"
                );
            }
        });
    }

    /// Apply a [`HydrationResult`] to a title: persist metadata, create
    /// seasons/episodes, and enrich with anime mapping data.
    async fn apply_hydration_result(
        &self,
        title: Title,
        result: super::HydrationResult,
        source: HydrationSource,
    ) -> AppResult<Title> {
        let _location_guard = self
            .acquire_location_title_mutation(
                &crate::location::ownership_guard::TITLE_HYDRATION_ENTRY,
                &title.id,
            )
            .await?;
        let has_episodes = self
            .facet_registry
            .get(&title.facet)
            .is_some_and(|h| h.has_episodes());

        if has_episodes {
            debug!(
                hydration_source = source.as_str(),
                facet = title.facet.as_str(),
                title_id = %title.id,
                seasons = result.seasons.len(),
                episodes = result.episodes.len(),
                "received series metadata from gateway"
            );
        }

        let mut metadata_update = result.metadata_update;

        // Store anime-specific metadata as tags on the title
        if let Some(primary) =
            crate::catalog::facets::handler::primary_anime_mapping(&result.anime_mappings)
        {
            if let Some(score) = primary.score {
                metadata_update
                    .extra_tags
                    .push(format!("scryer:mal-score:{score}"));
            }
            if !primary.anime_media_type.is_empty() {
                metadata_update.extra_tags.push(format!(
                    "scryer:anime-media-type:{}",
                    primary.anime_media_type
                ));
            }
            if !primary.status.is_empty() {
                metadata_update
                    .extra_tags
                    .push(format!("scryer:anime-status:{}", primary.status));
            }
        }
        let recommendation_external_ids =
            crate::catalog::facets::handler::external_ids_from_hydration_metadata(
                title.external_ids.clone(),
                &metadata_update,
            );
        let persistence_started_at = Instant::now();

        let title = match self
            .services
            .catalog
            .titles
            .update_title_hydrated_metadata(&title.id, metadata_update)
            .await
        {
            Ok(updated) => {
                // Hydration is the main writer of names, aliases, year and
                // external ids; without this the matcher would keep serving
                // pre-hydration identities until some other write dirtied it.
                self.invalidate_monitored_title_matcher().await;
                updated
            }
            Err(err) => {
                warn!(
                    hydration_source = source.as_str(),
                    facet = title.facet.as_str(),
                    title_id = %title.id,
                    error = %err,
                    "failed to persist metadata"
                );
                // A deterministic failure is reported to the caller so the
                // retry scheduler can stop instead of replaying the identical
                // write; transient failures keep today's tolerant behaviour
                // and continue with the pre-hydration title.
                if is_non_retryable_hydration_failure(&err.to_string()) {
                    return Err(err);
                }
                title
            }
        };

        if !result.seasons.is_empty() || !result.episodes.is_empty() {
            self.create_series_seasons_and_episodes_with_movie_metadata(
                &title,
                &result.seasons,
                &result.episodes,
                &result.anime_mappings,
                &result.anime_movies,
                &result.movie_metadata,
            )
            .await;
        }

        self.replace_numbering_bridge_after_hydration(
            &title,
            result.anime_numbering_bridge.as_ref(),
            &result.episode_orders,
        )
        .await;

        info!(
            hydration_source = source.as_str(),
            facet = title.facet.as_str(),
            title_id = %title.id,
            seasons = result.seasons.len(),
            episodes = result.episodes.len(),
            elapsed_ms = persistence_started_at.elapsed().as_millis(),
            "metadata hydration persistence complete"
        );

        self.refresh_or_queue_title_more_like_this_after_hydration(
            &title,
            &recommendation_external_ids,
            &result.more_like_this,
            source,
        )
        .await;

        if title
            .poster_url
            .as_ref()
            .is_some_and(|value| !value.trim().is_empty())
        {
            self.runtime.catalog.poster_wake.notify_one();
        }
        if title
            .background_url
            .as_ref()
            .is_some_and(|value| !value.trim().is_empty())
        {
            self.runtime.catalog.fanart_wake.notify_one();
        }

        Ok(title)
    }

    async fn refresh_or_queue_title_more_like_this_after_hydration(
        &self,
        title: &Title,
        external_ids: &[scryer_domain::ExternalId],
        seeded_more_like_this: &[crate::DiscoveryTitle],
        source: HydrationSource,
    ) {
        if source.refresh_recommendations_inline() {
            let started_at = Instant::now();
            if let Err(err) = self
                .refresh_title_more_like_this_after_hydration_once(
                    title,
                    external_ids,
                    seeded_more_like_this,
                    source,
                )
                .await
            {
                warn!(
                    hydration_source = source.as_str(),
                    facet = title.facet.as_str(),
                    title_id = %title.id,
                    error = %err,
                    elapsed_ms = started_at.elapsed().as_millis(),
                    "failed to refresh title recommendations inline; keeping existing recommendations"
                );
            }
            return;
        }

        self.ensure_title_recommendation_refresh_workers_started()
            .await;
        let job = TitleRecommendationRefreshJob::new(
            title.clone(),
            external_ids.to_vec(),
            seeded_more_like_this.to_vec(),
            source,
        );
        let outcome = {
            let mut queue = self
                .runtime
                .catalog
                .title_recommendation_refresh_queue
                .lock()
                .await;
            queue.enqueue(job)
        };

        match outcome {
            TitleRecommendationRefreshEnqueueOutcome::Queued => info!(
                hydration_source = source.as_str(),
                facet = title.facet.as_str(),
                title_id = %title.id,
                seeded_count = seeded_more_like_this.len(),
                "queued title recommendations refresh after hydration"
            ),
            TitleRecommendationRefreshEnqueueOutcome::ReplacedPending => debug!(
                hydration_source = source.as_str(),
                facet = title.facet.as_str(),
                title_id = %title.id,
                seeded_count = seeded_more_like_this.len(),
                "coalesced pending title recommendations refresh after hydration"
            ),
            TitleRecommendationRefreshEnqueueOutcome::QueuedAfterInFlight => debug!(
                hydration_source = source.as_str(),
                facet = title.facet.as_str(),
                title_id = %title.id,
                seeded_count = seeded_more_like_this.len(),
                "queued follow-up title recommendations refresh after in-flight refresh"
            ),
        }
        self.runtime
            .catalog
            .title_recommendation_refresh_wake
            .notify_one();
    }

    pub(crate) async fn queue_title_more_like_this_refresh_if_due(
        &self,
        title: &Title,
        source: HydrationSource,
    ) -> AppResult<bool> {
        if crate::discovery::title_recommendations_subject(title, &[]).is_none() {
            debug!(
                hydration_source = source.as_str(),
                facet = title.facet.as_str(),
                title_id = %title.id,
                "skipping background title recommendations freshness check: title has no recommendation subject ids"
            );
            return Ok(false);
        }

        let existing = self
            .services
            .library
            .discovery
            .list_title_more_like_this_items(&title.id, 1)
            .await?;
        let now = self.runtime.environment.now();
        // A successful zero-result refresh intentionally stores no marker row;
        // those titles stay due so sparse SMG recommendation coverage can fill
        // in as upstream discovery data improves.
        let due =
            title_more_like_this_refresh_due(now, existing.first().map(|item| item.updated_at));
        if !due {
            return Ok(false);
        }

        self.refresh_or_queue_title_more_like_this_after_hydration(title, &[], &[], source)
            .await;
        Ok(true)
    }

    async fn ensure_title_recommendation_refresh_workers_started(&self) {
        let should_start = {
            let mut queue = self
                .runtime
                .catalog
                .title_recommendation_refresh_queue
                .lock()
                .await;
            queue.mark_workers_started()
        };
        if !should_start {
            return;
        }

        for worker_index in 0..TITLE_RECOMMENDATION_REFRESH_WORKER_COUNT {
            let app = self.clone();
            tokio::spawn(async move {
                app.run_title_recommendation_refresh_worker(worker_index)
                    .await;
            });
        }
    }

    async fn run_title_recommendation_refresh_worker(&self, worker_index: usize) {
        loop {
            let Some(job) = self.take_next_title_recommendation_refresh_job().await else {
                self.runtime
                    .catalog
                    .title_recommendation_refresh_wake
                    .notified()
                    .await;
                continue;
            };
            let title_id = job.title.id.clone();
            self.run_queued_title_more_like_this_refresh(job, worker_index)
                .await;
            let has_pending = {
                let mut queue = self
                    .runtime
                    .catalog
                    .title_recommendation_refresh_queue
                    .lock()
                    .await;
                queue.complete(&title_id);
                queue.has_pending()
            };
            if has_pending {
                self.runtime
                    .catalog
                    .title_recommendation_refresh_wake
                    .notify_one();
            }
        }
    }

    async fn take_next_title_recommendation_refresh_job(
        &self,
    ) -> Option<TitleRecommendationRefreshJob> {
        let mut queue = self
            .runtime
            .catalog
            .title_recommendation_refresh_queue
            .lock()
            .await;
        queue.take_next()
    }

    async fn run_queued_title_more_like_this_refresh(
        &self,
        job: TitleRecommendationRefreshJob,
        worker_index: usize,
    ) {
        let TitleRecommendationRefreshJob {
            title,
            external_ids,
            seeded_more_like_this,
            source,
            queued_at,
        } = job;

        let mut last_error = None;
        for attempt in 1_u32..=3 {
            let attempt_started_at = Instant::now();
            match self
                .refresh_title_more_like_this_after_hydration_once(
                    &title,
                    &external_ids,
                    &seeded_more_like_this,
                    source,
                )
                .await
            {
                Ok(()) => {
                    info!(
                        hydration_source = source.as_str(),
                        facet = title.facet.as_str(),
                        title_id = %title.id,
                        worker_index,
                        attempts = attempt,
                        elapsed_ms = queued_at.elapsed().as_millis(),
                        attempt_elapsed_ms = attempt_started_at.elapsed().as_millis(),
                        "completed queued title recommendations refresh"
                    );
                    return;
                }
                Err(err) => {
                    let error = err.to_string();
                    warn!(
                        hydration_source = source.as_str(),
                        facet = title.facet.as_str(),
                        title_id = %title.id,
                        worker_index,
                        attempt,
                        error = %error,
                        attempt_elapsed_ms = attempt_started_at.elapsed().as_millis(),
                        "queued title recommendations refresh attempt failed"
                    );
                    last_error = Some(error);
                    if attempt < 3 {
                        tokio::time::sleep(Duration::from_secs(1 << (attempt - 1))).await;
                    }
                }
            }
        }

        warn!(
            hydration_source = source.as_str(),
            facet = title.facet.as_str(),
            title_id = %title.id,
            worker_index,
            attempts = 3,
            error = %last_error.unwrap_or_else(|| "unknown error".to_string()),
            elapsed_ms = queued_at.elapsed().as_millis(),
            "queued title recommendations refresh exhausted retries"
        );
    }

    async fn refresh_title_more_like_this_after_hydration_once(
        &self,
        title: &Title,
        external_ids: &[scryer_domain::ExternalId],
        seeded_more_like_this: &[crate::DiscoveryTitle],
        source: HydrationSource,
    ) -> AppResult<()> {
        let Some((subject, source_target_keys)) =
            crate::discovery::title_recommendations_subject(title, external_ids)
        else {
            debug!(
                hydration_source = source.as_str(),
                facet = title.facet.as_str(),
                title_id = %title.id,
                "skipping title recommendations refresh: title has no recommendation subject ids"
            );
            return Ok(());
        };

        let language = self.metadata_language().await;
        let recommendations = if seeded_more_like_this.is_empty() {
            let input = crate::TitleRecommendationsInput {
                subject,
                query: String::new(),
                limit: TITLE_MORE_LIKE_THIS_HYDRATION_LIMIT as i32,
                language: language.clone(),
                include_unresolved: true,
            };
            self.services
                .library
                .metadata_gateway
                .title_recommendations(&input)
                .await?
                .results
        } else {
            seeded_more_like_this.to_vec()
        };

        let now = self.runtime.environment.now();
        let records = crate::discovery::title_more_like_this_item_records(
            &title.id,
            &source_target_keys,
            &recommendations,
            TITLE_MORE_LIKE_THIS_HYDRATION_LIMIT,
            now,
        )?;

        // Per-title work, so it stays out of the log at INFO. The store write
        // holds the single writer for its whole transaction; when it grows with
        // the catalog this histogram is what shows it, without a scan's worth of
        // log lines.
        let write_started_at = Instant::now();
        let outcome = self
            .services
            .library
            .discovery
            .replace_title_more_like_this_items(&title.id, &language, &records)
            .await;
        metrics::histogram!(
            "scryer_title_recommendations_store_duration_seconds",
            "outcome" => if outcome.is_ok() { "ok" } else { "error" },
        )
        .record(write_started_at.elapsed().as_secs_f64());
        metrics::histogram!("scryer_title_recommendations_card_count").record(records.len() as f64);
        outcome?;

        Ok(())
    }
}
impl AppUseCase {
    pub async fn hydrate_all_titles_for_current_language(&self) -> AppResult<u32> {
        const HYDRATE_ALL_TITLES_BATCH_SIZE: usize = 100;

        let mut refreshed = 0_u32;
        let mut after_id = None;
        loop {
            let titles = self
                .services
                .catalog
                .titles
                .list_page_after_id(after_id.clone(), HYDRATE_ALL_TITLES_BATCH_SIZE)
                .await?;
            if titles.is_empty() {
                break;
            }
            after_id = titles.last().map(|title| title.id.clone());
            refreshed += titles.len() as u32;
            let targets = titles
                .into_iter()
                .map(|title| HydrationTarget {
                    title,
                    requested_tvdb_id: None,
                    requested_movie_ref: None,
                    sync_wanted_after_completion: false,
                    source: HydrationSource::Maintenance,
                })
                .collect::<Vec<_>>();
            let _ = self.hydrate_titles_bulk(targets).await?;
            debug!(
                refreshed_titles = refreshed,
                batch_size = HYDRATE_ALL_TITLES_BATCH_SIZE,
                "metadata rehydration processed title batch"
            );
        }
        Ok(refreshed)
    }
}
impl AppUseCase {
    pub async fn rehydrate_all_metadata(&self, actor: &User, language: &str) -> AppResult<u64> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        let language = crate::normalize_metadata_language_code(language).ok_or_else(|| {
            AppError::Validation(
                "metadata language must be one of eng, spa, fra, deu, ita, por, kor, zho, or jpn"
                    .to_string(),
            )
        })?;

        self.services
            .config
            .settings
            .upsert_setting_json(
                SETTINGS_SCOPE_SYSTEM,
                METADATA_LANGUAGE_KEY,
                None,
                serde_json::to_string(&language)
                    .map_err(|error| AppError::Repository(error.to_string()))?,
                "rehydrate_metadata",
                Some(actor.id.clone()),
            )
            .await?;

        let cleared = self
            .services
            .catalog
            .titles
            .clear_metadata_language_for_all()
            .await?;
        // `metadata_language` is the language the matcher tags every untagged
        // name with, so clearing it catalog-wide reshapes every identity.
        self.invalidate_monitored_title_matcher().await;
        let discovery_app = self.clone();
        let discovery_language = language.clone();
        tokio::spawn(async move {
            match discovery_app
                .refresh_public_discovery_feed_now(JobTriggerSource::SystemInternal)
                .await
            {
                Ok(()) => {
                    info!(
                        language = %discovery_language,
                        "public discovery feed refreshed after metadata language change"
                    );
                }
                Err(error) => {
                    warn!(
                        error = %error,
                        language = %discovery_language,
                        "public discovery feed refresh failed after metadata language change"
                    );
                }
            }
        });
        let app = self.clone();
        tokio::spawn(async move {
            match app.hydrate_all_titles_for_current_language().await {
                Ok(refreshed) => {
                    info!(
                        language = %language,
                        titles_cleared = cleared,
                        titles_refreshed = refreshed,
                        "metadata rehydration completed"
                    );
                }
                Err(error) => {
                    warn!(
                        error = %error,
                        language = %language,
                        titles_cleared = cleared,
                        "metadata rehydration failed"
                    );
                }
            }
        });

        Ok(cleared)
    }
}
/// After successful hydration, sync wanted items for monitored titles.
async fn sync_wanted_after_hydration(app: &AppUseCase, title: &scryer_domain::Title) {
    if title.monitored && title.metadata_fetched_at.is_some() {
        app.sync_title_for_immediate_acquisition(title).await;
    }
}

#[cfg(test)]
mod numbering_bridge_from_orders_tests {
    use super::numbering_bridge_from_orders;
    use scryer_domain::{
        EpisodeOrderEntry, EpisodeOrderSet, NumberingBridgeSource, ReleaseNumbering,
    };

    fn entry(tvdb_id: i64, episode_number: i32, name: &str) -> EpisodeOrderEntry {
        EpisodeOrderEntry {
            tvdb_id,
            season_number: Some(1),
            episode_number: Some(episode_number),
            absolute_number: None,
            name: name.to_string(),
        }
    }

    /// Official 1..=3, alternate the same three episodes shifted by one, dvd the
    /// same three shifted by two, so the two alternate readings are telling
    /// apart by the offset they produce.
    pub(super) fn orders() -> Vec<EpisodeOrderSet> {
        vec![
            EpisodeOrderSet {
                season_type: "official".into(),
                entries: vec![
                    entry(9001, 1, "Lantern Pass"),
                    entry(9002, 2, "Salt Marsh Relay"),
                    entry(9003, 3, "Quarry Signal Tower"),
                ],
            },
            EpisodeOrderSet {
                season_type: "alternate".into(),
                entries: vec![
                    entry(9001, 2, "Lantern Pass"),
                    entry(9002, 3, "Salt Marsh Relay"),
                    entry(9003, 4, "Quarry Signal Tower"),
                ],
            },
            EpisodeOrderSet {
                season_type: "dvd".into(),
                entries: vec![
                    entry(9001, 3, "Lantern Pass"),
                    entry(9002, 4, "Salt Marsh Relay"),
                    entry(9003, 5, "Quarry Signal Tower"),
                ],
            },
        ]
    }

    fn first_offset(bridge: &scryer_domain::AnimeNumberingBridge) -> i32 {
        let season = &bridge.seasons[0];
        let range = &season.ranges[0];
        range.community_episode_start - range.tvdb_episode_start
    }

    #[test]
    fn auto_prefers_the_alternate_order_over_the_dvd_one() {
        let bridge = numbering_bridge_from_orders(&orders(), ReleaseNumbering::Auto)
            .expect("an alternate order should build a bridge");
        assert_eq!(bridge.source, NumberingBridgeSource::TvdbAlternate);
        assert_eq!(first_offset(&bridge), 1);
    }

    #[test]
    fn pinning_the_dvd_order_skips_the_alternate_one() {
        let bridge = numbering_bridge_from_orders(&orders(), ReleaseNumbering::Dvd)
            .expect("a dvd order should build a bridge");
        assert_eq!(bridge.source, NumberingBridgeSource::TvdbDvd);
        assert_eq!(first_offset(&bridge), 2);
    }

    #[test]
    fn pinning_the_alternate_order_never_falls_back_to_dvd() {
        let orders = vec![
            orders().swap_remove(0),
            EpisodeOrderSet {
                season_type: "dvd".into(),
                entries: vec![entry(9001, 3, "Lantern Pass")],
            },
        ];
        assert!(numbering_bridge_from_orders(&orders, ReleaseNumbering::Alternate).is_none());
    }

    #[test]
    fn orders_without_an_official_set_build_nothing() {
        let orders = vec![orders().swap_remove(1)];
        assert!(numbering_bridge_from_orders(&orders, ReleaseNumbering::Auto).is_none());
    }
}

/// Bulk hydration never asks SMG for episode orders, so it reaches the bridge
/// replacement with none. These cover the three readings of "no orders": keep a
/// TVDB-derived row, still honour an `Official` pin, and still clear an anime
/// community row SMG has stopped supplying.
#[cfg(test)]
mod numbering_bridge_replacement_tests {
    use crate::lib_tests::bootstrap;
    use scryer_domain::{
        AnimeCommunitySeason, AnimeCommunitySeasonRange, AnimeNumberingBridge, MediaFacet,
        NewTitle, NumberingBridgeSource, RELEASE_NUMBERING_TAG_PREFIX,
    };

    fn bridge(source: NumberingBridgeSource) -> AnimeNumberingBridge {
        AnimeNumberingBridge {
            source,
            generated_on: "fixture".into(),
            corroborating_order: None,
            seasons: vec![AnimeCommunitySeason {
                index: 1,
                ranges: vec![AnimeCommunitySeasonRange {
                    community_episode_start: 2,
                    community_episode_end: Some(30),
                    tvdb_season: 1,
                    tvdb_episode_start: 1,
                    tvdb_episode_end: Some(29),
                }],
                ..Default::default()
            }],
        }
    }

    /// The preserve case: an ordinary bulk sweep must leave a TVDB-derived row
    /// exactly where it is, or every sweep would delete the numbering the
    /// import and search lanes depend on.
    #[tokio::test]
    async fn a_bulk_hydration_without_orders_keeps_a_tvdb_sourced_bridge() {
        let (app, user) = bootstrap();
        let title = app
            .add_title(
                &user,
                NewTitle {
                    name: "Harbor Kiln".into(),
                    facet: MediaFacet::Series,
                    ..Default::default()
                },
            )
            .await
            .expect("create the series title");
        app.services
            .catalog
            .shows
            .replace_anime_numbering_bridge(
                &title.id,
                Some(&bridge(NumberingBridgeSource::TvdbAlternate)),
            )
            .await
            .expect("store the derived bridge");

        app.replace_numbering_bridge_after_hydration(&title, None, &[])
            .await;

        let stored = app
            .services
            .catalog
            .shows
            .get_anime_numbering_bridge(&title.id)
            .await
            .expect("read the bridge back");
        assert_eq!(
            stored.map(|stored| stored.source),
            Some(NumberingBridgeSource::TvdbAlternate),
            "a sweep that fetched no orders must not delete the derived bridge"
        );
    }

    /// Pinning a title back to the official order is an operator decision, and
    /// it clears the row even on a sweep that carried no orders.
    #[tokio::test]
    async fn an_official_pin_still_clears_a_tvdb_sourced_bridge() {
        let (app, user) = bootstrap();
        let mut title = app
            .add_title(
                &user,
                NewTitle {
                    name: "Salt Marsh Relay".into(),
                    facet: MediaFacet::Series,
                    ..Default::default()
                },
            )
            .await
            .expect("create the series title");
        app.services
            .catalog
            .shows
            .replace_anime_numbering_bridge(
                &title.id,
                Some(&bridge(NumberingBridgeSource::TvdbDvd)),
            )
            .await
            .expect("store the derived bridge");
        title.tags = vec![format!("{RELEASE_NUMBERING_TAG_PREFIX}official")];

        app.replace_numbering_bridge_after_hydration(&title, None, &[])
            .await;

        assert!(
            app.services
                .catalog
                .shows
                .get_anime_numbering_bridge(&title.id)
                .await
                .expect("read the bridge back")
                .is_none(),
            "an official pin must clear the derived bridge"
        );
    }

    /// The anime lane is untouched: SMG owning the bridge means SMG dropping it
    /// clears the row, exactly as before this change.
    #[tokio::test]
    async fn an_anime_community_bridge_is_still_cleared_when_smg_stops_supplying_one() {
        let (app, user) = bootstrap();
        let title = app
            .add_title(
                &user,
                NewTitle {
                    name: "Quarry Signal Tower".into(),
                    facet: MediaFacet::Anime,
                    ..Default::default()
                },
            )
            .await
            .expect("create the anime title");
        app.services
            .catalog
            .shows
            .replace_anime_numbering_bridge(
                &title.id,
                Some(&bridge(NumberingBridgeSource::AnimeCommunity)),
            )
            .await
            .expect("store the community bridge");

        app.replace_numbering_bridge_after_hydration(&title, None, &[])
            .await;

        assert!(
            app.services
                .catalog
                .shows
                .get_anime_numbering_bridge(&title.id)
                .await
                .expect("read the bridge back")
                .is_none(),
            "a community bridge SMG no longer supplies must be cleared"
        );
    }

    async fn stored_source(
        app: &crate::AppUseCase,
        title_id: &str,
    ) -> Option<NumberingBridgeSource> {
        app.services
            .catalog
            .shows
            .get_anime_numbering_bridge(title_id)
            .await
            .expect("read the bridge back")
            .map(|stored| stored.source)
    }

    /// A pinned TVDB order is the operator's explicit choice, so it beats the
    /// community bridge SMG supplies for an anime title.
    #[tokio::test]
    async fn a_pinned_order_beats_the_anime_community_bridge() {
        let (app, user) = bootstrap();
        let mut title = app
            .add_title(
                &user,
                NewTitle {
                    name: "Lantern Pass Choir".into(),
                    facet: MediaFacet::Anime,
                    ..Default::default()
                },
            )
            .await
            .expect("create the anime title");
        title.tags = vec![format!("{RELEASE_NUMBERING_TAG_PREFIX}dvd")];

        app.replace_numbering_bridge_after_hydration(
            &title,
            Some(&bridge(NumberingBridgeSource::AnimeCommunity)),
            &super::numbering_bridge_from_orders_tests::orders(),
        )
        .await;

        assert_eq!(
            stored_source(&app, &title.id).await,
            Some(NumberingBridgeSource::TvdbDvd),
            "the pinned dvd order must be stored, not the community bridge"
        );
    }

    /// A bulk sweep carries the community bridge but no orders; for a pinned
    /// title that must not overwrite the TVDB row the pin reads.
    #[tokio::test]
    async fn a_bulk_hydration_keeps_a_pinned_tvdb_bridge_over_the_community_one() {
        let (app, user) = bootstrap();
        let mut title = app
            .add_title(
                &user,
                NewTitle {
                    name: "Ferrous Delta Watch".into(),
                    facet: MediaFacet::Anime,
                    ..Default::default()
                },
            )
            .await
            .expect("create the anime title");
        app.services
            .catalog
            .shows
            .replace_anime_numbering_bridge(
                &title.id,
                Some(&bridge(NumberingBridgeSource::TvdbAlternate)),
            )
            .await
            .expect("store the derived bridge");
        title.tags = vec![format!("{RELEASE_NUMBERING_TAG_PREFIX}alternate")];

        app.replace_numbering_bridge_after_hydration(
            &title,
            Some(&bridge(NumberingBridgeSource::AnimeCommunity)),
            &[],
        )
        .await;

        assert_eq!(
            stored_source(&app, &title.id).await,
            Some(NumberingBridgeSource::TvdbAlternate)
        );
    }

    /// Changing the setting clears a bridge built for a different order at
    /// once, and keeps one the new setting still reads.
    #[tokio::test]
    async fn a_numbering_setting_change_clears_only_a_bridge_the_new_setting_does_not_read() {
        let (app, user) = bootstrap();
        let mut title = app
            .add_title(
                &user,
                NewTitle {
                    name: "Copper Weir Lights".into(),
                    facet: MediaFacet::Series,
                    ..Default::default()
                },
            )
            .await
            .expect("create the series title");
        app.services
            .catalog
            .shows
            .replace_anime_numbering_bridge(
                &title.id,
                Some(&bridge(NumberingBridgeSource::TvdbAlternate)),
            )
            .await
            .expect("store the derived bridge");

        title.tags = vec![format!("{RELEASE_NUMBERING_TAG_PREFIX}alternate")];
        app.reconcile_numbering_bridge_after_setting_change(&title)
            .await;
        assert_eq!(
            stored_source(&app, &title.id).await,
            Some(NumberingBridgeSource::TvdbAlternate),
            "an alternate pin still reads the alternate bridge"
        );

        title.tags = vec![format!("{RELEASE_NUMBERING_TAG_PREFIX}dvd")];
        app.reconcile_numbering_bridge_after_setting_change(&title)
            .await;
        assert_eq!(
            stored_source(&app, &title.id).await,
            None,
            "a dvd pin must not keep reading the alternate bridge"
        );
    }
}

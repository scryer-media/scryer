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
            let error = AppError::Repository(format!(
                "Cannot query field \"{field}\" on type \"Query\"."
            ));
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
    async fn new_title_for_library(
        &self,
        actor: &User,
        request: NewTitle,
        library_id: String,
    ) -> AppResult<Title> {
        if request.name.trim().is_empty() {
            return Err(AppError::Validation("title name is required".into()));
        }
        let root_folder_id = self
            .resolve_title_root_folder_id_for_library(&library_id, request.root_folder_id.as_deref())
            .await?;

        let name = request.name.trim().to_string();
        let mut tags = normalize_tags(&request.tags);
        self.canonicalize_title_quality_profile_tags(&mut tags).await?;
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
        let title = self.new_title_for_library(actor, request, library_id).await?;

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
        let title = self.new_title_for_library(actor, request, library_id).await?;
        let created = self
            .services
            .catalog
            .titles
            .create_or_get_existing_and_bind_pending_import(title, pending_import_id)
            .await?;
        if !created.reused_existing {
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
impl AppUseCase {
    async fn complete_movie_hydration(
        &self,
        target: &HydrationTarget,
        movie: MovieMetadata,
        language: &str,
        redirects: &[(i64, i64)],
    ) -> AppResult<Title> {
        let movie_smg_id = movie.smg_id;
        let redirected_from = extract_smg_id(&target.title).filter(|stored_smg_id| {
            movie_smg_id.is_some_and(|movie_smg_id| movie_smg_id != *stored_smg_id)
                && redirects.iter().any(|(from, to)| {
                    *from == *stored_smg_id && Some(*to) == movie_smg_id
                })
        });
        let result = super::movie_to_hydration_result(movie, language);
        let hydrated = self
            .apply_hydration_result(target.title.clone(), result, target.source)
            .await;

        if let (Some(movie_smg_id), Some(redirected_from)) = (movie_smg_id, redirected_from)
            && let Err(error) = self
                .services
                .catalog
                .titles
                .persist_smg_id(&hydrated.id, movie_smg_id, Some(redirected_from))
                .await
        {
            warn!(
                title_id = %hydrated.id,
                smg_id = movie_smg_id,
                redirected_from,
                error = %error,
                "failed to persist redirected movie SMG title id"
            );
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
            targets_by_language.entry(language).or_default().push(target);
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
                                if crate::library::library::library_scan_cancel_requested(cancel_token)
                                {
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
                                            let tvdb_id = movie_ref.tvdb_id.expect("filtered above");
                                            let Some(movie) = legacy_result.movies.get(&tvdb_id) else {
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
                                            self.emit_hydration_failed(&target.title, &reason).await;
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
                                if crate::library::library::library_scan_cancel_requested(cancel_token)
                                {
                                    break 'languages;
                                }
                                let title_id = target.title.id.clone();
                                let title_facet = target.title.facet.clone();
                                let title_source = target.source;
                                if let Some(series) = series_result.series.get(&tvdb_id) {
                                    let mut result =
                                        super::series_to_hydration_result(series.clone(), &language);
                                    result.movie_metadata = movie_metadata.clone();
                                    let hydrated = self
                                        .apply_hydration_result(target.title, result, title_source)
                                        .await;
                                    self.complete_title_hydration(
                                        &hydrated,
                                        HydrationCompletionOptions {
                                            sync_wanted_after_completion: target
                                                .sync_wanted_after_completion,
                                        },
                                    )
                                    .await;
                                    let refreshed = self
                                        .services
                                        .catalog
                                        .titles
                                        .get_by_id(&hydrated.id)
                                        .await?
                                        .unwrap_or(hydrated);
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
        let language = self.resolve_metadata_language_for_title(&target.title).await;
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
                let hydrated = self
                    .apply_hydration_result(target.title, result, target.source)
                    .await;
                self.complete_title_hydration(
                    &hydrated,
                    HydrationCompletionOptions {
                        sync_wanted_after_completion: target.sync_wanted_after_completion,
                    },
                )
                .await;
                let refreshed = self
                    .services
                    .catalog
                    .titles
                    .get_by_id(&hydrated.id)
                    .await?
                    .unwrap_or(hydrated);
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
    /// Apply a [`HydrationResult`] to a title: persist metadata, create
    /// seasons/episodes, and enrich with anime mapping data.
    async fn apply_hydration_result(
        &self,
        title: Title,
        result: super::HydrationResult,
        source: HydrationSource,
    ) -> Title {
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
            Ok(updated) => updated,
            Err(err) => {
                warn!(
                    hydration_source = source.as_str(),
                    facet = title.facet.as_str(),
                    title_id = %title.id,
                    error = %err,
                    "failed to persist metadata"
                );
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

        title
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

        self.services
            .library
            .discovery
            .replace_title_more_like_this_items(&title.id, &language, &records)
            .await?;

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

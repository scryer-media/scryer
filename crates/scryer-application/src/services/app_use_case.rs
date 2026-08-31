use super::*;

#[derive(Clone)]
pub struct AppUseCase {
    pub(crate) services: AppServices,
    pub(crate) runtime: AppRuntimeState,
    pub auth: JwtAuthConfig,
    pub facet_registry: Arc<FacetRegistry>,
    pub(crate) pending_import_resolution_locks: Arc<std::sync::Mutex<HashSet<String>>>,
    pub(crate) jwt_signing_keys: Arc<RwLock<HashMap<String, Vec<u8>>>>,
    pub(crate) jwt_signing_keys_loaded: Arc<OnceCell<()>>,
    pub(crate) jwt_signing_keys_seed_lock: Arc<Mutex<()>>,
    pub webauthn: RuntimeFeature<Arc<webauthn_rs::Webauthn>>,
}

impl AppUseCase {
    /// Install the executable-host restart callback used by application upgrades.
    pub fn set_application_upgrade_restart_handle(
        &self,
        handle: crate::application_upgrade::ApplicationUpgradeRestartHandle,
    ) {
        if let Ok(mut restart) = self.runtime.jobs.application_upgrade_restart.write() {
            *restart = Some(handle);
        }
    }

    /// Acquire the process-local coordinator for a destructive system-wide
    /// maintenance operation without waiting behind an existing operation.
    pub fn try_acquire_system_maintenance(&self) -> AppResult<tokio::sync::OwnedMutexGuard<()>> {
        self.runtime
            .jobs
            .system_maintenance_lock
            .clone()
            .try_lock_owned()
            .map_err(|_| AppError::Validation("maintenance operation in progress".to_string()))
    }

    pub async fn upstream_scheduler_snapshot(
        &self,
        filter: SchedulerSnapshotFilter,
    ) -> AppResult<SchedulerSnapshot> {
        self.services
            .integrations
            .upstream_scheduler
            .snapshot(filter)
            .await
    }

    pub async fn flush_upstream_scheduler(&self) -> AppResult<()> {
        self.services
            .integrations
            .upstream_scheduler
            .flush_pending()
            .await
    }

    pub fn set_recovery_admin_login_enabled(&self, enabled: bool) {
        self.runtime
            .security
            .recovery_admin_login_enabled
            .store(enabled, std::sync::atomic::Ordering::SeqCst);
    }

    pub(crate) fn recovery_admin_login_enabled(&self) -> bool {
        self.runtime
            .security
            .recovery_admin_login_enabled
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    async fn invalidate_monitored_title_matcher(&self) {
        let mut state = self.runtime.catalog.monitored_title_matcher.write().await;
        state.dirty = true;
        state.generation = state.generation.wrapping_add(1);
    }

    pub(crate) async fn monitored_title_matcher(
        &self,
    ) -> AppResult<Arc<crate::import_title_resolution::MonitoredTitleMatcher>> {
        const MATCHER_MAX_AGE: std::time::Duration = std::time::Duration::from_secs(60);

        let observed_generation = {
            let state = self.runtime.catalog.monitored_title_matcher.read().await;
            let fresh = state
                .built_at
                .is_some_and(|built_at| built_at.elapsed() <= MATCHER_MAX_AGE);
            if !state.dirty
                && fresh
                && let Some(matcher) = state.matcher.clone()
            {
                return Ok(matcher);
            }
            state.generation
        };

        let titles = self
            .services
            .catalog
            .titles
            .list_for_matching(None, None)
            .await?;
        let matcher = Arc::new(crate::import_title_resolution::MonitoredTitleMatcher::new(
            titles,
        ));

        let mut state = self.runtime.catalog.monitored_title_matcher.write().await;
        state.matcher = Some(matcher.clone());
        state.built_at = Some(std::time::Instant::now());
        // Only clear dirty when no invalidation raced the rebuild; a bumped
        // generation means this matcher may already be stale, so the next
        // caller rebuilds again rather than trusting it.
        if state.generation == observed_generation {
            state.dirty = false;
        }
        Ok(matcher)
    }

    pub fn runtime_build_lane(&self) -> BinaryLane {
        self.runtime.environment.build_lane
    }

    pub fn runtime_build_class(&self) -> BinaryClass {
        self.runtime.environment.build_class
    }

    pub(crate) fn runtime_supported_plugin_required_features(&self) -> Arc<HashSet<String>> {
        self.runtime
            .environment
            .supported_plugin_required_features
            .clone()
    }

    pub async fn runtime_performance(&self) -> RuntimePerformanceSnapshot {
        let environment = self.runtime.environment.clone();
        initialize_runtime_performance_snapshot(
            environment.performance_snapshot.as_ref(),
            environment.config_dir.clone(),
            Arc::new(probe_runtime_performance_snapshot),
        )
        .await
    }

    pub fn warm_runtime_performance(&self) {
        let app = self.clone();
        tokio::spawn(async move {
            let _ = app.runtime_performance().await;
        });
    }

    /// Test-only escape hatch for selectively overriding already-assembled services.
    ///
    /// Production assembly should go through `AppServices::builder(...).build()`.
    pub(crate) fn with_test_overrides<F>(&self, configure: F) -> Self
    where
        F: FnOnce(AppServicesBuilder) -> AppServicesBuilder,
    {
        let assembly = configure(AppServicesBuilder {
            services: self.services.clone(),
            runtime: self.runtime.clone(),
            configured: AppServicesBuildConfiguration::default(),
        })
        .build_partial_for_tests();
        Self {
            services: assembly.services,
            runtime: assembly.runtime,
            auth: self.auth.clone(),
            facet_registry: self.facet_registry.clone(),
            pending_import_resolution_locks: self.pending_import_resolution_locks.clone(),
            jwt_signing_keys: self.jwt_signing_keys.clone(),
            jwt_signing_keys_loaded: self.jwt_signing_keys_loaded.clone(),
            jwt_signing_keys_seed_lock: self.jwt_signing_keys_seed_lock.clone(),
            webauthn: self.webauthn.clone(),
        }
    }

    pub async fn append_domain_event(&self, event: NewDomainEvent) -> AppResult<DomainEvent> {
        let stored = self.services.events.domain_events.append(event).await?;
        self.publish_stored_domain_event(&stored).await;
        Ok(stored)
    }

    pub async fn publish_stored_domain_event(&self, stored: &DomainEvent) {
        if should_invalidate_wanted_projection(&stored.payload) {
            self.runtime
                .acquisition
                .invalidate_wanted_projection_cache();
        }
        if should_invalidate_monitored_title_matcher(&stored.payload) {
            self.invalidate_monitored_title_matcher().await;
        }
        let _ = self
            .runtime
            .events
            .domain_event_broadcast
            .send(stored.sequence);
        if crate::notifications::dispatcher::notification_event_type(&stored.payload).is_some() {
            tracing::debug!(
                sequence = stored.sequence,
                event_type = stored.payload.event_type().as_str(),
                "queued notification dispatcher wake for notification-relevant domain event"
            );
            let _ = self
                .runtime
                .events
                .notification_event_broadcast
                .send(stored.sequence);
        }
        self.maybe_accelerate_discovery_sync_for_scan_completion(stored)
            .await;
    }

    pub async fn append_domain_events(
        &self,
        events: Vec<NewDomainEvent>,
    ) -> AppResult<Vec<DomainEvent>> {
        let stored = self
            .services
            .events
            .domain_events
            .append_many(events)
            .await?;
        if stored
            .iter()
            .any(|event| should_invalidate_wanted_projection(&event.payload))
        {
            self.runtime
                .acquisition
                .invalidate_wanted_projection_cache();
        }
        if stored
            .iter()
            .any(|event| should_invalidate_monitored_title_matcher(&event.payload))
        {
            self.invalidate_monitored_title_matcher().await;
        }
        if let Some(last) = stored.last() {
            let _ = self
                .runtime
                .events
                .domain_event_broadcast
                .send(last.sequence);
        }
        let notification_count = stored
            .iter()
            .filter(|event| {
                crate::notifications::dispatcher::notification_event_type(&event.payload).is_some()
            })
            .count();
        if notification_count > 0
            && let Some(last) = stored.last()
        {
            tracing::debug!(
                high_water_sequence = last.sequence,
                batch_len = stored.len(),
                notification_events = notification_count,
                "queued notification dispatcher wake for notification-relevant domain event batch"
            );
            let _ = self
                .runtime
                .events
                .notification_event_broadcast
                .send(last.sequence);
        }
        for event in &stored {
            self.maybe_accelerate_discovery_sync_for_scan_completion(event)
                .await;
        }
        Ok(stored)
    }

    async fn maybe_accelerate_discovery_sync_for_scan_completion(&self, event: &DomainEvent) {
        let scryer_domain::DomainEventPayload::LibraryScanCompleted(data) = &event.payload else {
            return;
        };
        if data.found_titles <= 0 {
            return;
        }

        match self
            .services
            .library
            .discovery
            .get_discovery_sync_state(DISCOVERY_DEFAULT_SCOPE_KEY)
            .await
        {
            Ok(Some(state)) if state.last_success_generation_id.is_some() => {}
            Ok(_) => {
                if let Err(error) = self
                    .schedule_discovery_sync_soon_silent(
                        "library_scan_completed_before_first_snapshot",
                    )
                    .await
                {
                    tracing::warn!(
                        error = %error,
                        sequence = event.sequence,
                        facet = event.facet.as_ref().map(MediaFacet::as_str),
                        "failed to accelerate discovery sync after library scan"
                    );
                }
            }
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    sequence = event.sequence,
                    facet = event.facet.as_ref().map(MediaFacet::as_str),
                    "failed to inspect discovery sync state for scan acceleration"
                );
            }
        }
    }

    pub(crate) async fn refresh_import_record_queue_snapshot(&self, import_id: &str) {
        let record = match self
            .services
            .workflow
            .imports
            .get_import_by_id(import_id)
            .await
        {
            Ok(Some(record)) => record,
            Ok(None) => {
                tracing::warn!(
                    import_id,
                    "import record disappeared before queue snapshot refresh"
                );
                return;
            }
            Err(error) => {
                tracing::warn!(import_id, error = %error, "failed to refresh queue snapshot from import record");
                return;
            }
        };
        let (error_code, error_message) =
            crate::integration::workflow::import_record_error_overlay(&record);
        self.runtime
            .acquisition
            .download_queue_snapshot
            .stage_import_record(&record, error_code, error_message)
            .await;
    }

    pub async fn update_import_status_and_notify(
        &self,
        import_id: &str,
        status: ImportStatus,
        result_json: Option<String>,
    ) -> AppResult<()> {
        self.services
            .workflow
            .imports
            .update_import_status(import_id, status, result_json.clone())
            .await?;
        self.refresh_import_record_queue_snapshot(import_id).await;
        if matches!(status, ImportStatus::Completed | ImportStatus::Failed) {
            let _ = self.runtime.events.import_history_broadcast.send(());
        }

        if let Some(ref json) = result_json
            && let Ok(result) = serde_json::from_str::<ImportResult>(json)
            && matches!(status, ImportStatus::Failed | ImportStatus::Skipped)
        {
            let title = match result.title_id.as_ref() {
                Some(title_id) => self
                    .services
                    .catalog
                    .titles
                    .get_by_id(title_id)
                    .await?
                    .map(|title| crate::domain_events::title_context_snapshot(&title)),
                None => None,
            };
            let reason = result.error_message.clone().or_else(|| {
                result
                    .skip_reason
                    .as_ref()
                    .map(|reason| reason.as_str().to_string())
            });

            let event = if let Some(title_id) = result.title_id.as_ref() {
                let facet = title.as_ref().map(|snapshot| snapshot.facet.clone());
                NewDomainEvent {
                    event_id: Id::new().0,
                    occurred_at: Utc::now(),
                    actor_kind: scryer_domain::DomainEventActorKind::System,
                    actor_user_id: None,
                    actor_display_name: "System".to_string(),
                    title_id: Some(title_id.clone()),
                    facet,
                    correlation_id: None,
                    causation_id: None,
                    schema_version: 1,
                    stream: scryer_domain::DomainEventStream::Title {
                        title_id: title_id.clone(),
                    },
                    payload: scryer_domain::DomainEventPayload::ImportRejected(
                        scryer_domain::ImportRejectedEventData {
                            title,
                            status,
                            import_id: Some(result.import_id.clone()),
                            source_system: result.source_system.clone(),
                            source_ref: result.source_ref.clone(),
                            source_title: result.source_title.clone(),
                            source_path: Some(result.source_path.clone()),
                            dest_path: result.dest_path.clone(),
                            quality: result.quality.clone(),
                            reason,
                            skip_reason: result.skip_reason.clone(),
                            episode_ids: result.episode_ids.clone(),
                        },
                    ),
                }
            } else {
                crate::domain_events::new_global_domain_event(
                    None,
                    scryer_domain::DomainEventPayload::ImportRejected(
                        scryer_domain::ImportRejectedEventData {
                            title: None,
                            status,
                            import_id: Some(result.import_id.clone()),
                            source_system: result.source_system.clone(),
                            source_ref: result.source_ref.clone(),
                            source_title: result.source_title.clone(),
                            source_path: Some(result.source_path.clone()),
                            dest_path: result.dest_path.clone(),
                            quality: result.quality.clone(),
                            reason,
                            skip_reason: result.skip_reason.clone(),
                            episode_ids: result.episode_ids.clone(),
                        },
                    ),
                )
            };

            let _ = self.append_domain_event(event).await;
        }
        Ok(())
    }

    pub fn publish_settings_changed(&self, changed_keys: Vec<String>) {
        let _ = self
            .runtime
            .events
            .settings_changed_broadcast
            .send(changed_keys);
    }

    pub fn publish_indexers_changed(&self) {
        let _ = self.runtime.events.indexers_changed_broadcast.send(());
    }

    pub fn publish_provider_catalog_changed(&self, families: Vec<ProviderCatalogFamily>) {
        if families.is_empty() {
            return;
        }

        let _ = self
            .runtime
            .events
            .provider_catalog_changed_broadcast
            .send(families);
    }

    pub async fn indexer_query_stats(&self, actor: &User) -> AppResult<Vec<IndexerQueryStats>> {
        let settings_permissions = scryer_domain::AppPermissionMask::from_permissions([
            scryer_domain::AppPermission::ManageSystemSettings,
            scryer_domain::AppPermission::ManageCatalogSettings,
        ]);
        if !self
            .has_any_app_permission(actor, settings_permissions)
            .await?
        {
            return Err(AppError::Unauthorized(
                "You do not have permission to perform this action".to_string(),
            ));
        }
        Ok(self.services.integrations.indexer_stats.all_stats())
    }

    /// Count one release grabbed through `indexer_id` toward that indexer's
    /// trailing-24h grab total.
    ///
    /// Call this only after a download client has *accepted* the submission;
    /// failed submissions are not grabs. A submission with no indexer identity
    /// (a manual push, or a release whose provider Scryer never recorded) is
    /// skipped rather than bucketed under a placeholder, so the dashboard's
    /// per-indexer column stays attributable.
    pub(crate) fn record_indexer_grab(&self, indexer_id: Option<&str>, indexer_name: Option<&str>) {
        let Some(indexer_id) = indexer_id.map(str::trim).filter(|id| !id.is_empty()) else {
            return;
        };
        let indexer_name = indexer_name
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or(indexer_id);
        self.services
            .integrations
            .indexer_stats
            .record_grab(indexer_id, indexer_name);
    }

    pub async fn cached_health_check_results(
        &self,
        actor: &User,
    ) -> AppResult<Vec<HealthCheckResult>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        Ok(self.runtime.health.results.read().await.clone())
    }

    pub async fn list_import_history(
        &self,
        actor: &User,
        limit: usize,
    ) -> AppResult<Vec<ImportRecord>> {
        let allowed_library_ids = self
            .authorized_library_ids(actor, None, scryer_domain::LibraryPermission::View)
            .await?
            .into_iter()
            .collect::<std::collections::HashSet<_>>();
        let records = self.services.workflow.imports.list_imports(limit).await?;
        let mut title_library_cache = std::collections::HashMap::<String, Option<String>>::new();
        let mut visible = Vec::new();
        for record in records {
            let title_id = record
                .result_json
                .as_deref()
                .and_then(|json| serde_json::from_str::<scryer_domain::ImportResult>(json).ok())
                .and_then(|result| result.title_id)
                .or_else(|| {
                    serde_json::from_str::<crate::ManualImportRequestPayload>(&record.payload_json)
                        .ok()
                        .and_then(|payload| payload.title_id)
                });
            let allowed = if let Some(title_id) = title_id {
                let library_id = if let Some(cached) = title_library_cache.get(&title_id) {
                    cached.clone()
                } else {
                    let library_id = self
                        .services
                        .catalog
                        .titles
                        .get_by_id(&title_id)
                        .await?
                        .map(|title| title.library_id);
                    title_library_cache.insert(title_id.clone(), library_id.clone());
                    library_id
                };
                library_id
                    .as_ref()
                    .is_some_and(|library_id| allowed_library_ids.contains(library_id))
            } else {
                false
            };
            if allowed {
                visible.push(record);
            }
        }
        Ok(visible)
    }

    async fn require_library_permission_for_title(
        &self,
        actor: &User,
        title_id: &str,
        permission: scryer_domain::LibraryPermission,
    ) -> AppResult<()> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;
        self.require_library_permission(actor, &title.library_id, permission)
            .await
    }

    async fn require_any_library_permission_for_service(
        &self,
        actor: &User,
        permission: scryer_domain::LibraryPermission,
    ) -> AppResult<()> {
        if self
            .authorized_library_ids(actor, None, permission)
            .await?
            .is_empty()
        {
            Err(AppError::Unauthorized(
                "You do not have access to this library".to_string(),
            ))
        } else {
            Ok(())
        }
    }

    async fn derive_wanted_item_library_id(
        &self,
        wanted: &AcquisitionScopeState,
    ) -> AppResult<String> {
        if let Some(library_id) = wanted.library_id.as_deref() {
            return Ok(library_id.to_string());
        }
        self.services
            .catalog
            .titles
            .get_by_id(&wanted.title_id)
            .await?
            .map(|title| title.library_id)
            .ok_or_else(|| AppError::NotFound(format!("title {}", wanted.title_id)))
    }

    /// Retain only the acquisition scope states whose owning library the actor
    /// holds `permission` on. Mirrors the per-item permission derivation used by
    /// `get_wanted_item` / `get_wanted_item_for_management` (the joined
    /// `library_id` is the title's library), silently dropping forbidden or
    /// orphaned rows for batch/dataloader callers.
    pub(crate) async fn filter_wanted_items_for_permission(
        &self,
        actor: &User,
        items: Vec<AcquisitionScopeState>,
        permission: scryer_domain::LibraryPermission,
    ) -> AppResult<Vec<AcquisitionScopeState>> {
        let allowed_library_ids = self
            .authorized_library_ids(actor, None, permission)
            .await?
            .into_iter()
            .collect::<std::collections::HashSet<_>>();
        if allowed_library_ids.is_empty() {
            return Ok(Vec::new());
        }
        Ok(items
            .into_iter()
            .filter(|item| {
                item.library_id
                    .as_deref()
                    .is_some_and(|library_id| allowed_library_ids.contains(library_id))
            })
            .collect())
    }

    pub async fn find_download_submission_by_client_item_id(
        &self,
        actor: &User,
        client_id: Option<&str>,
        client_type: &str,
        download_client_item_id: &str,
    ) -> AppResult<Option<DownloadSubmission>> {
        let submission = self
            .services
            .workflow
            .download_submissions
            .find_by_client_item_id(&ClientJobLocator::new(
                client_id,
                client_type,
                download_client_item_id,
            ))
            .await?;
        if let Some(submission) = submission.as_ref() {
            self.require_library_permission_for_title(
                actor,
                &submission.title_id,
                scryer_domain::LibraryPermission::View,
            )
            .await?;
        }
        Ok(submission)
    }

    pub async fn search_metadata(
        &self,
        actor: &User,
        query: &str,
        type_hint: &str,
        limit: i32,
        language: &str,
        year: Option<i32>,
    ) -> AppResult<Vec<RichMetadataSearchItem>> {
        self.require_any_library_permission_for_service(
            actor,
            scryer_domain::LibraryPermission::View,
        )
        .await?;
        let gateway = &self.services.library.metadata_gateway;
        if type_hint.eq_ignore_ascii_case("movie") {
            match gateway
                .search_titles(query, "movie", limit, language, year)
                .await
            {
                Ok(results) => Ok(results),
                Err(error)
                    if crate::catalog_workflow::movie_title_queries_not_supported(&error) =>
                {
                    gateway
                        .search_tvdb_rich(query, type_hint, limit, language, year)
                        .await
                }
                Err(error) => Err(error),
            }
        } else {
            gateway
                .search_tvdb_rich(query, type_hint, limit, language, year)
                .await
        }
    }

    pub async fn search_metadata_tvdb(
        &self,
        actor: &User,
        query: &str,
        type_hint: &str,
        year: Option<i32>,
    ) -> AppResult<Vec<MetadataSearchItem>> {
        self.require_any_library_permission_for_service(
            actor,
            scryer_domain::LibraryPermission::View,
        )
        .await?;
        self.services
            .library
            .metadata_gateway
            .search_tvdb(query, type_hint, year)
            .await
    }

    pub async fn search_metadata_batch(
        &self,
        actor: &User,
        queries: &[MetadataSearchQuery],
        language: &str,
    ) -> AppResult<HashMap<MetadataSearchQuery, Vec<MetadataSearchItem>>> {
        self.require_any_library_permission_for_service(
            actor,
            scryer_domain::LibraryPermission::View,
        )
        .await?;
        self.services
            .library
            .metadata_gateway
            .search_tvdb_batch(queries, language)
            .await
    }

    pub async fn search_metadata_multi(
        &self,
        actor: &User,
        query: &str,
        limit: i32,
        language: &str,
    ) -> AppResult<MultiMetadataSearchResult> {
        self.require_any_library_permission_for_service(
            actor,
            scryer_domain::LibraryPermission::View,
        )
        .await?;
        let gateway = &self.services.library.metadata_gateway;
        let mut legacy = gateway.search_tvdb_multi(query, limit, language).await?;
        match gateway
            .search_titles(query, "movie", limit, language, None)
            .await
        {
            Ok(movies) => legacy.movies = movies,
            Err(error) if crate::catalog_workflow::movie_title_queries_not_supported(&error) => {}
            // The legacy multi-search already succeeded for every facet. A
            // failure of the added movie call must not throw away the series and
            // anime results with it; keep the legacy movie bucket instead.
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "metadata gateway title search failed during multi-search; keeping the legacy movie results"
                );
            }
        }
        Ok(legacy)
    }

    pub async fn get_metadata_movie(
        &self,
        actor: &User,
        tvdb_id: i64,
        language: &str,
    ) -> AppResult<MovieMetadata> {
        self.require_any_library_permission_for_service(
            actor,
            scryer_domain::LibraryPermission::View,
        )
        .await?;
        self.services
            .library
            .metadata_gateway
            .get_movie(tvdb_id, language)
            .await
    }

    pub async fn get_metadata_movie_by_ref(
        &self,
        actor: &User,
        movie_ref: &MovieTitleRef,
        language: &str,
    ) -> AppResult<MovieMetadata> {
        self.require_any_library_permission_for_service(
            actor,
            scryer_domain::LibraryPermission::View,
        )
        .await?;
        if movie_ref.smg_id.is_none()
            && movie_ref.tvdb_id.is_none()
            && movie_ref.tmdb_id.is_none()
            && movie_ref
                .imdb_id
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
        {
            return Err(AppError::Validation("a title identity is required".into()));
        }

        match self
            .services
            .library
            .metadata_gateway
            .get_movie_titles(std::slice::from_ref(movie_ref), language)
            .await
        {
            Ok(result) => {
                result.by_ref_index.get(&0).cloned().ok_or_else(|| {
                    AppError::NotFound("movie metadata response missing title".into())
                })
            }
            Err(error) if crate::catalog_workflow::movie_title_queries_not_supported(&error) => {
                let tvdb_id = movie_ref.tvdb_id.ok_or_else(|| {
                    AppError::Repository("legacy metadata gateway requires a tvdb id".into())
                })?;
                self.services
                    .library
                    .metadata_gateway
                    .get_movie(tvdb_id, language)
                    .await
            }
            Err(error) => Err(error),
        }
    }

    pub async fn get_metadata_series(
        &self,
        actor: &User,
        tvdb_id: i64,
        language: &str,
    ) -> AppResult<SeriesMetadata> {
        self.require_any_library_permission_for_service(
            actor,
            scryer_domain::LibraryPermission::View,
        )
        .await?;
        self.services
            .library
            .metadata_gateway
            .get_series(tvdb_id, language)
            .await
    }

    pub async fn list_title_media_files(
        &self,
        actor: &User,
        title_id: &str,
    ) -> AppResult<Vec<TitleMediaFile>> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::View,
        )
        .await?;
        self.services
            .library
            .media_files
            .list_media_files_for_title(title_id)
            .await
    }

    pub async fn list_episode_media_files(
        &self,
        actor: &User,
        title_id: &str,
        episode_id: &str,
    ) -> AppResult<Vec<TitleMediaFile>> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::View,
        )
        .await?;

        let episode_ids = vec![episode_id.to_string()];
        let scoped_files = self
            .services
            .library
            .media_files
            .list_live_media_files_for_episode_ids(title_id, &episode_ids)
            .await?;

        Ok(scoped_files
            .into_iter()
            .filter_map(|scoped_file| {
                if !scoped_file
                    .episode_ids
                    .iter()
                    .any(|scoped_episode_id| scoped_episode_id == episode_id)
                {
                    return None;
                }

                let is_primary = scoped_file
                    .primary_episode_ids
                    .iter()
                    .any(|primary_episode_id| primary_episode_id == episode_id);
                let mut media_file = scoped_file.media_file;
                media_file.episode_id = Some(episode_id.to_string());
                media_file.role = if is_primary {
                    crate::MediaFileRole::Primary
                } else {
                    crate::MediaFileRole::Additional
                };
                Some(media_file)
            })
            .collect())
    }

    /// Batch variant of [`Self::list_episode_media_files`] for one title:
    /// one permission check and one scoped-files fetch cover every requested
    /// episode id, grouped per episode. A missing or non-`View`-visible title
    /// yields an empty map (silent drop, matching the loader-facing batches).
    pub async fn list_episode_media_files_for_title(
        &self,
        actor: &User,
        title_id: &str,
        episode_ids: &[String],
    ) -> AppResult<std::collections::HashMap<String, Vec<TitleMediaFile>>> {
        if episode_ids.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        let Some(title) = self.services.catalog.titles.get_by_id(title_id).await? else {
            return Ok(std::collections::HashMap::new());
        };
        let allowed_library_ids = self
            .authorized_library_ids(actor, None, scryer_domain::LibraryPermission::View)
            .await?;
        if !allowed_library_ids.contains(&title.library_id) {
            return Ok(std::collections::HashMap::new());
        }
        let scoped_files = self
            .services
            .library
            .media_files
            .list_live_media_files_for_episode_ids(title_id, episode_ids)
            .await?;
        let mut files_by_episode: std::collections::HashMap<String, Vec<TitleMediaFile>> =
            std::collections::HashMap::new();
        for scoped_file in scoped_files {
            for episode_id in episode_ids {
                if scoped_file
                    .episode_ids
                    .iter()
                    .any(|scoped_episode_id| scoped_episode_id == episode_id)
                {
                    let mut media_file = scoped_file.media_file.clone();
                    media_file.episode_id = Some(episode_id.clone());
                    media_file.role = if scoped_file
                        .primary_episode_ids
                        .iter()
                        .any(|primary_episode_id| primary_episode_id == episode_id)
                    {
                        crate::MediaFileRole::Primary
                    } else {
                        crate::MediaFileRole::Additional
                    };
                    files_by_episode
                        .entry(episode_id.clone())
                        .or_default()
                        .push(media_file);
                }
            }
        }
        Ok(files_by_episode)
    }

    pub async fn get_title_wanted_item(
        &self,
        actor: &User,
        title_id: &str,
        episode_id: Option<&str>,
    ) -> AppResult<Option<AcquisitionScopeState>> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::View,
        )
        .await?;
        self.services
            .workflow
            .acquisition_scope_states
            .get_acquisition_scope_state_for_title(title_id, episode_id)
            .await
    }

    /// Batch variant of [`Self::get_title_wanted_item`]: returns every acquisition
    /// scope state for the `View`-visible subset of `title_ids`. Callers key the
    /// flat result by `(title_id, episode_id)`.
    pub async fn get_title_wanted_items_for_titles(
        &self,
        actor: &User,
        title_ids: &[String],
    ) -> AppResult<Vec<AcquisitionScopeState>> {
        let visible_title_ids = self
            .get_titles_by_ids(actor, title_ids)
            .await?
            .into_iter()
            .map(|title| title.id)
            .collect::<Vec<_>>();
        if visible_title_ids.is_empty() {
            return Ok(Vec::new());
        }
        self.services
            .workflow
            .acquisition_scope_states
            .list_acquisition_scope_states_for_title_ids(&visible_title_ids)
            .await
    }

    pub async fn get_title_for_management(
        &self,
        actor: &User,
        title_id: &str,
    ) -> AppResult<Option<Title>> {
        let title = self.services.catalog.titles.get_by_id(title_id).await?;
        if let Some(title) = title.as_ref() {
            self.require_library_permission(
                actor,
                &title.library_id,
                scryer_domain::LibraryPermission::ManageTitles,
            )
            .await?;
        }
        Ok(title)
    }

    pub async fn get_wanted_item_for_management(
        &self,
        actor: &User,
        wanted_item_id: &str,
    ) -> AppResult<Option<AcquisitionScopeState>> {
        let wanted = self
            .services
            .workflow
            .acquisition_scope_states
            .get_acquisition_scope_state_by_id(wanted_item_id)
            .await?;
        if let Some(wanted) = wanted.as_ref() {
            let library_id = self.derive_wanted_item_library_id(wanted).await?;
            self.require_library_permission(
                actor,
                &library_id,
                scryer_domain::LibraryPermission::ManageTitles,
            )
            .await?;
        }
        Ok(wanted)
    }

    /// Batch variant of [`Self::get_wanted_item_for_management`]: loads wanted
    /// items by id and silently drops those the actor cannot manage.
    pub async fn get_wanted_items_by_ids_for_management(
        &self,
        actor: &User,
        ids: &[String],
    ) -> AppResult<Vec<AcquisitionScopeState>> {
        let items = self
            .services
            .workflow
            .acquisition_scope_states
            .list_acquisition_scope_states_by_ids(ids)
            .await?;
        self.filter_wanted_items_for_permission(
            actor,
            items,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await
    }

    pub async fn get_title_for_download_actions(
        &self,
        actor: &User,
        title_id: &str,
    ) -> AppResult<Option<Title>> {
        let title = self.services.catalog.titles.get_by_id(title_id).await?;
        if let Some(title) = title.as_ref() {
            self.require_library_permission(
                actor,
                &title.library_id,
                scryer_domain::LibraryPermission::ManageTitles,
            )
            .await?;
        }
        Ok(title)
    }

    pub async fn get_completed_download(
        &self,
        actor: &User,
        download_client_item_id: &str,
    ) -> AppResult<CompletedDownload> {
        if self
            .authorized_library_ids(
                actor,
                None,
                scryer_domain::LibraryPermission::ResolveImports,
            )
            .await?
            .is_empty()
        {
            return Err(AppError::Unauthorized(
                "You do not have access to this library".to_string(),
            ));
        }
        let download_client_item_id = download_client_item_id.trim();
        if download_client_item_id.is_empty() {
            return Err(AppError::Validation(
                "download client item id is required".into(),
            ));
        }

        self.services
            .integrations
            .download_client
            .list_completed_downloads()
            .await?
            .into_iter()
            .find(|download| download.download_client_item_id == download_client_item_id)
            .ok_or_else(|| {
                AppError::NotFound(format!("completed download {download_client_item_id}"))
            })
    }

    pub async fn connect_library_scan_tracker(&self) {
        self.runtime
            .library
            .library_scan_tracker
            .set_job_run_tracker(self.runtime.jobs.job_run_tracker.clone())
            .await;
    }

    pub fn wake_title_image_loops(&self) {
        self.runtime.catalog.poster_wake.notify_one();
        self.runtime.catalog.fanart_wake.notify_one();
    }

    pub async fn primary_enabled_download_client_config(
        &self,
    ) -> AppResult<Option<DownloadClientConfig>> {
        Ok(self
            .services
            .integrations
            .download_client_configs
            .list(None)
            .await?
            .into_iter()
            .filter(|config| config.is_enabled)
            .min_by_key(|config| config.client_priority))
    }

    pub async fn active_library_scan_sessions(&self) -> Vec<LibraryScanSession> {
        self.runtime
            .library
            .library_scan_tracker
            .list_active()
            .await
    }

    pub fn user_rules_engine_snapshot(&self) -> scryer_rules::UserRulesEngine {
        self.services
            .customization
            .user_rules
            .read()
            .unwrap()
            .clone()
    }
}

fn should_invalidate_wanted_projection(payload: &scryer_domain::DomainEventPayload) -> bool {
    use scryer_domain::DomainEventPayload;

    match payload {
        DomainEventPayload::TitleAdded(_)
        | DomainEventPayload::TitleUpdated(_)
        | DomainEventPayload::TitleRematched(_)
        | DomainEventPayload::TitleDeleted(_)
        | DomainEventPayload::MetadataHydrationUpdated(_)
        | DomainEventPayload::MediaFileImported(_)
        | DomainEventPayload::MediaFileAnalyzed(_)
        | DomainEventPayload::MediaFileRenamed(_)
        | DomainEventPayload::MediaFileDeleted(_)
        | DomainEventPayload::MediaFileUpgraded(_)
        | DomainEventPayload::LibraryScanTitleDiscovered(_)
        | DomainEventPayload::LibraryScanDeltaRecorded(_)
        | DomainEventPayload::LibraryScanCompleted(_) => true,
        DomainEventPayload::ConfigurationChanged(data) => matches!(
            data.resource_type.trim().to_ascii_lowercase().as_str(),
            "quality_profile" | "quality_profiles" | "quality_definition" | "quality_definitions"
        ),
        _ => false,
    }
}

fn should_invalidate_monitored_title_matcher(payload: &scryer_domain::DomainEventPayload) -> bool {
    matches!(
        payload,
        scryer_domain::DomainEventPayload::TitleAdded(_)
            | scryer_domain::DomainEventPayload::TitleUpdated(_)
            | scryer_domain::DomainEventPayload::TitleDeleted(_)
    )
}

pub(super) type RuntimePerformanceProbe =
    Arc<dyn Fn(PathBuf) -> RuntimePerformanceSnapshot + Send + Sync + 'static>;

pub(super) async fn initialize_runtime_performance_snapshot(
    cell: &OnceCell<RuntimePerformanceSnapshot>,
    config_dir: Arc<PathBuf>,
    probe: RuntimePerformanceProbe,
) -> RuntimePerformanceSnapshot {
    cell.get_or_init(|| async move {
        let config_dir_for_probe = config_dir.as_ref().clone();
        let config_dir_for_log = config_dir_for_probe.clone();
        let snapshot =
            match tokio::task::spawn_blocking(move || (probe)(config_dir_for_probe)).await {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        "runtime performance probe task failed; using conservative slow defaults"
                    );
                    RuntimePerformanceSnapshot::slow()
                }
            };
        tracing::info!(
            cpu_class = %snapshot.cpu_class,
            config_io_class = %snapshot.config_io_class,
            cpu_probe_elapsed_ms = snapshot.cpu_probe_elapsed_ms,
            config_io_probe_elapsed_ms = snapshot.config_io_probe_elapsed_ms,
            config_dir = %config_dir_for_log.display(),
            "runtime performance probe settled"
        );
        snapshot
    })
    .await
    .clone()
}

pub(super) fn probe_runtime_performance_snapshot(
    config_dir: PathBuf,
) -> RuntimePerformanceSnapshot {
    let (cpu_class, cpu_probe_elapsed_ms) = probe_cpu_performance();
    let (config_io_class, config_io_probe_elapsed_ms) = probe_config_io_performance(&config_dir);
    RuntimePerformanceSnapshot {
        cpu_class,
        config_io_class,
        cpu_probe_elapsed_ms,
        config_io_probe_elapsed_ms,
    }
}

pub(super) fn classify_cpu_elapsed(elapsed: std::time::Duration) -> RuntimePerformanceClass {
    if elapsed <= std::time::Duration::from_millis(125) {
        RuntimePerformanceClass::Fast
    } else {
        RuntimePerformanceClass::Slow
    }
}

pub(super) fn probe_cpu_performance() -> (RuntimePerformanceClass, Option<u64>) {
    const CPU_PROBE_BYTES: usize = 8 * 1024 * 1024;
    const CPU_PROBE_PASSES: usize = 32;
    const SLOW_CAP: std::time::Duration = std::time::Duration::from_millis(250);

    let mut buffer = vec![0_u64; CPU_PROBE_BYTES / std::mem::size_of::<u64>()];
    let start = std::time::Instant::now();
    let mut state = 0x9E37_79B9_7F4A_7C15_u64;

    for pass in 0..CPU_PROBE_PASSES {
        for word in &mut buffer {
            state = state
                .wrapping_add(0xA076_1D64_78BD_642F_u64 ^ (pass as u64))
                .rotate_left(13);
            let mixed = state ^ word.rotate_left((state & 31) as u32) ^ 0xE703_7ED1_A0B4_28DB_u64;
            *word = word.wrapping_add(mixed).rotate_left(7) ^ mixed;
            std::hint::black_box(*word);
        }

        if start.elapsed() > SLOW_CAP {
            let elapsed_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
            return (RuntimePerformanceClass::Slow, Some(elapsed_ms));
        }
    }

    std::hint::black_box(state);
    let elapsed = start.elapsed();
    let elapsed_ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
    (classify_cpu_elapsed(elapsed), Some(elapsed_ms))
}

pub(super) fn classify_config_io_elapsed(elapsed: std::time::Duration) -> RuntimePerformanceClass {
    if elapsed <= std::time::Duration::from_millis(200) {
        RuntimePerformanceClass::Fast
    } else {
        RuntimePerformanceClass::Slow
    }
}

pub(super) fn probe_config_io_performance(
    config_dir: &Path,
) -> (RuntimePerformanceClass, Option<u64>) {
    const PAYLOAD_BYTES: usize = 8 * 1024 * 1024;
    const CHUNK_BYTES: usize = 1024 * 1024;
    const SLOW_CAP: std::time::Duration = std::time::Duration::from_millis(500);

    if !config_dir.is_dir() && std::fs::create_dir_all(config_dir).is_err() {
        return (RuntimePerformanceClass::Slow, None);
    }

    let probe_name = format!(
        ".scryer-runtime-probe-{}-{}.tmp",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let probe_path = config_dir.join(probe_name);
    let chunk = vec![0x5Au8; CHUNK_BYTES];
    let start = std::time::Instant::now();

    let result = (|| -> std::io::Result<()> {
        let mut file = std::fs::File::create(&probe_path)?;
        let mut written = 0;
        while written < PAYLOAD_BYTES {
            let to_write = std::cmp::min(CHUNK_BYTES, PAYLOAD_BYTES - written);
            file.write_all(&chunk[..to_write])?;
            written += to_write;
            if start.elapsed() > SLOW_CAP {
                return Ok(());
            }
        }
        file.flush()?;
        file.sync_all()?;

        let mut file = std::fs::File::open(&probe_path)?;
        let mut read_buffer = vec![0_u8; CHUNK_BYTES];
        loop {
            let bytes_read = file.read(&mut read_buffer)?;
            if bytes_read == 0 {
                break;
            }
            std::hint::black_box(&read_buffer[..bytes_read]);
            if start.elapsed() > SLOW_CAP {
                return Ok(());
            }
        }

        Ok(())
    })();

    let cleanup_result = std::fs::remove_file(&probe_path);

    if result.is_err() || cleanup_result.is_err() {
        let elapsed_ms = u64::try_from(start.elapsed().as_millis()).ok();
        return (RuntimePerformanceClass::Slow, elapsed_ms);
    }

    let elapsed = start.elapsed();
    let elapsed_ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
    (classify_config_io_elapsed(elapsed), Some(elapsed_ms))
}

#[cfg(test)]
mod wanted_projection_invalidation_classifier_tests {
    use super::should_invalidate_wanted_projection;
    use scryer_domain::{
        ConfigurationChangeAction, ConfigurationChangedEventData, DomainEventPayload,
    };

    fn configuration(resource_type: &str) -> DomainEventPayload {
        DomainEventPayload::ConfigurationChanged(ConfigurationChangedEventData {
            resource_type: resource_type.to_string(),
            resource_id: None,
            action: ConfigurationChangeAction::Updated,
        })
    }

    #[test]
    fn quality_profile_configuration_changes_invalidate_wanted_projection() {
        assert!(should_invalidate_wanted_projection(&configuration(
            "quality_profiles"
        )));
        assert!(should_invalidate_wanted_projection(&configuration(
            "quality_profile"
        )));
    }

    #[test]
    fn unrelated_configuration_changes_do_not_invalidate_wanted_projection() {
        assert!(!should_invalidate_wanted_projection(&configuration(
            "download_client"
        )));
    }
}

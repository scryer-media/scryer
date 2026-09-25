use crate::services::NavigationBadgeSection;

/// How long the durable badge facts may stand between recounts.
///
/// The web polls `navigationBadgeCounts` every 30 seconds, so counting on the
/// same cadence answers each poll with facts at most one poll old. Only this
/// interval recounts the durable half; the half that actually moves while a
/// download runs — the import attention list — is recounted from the in-memory
/// queue read model as snapshots land.
pub const NAVIGATION_BADGE_FACTS_REFRESH_INTERVAL: Duration = Duration::from_secs(30);

/// The shortest gap between two snapshot-driven attention recounts.
///
/// A download client that is doing anything at all commits a snapshot every few
/// seconds — progress alone bumps the revision — and the badge only needs to be
/// roughly current. The loop therefore waits out this floor before recounting
/// and then drops whatever else arrived meanwhile: the watch channel keeps the
/// latest value, so the recount that follows sees the newest snapshot, not the
/// one that woke it.
pub const NAVIGATION_BADGE_ATTENTION_COALESCE_FLOOR: Duration = Duration::from_secs(2);

impl AppUseCase {
    /// Recount the durable badge facts and publish them.
    ///
    /// Every read here is unfiltered: the counts are kept per library so the
    /// per-actor permission filter can still be applied in memory, without a
    /// per-actor query. Each section stands on its own — a store that refuses
    /// one of them leaves that section at its last value and still publishes
    /// the rest, because a badge that is partly stale beats a badge that is
    /// blank.
    pub async fn refresh_navigation_badge_durable_facts(&self) {
        let previous = self.published_navigation_badge_facts().await;
        let mut durable = previous
            .as_ref()
            .map(|facts| (*facts.durable).clone())
            .unwrap_or_default();

        match self.permission_candidate_library_ids(None).await {
            Ok(library_ids) => {
                self.clear_navigation_badge_section_failure(NavigationBadgeSection::Libraries)
                    .await;
                durable.candidate_library_ids = library_ids;
            }
            Err(error) => {
                self.report_navigation_badge_section_failure(
                    NavigationBadgeSection::Libraries,
                    &error,
                )
                .await;
            }
        }

        match self.pending_import_counts_by_library().await {
            Ok(counts) => {
                self.clear_navigation_badge_section_failure(NavigationBadgeSection::PendingImports)
                    .await;
                durable.pending_imports = counts;
            }
            Err(error) => {
                self.report_navigation_badge_section_failure(
                    NavigationBadgeSection::PendingImports,
                    &error,
                )
                .await;
            }
        }

        match self
            .pending_media_request_counts_by_library(&durable.candidate_library_ids)
            .await
        {
            Ok(counts) => {
                self.clear_navigation_badge_section_failure(NavigationBadgeSection::MediaRequests)
                    .await;
                durable.media_requests = counts;
            }
            Err(error) => {
                self.report_navigation_badge_section_failure(
                    NavigationBadgeSection::MediaRequests,
                    &error,
                )
                .await;
            }
        }

        match self.build_available_plugins().await {
            Ok(plugins) => {
                self.clear_navigation_badge_section_failure(NavigationBadgeSection::Plugins)
                    .await;
                durable.plugin_update_count = plugins
                    .iter()
                    .filter(|plugin| plugin.update_available)
                    .count() as i64;
                durable.plugin_blocked_count = plugins
                    .iter()
                    .filter(|plugin| plugin.blocked_reason.is_some())
                    .count() as i64;
            }
            Err(error) => {
                self.report_navigation_badge_section_failure(
                    NavigationBadgeSection::Plugins,
                    &error,
                )
                .await;
            }
        }

        let mut published = self
            .runtime
            .integrations
            .navigation_badge_facts
            .current
            .write()
            .await;
        let import_attention = published
            .as_ref()
            .map(|facts| facts.import_attention.clone())
            .unwrap_or_default();
        published.replace(std::sync::Arc::new(crate::services::NavigationBadgeFacts {
            durable: std::sync::Arc::new(durable),
            import_attention,
        }));
    }

    /// Recount the import attention list from the queue read model.
    ///
    /// This is the half a download in flight actually moves, and it costs no
    /// durable read of its own: the read model is the same one every download
    /// surface shares, cached per snapshot revision.
    pub async fn refresh_navigation_badge_import_attention(&self) -> AppResult<()> {
        let (_, model) = self.current_download_queue_read_model().await?;
        // The same two filters `count_download_import_items` applies, in the
        // same order; only the permission filter is deferred to the request.
        let import_attention = model
            .items
            .iter()
            .filter(|item| is_history_download_state(&item.state))
            .filter(|item| matches_download_import_filter(item, DownloadImportFilter::Attention))
            .map(|item| {
                item.title_id
                    .as_deref()
                    .and_then(|title_id| model.title_library_ids.get(title_id).cloned())
            })
            .collect::<Vec<_>>();

        let mut published = self
            .runtime
            .integrations
            .navigation_badge_facts
            .current
            .write()
            .await;
        let durable = published
            .as_ref()
            .map(|facts| facts.durable.clone())
            .unwrap_or_default();
        published.replace(std::sync::Arc::new(crate::services::NavigationBadgeFacts {
            durable,
            import_attention,
        }));
        Ok(())
    }

    async fn published_navigation_badge_facts(
        &self,
    ) -> Option<std::sync::Arc<crate::services::NavigationBadgeFacts>> {
        self.runtime
            .integrations
            .navigation_badge_facts
            .current
            .read()
            .await
            .clone()
    }

    /// Log a failing durable section once per failure streak.
    ///
    /// A store that stays down is read again on every refresh, and a warning
    /// per pass would bury the log; the badge simply keeps serving that
    /// section's previous value until it answers again.
    async fn report_navigation_badge_section_failure(
        &self,
        section: NavigationBadgeSection,
        error: &AppError,
    ) {
        let newly_failing = self
            .runtime
            .integrations
            .navigation_badge_facts
            .failing_sections
            .lock()
            .await
            .insert(section);
        if newly_failing {
            tracing::warn!(
                section = section.as_str(),
                "navigation badge facts kept their previous value: {error}"
            );
        }
    }

    async fn clear_navigation_badge_section_failure(&self, section: NavigationBadgeSection) {
        self.runtime
            .integrations
            .navigation_badge_facts
            .failing_sections
            .lock()
            .await
            .remove(&section);
    }

    /// The published facts.
    ///
    /// Only a request that arrives before the refresh loop's first pass finds
    /// nothing published; that one counts them itself, single-flighted through
    /// the build lock the way the download-queue read model is, and a section
    /// that fails there simply comes back empty rather than failing the whole
    /// badge. Afterwards the request path never reads a store for these
    /// numbers, however stale they are — a poll must not pay for the archive.
    async fn navigation_badge_facts(
        &self,
    ) -> std::sync::Arc<crate::services::NavigationBadgeFacts> {
        let cache = &self.runtime.integrations.navigation_badge_facts;
        if let Some(facts) = cache.current.read().await.clone() {
            return facts;
        }
        let _build_guard = cache.build_lock.lock().await;
        if let Some(facts) = cache.current.read().await.clone() {
            return facts;
        }
        self.refresh_navigation_badge_durable_facts().await;
        if let Err(error) = self.refresh_navigation_badge_import_attention().await {
            tracing::warn!("navigation badge attention count is unavailable: {error}");
        }
        cache.current.read().await.clone().unwrap_or_else(|| {
            std::sync::Arc::new(crate::services::NavigationBadgeFacts::default())
        })
    }

    /// The navigation badge counts this actor may see.
    pub async fn navigation_badge_counts(
        &self,
        actor: &User,
    ) -> AppResult<crate::types::NavigationBadgeCounts> {
        let facts = self.navigation_badge_facts().await;
        let authorization = self.authorization_for_actor(actor).await?;
        let libraries_for = |permission: scryer_domain::LibraryPermission| {
            facts
                .durable
                .candidate_library_ids
                .iter()
                .filter(|library_id| {
                    crate::authorization::effective_library_permission(
                        &authorization,
                        library_id,
                        permission,
                    )
                })
                .cloned()
                .collect::<HashSet<_>>()
        };
        let resolvable = libraries_for(scryer_domain::LibraryPermission::ResolveImports);
        let manageable = libraries_for(scryer_domain::LibraryPermission::ManageTitles);
        let can_view_operational_history =
            authorization.has_app_permission(scryer_domain::AppPermission::ManageSystemSettings);

        let mut pending_imports = PendingImportCounts::default();
        if !resolvable.is_empty() {
            for (library_id, counts) in &facts.durable.pending_imports {
                if resolvable.contains(library_id) {
                    pending_imports.movie += counts.movie;
                    pending_imports.series += counts.series;
                    pending_imports.anime += counts.anime;
                }
            }
        }

        let mut pending_media_requests = MediaRequestCounts::default();
        if !manageable.is_empty() {
            for (library_id, counts) in &facts.durable.media_requests {
                if manageable.contains(library_id) {
                    pending_media_requests.movie += counts.movie;
                    pending_media_requests.series += counts.series;
                    pending_media_requests.anime += counts.anime;
                }
            }
        }

        let activity_import_count = if resolvable.is_empty() {
            0
        } else {
            facts
                .import_attention
                .iter()
                .filter(|library_id| match library_id {
                    // A row with no title an operator can be scoped by is
                    // operational history, which only a system administrator
                    // sees — the same rule the history collector applies.
                    None => can_view_operational_history,
                    Some(library_id) => resolvable.contains(library_id),
                })
                .count() as i64
        };

        Ok(crate::types::NavigationBadgeCounts {
            pending_imports,
            pending_media_requests,
            activity_import_count,
            plugin_update_count: if can_view_operational_history {
                facts.durable.plugin_update_count
            } else {
                0
            },
            plugin_blocked_count: if can_view_operational_history {
                facts.durable.plugin_blocked_count
            } else {
                0
            },
        })
    }
}

/// Keep the navigation badge facts current, off every request path.
///
/// The durable half is recounted on the refresh interval. A new download-queue
/// snapshot recounts the attention list alone, and only after the coalesce
/// floor has passed, so a queue committing a snapshot every couple of seconds
/// cannot drag the durable reads along with it.
pub async fn start_navigation_badge_facts_refresh(
    app: AppUseCase,
    token: tokio_util::sync::CancellationToken,
) {
    let mut queue_sync = app.runtime.acquisition.download_queue_snapshot.subscribe();
    let refresh_everything = |app: AppUseCase| async move {
        app.refresh_navigation_badge_durable_facts().await;
        if let Err(error) = app.refresh_navigation_badge_import_attention().await {
            tracing::warn!("navigation badge attention count is unavailable: {error}");
        }
    };
    // An interval rather than a per-iteration sleep: queue snapshots land every
    // few seconds while anything downloads, and a sleep re-armed on each of
    // those wake-ups would never reach 30 seconds, starving the durable half.
    let mut durable_refresh = tokio::time::interval(NAVIGATION_BADGE_FACTS_REFRESH_INTERVAL);
    durable_refresh.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            _ = token.cancelled() => return,
            // The first tick completes immediately and is the start-up pass.
            _ = durable_refresh.tick() => {
                refresh_everything(app.clone()).await;
            }
            changed = queue_sync.changed() => {
                if changed.is_err() {
                    return;
                }
                tokio::select! {
                    _ = token.cancelled() => return,
                    _ = tokio::time::sleep(NAVIGATION_BADGE_ATTENTION_COALESCE_FLOOR) => {}
                }
                // Whatever landed during the floor is already in the snapshot
                // the recount is about to read, so those wake-ups are spent
                // here rather than costing a recount each.
                queue_sync.mark_unchanged();
                if let Err(error) = app.refresh_navigation_badge_import_attention().await {
                    tracing::warn!("navigation badge attention count is unavailable: {error}");
                }
            }
        }
    }
}

use super::*;

pub(super) type DeleteOperationLog = Arc<Mutex<Vec<String>>>;
pub(super) type OptionalDeleteOperationLog = Arc<Mutex<Option<DeleteOperationLog>>>;
pub(super) type TrackedDownloadStateKey = (String, String, String);
pub(super) type TrackedDownloadStates = Arc<Mutex<HashMap<TrackedDownloadStateKey, String>>>;
pub(super) type DownloadSubmissionIdentities =
    Arc<Mutex<HashMap<TrackedDownloadStateKey, DownloadSubmissionIdentity>>>;
pub(super) type DownloadIdentityStates = Arc<Mutex<HashMap<String, String>>>;
pub(super) type ImportIdentities = Arc<Mutex<HashMap<String, DownloadSubmissionIdentity>>>;
/// `(client_id, client_type, item_id, is_history, remove_data)` for a delete
/// the caller issued.
pub(super) type DeletedDownloadRequest = (Option<String>, Option<String>, String, bool, bool);
pub(super) type DeletedDownloadRequests = Arc<Mutex<Vec<DeletedDownloadRequest>>>;
/// `(client_id, item_id)` for a pause the caller issued.
pub(super) type PausedDownloadRequest = (Option<String>, String);
pub(super) type PausedDownloadRequests = Arc<Mutex<Vec<PausedDownloadRequest>>>;

#[derive(Default)]
pub(super) struct MockTitleRepo {
    pub(super) store: Arc<Mutex<Vec<Title>>>,
    pub(super) smg_identity_backfill_attempts: Arc<Mutex<HashMap<String, i64>>>,
    pub(super) create_or_get_existing_error: Arc<Mutex<Option<String>>>,
    /// `(title_id, message)`: fail folder-ownership writes for one title, so a
    /// compensating transaction can be exercised at the exact commit that fails.
    pub(super) folder_path_write_error: Arc<Mutex<Option<(String, String)>>>,
    pub(super) delete_operation_log: OptionalDeleteOperationLog,
    pub(super) pending_import_items: Option<Arc<Mutex<Vec<LibraryScanUnmatchedItem>>>>,
    pub(super) external_id_batch_lookup_calls: AtomicUsize,
}
#[derive(Default)]
pub(crate) struct RecordingJobRunRepo {
    pub(super) runs: Arc<Mutex<Vec<JobRunRecord>>>,
    pub(super) list_job_runs_calls: AtomicUsize,
    pub(super) list_job_runs_for_actor_calls: AtomicUsize,
    pub(super) list_active_job_runs_calls: AtomicUsize,
}

impl RecordingJobRunRepo {
    pub(crate) async fn seed(&self, run: JobRunRecord) {
        self.runs.lock().await.push(run);
    }
}

#[async_trait]
impl JobRunRepository for RecordingJobRunRepo {
    async fn create_job_run(&self, run: &JobRunRecord) -> AppResult<JobRunRecord> {
        let mut runs = self.runs.lock().await;
        runs.push(run.clone());
        Ok(run.clone())
    }

    async fn update_job_run(&self, run: &JobRunRecord) -> AppResult<JobRunRecord> {
        let mut runs = self.runs.lock().await;
        if let Some(existing) = runs.iter_mut().find(|candidate| candidate.id == run.id) {
            *existing = run.clone();
        } else {
            runs.push(run.clone());
        }
        Ok(run.clone())
    }

    async fn get_job_run(&self, run_id: &str) -> AppResult<Option<JobRunRecord>> {
        Ok(self
            .runs
            .lock()
            .await
            .iter()
            .find(|run| run.id == run_id)
            .cloned())
    }

    async fn list_job_runs(
        &self,
        job_key: Option<JobKey>,
        limit: usize,
    ) -> AppResult<Vec<JobRunRecord>> {
        self.list_job_runs_calls.fetch_add(1, Ordering::SeqCst);
        Ok(sorted_limited_job_runs(
            self.runs
                .lock()
                .await
                .iter()
                .filter(|run| job_key.is_none_or(|job_key| run.job_key == job_key))
                .cloned()
                .collect(),
            limit,
        ))
    }

    async fn list_job_runs_for_actor(
        &self,
        job_key: Option<JobKey>,
        actor_user_id: &str,
        limit: usize,
    ) -> AppResult<Vec<JobRunRecord>> {
        self.list_job_runs_for_actor_calls
            .fetch_add(1, Ordering::SeqCst);
        Ok(sorted_limited_job_runs(
            self.runs
                .lock()
                .await
                .iter()
                .filter(|run| job_key.is_none_or(|job_key| run.job_key == job_key))
                .filter(|run| run.actor_user_id.as_deref() == Some(actor_user_id))
                .cloned()
                .collect(),
            limit,
        ))
    }

    async fn list_active_job_runs(&self) -> AppResult<Vec<JobRunRecord>> {
        self.list_active_job_runs_calls
            .fetch_add(1, Ordering::SeqCst);
        Ok(self
            .runs
            .lock()
            .await
            .iter()
            .filter(|run| !run.status.is_terminal())
            .cloned()
            .collect())
    }

    async fn reconcile_interrupted_job_runs(&self, excluded_run_ids: &[String]) -> AppResult<u64> {
        let now = chrono::Utc::now();
        let mut runs = self.runs.lock().await;
        let mut reconciled = 0u64;
        for run in runs
            .iter_mut()
            .filter(|run| !run.status.is_terminal() && !excluded_run_ids.contains(&run.id))
        {
            run.status = JobRunStatus::Failed;
            run.progress_json = None;
            run.error_text = Some("interrupted by restart".to_string());
            run.completed_at = Some(now);
            run.updated_at = now;
            reconciled += 1;
        }
        Ok(reconciled)
    }
}

pub(super) fn sorted_limited_job_runs(
    mut runs: Vec<JobRunRecord>,
    limit: usize,
) -> Vec<JobRunRecord> {
    runs.sort_by(|left, right| {
        right
            .started_at
            .cmp(&left.started_at)
            .then_with(|| right.created_at.cmp(&left.created_at))
            .then_with(|| right.id.cmp(&left.id))
    });
    runs.truncate(limit);
    runs
}

impl MockTitleRepo {
    pub(super) async fn fail_create_or_get_existing(&self, message: &str) {
        *self.create_or_get_existing_error.lock().await = Some(message.to_string());
    }

    /// Make every `set_folder_path`/`clear_folder_path` for `title_id` fail.
    pub(super) async fn fail_folder_path_writes_for(&self, title_id: &str, message: &str) {
        *self.folder_path_write_error.lock().await =
            Some((title_id.to_string(), message.to_string()));
    }

    async fn folder_path_write_failure(&self, id: &str) -> Option<AppError> {
        self.folder_path_write_error
            .lock()
            .await
            .as_ref()
            .filter(|(failing_id, _)| failing_id == id)
            .map(|(_, message)| AppError::Repository(message.clone()))
    }

    pub(super) async fn set_delete_operation_log(&self, operation_log: Arc<Mutex<Vec<String>>>) {
        *self.delete_operation_log.lock().await = Some(operation_log);
    }
}
#[derive(Default)]
pub(super) struct BlockingTitleImageRepo {
    pub(super) clear_calls: AtomicUsize,
    pub(super) release_clear: Notify,
}

#[async_trait]
impl TitleImageRepository for BlockingTitleImageRepo {
    async fn list_title_image_refresh_work(
        &self,
        _limit: usize,
        _skipped: &[TitleImageSyncTask],
    ) -> AppResult<Vec<TitleImageSyncTask>> {
        Ok(Vec::new())
    }

    async fn clear_title_image_cache(&self) -> AppResult<()> {
        self.clear_calls.fetch_add(1, Ordering::SeqCst);
        self.release_clear.notified().await;
        Ok(())
    }

    async fn upsert_title_image_source_result(
        &self,
        _title_id: &str,
        _result: TitleImageSourceResult,
        _event: Option<NewDomainEvent>,
    ) -> AppResult<Option<DomainEvent>> {
        Ok(None)
    }

    async fn get_title_image_blob(
        &self,
        _title_id: &str,
        _kind: TitleImageKind,
        _variant_key: &str,
    ) -> AppResult<Option<TitleImageBlob>> {
        Ok(None)
    }
}

#[async_trait]
impl TitleRepository for MockTitleRepo {
    async fn list(
        &self,
        facet: Option<MediaFacet>,
        query: Option<String>,
    ) -> AppResult<Vec<Title>> {
        let list = self.store.lock().await.clone();
        let normalized_query = query.map(|value| value.to_lowercase());
        Ok(list
            .into_iter()
            .filter(|title| {
                let facet_match = facet
                    .as_ref()
                    .is_none_or(|expected| &title.facet == expected);
                let query_match = normalized_query
                    .as_ref()
                    .is_none_or(|term| title.name.to_lowercase().contains(term));
                facet_match && query_match
            })
            .collect())
    }

    async fn list_by_external_ids(&self, source: &str, values: &[String]) -> AppResult<Vec<Title>> {
        let requested: Vec<&str> = values
            .iter()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .collect();
        let list = self.store.lock().await;
        let mut matches = Vec::new();
        let mut seen = HashSet::new();
        for value in requested {
            if let Some(title) = list.iter().find(|title| {
                title.external_ids.iter().any(|external_id| {
                    external_id.source.eq_ignore_ascii_case(source) && external_id.value == value
                })
            }) && seen.insert(title.id.clone())
            {
                matches.push(title.clone());
            }
        }
        Ok(matches)
    }

    async fn list_existing_external_ids_in_library_and_facet(
        &self,
        library_id: &str,
        facet: MediaFacet,
        source: &str,
        values: &[String],
    ) -> AppResult<std::collections::BTreeSet<String>> {
        self.external_id_batch_lookup_calls
            .fetch_add(1, Ordering::SeqCst);

        let requested = values
            .iter()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .collect::<HashSet<_>>();
        if requested.is_empty() {
            return Ok(std::collections::BTreeSet::new());
        }

        let list = self.store.lock().await;
        let mut existing = std::collections::BTreeSet::new();
        for title in list
            .iter()
            .filter(|title| title.library_id == library_id.trim() && title.facet == facet)
        {
            for external_id in &title.external_ids {
                let value = external_id.value.trim();
                if external_id.source.eq_ignore_ascii_case(source) && requested.contains(value) {
                    existing.insert(value.to_string());
                }
            }
        }
        Ok(existing)
    }

    async fn list_for_matching(
        &self,
        facet: Option<MediaFacet>,
        query: Option<String>,
    ) -> AppResult<Vec<Title>> {
        self.list(facet, query).await
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<Title>> {
        let list = self.store.lock().await;
        Ok(list.iter().find(|title| title.id == id).cloned())
    }

    async fn get_by_facet_and_slug(
        &self,
        facet: MediaFacet,
        slug: &str,
    ) -> AppResult<Option<Title>> {
        let normalized_slug = slug.trim();
        if normalized_slug.is_empty() {
            return Ok(None);
        }

        let list = self.store.lock().await;
        let matches = list
            .iter()
            .filter(|title| {
                title.facet == facet
                    && title.slug.as_deref().is_some_and(|candidate| {
                        candidate.trim().eq_ignore_ascii_case(normalized_slug)
                    })
            })
            .cloned()
            .collect::<Vec<_>>();

        match matches.as_slice() {
            [] => Ok(None),
            [title] => Ok(Some(title.clone())),
            _ => Err(AppError::Validation(
                "multiple titles found for slug lookup".into(),
            )),
        }
    }

    async fn find_by_external_id(&self, source: &str, value: &str) -> AppResult<Option<Title>> {
        let list = self.store.lock().await;
        Ok(list
            .iter()
            .find(|title| {
                title.external_ids.iter().any(|external_id| {
                    external_id.source.eq_ignore_ascii_case(source) && external_id.value == value
                })
            })
            .cloned())
    }

    async fn find_by_external_id_in_facet(
        &self,
        facet: MediaFacet,
        source: &str,
        value: &str,
    ) -> AppResult<Option<Title>> {
        let list = self.store.lock().await;
        Ok(list
            .iter()
            .find(|title| {
                title.facet == facet
                    && title.external_ids.iter().any(|external_id| {
                        external_id.source.eq_ignore_ascii_case(source)
                            && external_id.value == value
                    })
            })
            .cloned())
    }

    async fn create_or_get_existing(&self, title: Title) -> AppResult<CreateTitleOutcome> {
        if let Some(message) = self.create_or_get_existing_error.lock().await.clone() {
            return Err(AppError::Repository(message));
        }

        let mut list = self.store.lock().await;
        let mut matching_ids = list
            .iter()
            .filter(|existing| {
                existing.library_id == title.library_id
                    && existing.facet == title.facet
                    && existing.external_ids.iter().any(|existing_external_id| {
                        title.external_ids.iter().any(|incoming_external_id| {
                            existing_external_id
                                .source
                                .eq_ignore_ascii_case(&incoming_external_id.source)
                                && existing_external_id.value == incoming_external_id.value
                        })
                    })
            })
            .map(|existing| existing.id.clone())
            .collect::<Vec<_>>();
        matching_ids.sort();
        matching_ids.dedup();

        if matching_ids.len() > 1 {
            return Err(AppError::Validation(
                "external ids already map to multiple titles".into(),
            ));
        }

        if let Some(existing_id) = matching_ids.first()
            && let Some(existing) = list.iter().find(|entry| entry.id == *existing_id)
        {
            return Ok(CreateTitleOutcome {
                title: existing.clone(),
                reused_existing: true,
            });
        }

        list.push(title.clone());
        Ok(CreateTitleOutcome {
            title,
            reused_existing: false,
        })
    }

    async fn create_or_get_existing_with_options_patch(
        &self,
        title: Title,
        options_patch: TitleOptionsPatch,
    ) -> AppResult<CreateTitleOutcome> {
        let outcome = self.create_or_get_existing(title).await?;
        if !outcome.reused_existing {
            return Ok(outcome);
        }

        let mut list = self.store.lock().await;
        let existing = list
            .iter_mut()
            .find(|existing| existing.id == outcome.title.id)
            .expect("reused title should remain stored");
        if let Some(profile_id) = options_patch.quality_profile_id {
            existing
                .tags
                .retain(|tag| !tag.starts_with("scryer:quality-profile:"));
            if let Some(profile_id) = profile_id.filter(|profile_id| !profile_id.trim().is_empty())
            {
                existing
                    .tags
                    .push(format!("scryer:quality-profile:{}", profile_id.trim()));
            }
        }

        Ok(CreateTitleOutcome {
            title: existing.clone(),
            reused_existing: true,
        })
    }

    async fn create_or_get_existing_and_bind_pending_import(
        &self,
        title: Title,
        pending_import_id: &str,
    ) -> AppResult<CreateTitleOutcome> {
        if let Some(message) = self.create_or_get_existing_error.lock().await.clone() {
            return Err(AppError::Repository(message));
        }

        let pending_import_items = self.pending_import_items.as_ref().ok_or_else(|| {
            AppError::Repository(
                "transactional pending import title creation is not configured".into(),
            )
        })?;
        let mut list = self.store.lock().await;
        let mut matching_ids = list
            .iter()
            .filter(|existing| {
                existing.library_id == title.library_id
                    && existing.facet == title.facet
                    && existing.external_ids.iter().any(|existing_external_id| {
                        title.external_ids.iter().any(|incoming_external_id| {
                            existing_external_id
                                .source
                                .eq_ignore_ascii_case(&incoming_external_id.source)
                                && existing_external_id.value == incoming_external_id.value
                        })
                    })
            })
            .map(|existing| existing.id.clone())
            .collect::<Vec<_>>();
        matching_ids.sort();
        matching_ids.dedup();

        if matching_ids.len() > 1 {
            return Err(AppError::Validation(
                "external ids already map to multiple titles".into(),
            ));
        }

        if let Some(existing_id) = matching_ids.first()
            && let Some(existing) = list.iter().find(|entry| entry.id == *existing_id)
        {
            return Ok(CreateTitleOutcome {
                title: existing.clone(),
                reused_existing: true,
            });
        }

        let mut pending_items = pending_import_items.lock().await;
        let pending_index = pending_items
            .iter()
            .position(|item| {
                item.id == pending_import_id
                    && item.library_id == title.library_id
                    && item.facet == title.facet
                    && item.title_id.is_none()
            })
            .ok_or_else(|| {
                AppError::Validation(format!(
                    "pending import {pending_import_id} could not be resolved for title {}",
                    title.id
                ))
            })?;
        if title.facet == MediaFacet::Movie {
            pending_items.remove(pending_index);
        } else if let Some(pending_item) = pending_items.get_mut(pending_index) {
            pending_item.status = PendingImportStatus::Pending;
            pending_item.title_id = Some(title.id.clone());
            pending_item.updated_at = Utc::now().to_rfc3339();
        }

        list.push(title.clone());
        Ok(CreateTitleOutcome {
            title,
            reused_existing: false,
        })
    }

    async fn create(&self, title: Title) -> AppResult<Title> {
        self.store.lock().await.push(title.clone());
        Ok(title)
    }

    async fn list_titles_due_for_hydration(
        &self,
        _limit: usize,
        excluded_facets: &[MediaFacet],
    ) -> AppResult<Vec<PendingTitleHydration>> {
        Ok(self
            .store
            .lock()
            .await
            .iter()
            .filter(|title| {
                title.metadata_fetched_at.is_none()
                    && !excluded_facets.iter().any(|facet| facet == &title.facet)
                    && match title.facet {
                        MediaFacet::Movie => crate::MovieTitleRef::from_title(title).is_some(),
                        MediaFacet::Series | MediaFacet::Anime => {
                            title.external_ids.iter().any(|external_id| {
                                external_id.source.eq_ignore_ascii_case("tvdb")
                                    && !external_id.value.trim().is_empty()
                            })
                        }
                    }
            })
            .cloned()
            .map(|title| PendingTitleHydration {
                title,
                attempt_count: 0,
            })
            .collect())
    }

    async fn list_movie_titles_missing_smg_id_after_id(
        &self,
        after_id: Option<&str>,
        limit: usize,
    ) -> AppResult<Vec<Title>> {
        let after_id = after_id.unwrap_or_default().trim().to_string();
        let attempts = self.smg_identity_backfill_attempts.lock().await.clone();
        let mut titles = self
            .store
            .lock()
            .await
            .iter()
            .filter(|title| {
                title.facet == MediaFacet::Movie
                    && (title.id > after_id
                        || attempts.get(&title.id).copied().unwrap_or_default() == 0)
                    && attempts.get(&title.id).copied().unwrap_or_default() < 5
                    && title.external_ids.iter().any(|external_id| {
                        matches!(
                            external_id.source.to_ascii_lowercase().as_str(),
                            "tvdb" | "tmdb" | "imdb"
                        ) && !external_id.value.trim().is_empty()
                    })
                    && !title
                        .external_ids
                        .iter()
                        .any(|external_id| external_id.source.eq_ignore_ascii_case("smg"))
            })
            .cloned()
            .collect::<Vec<_>>();
        titles.sort_by(|left, right| left.id.cmp(&right.id));
        titles.truncate(limit);
        Ok(titles)
    }

    async fn record_movie_smg_identity_backfill_unresolved(&self, title_id: &str) -> AppResult<()> {
        let mut attempts = self.smg_identity_backfill_attempts.lock().await;
        *attempts.entry(title_id.to_string()).or_default() += 1;
        Ok(())
    }

    async fn persist_smg_id(
        &self,
        title_id: &str,
        smg_id: i64,
        redirected_from: Option<i64>,
    ) -> AppResult<()> {
        let mut titles = self.store.lock().await;
        let title = titles
            .iter_mut()
            .find(|title| title.id == title_id)
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;
        if redirected_from.is_some() {
            title
                .external_ids
                .retain(|external_id| !external_id.source.eq_ignore_ascii_case("smg"));
        }
        title
            .external_ids
            .retain(|external_id| external_id.source != "smg");
        title.external_ids.push(ExternalId {
            source: "smg".to_string(),
            value: smg_id.to_string(),
        });
        self.smg_identity_backfill_attempts
            .lock()
            .await
            .remove(title_id);
        Ok(())
    }

    async fn mark_title_metadata_hydration_due_now(&self, _: &str) -> AppResult<()> {
        Ok(())
    }

    async fn schedule_title_metadata_hydration_retry(
        &self,
        _: &str,
        _: &str,
        _: i64,
    ) -> AppResult<()> {
        Ok(())
    }

    async fn clear_title_metadata_hydration_retry_state(&self, _: &str) -> AppResult<()> {
        Ok(())
    }

    async fn update_metadata(
        &self,
        id: &str,
        name: Option<String>,
        facet: Option<MediaFacet>,
        tags: Option<Vec<String>>,
        root_folder_id: Option<String>,
    ) -> AppResult<Title> {
        let mut list = self.store.lock().await;
        let title = list
            .iter_mut()
            .find(|entry| entry.id == id)
            .ok_or_else(|| AppError::NotFound(format!("title {}", id)))?;

        if let Some(name) = name {
            let normalized = name.trim();
            if normalized.is_empty() {
                return Err(AppError::Validation("title name cannot be empty".into()));
            }
            title.name = normalized.to_string();
        }

        if let Some(facet) = facet {
            if facet != title.facet {
                return Err(AppError::Validation(
                    "changing a title facet is not supported because titles cannot move between libraries"
                        .into(),
                ));
            }
            title.facet = facet;
        }

        if let Some(tags) = tags {
            title.tags = tags;
        }

        if let Some(root_folder_id) = root_folder_id {
            title.root_folder_id = root_folder_id;
        }

        Ok(title.clone())
    }

    /// FR-056: the transfer repoints the existing row, so everything keyed on
    /// the title id stays exactly where it is. The mock mirrors the store's
    /// behaviour rather than the store's SQL: the row's library and root change,
    /// and nothing else about the title does.
    async fn transfer_to_library(
        &self,
        id: &str,
        library_id: &str,
        root_folder_id: &str,
        facet: Option<MediaFacet>,
        drop_tag_prefixes: &[String],
    ) -> AppResult<()> {
        let mut list = self.store.lock().await;
        let title = list
            .iter_mut()
            .find(|entry| entry.id == id)
            .ok_or_else(|| AppError::NotFound(format!("title {id}")))?;
        title.library_id = library_id.to_string();
        title.root_folder_id = root_folder_id.to_string();
        // FR-057: the facet converts in the same write, and the values derived
        // under the old facet go with it.
        if let Some(facet) = facet {
            title.facet = facet;
            title
                .tags
                .retain(|tag| !drop_tag_prefixes.iter().any(|prefix| tag.starts_with(prefix)));
        }
        Ok(())
    }

    async fn update_monitored(&self, id: &str, monitored: bool) -> AppResult<Title> {
        let mut list = self.store.lock().await;
        let title = list
            .iter_mut()
            .find(|entry| entry.id == id)
            .ok_or_else(|| AppError::NotFound(format!("title {}", id)))?;
        title.monitored = monitored;
        Ok(title.clone())
    }

    async fn update_title_hydrated_metadata(
        &self,
        id: &str,
        metadata: TitleMetadataUpdate,
    ) -> AppResult<Title> {
        let mut list = self.store.lock().await;
        let title = list
            .iter_mut()
            .find(|entry| entry.id == id)
            .ok_or_else(|| AppError::NotFound(format!("title {}", id)))?;
        title.name = metadata.name.unwrap_or(title.name.clone());
        title.year = metadata.year;
        title.overview = metadata.overview;
        title.poster_url = metadata.poster_url;
        title.background_url = metadata.background_url;
        title.sort_title = metadata.sort_title;
        title.slug = metadata.slug;
        title.imdb_id = metadata.imdb_id;
        title.runtime_minutes = metadata.runtime_minutes;
        title.content_status = metadata.content_status;
        match metadata.language {
            MetadataFieldUpdate::Unchanged => {}
            MetadataFieldUpdate::Set(language) => title.language = Some(language),
            MetadataFieldUpdate::Clear => title.language = None,
        }
        title.first_aired = metadata.first_aired;
        title.network = metadata.network;
        title.studio = metadata.studio;
        title.country = metadata.country;
        title.aliases = metadata.aliases;
        title.tagged_aliases = metadata.tagged_aliases;
        title.metadata_language = metadata.metadata_language;
        title.metadata_fetched_at = Some(chrono::Utc::now());
        for external_id in metadata.extra_external_ids {
            title
                .external_ids
                .retain(|candidate| candidate.source != external_id.source);
            title.external_ids.push(external_id);
        }
        Ok(title.clone())
    }

    async fn replace_match_state(
        &self,
        id: &str,
        external_ids: Vec<ExternalId>,
        tags: Vec<String>,
    ) -> AppResult<Title> {
        let mut list = self.store.lock().await;
        let title = list
            .iter_mut()
            .find(|entry| entry.id == id)
            .ok_or_else(|| AppError::NotFound(format!("title {}", id)))?;
        title.external_ids = external_ids;
        title.tags = tags;
        Ok(title.clone())
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        if let Some(operation_log) = self.delete_operation_log.lock().await.clone() {
            operation_log
                .lock()
                .await
                .push(format!("delete_title:{id}"));
        }
        let mut list = self.store.lock().await;
        let position = list
            .iter()
            .position(|entry| entry.id == id)
            .ok_or_else(|| AppError::NotFound(format!("title {}", id)))?;
        list.remove(position);
        Ok(())
    }

    async fn set_folder_path(&self, id: &str, folder_path: &str) -> AppResult<()> {
        if let Some(error) = self.folder_path_write_failure(id).await {
            return Err(error);
        }
        let mut list = self.store.lock().await;
        let title = list
            .iter_mut()
            .find(|entry| entry.id == id)
            .ok_or_else(|| AppError::NotFound(format!("title {}", id)))?;
        title.folder_path = Some(folder_path.to_string());
        Ok(())
    }

    async fn clear_folder_path(&self, id: &str) -> AppResult<()> {
        if let Some(error) = self.folder_path_write_failure(id).await {
            return Err(error);
        }
        let mut list = self.store.lock().await;
        let title = list
            .iter_mut()
            .find(|entry| entry.id == id)
            .ok_or_else(|| AppError::NotFound(format!("title {}", id)))?;
        title.folder_path = None;
        Ok(())
    }

    async fn clear_metadata_language_for_all(&self) -> AppResult<u64> {
        let mut list = self.store.lock().await;
        let mut count = 0u64;
        for title in list.iter_mut() {
            if title.metadata_language.is_some() {
                title.metadata_language = None;
                count += 1;
            }
        }
        Ok(count)
    }
}

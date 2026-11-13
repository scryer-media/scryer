use super::maintenance_evaluation::MaintenanceActionJobReceipts;
use super::*;
use std::sync::atomic::AtomicBool;

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
pub(crate) struct MockTitleRepo {
    pub(crate) store: Arc<Mutex<Vec<Title>>>,
    pub(super) fail_monitoring_for: Mutex<HashSet<String>>,
    /// The title tag registry, in definition order. Mirrors the store's
    /// behaviour rather than its SQL: unique labels, a rename that rewrites
    /// every bag carrying the old label, and a delete that strips it.
    pub(super) title_tag_definitions: Arc<Mutex<Vec<scryer_domain::TitleTagDefinition>>>,
    pub(super) fail_tag_reads: AtomicBool,
    pub(crate) fail_get_by_id: AtomicBool,
    /// `(title_id, id_list_reads_allowed)`: once that many id-list reads
    /// naming the title have succeeded, every read of it fails, whether by
    /// id list or on its own. Lets a fault start at a later read of a job.
    pub(super) fail_reads_of_title_after: Mutex<Option<(String, usize)>>,
    pub(super) lose_folder_claim: AtomicBool,
    /// Keep the store's folder-write counter: every folder write, title
    /// created with a folder and title delete advances it. Off by default,
    /// so a repository without a counter is the default.
    pub(crate) counts_folder_writes: AtomicBool,
    pub(crate) folder_writes: std::sync::atomic::AtomicU64,
    /// Make every pre-flight folder check refuse, as if another title owned
    /// the folder, while the write path itself is left alone.
    pub(super) refuse_folder_checks: AtomicBool,
    pub(super) smg_identity_backfill_attempts: Arc<Mutex<HashMap<String, i64>>>,
    pub(super) metadata_hydration_due: Mutex<HashSet<String>>,
    pub(super) create_or_get_existing_error: Arc<Mutex<Option<String>>>,
    /// `(title_id, message)`: fail folder-ownership writes for one title, so a
    /// compensating transaction can be exercised at the exact commit that fails.
    pub(super) folder_path_write_error: Arc<Mutex<Option<(String, String)>>>,
    pub(super) delete_operation_log: OptionalDeleteOperationLog,
    /// Explicit per-title arrival signal, recorded before any injected delete barrier.
    pub(super) delete_started: Mutex<HashSet<String>>,
    /// `(deletes_allowed, reached)`: once that many title rows are deleted,
    /// the next delete signals `reached` and never finishes, as a process
    /// stopped at that point would.
    pub(super) halt_deletes_after: Mutex<Option<(usize, Arc<tokio::sync::Notify>)>>,
    pub(super) pending_import_items: Option<Arc<Mutex<Vec<LibraryScanUnmatchedItem>>>>,
    pub(super) external_id_batch_lookup_calls: AtomicUsize,
    /// Batched library-presence reads, and the library ids each one asked
    /// about, so a test can prove one read per search and the libraries it
    /// was scoped to.
    pub(super) external_id_presence_reads: std::sync::Mutex<Vec<Vec<String>>>,
    /// How many times a caller asked for the whole catalog to match against.
    /// Scheduled pollers must not pay for this before they know they have
    /// somewhere to send a result.
    pub(super) list_for_matching_calls: AtomicUsize,
    /// Every read title matching makes through the index port: key shapes,
    /// external ids, spelling buckets, index names, collision keys and title
    /// rows by id. A batched call is one read, as it is one round trip (per
    /// chunk) in the SQL store.
    pub(super) matching_reads: AtomicUsize,
    pub(super) list_calls: AtomicUsize,
    pub(super) discovery_context_reads: AtomicUsize,
    pub(super) discovery_read_gate: Mutex<Option<Arc<tokio::sync::Barrier>>>,
    pub(super) monitor_selections: Arc<Mutex<HashMap<String, scryer_domain::MonitorSelection>>>,
    /// Names the search index holds for a title that its row does not — an
    /// anime numbering bridge's cour names. The real store writes these to
    /// `title_search_terms` only, so they are kept apart here too: a read of
    /// the row never returns them, and only the index reads below do.
    pub(super) index_only_names: Arc<Mutex<HashMap<String, Vec<scryer_domain::TaggedAlias>>>>,
    /// The library store the fixture's libraries live in, so folder writes
    /// are checked against the configured roots the way the real store
    /// checks them.
    pub(super) library_store: std::sync::OnceLock<Arc<Mutex<Vec<Library>>>>,
    /// Deleted titles awaiting recycle cleanup, as the store's
    /// `title_recycle_purges` table holds them.
    pub(super) recycle_purges: Arc<Mutex<Vec<crate::TitleRecyclePurge>>>,
    /// Reads and writes of the recycle cleanup records.
    pub(super) recycle_purge_reads: AtomicUsize,
    pub(super) recycle_purge_writes: AtomicUsize,
}
#[derive(Default)]
pub(crate) struct RecordingJobRunRepo {
    pub(super) runs: Arc<Mutex<Vec<JobRunRecord>>>,
    maintenance_action_job_receipts: Option<MaintenanceActionJobReceipts>,
    pub(super) list_job_runs_calls: AtomicUsize,
    pub(super) list_job_runs_for_actor_calls: AtomicUsize,
    pub(super) list_active_job_runs_calls: AtomicUsize,
    /// Stands in for the sqlite writer gate: set it and every job-run write
    /// takes the same process-wide mutex a real sqlite write would.
    writer_gate: Option<Arc<Mutex<()>>>,
    /// Set it and every update that ends a run fails, as a store that is
    /// locked or unreachable at that moment would. Other writes still land.
    pub(super) reject_terminal_updates: AtomicBool,
}

impl RecordingJobRunRepo {
    /// A repository whose writes take the given modeled writer gate.
    pub(super) fn gated_on(writer_gate: Arc<Mutex<()>>) -> Self {
        Self {
            writer_gate: Some(writer_gate),
            ..Self::default()
        }
    }

    pub(super) fn with_maintenance_action_job_receipts(
        maintenance_action_job_receipts: MaintenanceActionJobReceipts,
    ) -> Self {
        Self {
            maintenance_action_job_receipts: Some(maintenance_action_job_receipts),
            ..Self::default()
        }
    }

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

    async fn create_maintenance_search_job_run(
        &self,
        run: &JobRunRecord,
        receipt: &scryer_domain::MaintenanceActionJobReceipt,
    ) -> AppResult<MaintenanceSearchJobRunCreation> {
        receipt
            .validate_schema()
            .map_err(|error| AppError::Validation(error.to_string()))?;
        if receipt.state != scryer_domain::MaintenanceActionJobReceiptState::Accepted
            || receipt.job_run_id.as_deref() != Some(run.id.as_str())
            || run.job_key != JobKey::AcquisitionSearch
        {
            return Err(AppError::Validation(
                "maintenance Search requires an accepted receipt for its acquisition job run"
                    .to_string(),
            ));
        }
        let Some(stored_receipts) = &self.maintenance_action_job_receipts else {
            return Err(AppError::Repository(
                "maintenance Search dispatch is not configured".to_string(),
            ));
        };

        // Keep this receipt store shared with the evaluator. Holding its lock
        // through the job insertion gives the test adapter the same no-duplicate
        // acceptance boundary that the workflow store provides in SQL.
        let mut receipts = stored_receipts.lock().await;
        if let Some(existing) = receipts.iter().find(|existing| {
            existing.key == receipt.key
                && existing.request_hash == receipt.request_hash
                && matches!(
                    existing.state,
                    scryer_domain::MaintenanceActionJobReceiptState::Accepted
                        | scryer_domain::MaintenanceActionJobReceiptState::Completed
                )
        }) {
            return Ok(MaintenanceSearchJobRunCreation::Existing {
                receipt: existing.clone(),
            });
        }
        if let Some(existing) = receipts.iter().find(|existing| {
            existing.key == receipt.key && existing.dispatch_attempt == receipt.dispatch_attempt
        }) {
            return Ok(MaintenanceSearchJobRunCreation::Existing {
                receipt: existing.clone(),
            });
        }

        let mut runs = self.runs.lock().await;
        receipts.push(receipt.clone());
        runs.push(run.clone());
        Ok(MaintenanceSearchJobRunCreation::Created {
            run: Box::new(run.clone()),
            receipt: receipt.clone(),
        })
    }

    async fn update_job_run(&self, run: &JobRunRecord) -> AppResult<JobRunRecord> {
        if run.status.is_terminal() && self.reject_terminal_updates.load(Ordering::SeqCst) {
            return Err(AppError::Repository(
                "job run store refused the terminal write".to_string(),
            ));
        }
        let _writer = match &self.writer_gate {
            Some(gate) => Some(gate.lock().await),
            None => None,
        };
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
    /// Advance the folder-write counter after a write, when it is kept.
    fn advance_folder_writes(&self) -> Option<u64> {
        self.counts_folder_writes
            .load(Ordering::SeqCst)
            .then(|| self.folder_writes.fetch_add(1, Ordering::SeqCst) + 1)
    }

    /// [`TitleRepository::assign_title_folders`] without the counter advance.
    async fn assign_title_folders_unadvanced(
        &self,
        writes: &[crate::TitleFolderWrite],
    ) -> AppResult<()> {
        for write in writes {
            if let Some(error) = self.folder_path_write_failure(&write.title_id).await {
                return Err(error);
            }
            if self.lose_folder_claim.load(Ordering::SeqCst)
                && matches!(
                    write.expected_current,
                    crate::ExpectedTitleFolder::Exactly(_)
                )
            {
                return Err(AppError::validation_refused(
                    crate::error_reasons::folder_ownership::TITLE_FOLDER_CHANGED,
                    "title folder changed before this write",
                ));
            }
        }
        let mut list = self.store.lock().await;
        let planned = self.plan_folder_writes(&list, writes, None, false).await?;
        for write in planned {
            if let Some(title) = list.iter_mut().find(|entry| entry.id == write.title_id) {
                title.folder_path = write.folder_path;
            }
        }
        Ok(())
    }

    fn advance_folder_writes_for_created(&self, title: &Title) {
        if title
            .folder_path
            .as_deref()
            .is_some_and(|path| !path.is_empty())
        {
            self.advance_folder_writes();
        }
    }

    /// Check this repository's folder writes against `libraries`' roots.
    pub(super) fn share_library_roots(&self, libraries: MockLibraryRepo) -> Arc<MockLibraryRepo> {
        let _ = self.library_store.set(libraries.libraries.clone());
        Arc::new(libraries)
    }

    async fn library_roots(&self) -> Vec<String> {
        let Some(store) = self.library_store.get() else {
            return Vec::new();
        };
        let libraries = store.lock().await;
        crate::catalog_workflow::library_root_folders_from_libraries(&libraries, None)
            .into_iter()
            .map(|root| root.path)
            .collect()
    }

    /// The store's folder-write checks, run in memory over `list`. The
    /// injected write failures apply to the write alone, so a pre-flight
    /// check passes where the write after it is lost.
    /// `extra_current` stands for a title being created, which the real
    /// store has already inserted when it checks the title's folder.
    async fn plan_folder_writes(
        &self,
        list: &[Title],
        writes: &[crate::TitleFolderWrite],
        extra_current: Option<&Title>,
        unknown_titles_have_no_folder: bool,
    ) -> AppResult<Vec<crate::title_folder_keys::PlannedTitleFolderWrite>> {
        let mut current: std::collections::BTreeMap<String, Option<String>> = list
            .iter()
            .chain(extra_current)
            .map(|title| (title.id.clone(), title.folder_path.clone()))
            .collect();
        if unknown_titles_have_no_folder {
            for write in writes {
                current.entry(write.title_id.clone()).or_insert(None);
            }
        }
        let occupants = list
            .iter()
            .filter_map(|title| {
                let folder = title.folder_path.clone()?;
                Some(crate::title_folder_keys::TitleFolderOccupant {
                    title_id: title.id.clone(),
                    title_name: title.name.clone(),
                    key: crate::title_folder_keys::title_folder_key(&folder)?,
                    folder_path: folder,
                })
            })
            .collect::<Vec<_>>();
        let roots = self.library_roots().await;
        crate::title_folder_keys::plan_title_folder_writes(writes, &current, &occupants, &roots)
    }

    /// A new title's folder goes through the folder-write checks, as the
    /// real store's creation does.
    async fn check_created_title_folder(&self, list: &[Title], title: &Title) -> AppResult<()> {
        let Some(folder) = title
            .folder_path
            .as_deref()
            .filter(|folder| crate::title_folder_keys::title_folder_key(folder).is_some())
        else {
            return Ok(());
        };
        self.plan_folder_writes(
            list,
            &[crate::TitleFolderWrite::assign(&title.id, folder)],
            Some(title),
            false,
        )
        .await
        .map(|_| ())
    }

    /// The catalog rows behind titles found through the index view, which
    /// carries names no row does.
    async fn rows_of(&self, found: Vec<Title>) -> AppResult<Vec<Title>> {
        let store = self.store.lock().await;
        Ok(found
            .iter()
            .filter_map(|found| store.iter().find(|title| title.id == found.id).cloned())
            .collect())
    }

    async fn titles_naming_key_shapes(&self, keys: &[String]) -> AppResult<Vec<Title>> {
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        let found = crate::ports::titles_matching_lookup_key_shapes(
            self.list_for_matching(None, None).await?,
            keys,
        );
        self.rows_of(found).await
    }

    async fn index_names(
        &self,
        title_id: &str,
    ) -> AppResult<Vec<crate::ports::TitleNameCandidate>> {
        let Some(mut title) = self.get_by_id(title_id).await? else {
            return Ok(Vec::new());
        };
        if let Some(names) = self.index_only_names.lock().await.get(title_id) {
            title.tagged_aliases.extend(names.iter().cloned());
        }
        Ok(crate::ports::title_name_candidates(&title))
    }

    pub(super) async fn fail_create_or_get_existing(&self, message: &str) {
        *self.create_or_get_existing_error.lock().await = Some(message.to_string());
    }

    /// Make every `set_folder_path`/`clear_folder_path` for `title_id` fail.
    pub(crate) async fn fail_folder_path_writes_for(&self, title_id: &str, message: &str) {
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
    async fn list_discovery_context_titles(&self) -> AppResult<Vec<crate::DiscoveryContextTitle>> {
        self.discovery_context_reads.fetch_add(1, Ordering::SeqCst);
        let gate = self.discovery_read_gate.lock().await.clone();
        if let Some(gate) = gate {
            tokio::time::timeout(crate::test_wait::TEST_WAIT_DEADLINE, gate.wait())
                .await
                .expect("read admitted");
            tokio::time::timeout(crate::test_wait::TEST_WAIT_DEADLINE, gate.wait())
                .await
                .expect("read released");
        }
        Ok(self
            .store
            .lock()
            .await
            .iter()
            .map(crate::DiscoveryContextTitle::from)
            .collect())
    }

    async fn replace_title_monitor_selection(
        &self,
        title_id: &str,
        selection: Option<scryer_domain::MonitorSelection>,
    ) -> AppResult<()> {
        let mut selections = self.monitor_selections.lock().await;
        match selection.map(|selection| selection.normalized()) {
            Some(selection) if !selection.is_empty() => {
                selections.insert(title_id.to_string(), selection);
            }
            _ => {
                selections.remove(title_id);
            }
        }
        Ok(())
    }

    async fn get_title_monitor_selection(
        &self,
        title_id: &str,
    ) -> AppResult<Option<scryer_domain::MonitorSelection>> {
        Ok(self.monitor_selections.lock().await.get(title_id).cloned())
    }

    async fn list(
        &self,
        facet: Option<MediaFacet>,
        query: Option<String>,
    ) -> AppResult<Vec<Title>> {
        self.list_calls.fetch_add(1, Ordering::SeqCst);
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

    async fn list_title_tag_definitions(&self) -> AppResult<Vec<TitleTagDefinitionSummary>> {
        if self.fail_tag_reads.load(Ordering::SeqCst) {
            return Err(AppError::Repository(
                "synthetic tag registry read failure".into(),
            ));
        }
        let definitions = self.title_tag_definitions.lock().await.clone();
        let titles = self.store.lock().await.clone();
        let mut summaries = definitions
            .into_iter()
            .map(|definition| {
                let title_count = titles
                    .iter()
                    .filter(|title| title.tags.iter().any(|tag| tag == &definition.label))
                    .count() as u64;
                TitleTagDefinitionSummary {
                    definition,
                    title_count,
                    // The mock owns titles only; series-movie membership lives
                    // on the show repository, so the count it can honestly give
                    // for links is zero.
                    series_movie_count: 0,
                }
            })
            .collect::<Vec<_>>();
        summaries.sort_by(|left, right| left.definition.label.cmp(&right.definition.label));
        Ok(summaries)
    }

    async fn get_title_tag_definition(
        &self,
        id: &str,
    ) -> AppResult<Option<scryer_domain::TitleTagDefinition>> {
        Ok(self
            .title_tag_definitions
            .lock()
            .await
            .iter()
            .find(|definition| definition.id == id)
            .cloned())
    }

    async fn create_title_tag_definition(
        &self,
        definition: &scryer_domain::TitleTagDefinition,
    ) -> AppResult<scryer_domain::TitleTagDefinition> {
        let mut definitions = self.title_tag_definitions.lock().await;
        if definitions
            .iter()
            .any(|existing| existing.label == definition.label)
        {
            return Err(AppError::Validation(format!(
                "a tag named '{}' already exists",
                definition.label
            )));
        }
        definitions.push(definition.clone());
        Ok(definition.clone())
    }

    async fn update_title_tag_definition(
        &self,
        id: &str,
        label: Option<String>,
        description: Option<Option<String>>,
        updated_at: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<(
        scryer_domain::TitleTagDefinition,
        crate::TitleTagMembershipCounts,
    )> {
        let mut definitions = self.title_tag_definitions.lock().await;
        let existing = definitions
            .iter()
            .find(|definition| definition.id == id)
            .cloned()
            .ok_or_else(|| AppError::NotFound(format!("title tag {id}")))?;
        let next_label = label.unwrap_or_else(|| existing.label.clone());
        if next_label != existing.label
            && definitions
                .iter()
                .any(|definition| definition.id != id && definition.label == next_label)
        {
            return Err(AppError::Validation(format!(
                "a tag named '{next_label}' already exists"
            )));
        }

        let updated = scryer_domain::TitleTagDefinition {
            label: next_label.clone(),
            description: description.unwrap_or_else(|| existing.description.clone()),
            updated_at,
            ..existing.clone()
        };
        if let Some(slot) = definitions
            .iter_mut()
            .find(|definition| definition.id == id)
        {
            *slot = updated.clone();
        }
        drop(definitions);

        let mut titles = self.store.lock().await;
        let mut rewritten = 0_u64;
        if next_label != existing.label {
            for title in titles.iter_mut() {
                if !title.tags.iter().any(|tag| tag == &existing.label) {
                    continue;
                }
                title.tags.retain(|tag| tag != &existing.label);
                if !title.tags.iter().any(|tag| tag == &next_label) {
                    title.tags.push(next_label.clone());
                }
                rewritten += 1;
            }
        }
        Ok((
            updated,
            crate::TitleTagMembershipCounts {
                titles: rewritten,
                series_movies: 0,
            },
        ))
    }

    async fn delete_title_tag_definition(
        &self,
        id: &str,
    ) -> AppResult<(
        scryer_domain::TitleTagDefinition,
        crate::TitleTagMembershipCounts,
    )> {
        let mut definitions = self.title_tag_definitions.lock().await;
        let index = definitions
            .iter()
            .position(|definition| definition.id == id)
            .ok_or_else(|| AppError::NotFound(format!("title tag {id}")))?;
        let removed = definitions.remove(index);
        drop(definitions);

        let mut titles = self.store.lock().await;
        let mut stripped = 0_u64;
        for title in titles.iter_mut() {
            if !title.tags.iter().any(|tag| tag == &removed.label) {
                continue;
            }
            title.tags.retain(|tag| tag != &removed.label);
            stripped += 1;
        }
        Ok((
            removed,
            crate::TitleTagMembershipCounts {
                titles: stripped,
                series_movies: 0,
            },
        ))
    }

    async fn update_user_tags(
        &self,
        title_id: &str,
        add: &[String],
        remove: &[String],
    ) -> AppResult<Title> {
        let mut titles = self.store.lock().await;
        let title = titles
            .iter_mut()
            .find(|entry| entry.id == title_id)
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;
        // Removals first, so a label in both lists survives; reserved entries
        // are never removed and never counted against the ceiling. The bag is
        // assembled and validated before it is assigned, because the mock has
        // no transaction to roll back a refused write.
        let mut next_tags = title
            .tags
            .iter()
            .filter(|tag| {
                crate::is_reserved_title_tag(tag) || !remove.iter().any(|removed| removed == *tag)
            })
            .cloned()
            .collect::<Vec<_>>();
        for label in add {
            if !next_tags.iter().any(|tag| tag == label) {
                next_tags.push(label.clone());
            }
        }
        let user_tag_count = next_tags
            .iter()
            .filter(|tag| !crate::is_reserved_title_tag(tag))
            .count();
        if user_tag_count > crate::MAX_USER_TAGS_PER_TITLE {
            return Err(AppError::Validation(format!(
                "a title can carry at most {} tags; this change would leave it with {user_tag_count}",
                crate::MAX_USER_TAGS_PER_TITLE
            )));
        }
        title.tags = next_tags;
        Ok(title.clone())
    }

    async fn list_by_external_ids(&self, source: &str, values: &[String]) -> AppResult<Vec<Title>> {
        let requested: HashSet<&str> = values
            .iter()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .collect();
        let list = self.store.lock().await;
        Ok(list
            .iter()
            .filter(|title| {
                title.external_ids.iter().any(|external_id| {
                    external_id.source.eq_ignore_ascii_case(source)
                        && requested.contains(external_id.value.trim())
                })
            })
            .cloned()
            .collect())
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

    async fn list_existing_external_ids_in_libraries(
        &self,
        library_ids: &[String],
        lookups: &[crate::TitleExternalIdPresenceLookup],
    ) -> AppResult<HashSet<crate::TitleExternalIdPresence>> {
        self.external_id_presence_reads
            .lock()
            .expect("presence read log")
            .push(library_ids.to_vec());
        let list = self.store.lock().await;
        let mut present = HashSet::new();
        for title in list
            .iter()
            .filter(|title| library_ids.contains(&title.library_id))
        {
            for lookup in lookups.iter().filter(|lookup| lookup.facet == title.facet) {
                if title.external_ids.iter().any(|external_id| {
                    external_id
                        .source
                        .eq_ignore_ascii_case(lookup.source.trim())
                        && external_id.value.trim() == lookup.value.trim()
                }) {
                    present.insert(crate::TitleExternalIdPresence {
                        library_id: title.library_id.clone(),
                        lookup: lookup.clone(),
                    });
                }
            }
        }
        Ok(present)
    }

    async fn map_existing_external_ids_to_title_ids_in_library_and_facet(
        &self,
        library_id: &str,
        facet: MediaFacet,
        source: &str,
        values: &[String],
    ) -> AppResult<std::collections::BTreeMap<String, String>> {
        self.external_id_batch_lookup_calls
            .fetch_add(1, Ordering::SeqCst);

        let requested = values
            .iter()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .collect::<HashSet<_>>();
        if requested.is_empty() {
            return Ok(std::collections::BTreeMap::new());
        }

        let list = self.store.lock().await;
        let mut existing = std::collections::BTreeMap::new();
        for title in list
            .iter()
            .filter(|title| title.library_id == library_id.trim() && title.facet == facet)
        {
            for external_id in &title.external_ids {
                let value = external_id.value.trim();
                if external_id.source.eq_ignore_ascii_case(source) && requested.contains(value) {
                    existing
                        .entry(value.to_string())
                        .or_insert_with(|| title.id.clone());
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
        self.list_for_matching_calls
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        // The matching read is the index's view of the catalog.
        let mut titles = self.list(facet, query).await?;
        let index_only_names = self.index_only_names.lock().await;
        for title in &mut titles {
            if let Some(names) = index_only_names.get(&title.id) {
                title.tagged_aliases.extend(names.iter().cloned());
            }
        }
        Ok(titles)
    }

    /// The index finds a title by any name it holds, and hands back the row.
    async fn find_titles_by_lookup_keys(&self, keys: &[String]) -> AppResult<Vec<Title>> {
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        let found = crate::ports::titles_matching_lookup_keys(
            self.list_for_matching(None, None).await?,
            keys,
        );
        self.rows_of(found).await
    }

    async fn find_titles_by_lookup_key_shapes(&self, keys: &[String]) -> AppResult<Vec<Title>> {
        self.matching_reads.fetch_add(1, Ordering::SeqCst);
        self.titles_naming_key_shapes(keys).await
    }

    async fn find_titles_by_lookup_key_shapes_batch(
        &self,
        key_sets: &[Vec<String>],
    ) -> AppResult<Vec<Vec<Title>>> {
        self.matching_reads.fetch_add(1, Ordering::SeqCst);
        let mut results = Vec::with_capacity(key_sets.len());
        for keys in key_sets {
            results.push(self.titles_naming_key_shapes(keys).await?);
        }
        Ok(results)
    }

    async fn find_titles_by_external_id(&self, source: &str, value: &str) -> AppResult<Vec<Title>> {
        self.matching_reads.fetch_add(1, Ordering::SeqCst);
        Ok(crate::ports::titles_matching_external_id(
            self.list_for_matching(None, None).await?,
            source,
            value,
        ))
    }

    async fn find_titles_by_external_ids_batch(
        &self,
        lookups: &[(String, String)],
    ) -> AppResult<Vec<Vec<Title>>> {
        self.matching_reads.fetch_add(1, Ordering::SeqCst);
        let mut results = Vec::with_capacity(lookups.len());
        for (source, value) in lookups {
            results.push(crate::ports::titles_matching_external_id(
                self.list_for_matching(None, None).await?,
                source,
                value,
            ));
        }
        Ok(results)
    }

    async fn lookup_keys_claimed_by_other_titles(
        &self,
        title_id: &str,
        keys: &[String],
    ) -> AppResult<Vec<String>> {
        self.matching_reads.fetch_add(1, Ordering::SeqCst);
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        Ok(crate::ports::lookup_keys_claimed_by_others(
            &self.list_for_matching(None, None).await?,
            title_id,
            keys,
        ))
    }

    async fn lookup_keys_claimed_by_other_titles_batch(
        &self,
        requests: &[(String, Vec<String>)],
    ) -> AppResult<Vec<Vec<String>>> {
        self.matching_reads.fetch_add(1, Ordering::SeqCst);
        let titles = self.list_for_matching(None, None).await?;
        Ok(requests
            .iter()
            .map(|(title_id, keys)| {
                crate::ports::lookup_keys_claimed_by_others(&titles, title_id, keys)
            })
            .collect())
    }

    async fn find_title_name_candidates(
        &self,
        query: crate::ports::TitleNameBucketQuery<'_>,
    ) -> AppResult<Vec<crate::ports::TitleNameCandidate>> {
        self.matching_reads.fetch_add(1, Ordering::SeqCst);
        Ok(crate::ports::name_candidates_in_bucket(
            &self.list_for_matching(None, None).await?,
            &query,
        ))
    }

    async fn find_title_name_candidates_batch(
        &self,
        queries: &[crate::ports::TitleNameBucketQuery<'_>],
    ) -> AppResult<Vec<Vec<crate::ports::TitleNameCandidate>>> {
        self.matching_reads.fetch_add(1, Ordering::SeqCst);
        let titles = self.list_for_matching(None, None).await?;
        Ok(queries
            .iter()
            .map(|query| crate::ports::name_candidates_in_bucket(&titles, query))
            .collect())
    }

    async fn list_title_index_names(
        &self,
        title_id: &str,
    ) -> AppResult<Vec<crate::ports::TitleNameCandidate>> {
        self.matching_reads.fetch_add(1, Ordering::SeqCst);
        self.index_names(title_id).await
    }

    async fn list_title_index_names_batch(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<Vec<crate::ports::TitleNameCandidate>>> {
        self.matching_reads.fetch_add(1, Ordering::SeqCst);
        let mut results = Vec::with_capacity(title_ids.len());
        for title_id in title_ids {
            results.push(self.index_names(title_id).await?);
        }
        Ok(results)
    }

    async fn get_by_ids(&self, ids: &[String]) -> AppResult<Vec<Title>> {
        self.matching_reads.fetch_add(1, Ordering::SeqCst);
        if let Some((title_id, allowed)) = self.fail_reads_of_title_after.lock().await.as_mut()
            && ids.contains(title_id)
        {
            if *allowed == 0 {
                return Err(AppError::Repository("synthetic title read failure".into()));
            }
            *allowed -= 1;
        }
        let list = self.store.lock().await;
        Ok(ids
            .iter()
            .filter_map(|id| list.iter().find(|title| &title.id == id).cloned())
            .collect())
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<Title>> {
        if self.fail_get_by_id.load(Ordering::SeqCst)
            || matches!(
                self.fail_reads_of_title_after.lock().await.as_ref(),
                Some((title_id, 0)) if title_id == id
            )
        {
            return Err(AppError::Repository("synthetic title read failure".into()));
        }
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
        id: &ExternalId,
    ) -> AppResult<Option<Title>> {
        let list = self.store.lock().await;
        Ok(list
            .iter()
            .find(|title| {
                title.facet == facet
                    && title
                        .external_ids
                        .iter()
                        .any(|external_id| external_id.same_entity_as(id))
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

        self.check_created_title_folder(&list, &title).await?;
        list.push(title.clone());
        drop(list);
        self.advance_folder_writes_for_created(&title);
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

        self.check_created_title_folder(&list, &title).await?;
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
        drop(pending_items);
        drop(list);
        self.advance_folder_writes_for_created(&title);
        Ok(CreateTitleOutcome {
            title,
            reused_existing: false,
        })
    }

    async fn create(&self, title: Title) -> AppResult<Title> {
        let mut list = self.store.lock().await;
        self.check_created_title_folder(&list, &title).await?;
        list.push(title.clone());
        drop(list);
        self.advance_folder_writes_for_created(&title);
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
        title
            .external_ids
            .push(ExternalId::new("smg".to_string(), smg_id.to_string()));
        self.smg_identity_backfill_attempts
            .lock()
            .await
            .remove(title_id);
        Ok(())
    }

    async fn mark_title_metadata_hydration_due_now(&self, title_id: &str) -> AppResult<()> {
        self.metadata_hydration_due
            .lock()
            .await
            .insert(title_id.to_string());
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
            title.tags.retain(|tag| {
                !drop_tag_prefixes
                    .iter()
                    .any(|prefix| tag.starts_with(prefix))
            });
        }
        Ok(())
    }

    async fn update_monitored(&self, id: &str, monitored: bool) -> AppResult<Title> {
        if self.fail_monitoring_for.lock().await.contains(id) {
            return Err(AppError::Repository(
                "injected monitoring write failure".into(),
            ));
        }
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
        self.metadata_hydration_due.lock().await.remove(id);
        if let Some(min_availability) = metadata.min_availability {
            title.min_availability = min_availability;
        }
        if let Some(movie_release_dates) = metadata.movie_release_dates {
            title.movie_release_dates = Some(movie_release_dates);
        }
        if let Some(digital_release_date) = metadata.digital_release_date {
            title.digital_release_date = Some(digital_release_date);
        }
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
        self.delete_started.lock().await.insert(id.to_owned());
        let halt = {
            let mut halt = self.halt_deletes_after.lock().await;
            match halt.as_mut() {
                Some((0, reached)) => Some(reached.clone()),
                Some((allowed, _)) => {
                    *allowed -= 1;
                    None
                }
                None => None,
            }
        };
        if let Some(reached) = halt {
            reached.notify_one();
            std::future::pending::<()>().await;
        }
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
        drop(list);
        self.advance_folder_writes();
        Ok(())
    }

    /// The row delete and the record land together, as in the store's single
    /// transaction: a delete that never finishes records nothing.
    async fn delete_recording_recycle_purge(
        &self,
        id: &str,
        recorded_at: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<()> {
        let library_id = self
            .store
            .lock()
            .await
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| entry.library_id.clone());
        self.delete(id).await?;
        if let Some(library_id) = library_id {
            let mut purges = self.recycle_purges.lock().await;
            if !purges.iter().any(|purge| purge.title_id == id) {
                purges.push(crate::TitleRecyclePurge {
                    title_id: id.to_owned(),
                    library_id,
                    recorded_at,
                });
            }
        }
        Ok(())
    }

    async fn list_recycle_purges(&self, limit: usize) -> AppResult<Vec<crate::TitleRecyclePurge>> {
        self.recycle_purge_reads.fetch_add(1, Ordering::SeqCst);
        let mut purges = self.recycle_purges.lock().await.clone();
        purges.sort_by(|a, b| (a.recorded_at, &a.title_id).cmp(&(b.recorded_at, &b.title_id)));
        purges.truncate(limit);
        Ok(purges)
    }

    /// The store's probe read, then a write only when a record is that old.
    async fn drop_recycle_purges_recorded_before(
        &self,
        cutoff: chrono::DateTime<chrono::Utc>,
        limit: usize,
    ) -> AppResult<u32> {
        self.recycle_purge_reads.fetch_add(1, Ordering::SeqCst);
        let mut purges = self.recycle_purges.lock().await;
        let mut expired = purges
            .iter()
            .filter(|purge| purge.recorded_at < cutoff)
            .map(|purge| (purge.recorded_at, purge.title_id.clone()))
            .collect::<Vec<_>>();
        if expired.is_empty() {
            return Ok(0);
        }
        self.recycle_purge_writes.fetch_add(1, Ordering::SeqCst);
        expired.sort();
        expired.truncate(limit);
        purges.retain(|purge| {
            !expired
                .iter()
                .any(|(_, title_id)| *title_id == purge.title_id)
        });
        Ok(expired.len() as u32)
    }

    async fn clear_recycle_purges(&self, title_ids: &[String]) -> AppResult<()> {
        self.recycle_purge_writes.fetch_add(1, Ordering::SeqCst);
        self.recycle_purges
            .lock()
            .await
            .retain(|purge| !title_ids.contains(&purge.title_id));
        Ok(())
    }

    async fn assign_title_folders_with_generation(
        &self,
        writes: &[crate::TitleFolderWrite],
    ) -> (AppResult<()>, Option<u64>) {
        if writes.is_empty() {
            return (Ok(()), None);
        }
        let result = self.assign_title_folders_unadvanced(writes).await;
        (result, self.advance_folder_writes())
    }

    fn title_folder_write_generation(&self) -> Option<u64> {
        self.counts_folder_writes
            .load(Ordering::SeqCst)
            .then(|| self.folder_writes.load(Ordering::SeqCst))
    }

    /// The production write path, with the store's checks run in memory.
    async fn assign_title_folders(&self, writes: &[crate::TitleFolderWrite]) -> AppResult<()> {
        self.assign_title_folders_with_generation(writes).await.0
    }

    async fn check_title_folder_writes(&self, writes: &[crate::TitleFolderWrite]) -> AppResult<()> {
        if self.refuse_folder_checks.load(Ordering::SeqCst) {
            return Err(AppError::validation_refused(
                crate::error_reasons::folder_ownership::FOLDER_ALREADY_OWNED_TITLE,
                "folder is already owned by another title",
            ));
        }
        let list = self.store.lock().await;
        self.plan_folder_writes(&list, writes, None, true)
            .await
            .map(|_| ())
    }

    /// Seeds a folder without the ownership checks, so fixtures can stage the
    /// overlapping records older releases left behind.
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
        drop(list);
        self.advance_folder_writes();
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
        drop(list);
        self.advance_folder_writes();
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

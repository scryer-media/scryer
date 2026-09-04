use super::*;

#[derive(Default)]
pub(super) struct MockDomainEventRepo {
    pub(super) events: Arc<Mutex<Vec<DomainEvent>>>,
    pub(super) subscriber_offsets: Arc<Mutex<HashMap<String, i64>>>,
    pub(super) delete_operation_log: OptionalDeleteOperationLog,
    pub(super) list_calls: AtomicUsize,
}

impl MockDomainEventRepo {
    pub(super) async fn set_delete_operation_log(&self, operation_log: Arc<Mutex<Vec<String>>>) {
        *self.delete_operation_log.lock().await = Some(operation_log);
    }
}

#[derive(Default)]
pub(super) struct MockExternalImportMonitorSnapshotRepo {
    pub(super) chunks: Arc<Mutex<Vec<ExternalImportMonitorSnapshotChunk>>>,
}

#[async_trait]
impl ExternalImportMonitorSnapshotRepository for MockExternalImportMonitorSnapshotRepo {
    async fn append_external_import_monitor_snapshot_chunk(
        &self,
        chunk: &ExternalImportMonitorSnapshotChunk,
    ) -> AppResult<()> {
        self.chunks.lock().await.push(chunk.clone());
        Ok(())
    }

    async fn list_external_import_monitor_snapshot_chunk_batch(
        &self,
        session_id: &str,
        facet: MediaFacet,
        entry_kind: ExternalImportMonitorSnapshotEntryKind,
        after_chunk_index: Option<i32>,
        limit: i32,
    ) -> AppResult<Vec<ExternalImportMonitorSnapshotChunk>> {
        let chunks = self.chunks.lock().await;
        let mut matched = chunks
            .iter()
            .filter(|chunk| {
                chunk.session_id == session_id
                    && chunk.facet == facet
                    && chunk.entry_kind == entry_kind
                    && after_chunk_index
                        .map(|after| chunk.chunk_index > after)
                        .unwrap_or(true)
            })
            .cloned()
            .collect::<Vec<_>>();
        matched.sort_by_key(|chunk| chunk.chunk_index);
        matched.truncate(limit.max(0) as usize);
        Ok(matched)
    }

    async fn delete_external_import_monitor_snapshot_chunks(
        &self,
        session_id: &str,
        facet: MediaFacet,
    ) -> AppResult<()> {
        let mut chunks = self.chunks.lock().await;
        chunks.retain(|chunk| chunk.session_id != session_id || chunk.facet != facet);
        Ok(())
    }

    async fn delete_external_import_monitor_snapshot_chunks_for_session_prefix(
        &self,
        session_prefix: &str,
        facet: MediaFacet,
    ) -> AppResult<()> {
        let mut chunks = self.chunks.lock().await;
        chunks
            .retain(|chunk| !chunk.session_id.starts_with(session_prefix) || chunk.facet != facet);
        Ok(())
    }

    async fn delete_external_import_monitor_snapshot_chunks_except_session_prefix(
        &self,
        preserved_session_prefix: &str,
    ) -> AppResult<()> {
        let mut chunks = self.chunks.lock().await;
        chunks.retain(|chunk| chunk.session_id.starts_with(preserved_session_prefix));
        Ok(())
    }
}

pub(super) async fn append_movie_monitor_snapshot_chunk_for_library(
    app: &AppUseCase,
    user: &User,
    library_id: &str,
    entries: Vec<ExternalImportMonitorMovieEntry>,
) {
    let payload_ndjson = entries
        .into_iter()
        .map(|entry| serde_json::to_string(&entry).expect("serialize movie snapshot entry"))
        .collect::<Vec<_>>()
        .join("\n");
    app.append_external_import_monitor_snapshot_chunk(
        user,
        ExternalImportMonitorSnapshotChunk {
            session_id: crate::external_import_monitor_apply_session_id_for_library(library_id),
            facet: MediaFacet::Movie,
            entry_kind: ExternalImportMonitorSnapshotEntryKind::Movie,
            chunk_index: 0,
            payload_ndjson,
            created_at: Utc::now().to_rfc3339(),
        },
    )
    .await
    .expect("append movie monitor snapshot chunk");
}

pub(super) async fn append_series_monitor_snapshot_chunk(
    app: &AppUseCase,
    user: &User,
    facet: MediaFacet,
    entries: Vec<ExternalImportMonitorSeriesEntry>,
) {
    let payload_ndjson = entries
        .into_iter()
        .map(|entry| serde_json::to_string(&entry).expect("serialize series snapshot entry"))
        .collect::<Vec<_>>()
        .join("\n");
    let session_id = crate::external_import_monitor_apply_session_id_for_library(
        &scryer_domain::default_library_id_for_facet(&facet),
    );
    app.append_external_import_monitor_snapshot_chunk(
        user,
        ExternalImportMonitorSnapshotChunk {
            session_id,
            facet,
            entry_kind: ExternalImportMonitorSnapshotEntryKind::Series,
            chunk_index: 0,
            payload_ndjson,
            created_at: Utc::now().to_rfc3339(),
        },
    )
    .await
    .expect("append monitor snapshot chunk");
}

#[async_trait]
impl DomainEventRepository for MockDomainEventRepo {
    async fn append(&self, event: NewDomainEvent) -> AppResult<DomainEvent> {
        let mut events = self.events.lock().await;
        let sequence = events
            .last()
            .map(|existing| existing.sequence + 1)
            .unwrap_or(1);
        let stored = DomainEvent {
            sequence,
            event_id: event.event_id,
            occurred_at: event.occurred_at,
            actor_kind: event.actor_kind,
            actor_user_id: event.actor_user_id,
            actor_display_name: event.actor_display_name,
            title_id: event.title_id,
            facet: event.facet,
            correlation_id: event.correlation_id,
            causation_id: event.causation_id,
            schema_version: event.schema_version,
            stream: event.stream,
            payload: event.payload,
        };
        events.push(stored.clone());
        Ok(stored)
    }

    async fn append_many(&self, events: Vec<NewDomainEvent>) -> AppResult<Vec<DomainEvent>> {
        let mut stored = Vec::with_capacity(events.len());
        for event in events {
            stored.push(self.append(event).await?);
        }
        Ok(stored)
    }

    async fn list(&self, filter: &DomainEventFilter) -> AppResult<Vec<DomainEvent>> {
        self.list_calls.fetch_add(1, Ordering::SeqCst);
        let events = self.events.lock().await;
        let limit = if filter.limit == 0 {
            usize::MAX
        } else {
            filter.limit
        };
        let iter: Box<dyn Iterator<Item = &DomainEvent>> =
            if filter.after_sequence.is_some() && filter.before_sequence.is_none() {
                Box::new(events.iter())
            } else {
                Box::new(events.iter().rev())
            };
        Ok(iter
            .filter(|event| {
                filter
                    .after_sequence
                    .is_none_or(|after| event.sequence > after)
                    && filter
                        .before_sequence
                        .is_none_or(|before| event.sequence < before)
                    && filter
                        .title_id
                        .as_ref()
                        .is_none_or(|title_id| event.title_id.as_deref() == Some(title_id.as_str()))
                    && filter
                        .facet
                        .as_ref()
                        .is_none_or(|facet| event.facet.as_ref() == Some(facet))
                    && filter.event_types.as_ref().is_none_or(|event_types| {
                        event_types
                            .iter()
                            .any(|event_type| &event.payload.event_type() == event_type)
                    })
            })
            .take(limit)
            .cloned()
            .collect())
    }

    async fn count_dashboard_activity_events(
        &self,
        _: &[String],
        _: chrono::DateTime<chrono::Utc>,
        _: chrono::DateTime<chrono::Utc>,
        _: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<crate::DashboardActivityStats> {
        Ok(crate::DashboardActivityStats::default())
    }

    async fn count_title_history_page_events(
        &self,
        event_types: Option<&[TitleHistoryEventType]>,
        title_ids: Option<&[String]>,
        download_id: Option<&str>,
    ) -> AppResult<i64> {
        let events = self.events.lock().await;
        Ok(events
            .iter()
            .rev()
            .filter_map(crate::event_views::title_history_record_from_domain_event)
            .filter(|record| {
                event_types.is_none_or(|values| values.contains(&record.event_type))
                    && title_ids.is_none_or(|values| values.contains(&record.title_id))
                    && download_id.is_none_or(|value| record.download_id.as_deref() == Some(value))
            })
            .count() as i64)
    }

    async fn list_title_history_page_events(
        &self,
        event_types: Option<&[TitleHistoryEventType]>,
        title_ids: Option<&[String]>,
        download_id: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> AppResult<Vec<DomainEvent>> {
        let page_size = if limit == 0 { usize::MAX } else { limit };
        let events = self.events.lock().await;
        Ok(events
            .iter()
            .rev()
            .filter(|event| {
                crate::event_views::title_history_record_from_domain_event(event).is_some_and(
                    |record| {
                        event_types.is_none_or(|values| values.contains(&record.event_type))
                            && title_ids.is_none_or(|values| values.contains(&record.title_id))
                            && download_id
                                .is_none_or(|value| record.download_id.as_deref() == Some(value))
                    },
                )
            })
            .skip(offset)
            .take(page_size)
            .cloned()
            .collect())
    }

    async fn list_after_sequence(
        &self,
        after_sequence: i64,
        limit: usize,
    ) -> AppResult<Vec<DomainEvent>> {
        let events = self.events.lock().await;
        Ok(events
            .iter()
            .filter(|event| event.sequence > after_sequence)
            .take(limit)
            .cloned()
            .collect())
    }

    async fn delete_for_title_ids(&self, title_ids: &[String]) -> AppResult<u32> {
        if let Some(operation_log) = self.delete_operation_log.lock().await.clone() {
            operation_log
                .lock()
                .await
                .push("delete_domain_events".to_string());
        }
        let mut events = self.events.lock().await;
        let before = events.len();
        events.retain(|event| {
            event
                .title_id
                .as_ref()
                .is_none_or(|title_id| !title_ids.iter().any(|candidate| candidate == title_id))
        });
        Ok((before - events.len()) as u32)
    }

    async fn get_subscriber_offset(&self, subscriber: &str) -> AppResult<i64> {
        let offsets = self.subscriber_offsets.lock().await;
        Ok(*offsets.get(subscriber).unwrap_or(&0))
    }

    async fn set_subscriber_offset(&self, subscriber: &str, sequence: i64) -> AppResult<()> {
        let mut offsets = self.subscriber_offsets.lock().await;
        offsets.insert(subscriber.to_string(), sequence);
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct MockMediaRequestRepo {
    pub(super) requests: Arc<Mutex<Vec<MediaRequest>>>,
    pub(super) domain_events: Option<Arc<MockDomainEventRepo>>,
}

impl MockMediaRequestRepo {
    pub(super) fn with_domain_events(domain_events: Arc<MockDomainEventRepo>) -> Self {
        Self {
            requests: Arc::new(Mutex::new(Vec::new())),
            domain_events: Some(domain_events),
        }
    }
}

pub(super) async fn append_mock_media_request_event(
    domain_events: Option<&Arc<MockDomainEventRepo>>,
    event: NewDomainEvent,
) -> AppResult<DomainEvent> {
    if let Some(domain_events) = domain_events {
        return domain_events.append(event).await;
    }

    Ok(DomainEvent {
        sequence: 0,
        event_id: event.event_id,
        occurred_at: event.occurred_at,
        actor_kind: event.actor_kind,
        actor_user_id: event.actor_user_id,
        actor_display_name: event.actor_display_name,
        title_id: event.title_id,
        facet: event.facet,
        correlation_id: event.correlation_id,
        causation_id: event.causation_id,
        schema_version: event.schema_version,
        stream: event.stream,
        payload: event.payload,
    })
}

#[async_trait]
impl MediaRequestRepository for MockMediaRequestRepo {
    async fn submit(
        &self,
        request: NewMediaRequest,
        requester: &User,
        submitted_event: NewDomainEvent,
    ) -> AppResult<MediaRequestSubmissionResult> {
        let mut requests = self.requests.lock().await;
        let now = Utc::now();
        let stored = MediaRequest {
            id: request.id,
            library_id: request.library_id,
            facet: request.facet,
            status: MediaRequestStatus::Pending,
            identity_fingerprint: request.identity_fingerprint,
            title: request.title,
            sort_title: request.sort_title,
            slug: request.slug,
            poster_url: request.poster_url,
            background_url: request.background_url,
            year: request.year,
            overview: request.overview,
            runtime_minutes: request.runtime_minutes,
            language: request.language,
            content_status: request.content_status,
            rating_summary: request.rating_summary,
            requested_quality_profile_id: request.requested_quality_profile_id,
            requested_quality_profile_name: request.requested_quality_profile_name,
            requested_monitor_type: request.requested_monitor_type,
            requested_monitor_selection: request.requested_monitor_selection,
            external_ids: request.external_ids,
            requesters: vec![MediaRequestRequester {
                user_id: requester.id.clone(),
                username: requester.username.clone(),
                avatar_url: None,
                requested_at: now,
            }],
            created_by_user_id: request.created_by_user_id,
            resolved_by_user_id: None,
            resolved_at: None,
            created_title_id: None,
            approved_quality_profile_id: None,
            approved_quality_profile_name: None,
            created_at: now,
            updated_at: now,
        };
        requests.push(stored.clone());
        drop(requests);
        let event =
            append_mock_media_request_event(self.domain_events.as_ref(), submitted_event).await?;
        Ok(MediaRequestSubmissionResult {
            request: stored,
            event,
        })
    }

    async fn get(&self, request_id: &str) -> AppResult<Option<MediaRequest>> {
        let requests = self.requests.lock().await;
        Ok(requests
            .iter()
            .find(|request| request.id == request_id)
            .cloned())
    }

    async fn resolve_pending_overlapping(
        &self,
        request: &MediaRequest,
        resolution: MediaRequestResolution,
    ) -> AppResult<MediaRequestResolutionResult> {
        let mut requests = self.requests.lock().await;
        let mut updated = 0;
        for candidate in requests.iter_mut().filter(|candidate| {
            candidate.status == MediaRequestStatus::Pending
                && candidate.library_id == request.library_id
                && candidate.facet == request.facet
                && candidate.external_ids.iter().any(|candidate_id| {
                    request.external_ids.iter().any(|request_id| {
                        candidate_id.source == request_id.source
                            && candidate_id.value == request_id.value
                    })
                })
        }) {
            candidate.status = resolution.status;
            candidate.resolved_by_user_id = Some(resolution.resolved_by_user_id.clone());
            candidate.resolved_at = Some(resolution.resolved_at);
            candidate.created_title_id = resolution.created_title_id.clone();
            candidate.approved_quality_profile_id = resolution.approved_quality_profile_id.clone();
            candidate.approved_quality_profile_name =
                resolution.approved_quality_profile_name.clone();
            candidate.updated_at = resolution.resolved_at;
            updated += 1;
        }
        drop(requests);

        let event = if updated > 0 {
            Some(
                append_mock_media_request_event(self.domain_events.as_ref(), resolution.event)
                    .await?,
            )
        } else {
            None
        };

        Ok(MediaRequestResolutionResult { updated, event })
    }

    async fn resolve_pending(
        &self,
        request_id: &str,
        resolution: MediaRequestResolution,
    ) -> AppResult<MediaRequestResolutionResult> {
        let mut requests = self.requests.lock().await;
        let mut updated = 0;
        for candidate in requests.iter_mut().filter(|candidate| {
            candidate.id == request_id && candidate.status == MediaRequestStatus::Pending
        }) {
            candidate.status = resolution.status;
            candidate.resolved_by_user_id = Some(resolution.resolved_by_user_id.clone());
            candidate.resolved_at = Some(resolution.resolved_at);
            candidate.created_title_id = resolution.created_title_id.clone();
            candidate.approved_quality_profile_id = resolution.approved_quality_profile_id.clone();
            candidate.approved_quality_profile_name =
                resolution.approved_quality_profile_name.clone();
            candidate.updated_at = resolution.resolved_at;
            updated += 1;
        }
        drop(requests);

        let event = if updated > 0 {
            Some(
                append_mock_media_request_event(self.domain_events.as_ref(), resolution.event)
                    .await?,
            )
        } else {
            None
        };

        Ok(MediaRequestResolutionResult { updated, event })
    }

    async fn update_pending_request_preferences(
        &self,
        request_id: &str,
        requested_quality_profile_id: String,
        requested_quality_profile_name: String,
        requested_monitor_type: Option<String>,
        requested_monitor_selection: Option<scryer_domain::MonitorSelection>,
        updated_event: NewDomainEvent,
    ) -> AppResult<MediaRequestUpdateResult> {
        let mut requests = self.requests.lock().await;
        let now = Utc::now();
        let Some(request) = requests.iter_mut().find(|request| {
            request.id == request_id && request.status == MediaRequestStatus::Pending
        }) else {
            return Err(AppError::Validation(
                "media request is no longer pending".into(),
            ));
        };
        request.requested_quality_profile_id = Some(requested_quality_profile_id);
        request.requested_quality_profile_name = Some(requested_quality_profile_name);
        request.requested_monitor_type = requested_monitor_type;
        request.requested_monitor_selection = requested_monitor_selection;
        request.updated_at = now;
        let updated = request.clone();
        drop(requests);

        let event =
            append_mock_media_request_event(self.domain_events.as_ref(), updated_event).await?;

        Ok(MediaRequestUpdateResult {
            request: updated,
            event,
        })
    }

    async fn count_pending_by_facet(
        &self,
        library_ids: &[String],
    ) -> AppResult<MediaRequestCounts> {
        let requests = self.requests.lock().await;
        let mut counts = MediaRequestCounts::default();
        let mut seen = HashSet::new();
        for request in requests.iter().filter(|request| {
            request.status == MediaRequestStatus::Pending
                && library_ids
                    .iter()
                    .any(|library_id| library_id == &request.library_id)
        }) {
            if !seen.insert((
                request.library_id.clone(),
                request.identity_fingerprint.clone(),
            )) {
                continue;
            }
            match request.facet {
                MediaFacet::Movie => counts.movie += 1,
                MediaFacet::Series => counts.series += 1,
                MediaFacet::Anime => counts.anime += 1,
            }
        }
        Ok(counts)
    }

    /// Mirrors the SQL store's contract: a title is a key only when a request
    /// created it, and each list is submitter-first, deduped, in request order.
    async fn requester_user_ids_by_title_ids(
        &self,
        title_ids: &[String],
    ) -> AppResult<HashMap<String, Vec<String>>> {
        let wanted: HashSet<&String> = title_ids.iter().collect();
        let mut requests: Vec<MediaRequest> = self
            .requests
            .lock()
            .await
            .iter()
            .filter(|request| {
                request
                    .created_title_id
                    .as_ref()
                    .is_some_and(|title_id| wanted.contains(title_id))
            })
            .cloned()
            .collect();
        requests.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.id.cmp(&right.id))
        });

        let mut by_title: HashMap<String, Vec<String>> = HashMap::new();
        for request in requests {
            let title_id = request
                .created_title_id
                .clone()
                .expect("filtered to requests with a created title");
            let entry = by_title.entry(title_id).or_default();
            let mut requesters = request.requesters.clone();
            requesters.sort_by(|left, right| {
                left.requested_at
                    .cmp(&right.requested_at)
                    .then_with(|| left.user_id.cmp(&right.user_id))
            });
            for user_id in std::iter::once(request.created_by_user_id.clone())
                .chain(requesters.into_iter().map(|requester| requester.user_id))
            {
                if !entry.contains(&user_id) {
                    entry.push(user_id);
                }
            }
        }
        Ok(by_title)
    }

    async fn list(&self, query: MediaRequestQuery) -> AppResult<Vec<MediaRequest>> {
        let requests = self.requests.lock().await;
        Ok(requests
            .iter()
            .filter(|request| {
                query
                    .facet
                    .as_ref()
                    .is_none_or(|facet| &request.facet == facet)
                    && query.status.is_none_or(|status| request.status == status)
                    && query.library_ids.as_ref().is_none_or(|library_ids| {
                        library_ids.iter().any(|id| id == &request.library_id)
                    })
                    && query.requester_user_id.as_ref().is_none_or(|user_id| {
                        request
                            .requesters
                            .iter()
                            .any(|requester| &requester.user_id == user_id)
                    })
            })
            .cloned()
            .collect())
    }
}

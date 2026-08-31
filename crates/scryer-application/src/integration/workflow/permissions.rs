impl AppUseCase {
    async fn require_title_library_permission(
        &self,
        actor: &User,
        title_id: &str,
        permission: scryer_domain::LibraryPermission,
    ) -> AppResult<Title> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;
        self.require_library_permission(actor, &title.library_id, permission)
            .await?;
        Ok(title)
    }
}
impl AppUseCase {
    async fn require_any_library_permission(
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
}
impl AppUseCase {
    async fn filter_download_queue_items_for_permission(
        &self,
        actor: &User,
        items: Vec<DownloadQueueItem>,
        permission: scryer_domain::LibraryPermission,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        let allowed_library_ids = self
            .authorized_library_ids(actor, None, permission)
            .await?
            .into_iter()
            .collect::<HashSet<_>>();
        if allowed_library_ids.is_empty() {
            return Ok(Vec::new());
        }

        let can_view_operational_history = self
            .has_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let title_ids = items
            .iter()
            .filter_map(|item| item.title_id.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let title_library_cache = self
            .services
            .catalog
            .titles
            .get_by_ids(&title_ids)
            .await?
            .into_iter()
            .map(|title| (title.id, title.library_id))
            .collect::<HashMap<_, _>>();
        let mut visible = Vec::new();
        for item in items {
            let allowed = if let Some(title_id) = item.title_id.as_deref() {
                title_library_cache
                    .get(title_id)
                    .map(|library_id| allowed_library_ids.contains(library_id))
                    .unwrap_or(can_view_operational_history)
            } else {
                can_view_operational_history
            };
            if allowed {
                visible.push(item);
            }
        }
        Ok(visible)
    }
}
impl AppUseCase {
    async fn require_download_item_permission(
        &self,
        actor: &User,
        client_id: Option<&str>,
        client_type: Option<&str>,
        download_client_item_id: &str,
        permission: scryer_domain::LibraryPermission,
    ) -> AppResult<()> {
        let item = self
            .find_download_queue_item_raw(client_id, client_type, download_client_item_id)
            .await?;
        if let Some(item) = item
            && let Some(title_id) = item.title_id.as_deref()
        {
            match self
                .require_title_library_permission(actor, title_id, permission)
                .await
            {
                Ok(_) => return Ok(()),
                Err(AppError::NotFound(_)) => {
                    tracing::warn!(
                        title_id,
                        download_client_item_id,
                        "download queue item references a missing title; using any-library permission"
                    );
                }
                Err(error) => return Err(error),
            }
        }
        self.require_any_library_permission(actor, permission).await
    }
}
impl AppUseCase {
    async fn require_completed_download_permission(
        &self,
        actor: &User,
        completed: &CompletedDownload,
        permission: scryer_domain::LibraryPermission,
    ) -> AppResult<()> {
        if let Some(title_id) =
            crate::import_parameters::extract_parameter(&completed.parameters, "*scryer_title_id")
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        {
            self.require_title_library_permission(actor, &title_id, permission)
                .await?;
            Ok(())
        } else {
            self.require_any_library_permission(actor, permission).await
        }
    }
}
/// How many durable history rows one page request may pull in.
///
/// Deliberately generous relative to the page sizes the history and import
/// queries clamp to (50 and 100), and bounded so a long-lived install never
/// pays for its whole download archive on every request.
const DURABLE_DOWNLOAD_HISTORY_ROW_LIMIT: usize = 500;

/// The canonical download this row belongs to, when it carries one.
///
/// The queue item's `download_id` is the wire token the grab handed to the
/// client; a foreign observation carries the bare id instead, so both forms are
/// accepted.
fn canonical_download_id_for_queue_item(
    item: &DownloadQueueItem,
) -> Option<scryer_domain::download_identity::DownloadId> {
    let value = item.download_id.as_deref()?.trim();
    scryer_domain::download_identity::DownloadId::from_wire(value)
        .or_else(|| scryer_domain::download_identity::DownloadId::parse(value))
}

/// The client-locator identity of a history row, for the rows that have no
/// canonical download id to key on.
fn download_history_locator_key(
    client_id: &str,
    client_type: &str,
    download_client_item_id: &str,
) -> String {
    format!(
        "{}:{}:{}",
        client_id.trim().to_ascii_lowercase(),
        client_type.trim().to_ascii_lowercase(),
        download_client_item_id.trim()
    )
}

/// Turn one durable row into the history item the live projection would have
/// produced, or `None` when its persisted state is not one the history surface
/// shows.
fn download_history_item_from_terminal_row(
    row: crate::TerminalDownloadHistoryRow,
) -> Option<DownloadQueueItem> {
    let tracked_state = TrackedDownloadState::from_str_opt(&row.tracked_state)?;
    let (state, import_status) = match tracked_state {
        TrackedDownloadState::Imported => (
            DownloadQueueState::Completed,
            Some(scryer_domain::ImportStatus::Completed),
        ),
        TrackedDownloadState::Failed => (DownloadQueueState::Failed, None),
        // A dismissed grab is closed history; the display layer reads
        // `Ignored` off the tracked state, not off the client state.
        TrackedDownloadState::Ignored => (DownloadQueueState::Completed, None),
        _ => return None,
    };

    let download_client_item_id = row.download_client_item_id.unwrap_or_default();
    let id = if download_client_item_id.is_empty() {
        row.download_id.to_string()
    } else {
        download_client_item_id.clone()
    };
    // Client timestamps are epoch seconds rendered as a string, and the history
    // sorts parse them as integers; a durable row has to speak the same form or
    // it sorts as if it had no timestamp at all.
    let epoch_seconds = |value: Option<chrono::DateTime<chrono::Utc>>| {
        value.map(|value| value.timestamp().to_string())
    };

    Some(DownloadQueueItem {
        id,
        title_id: row.title_id,
        episode_id: row.episode_id,
        title_name: row.source_title.unwrap_or_default(),
        facet: row.facet,
        category: None,
        client_id: row.client_id.unwrap_or_default(),
        client_name: row.client_name.unwrap_or_default(),
        client_type: row.client_type.unwrap_or_default(),
        state,
        progress_percent: if state == DownloadQueueState::Completed {
            100
        } else {
            0
        },
        import_transfer_phase: None,
        import_transfer_bytes: None,
        import_transfer_total_bytes: None,
        import_transfer_started_at: None,
        import_transfer_updated_at: None,
        size_bytes: row.size_bytes,
        remaining_seconds: None,
        queued_at: epoch_seconds(row.submitted_at),
        last_updated_at: epoch_seconds(row.last_state_at),
        attention_required: state == DownloadQueueState::Failed,
        attention_reason: row.tracked_reason.clone(),
        download_client_item_id,
        download_id: Some(row.download_id.to_wire()),
        import_status,
        import_error_code: None,
        import_error_message: None,
        imported_at: None,
        delete_status: None,
        delete_error_message: None,
        is_scryer_origin: row.origin == crate::DownloadOrigin::ScryerSubmission,
        source_provider: row.source_provider_name,
        tracked_state: Some(tracked_state),
        tracked_status: None,
        tracked_status_messages: row.tracked_detail.into_iter().collect(),
        tracked_match_type: None,
        seeding: None,
    })
}

impl AppUseCase {
    /// Durable history rows the live snapshot no longer carries.
    ///
    /// A client that evicts finished jobs from its own list (rTorrent does)
    /// takes the only copy of those history entries with it, so an imported
    /// download simply vanished. These are read back from the persisted
    /// registry/submission rows and merged in behind the live ones.
    async fn durable_download_history_items(
        &self,
        live: &[DownloadQueueItem],
    ) -> AppResult<Vec<DownloadQueueItem>> {
        let mut live_download_ids = HashSet::new();
        let mut live_locators = HashSet::new();
        for item in live {
            if let Some(download_id) = canonical_download_id_for_queue_item(item) {
                live_download_ids.insert(download_id);
            }
            live_locators.insert(download_history_locator_key(
                &item.client_id,
                &item.client_type,
                &item.download_client_item_id,
            ));
        }

        let rows = self
            .services
            .workflow
            .download_submissions
            .list_terminal_download_history_rows(DURABLE_DOWNLOAD_HISTORY_ROW_LIMIT)
            .await?;

        Ok(rows
            .into_iter()
            // The live row is the current truth for a download both sources
            // describe: it carries the client's own progress, delete state and
            // import overlay, none of which the durable row can reconstruct.
            .filter(|row| !live_download_ids.contains(&row.download_id))
            .filter(|row| {
                !live_locators.contains(&download_history_locator_key(
                    row.client_id.as_deref().unwrap_or_default(),
                    row.client_type.as_deref().unwrap_or_default(),
                    row.download_client_item_id.as_deref().unwrap_or_default(),
                ))
            })
            .filter_map(download_history_item_from_terminal_row)
            .filter(|item| is_history_download_state(&item.state))
            .collect())
    }

    async fn collect_download_history_items_for_actor(
        &self,
        actor: &User,
        permission: scryer_domain::LibraryPermission,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        let allowed_library_ids = self
            .authorized_library_ids(actor, None, permission)
            .await?
            .into_iter()
            .collect::<HashSet<_>>();
        let can_view_operational_history = self
            .has_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let (_, model) = self.current_download_queue_read_model().await?;
        let ordering = Self::legacy_download_queue_ordering(&model).await;
        let mut items = ordering
            .iter()
            .map(|index| &model.items[*index])
            .filter(|item| is_history_download_state(&item.state))
            .cloned()
            .collect::<Vec<_>>();

        let durable = self.durable_download_history_items(&items).await?;
        // The read model's title map only covers the live rows, and it is
        // shared behind an `Arc`; the durable rows' titles go in an overlay
        // rather than forcing a copy of the whole map on every history query.
        let mut durable_title_library_ids = HashMap::new();
        if !durable.is_empty() {
            let unknown_title_ids = durable
                .iter()
                .filter_map(|item| item.title_id.clone())
                .filter(|title_id| !model.title_library_ids.contains_key(title_id))
                .collect::<HashSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();
            if !unknown_title_ids.is_empty() {
                for title in self
                    .services
                    .catalog
                    .titles
                    .get_by_ids(&unknown_title_ids)
                    .await?
                {
                    durable_title_library_ids.insert(title.id, title.library_id);
                }
            }
            items.extend(durable);
            // The live ordering only covered the live rows; re-apply the same
            // comparator across the merged set so pagination stays stable.
            items.sort_by(compare_download_queue_items);
        }

        items.retain(|item| {
            item.title_id
                .as_deref()
                .map_or(can_view_operational_history, |title_id| {
                    model
                        .title_library_ids
                        .get(title_id)
                        .or_else(|| durable_title_library_ids.get(title_id))
                        .map_or(can_view_operational_history, |library_id| {
                            allowed_library_ids.contains(library_id)
                        })
                })
        });
        Ok(items)
    }
}

const EXTERNAL_IMPORT_MONITOR_SNAPSHOT_APPLY_CHUNK_BATCH_SIZE: i32 = 4;
fn parse_external_import_monitor_snapshot_line<T: serde::de::DeserializeOwned>(
    line: &str,
) -> AppResult<T> {
    serde_json::from_str(line).map_err(|err| {
        AppError::Validation(format!(
            "failed to parse external import monitor snapshot line: {err}"
        ))
    })
}
#[cfg(test)]
impl AppUseCase {
    pub(crate) async fn apply_pending_external_import_monitor_snapshot_for_facet(
        &self,
        facet: &MediaFacet,
    ) -> AppResult<bool> {
        let library_id = scryer_domain::default_library_id_for_facet(facet);
        self.apply_pending_external_import_monitor_snapshot_for_library(facet, &library_id)
            .await
    }
}
impl AppUseCase {
    /// Low-level title monitoring persistence and side effects. This helper
    /// intentionally does not emit domain events; canonical apply helpers do.
    async fn persist_title_monitoring(&self, title_id: &str, monitored: bool) -> AppResult<Title> {
        let title = self
            .services
            .catalog
            .titles
            .update_monitored(title_id, monitored)
            .await?;

        self.reconcile_series_movie_link_monitoring_for_title(&title)
            .await?;

        if title.monitored {
            self.sync_title_for_immediate_acquisition(&title).await;
        } else if let Err(err) = self
            .services
            .workflow
            .acquisition_scope_states
            .delete_acquisition_scope_states_for_title(&title.id)
            .await
        {
            warn!(
                title_id = title.id.as_str(),
                error = %err,
                "failed to delete wanted items after disabling monitoring"
            );
        }

        Ok(title)
    }
}
/// External ids used to match a stored series-movie selection against a link.
fn monitor_selection_external_ids_from_movie_entity(
    movie: &scryer_domain::MovieEntity,
) -> Vec<ExternalId> {
    [
        ("tvdb", movie.tvdb_id.as_deref()),
        ("tmdb", movie.tmdb_id.as_deref()),
        ("imdb", movie.imdb_id.as_deref()),
        ("anidb", movie.anidb_id.as_deref()),
        ("mal", movie.mal_id.as_deref()),
    ]
    .into_iter()
    .filter_map(|(source, value)| {
        let value = value?.trim();
        (!value.is_empty()).then(|| ExternalId {
            source: source.to_string(),
            value: value.to_string(),
        })
    })
    .collect()
}

/// External ids used to match a stored series-movie selection against the
/// metadata form of the same movie.
fn monitor_selection_external_ids_from_anime_movie(movie: &AnimeMovie) -> Vec<ExternalId> {
    [
        ("tvdb", movie.movie_tvdb_id.map(|id| id.to_string())),
        ("tmdb", movie.movie_tmdb_id.map(|id| id.to_string())),
        ("imdb", movie.movie_imdb_id.clone()),
        ("anidb", movie.movie_anidb_id.map(|id| id.to_string())),
        ("mal", movie.movie_mal_id.map(|id| id.to_string())),
    ]
    .into_iter()
    .filter_map(|(source, value)| {
        let value = value?.trim().to_string();
        (!value.is_empty()).then(|| ExternalId {
            source: source.to_string(),
            value,
        })
    })
    .collect()
}

fn title_policy_monitors_series_movie(
    title: &Title,
    continuity_status: &str,
    metadata_active: bool,
    monitor_selection: Option<&MonitorSelection>,
    movie_external_ids: &[ExternalId],
) -> bool {
    if title.facet != MediaFacet::Anime || !title.monitored || !metadata_active {
        return false;
    }
    if !continuity_status.eq_ignore_ascii_case("canon") {
        return false;
    }
    let monitor_type = title
        .tags
        .iter()
        .find_map(|tag| tag.strip_prefix("scryer:monitor-type:"))
        .map(|value| value.trim().to_ascii_lowercase())
        .unwrap_or_default();
    if monitor_type == MONITOR_TYPE_ADVANCED {
        // Advanced monitoring monitors exactly the movies the user picked; a
        // movie that only shows up in a later metadata refresh is not in the
        // stored selection and therefore stays unmonitored.
        return monitor_selection
            .is_some_and(|selection| selection.monitors_series_movie(movie_external_ids));
    }
    matches!(
        monitor_type.as_str(),
        "allepisodes" | "monitored" | "missing" | "missingandfutureepisodes"
    )
}

impl AppUseCase {
    async fn reconcile_series_movie_link_monitoring_for_title(
        &self,
        title: &Title,
    ) -> AppResult<()> {
        if title.facet != MediaFacet::Anime {
            return Ok(());
        }

        let monitor_selection = self.load_advanced_monitor_selection(title).await?;

        for mut link in self
            .services
            .catalog
            .shows
            .list_series_movie_links_for_title(&title.id)
            .await?
        {
            if link.monitoring_override.is_some() {
                continue;
            }
            let monitored = title_policy_monitors_series_movie(
                title,
                link.continuity_status.as_deref().unwrap_or_default(),
                link.metadata_active,
                monitor_selection.as_ref(),
                &monitor_selection_external_ids_from_movie_entity(&link.movie),
            );
            if link.monitored == monitored {
                continue;
            }

            link.monitored = monitored;
            let link = self
                .services
                .catalog
                .shows
                .upsert_series_movie_link(link)
                .await?;
            if !link.monitored
                && let Err(err) = self
                    .services
                    .workflow
                    .acquisition_scope_states
                    .delete_acquisition_scope_states_for_series_movie_link(&link.id)
                    .await
            {
                warn!(
                    series_movie_link_id = link.id.as_str(),
                    error = %err,
                    "failed to delete wanted items after policy disabled series movie monitoring"
                );
            }
        }
        Ok(())
    }
}
impl AppUseCase {
    /// Canonical owner for direct title monitoring changes.
    async fn apply_title_monitoring_change(
        &self,
        actor: impl Into<DomainEventActor>,
        title_id: &str,
        monitored: bool,
    ) -> AppResult<Title> {
        let current_title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", title_id)))?;
        if current_title.monitored == monitored {
            return Ok(current_title);
        }

        let title = self.persist_title_monitoring(title_id, monitored).await?;
        self.emit_title_updated_activity(actor, &title).await;
        Ok(title)
    }
}
impl AppUseCase {
    /// Low-level collection monitoring persistence. This helper intentionally
    /// does not emit domain events; canonical apply helpers do.
    async fn persist_collection_monitoring(
        &self,
        collection_id: &str,
        monitored: bool,
        propagate_to_episodes: bool,
    ) -> AppResult<Collection> {
        let collection = self
            .services
            .catalog
            .shows
            .update_collection(
                collection_id,
                CollectionUpdate {
                    monitored: Some(monitored),
                    ..Default::default()
                },
            )
            .await?;

        if propagate_to_episodes {
            self.services
                .catalog
                .shows
                .set_collection_episodes_monitored(collection_id, monitored)
                .await?;
        }

        if !monitored
            && let Err(err) = self
                .services
                .workflow
                .acquisition_scope_states
                .delete_acquisition_scope_states_for_collection(collection_id)
                .await
        {
            warn!(
                collection_id,
                error = %err,
                "failed to delete wanted items after disabling collection monitoring"
            );
        }

        Ok(collection)
    }
}
impl AppUseCase {
    /// Low-level episode monitoring persistence. This helper intentionally
    /// does not emit domain events; canonical apply helpers do.
    async fn persist_episode_monitoring(
        &self,
        episode_id: &str,
        monitored: bool,
    ) -> AppResult<Episode> {
        let episode = self
            .services
            .catalog
            .shows
            .update_episode(
                episode_id,
                EpisodeUpdate {
                    monitored: Some(monitored),
                    ..Default::default()
                },
            )
            .await?;

        if !monitored
            && let Err(err) = self
                .services
                .workflow
                .acquisition_scope_states
                .delete_acquisition_scope_states_for_episode(episode_id)
                .await
        {
            warn!(
                episode_id,
                error = %err,
                "failed to delete wanted items after disabling episode monitoring"
            );
        }

        Ok(episode)
    }
}
impl AppUseCase {
    async fn apply_movie_monitor_snapshot_chunks(
        &self,
        session_id: &str,
        library_id: &str,
        _now: &DateTime<Utc>,
    ) -> AppResult<()> {
        let titles = self
            .services
            .catalog
            .titles
            .list_for_libraries(Some(MediaFacet::Movie), &[library_id.to_string()], None)
            .await?;
        let mut titles_by_tmdb = HashMap::<String, Vec<Title>>::new();
        let mut titles_by_imdb = HashMap::<String, Vec<Title>>::new();

        for title in &titles {
            push_title_external_id_index(
                &mut titles_by_tmdb,
                title_external_id_value(title, "tmdb"),
                title,
            );
            push_title_external_id_index(
                &mut titles_by_imdb,
                title_external_id_value(title, "imdb"),
                title,
            );
        }

        let mut touched_title_ids = HashSet::new();
        let mut unresolved_entries = 0usize;
        let mut processed_chunk_count = 0i32;
        let mut after_chunk_index = None;
        loop {
            let chunks = self
                .services
                .workflow
                .external_import_monitor_snapshots
                .list_external_import_monitor_snapshot_chunk_batch(
                    session_id,
                    MediaFacet::Movie,
                    ExternalImportMonitorSnapshotEntryKind::Movie,
                    after_chunk_index,
                    EXTERNAL_IMPORT_MONITOR_SNAPSHOT_APPLY_CHUNK_BATCH_SIZE,
                )
                .await?;
            if chunks.is_empty() {
                break;
            }

            for chunk in chunks {
                after_chunk_index = Some(chunk.chunk_index);
                processed_chunk_count += 1;
                for line in chunk
                    .payload_ndjson
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                {
                    let entry: ExternalImportMonitorMovieEntry =
                        parse_external_import_monitor_snapshot_line(line)?;
                    let matched_title =
                        unique_title_match(&titles_by_tmdb, entry.tmdb_id.as_deref()).or_else(
                            || unique_title_match(&titles_by_imdb, entry.imdb_id.as_deref()),
                        );
                    let Some(title) = matched_title else {
                        unresolved_entries += 1;
                        continue;
                    };

                    let updated = self
                        .apply_title_monitoring_change(None, &title.id, entry.monitored)
                        .await?;
                    touched_title_ids.insert(updated.id);
                }
            }
        }
        if processed_chunk_count == 0 {
            return Ok(());
        }

        for title_id in touched_title_ids {
            let Some(title) = self.services.catalog.titles.get_by_id(&title_id).await? else {
                continue;
            };

            if title.monitored {
                // The derived target set already reflects the new monitored
                // state; the woken cycle picks the title up immediately.
                self.runtime.acquisition.acquisition_wake.notify_one();
            } else {
                self.services
                    .workflow
                    .acquisition_scope_states
                    .delete_acquisition_scope_states_for_title(&title.id)
                    .await?;
            }
        }

        if unresolved_entries > 0 {
            return Err(AppError::Repository(format!(
                "{unresolved_entries} imported movie monitoring entries did not match titles in library {library_id}"
            )));
        }

        Ok(())
    }
}
impl AppUseCase {
    async fn apply_series_monitor_snapshot_entry(
        &self,
        title: &Title,
        entry: &ExternalImportMonitorSeriesEntry,
    ) -> AppResult<(bool, bool)> {
        let collections = self
            .services
            .catalog
            .shows
            .list_collections_for_title(&title.id)
            .await?;
        let episodes = self
            .services
            .catalog
            .shows
            .list_episodes_for_title(&title.id)
            .await?;

        let mut season_overrides = HashMap::<String, bool>::new();
        for season in &entry.seasons {
            season_overrides.insert(season.season_number.to_string(), season.monitored);
        }

        let mut episode_overrides_by_tvdb = HashMap::<String, bool>::new();
        let mut episode_overrides_by_number = HashMap::<(String, String), bool>::new();
        for episode in &entry.episodes {
            if let Some(tvdb_id) = episode.tvdb_id.as_deref().map(str::trim)
                && !tvdb_id.is_empty()
            {
                episode_overrides_by_tvdb.insert(tvdb_id.to_string(), episode.monitored);
            }
            episode_overrides_by_number.insert(
                (
                    episode.season_number.to_string(),
                    episode.episode_number.to_string(),
                ),
                episode.monitored,
            );
        }

        let mut collections_with_monitored_episodes = HashSet::new();
        let mut episodes_to_enable = Vec::new();
        let mut episodes_to_disable = Vec::new();
        for episode in &episodes {
            let season_default = episode
                .season_number
                .as_deref()
                .map(str::trim)
                .and_then(|season_number| season_overrides.get(season_number))
                .copied()
                .unwrap_or(entry.monitored);
            let desired = episode
                .tvdb_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .and_then(|tvdb_id| episode_overrides_by_tvdb.get(tvdb_id))
                .copied()
                .or_else(|| {
                    let season_number = episode.season_number.as_deref()?.trim();
                    let episode_number = episode.episode_number.as_deref()?.trim();
                    episode_overrides_by_number
                        .get(&(season_number.to_string(), episode_number.to_string()))
                        .copied()
                })
                .unwrap_or(season_default);
            if desired && let Some(collection_id) = episode.collection_id.as_deref() {
                collections_with_monitored_episodes.insert(collection_id.to_string());
            }
            if episode.monitored != desired {
                if desired {
                    episodes_to_enable.push(episode.id.clone());
                } else {
                    episodes_to_disable.push(episode.id.clone());
                }
            }
        }

        let mut collections_to_enable = Vec::new();
        let mut collections_to_disable = Vec::new();
        for collection in &collections {
            let desired = season_overrides
                .get(collection.collection_index.trim())
                .copied()
                .unwrap_or(entry.monitored)
                || collections_with_monitored_episodes.contains(&collection.id);
            if collection.monitored != desired {
                if desired {
                    collections_to_enable.push(collection.id.clone());
                } else {
                    collections_to_disable.push(collection.id.clone());
                }
            }
        }

        if !collections_to_disable.is_empty() {
            self.services
                .catalog
                .shows
                .set_collections_monitored(&collections_to_disable, false)
                .await?;
        }
        if !collections_to_enable.is_empty() {
            self.services
                .catalog
                .shows
                .set_collections_monitored(&collections_to_enable, true)
                .await?;
        }
        if !episodes_to_disable.is_empty() {
            self.services
                .catalog
                .shows
                .set_episodes_monitored(&episodes_to_disable, false)
                .await?;
        }
        if !episodes_to_enable.is_empty() {
            self.services
                .catalog
                .shows
                .set_episodes_monitored(&episodes_to_enable, true)
                .await?;
        }

        let updated_title = self
            .apply_title_monitoring_change(None, &title.id, entry.monitored)
            .await?;

        Ok((
            updated_title.monitored != title.monitored
                || !collections_to_enable.is_empty()
                || !collections_to_disable.is_empty()
                || !episodes_to_enable.is_empty()
                || !episodes_to_disable.is_empty(),
            updated_title.monitored != title.monitored,
        ))
    }
}
impl AppUseCase {
    async fn apply_series_monitor_snapshot_chunks(
        &self,
        facet: &MediaFacet,
        session_id: &str,
        library_id: &str,
        _now: &DateTime<Utc>,
    ) -> AppResult<()> {
        let titles = self
            .services
            .catalog
            .titles
            .list_for_libraries(Some(facet.clone()), &[library_id.to_string()], None)
            .await?;
        let mut titles_by_tvdb = HashMap::<String, Vec<Title>>::new();

        for title in &titles {
            push_title_external_id_index(
                &mut titles_by_tvdb,
                title_external_id_value(title, "tvdb"),
                title,
            );
        }

        let mut touched_title_ids = HashSet::new();
        let mut title_ids_needing_activity = HashSet::<String>::new();
        let mut unresolved_entries = 0usize;
        let mut processed_chunk_count = 0i32;
        let mut after_chunk_index = None;
        loop {
            let chunks = self
                .services
                .workflow
                .external_import_monitor_snapshots
                .list_external_import_monitor_snapshot_chunk_batch(
                    session_id,
                    facet.clone(),
                    ExternalImportMonitorSnapshotEntryKind::Series,
                    after_chunk_index,
                    EXTERNAL_IMPORT_MONITOR_SNAPSHOT_APPLY_CHUNK_BATCH_SIZE,
                )
                .await?;
            if chunks.is_empty() {
                break;
            }

            for chunk in chunks {
                after_chunk_index = Some(chunk.chunk_index);
                processed_chunk_count += 1;
                for line in chunk
                    .payload_ndjson
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                {
                    let entry: ExternalImportMonitorSeriesEntry =
                        parse_external_import_monitor_snapshot_line(line)?;
                    let Some(title) = unique_title_match(&titles_by_tvdb, entry.tvdb_id.as_deref())
                    else {
                        unresolved_entries += 1;
                        continue;
                    };

                    let (changed, title_activity_emitted) = self
                        .apply_series_monitor_snapshot_entry(&title, &entry)
                        .await?;
                    if changed {
                        touched_title_ids.insert(title.id.clone());
                        if !title_activity_emitted {
                            title_ids_needing_activity.insert(title.id.clone());
                        }
                    }
                }
            }
        }
        if processed_chunk_count == 0 {
            return Ok(());
        }

        for title_id in touched_title_ids {
            let Some(title) = self.services.catalog.titles.get_by_id(&title_id).await? else {
                continue;
            };

            if title_ids_needing_activity.contains(&title_id) {
                self.emit_title_updated_activity(None, &title).await;
            }
            // Monitored-state changes flow straight into the derived target
            // set; waking the cycle is all immediate acquisition needs.
            self.runtime.acquisition.acquisition_wake.notify_one();
        }

        if unresolved_entries > 0 {
            return Err(AppError::Repository(format!(
                "{unresolved_entries} imported {} monitoring entries did not match titles in library {library_id}",
                facet.as_str()
            )));
        }

        Ok(())
    }
}
impl AppUseCase {
    pub(crate) async fn apply_pending_external_import_monitor_snapshot_for_library(
        &self,
        facet: &MediaFacet,
        library_id: &str,
    ) -> AppResult<bool> {
        let _apply_guard = self.acquire_external_import_apply_guard().await;
        let apply_session_id =
            crate::external_import_monitor_apply_session_id_for_library(library_id);
        let now = Utc::now();
        let chunk_entry_kind = match facet {
            MediaFacet::Movie => ExternalImportMonitorSnapshotEntryKind::Movie,
            MediaFacet::Series | MediaFacet::Anime => {
                ExternalImportMonitorSnapshotEntryKind::Series
            }
        };
        let chunk_batch = self
            .services
            .workflow
            .external_import_monitor_snapshots
            .list_external_import_monitor_snapshot_chunk_batch(
                &apply_session_id,
                facet.clone(),
                chunk_entry_kind,
                None,
                1,
            )
            .await?;

        if chunk_batch.is_empty() {
            return Ok(false);
        }

        match facet {
            MediaFacet::Movie => {
                self.apply_movie_monitor_snapshot_chunks(&apply_session_id, library_id, &now)
                    .await?;
            }
            MediaFacet::Series | MediaFacet::Anime => {
                self.apply_series_monitor_snapshot_chunks(
                    facet,
                    &apply_session_id,
                    library_id,
                    &now,
                )
                .await?;
            }
        }

        self.services
            .workflow
            .external_import_monitor_snapshots
            .delete_external_import_monitor_snapshot_chunks(&apply_session_id, facet.clone())
            .await?;

        Ok(true)
    }
}
impl AppUseCase {
    /// Canonical owner for collection monitoring orchestration. Dedicated
    /// monitor mutations and generic collection updates must both delegate here
    /// so propagation and immediate acquisition behavior cannot drift.
    async fn apply_collection_monitoring_change(
        &self,
        actor: impl Into<DomainEventActor>,
        collection_id: &str,
        monitored: bool,
        propagate_to_episodes: bool,
        sync_title_if_already_monitored: bool,
    ) -> AppResult<Collection> {
        let current_collection = self
            .services
            .catalog
            .shows
            .get_collection_by_id(collection_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("collection {}", collection_id)))?;
        let collection_changed = current_collection.monitored != monitored;
        let episode_propagation_changed = if propagate_to_episodes {
            self.services
                .catalog
                .shows
                .list_episodes_for_collection(collection_id)
                .await?
                .iter()
                .any(|episode| episode.monitored != monitored)
        } else {
            false
        };
        let effective_collection_change = collection_changed || episode_propagation_changed;
        let collection = if effective_collection_change {
            self.persist_collection_monitoring(collection_id, monitored, propagate_to_episodes)
                .await?
        } else {
            current_collection
        };
        let mut title_changed = false;
        let mut final_title = None;

        if monitored {
            let title = self
                .services
                .catalog
                .titles
                .get_by_id(&collection.title_id)
                .await?
                .ok_or_else(|| AppError::NotFound(format!("title {}", collection.title_id)))?;

            if !title.monitored {
                final_title = Some(self.persist_title_monitoring(&title.id, true).await?);
                title_changed = true;
                tracing::info!(
                    title_id = %title.id,
                    title_name = %title.name,
                    "auto-monitored title because a collection was monitored"
                );
            } else {
                if effective_collection_change && sync_title_if_already_monitored {
                    self.sync_title_for_immediate_acquisition(&title).await;
                }
                final_title = Some(title);
            }
        }

        if (effective_collection_change || title_changed) && final_title.is_none() {
            final_title = self
                .services
                .catalog
                .titles
                .get_by_id(&collection.title_id)
                .await?;
        }

        if let Some(title) = final_title {
            self.emit_title_updated_activity(actor, &title).await;
        }

        Ok(collection)
    }
}
impl AppUseCase {
    /// Canonical owner for episode monitoring orchestration. Dedicated monitor
    /// mutations and generic episode updates must both delegate here so parent
    /// propagation and immediate acquisition behavior stay single-sourced.
    async fn apply_episode_monitoring_change(
        &self,
        actor: impl Into<DomainEventActor>,
        episode_id: &str,
        monitored: bool,
        sync_title_if_already_monitored: bool,
    ) -> AppResult<Episode> {
        let current_episode = self
            .services
            .catalog
            .shows
            .get_episode_by_id(episode_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("episode {}", episode_id)))?;
        let episode_changed = current_episode.monitored != monitored;
        let episode = if episode_changed {
            self.persist_episode_monitoring(episode_id, monitored)
                .await?
        } else {
            current_episode
        };
        let mut collection_changed = false;
        let mut title_changed = false;
        let mut final_title = None;

        if monitored {
            if let Some(collection_id) = episode.collection_id.as_deref() {
                let collection = self
                    .services
                    .catalog
                    .shows
                    .get_collection_by_id(collection_id)
                    .await?
                    .ok_or_else(|| AppError::NotFound(format!("collection {}", collection_id)))?;

                if !collection.monitored {
                    self.persist_collection_monitoring(collection_id, true, false)
                        .await?;
                    collection_changed = true;
                    tracing::info!(
                        collection_id = %collection_id,
                        "auto-monitored collection because an episode was monitored"
                    );
                }
            }

            let title = self
                .services
                .catalog
                .titles
                .get_by_id(&episode.title_id)
                .await?
                .ok_or_else(|| AppError::NotFound(format!("title {}", episode.title_id)))?;

            if !title.monitored {
                final_title = Some(self.persist_title_monitoring(&title.id, true).await?);
                title_changed = true;
                tracing::info!(
                    title_id = %title.id,
                    title_name = %title.name,
                    "auto-monitored title because an episode was monitored"
                );
            } else {
                if (episode_changed || collection_changed) && sync_title_if_already_monitored {
                    self.sync_title_for_immediate_acquisition(&title).await;
                }
                final_title = Some(title);
            }
        }

        if (episode_changed || collection_changed || title_changed) && final_title.is_none() {
            final_title = self
                .services
                .catalog
                .titles
                .get_by_id(&episode.title_id)
                .await?;
        }

        if let Some(title) = final_title {
            self.emit_title_updated_activity(actor, &title).await;
        }

        Ok(episode)
    }
}
impl AppUseCase {
    pub async fn set_series_movie_monitored(
        &self,
        actor: &User,
        series_movie_link_id: &str,
        monitored: bool,
    ) -> AppResult<SeriesMovieLink> {
        let mut link = self
            .services
            .catalog
            .shows
            .get_series_movie_link_by_id(series_movie_link_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("series movie {}", series_movie_link_id)))?;
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(&link.series_title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", link.series_title_id)))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;

        link.monitored = monitored;
        link.monitoring_override = Some(monitored);
        let link = self
            .services
            .catalog
            .shows
            .upsert_series_movie_link(link)
            .await?;
        let mut final_title = Some(title);

        if monitored {
            if let Some(existing_title) = final_title.as_ref() {
                if !existing_title.monitored {
                    final_title = Some(
                        self.persist_title_monitoring(&existing_title.id, true)
                            .await?,
                    );
                } else {
                    self.sync_title_for_immediate_acquisition(existing_title)
                        .await;
                }
            }
        } else if let Err(err) = self
            .services
            .workflow
            .acquisition_scope_states
            .delete_acquisition_scope_states_for_series_movie_link(&link.id)
            .await
        {
            warn!(
                series_movie_link_id = link.id.as_str(),
                error = %err,
                "failed to delete wanted items after disabling series movie monitoring"
            );
        }

        if let Some(title) = final_title {
            self.emit_title_updated_activity(actor, &title).await;
        }

        Ok(link)
    }
}
impl AppUseCase {
    pub async fn set_title_monitored(
        &self,
        actor: &User,
        id: &str,
        monitored: bool,
    ) -> AppResult<Title> {
        let library_id = self
            .services
            .catalog
            .libraries
            .title_library_id(id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {id}")))?;
        self.require_library_permission(
            actor,
            &library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;

        self.apply_title_monitoring_change(actor, id, monitored)
            .await
    }
}
impl AppUseCase {
    pub async fn set_collection_monitored(
        &self,
        actor: &User,
        collection_id: &str,
        monitored: bool,
    ) -> AppResult<Collection> {
        let collection = self
            .services
            .catalog
            .shows
            .get_collection_by_id(collection_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("collection {}", collection_id)))?;
        let library_id = self
            .services
            .catalog
            .libraries
            .title_library_id(&collection.title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", collection.title_id)))?;
        self.require_library_permission(
            actor,
            &library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;

        let collection = self
            .apply_collection_monitoring_change(actor, collection_id, monitored, true, true)
            .await?;
        Ok(collection)
    }
}
impl AppUseCase {
    pub async fn set_episode_monitored(
        &self,
        actor: &User,
        episode_id: &str,
        monitored: bool,
    ) -> AppResult<Episode> {
        let episode = self
            .services
            .catalog
            .shows
            .get_episode_by_id(episode_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("episode {}", episode_id)))?;
        let library_id = self
            .services
            .catalog
            .libraries
            .title_library_id(&episode.title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", episode.title_id)))?;
        self.require_library_permission(
            actor,
            &library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;

        let episode = self
            .apply_episode_monitoring_change(actor, episode_id, monitored, true)
            .await?;
        Ok(episode)
    }
}
impl AppUseCase {
    #[expect(
        clippy::too_many_arguments,
        reason = "collection updates keep each mutable field explicit for callers and validation"
    )]
    pub async fn update_collection(
        &self,
        actor: &User,
        collection_id: String,
        collection_type: Option<String>,
        collection_index: Option<String>,
        label: Option<String>,
        ordered_path: Option<String>,
        first_episode_number: Option<String>,
        last_episode_number: Option<String>,
        monitored: Option<bool>,
    ) -> AppResult<Collection> {
        self.require_collection_permission(
            actor,
            &collection_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;

        if let Some(raw) = &collection_type
            && raw.trim().is_empty()
        {
            return Err(AppError::Validation(
                "collection type cannot be empty".into(),
            ));
        }
        let parsed_type = collection_type
            .map(|raw| {
                CollectionType::parse(raw.trim().to_lowercase().as_str()).ok_or_else(|| {
                    AppError::Validation(format!("unknown collection type: {}", raw))
                })
            })
            .transpose()?;

        if let Some(raw) = &collection_index
            && raw.trim().is_empty()
        {
            return Err(AppError::Validation(
                "collection index cannot be empty".into(),
            ));
        }

        let update = CollectionUpdate {
            collection_type: parsed_type,
            collection_index: collection_index.map(|value| value.trim().to_string()),
            label: normalize_show_text_opt(label),
            ordered_path: normalize_show_text_opt(ordered_path),
            clear_ordered_path: false,
            first_episode_number: normalize_show_text_opt(first_episode_number),
            last_episode_number: normalize_show_text_opt(last_episode_number),
            monitored,
        };
        if !update.has_changes() {
            return Err(AppError::Validation(
                "at least one collection field must be provided".into(),
            ));
        }

        let has_non_monitor_updates = update.has_non_monitor_changes();
        let monitored = update.monitored;

        let mut collection = if has_non_monitor_updates {
            let mut repo_update = update.clone();
            repo_update.monitored = None;
            Some(
                self.services
                    .catalog
                    .shows
                    .update_collection(&collection_id, repo_update)
                    .await?,
            )
        } else {
            None
        };

        if let Some(monitored) = monitored {
            collection = Some(
                self.apply_collection_monitoring_change(
                    actor,
                    &collection_id,
                    monitored,
                    true,
                    true,
                )
                .await?,
            );
        }

        let collection = collection.ok_or_else(|| {
            AppError::Validation("at least one collection field must be provided".into())
        })?;

        Ok(collection)
    }
}
impl AppUseCase {
    #[expect(
        clippy::too_many_arguments,
        reason = "episode updates keep each mutable field explicit for validation and auditing"
    )]
    pub async fn update_episode(
        &self,
        actor: &User,
        episode_id: String,
        episode_type: Option<String>,
        episode_number: Option<String>,
        season_number: Option<String>,
        episode_label: Option<String>,
        title: Option<String>,
        air_date: Option<String>,
        duration_seconds: Option<i64>,
        has_multi_audio: Option<bool>,
        has_subtitle: Option<bool>,
        monitored: Option<bool>,
        collection_id: Option<String>,
        overview: Option<String>,
    ) -> AppResult<Episode> {
        self.require_episode_permission(
            actor,
            &episode_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;

        if let Some(raw) = &episode_type
            && raw.trim().is_empty()
        {
            return Err(AppError::Validation("episode type cannot be empty".into()));
        }

        let parsed_episode_type = episode_type
            .map(|value| {
                scryer_domain::EpisodeType::parse(value.trim().to_lowercase().as_str())
                    .ok_or_else(|| AppError::Validation(format!("unknown episode type: {}", value)))
            })
            .transpose()?;

        let update = EpisodeUpdate {
            episode_type: parsed_episode_type,
            episode_number: normalize_show_text_opt(episode_number),
            season_number: normalize_show_text_opt(season_number),
            episode_label: normalize_show_text_opt(episode_label),
            title: normalize_show_text_opt(title),
            air_date: normalize_show_text_opt(air_date),
            duration_seconds,
            has_multi_audio,
            has_subtitle,
            monitored,
            collection_id,
            overview,
            tvdb_id: None,
            image_url: None,
            clear_image_url: false,
        };
        if !update.has_changes() {
            return Err(AppError::Validation(
                "at least one episode field must be provided".into(),
            ));
        }

        let has_non_monitor_updates = update.has_non_monitor_changes();
        let monitored = update.monitored;

        let mut episode = if has_non_monitor_updates {
            let mut repo_update = update.clone();
            repo_update.monitored = None;
            Some(
                self.services
                    .catalog
                    .shows
                    .update_episode(&episode_id, repo_update)
                    .await?,
            )
        } else {
            None
        };

        if let Some(monitored) = monitored {
            episode = Some(
                self.apply_episode_monitoring_change(actor, &episode_id, monitored, true)
                    .await?,
            );
        }

        let episode = episode.ok_or_else(|| {
            AppError::Validation("at least one episode field must be provided".into())
        })?;

        Ok(episode)
    }
}
/// Extract the monitor type from title tags (e.g. "scryer:monitor-type:none").
/// Defaults to "allEpisodes" when no tag is present for backward compatibility.
fn extract_monitor_type(tags: &[String]) -> String {
    // Tags are lowercased by normalize_tag(), so values like "futureEpisodes"
    // become "futureepisodes". We return the lowercased value.
    for tag in tags {
        if let Some(value) = tag.strip_prefix("scryer:monitor-type:") {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
    }
    "allepisodes".to_string()
}
/// Determine whether an individual episode should be monitored based on
/// the user's monitor type selection and the episode's air date.
///
/// NOTE: All values are lowercase because tags go through `normalize_tag`
/// which calls `.to_lowercase()`. The frontend sends camelCase values like
/// "futureEpisodes" which become "futureepisodes" after normalization.
fn should_monitor_season(
    monitor_type: &str,
    season_number: i32,
    monitor_specials: bool,
    monitor_selection: Option<&MonitorSelection>,
) -> bool {
    if monitor_type == MONITOR_TYPE_ADVANCED {
        // Everything the user did not pick is unmonitored, specials included:
        // under advanced the selection list replaces the monitor-specials rule.
        // Seasons that only appear in a later metadata refresh are absent from
        // the stored selection and are therefore created unmonitored.
        return monitor_selection.is_some_and(|selection| selection.monitors_season(season_number));
    }

    if season_number == 0 {
        return monitor_specials;
    }

    monitor_type != "none" && monitor_type != "unmonitored"
}
fn should_monitor_episode(
    monitor_type: &str,
    season_number: i32,
    air_date: Option<&str>,
    today: &str,
    monitor_specials: bool,
    monitor_selection: Option<&MonitorSelection>,
) -> bool {
    if monitor_type == MONITOR_TYPE_ADVANCED {
        // Every episode of a selected season is monitored regardless of air
        // date; filler/recap policies are applied by the caller.
        return monitor_selection.is_some_and(|selection| selection.monitors_season(season_number));
    }

    if season_number == 0 {
        return monitor_specials;
    }

    match monitor_type {
        "none" | "unmonitored" => false,
        "allepisodes" | "monitored" => true,
        "futureepisodes" => {
            // Monitor only episodes that haven't aired yet
            match air_date {
                Some(date) if !date.is_empty() => date >= today,
                _ => true, // no air date = assume future
            }
        }
        "missingandfutureepisodes" => {
            // Monitor episodes that haven't aired or are missing (not on disk).
            // At add time, no episodes are on disk yet, so all are "missing" — monitor all.
            true
        }
        _ => true,
    }
}

impl AppUseCase {
    /// The stored season/series-movie picks, but only when the title actually
    /// uses the `advanced` monitor type. Every other monitor type ignores the
    /// selection entirely.
    pub(crate) async fn load_advanced_monitor_selection(
        &self,
        title: &Title,
    ) -> AppResult<Option<MonitorSelection>> {
        if extract_monitor_type(&title.tags) != MONITOR_TYPE_ADVANCED {
            return Ok(None);
        }
        self.services
            .catalog
            .titles
            .get_title_monitor_selection(&title.id)
            .await
    }

    /// Apply an advanced-monitoring selection change to an existing title.
    /// The update-title path turns options into tags rather than a
    /// `TitleOptionsPatch`, so the selection is applied here against the tags
    /// that were just stored — including the clear that a move away from
    /// `advanced` implies.
    pub async fn set_title_monitor_selection(
        &self,
        actor: &User,
        title_id: &str,
        monitor_selection: Option<Option<MonitorSelection>>,
    ) -> AppResult<()> {
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
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;
        self.apply_title_monitor_selection_patch(
            &title,
            &TitleOptionsPatch {
                monitor_selection,
                ..TitleOptionsPatch::default()
            },
        )
        .await
    }

    /// Persist the advanced-monitoring selection carried by a title options
    /// patch. Titles that are not on the `advanced` monitor type never keep a
    /// selection, so a monitor-type change away from advanced clears it.
    pub(crate) async fn apply_title_monitor_selection_patch(
        &self,
        title: &Title,
        options_patch: &TitleOptionsPatch,
    ) -> AppResult<()> {
        if extract_monitor_type(&title.tags) != MONITOR_TYPE_ADVANCED {
            return self
                .services
                .catalog
                .titles
                .replace_title_monitor_selection(&title.id, None)
                .await;
        }
        // `None` preserves whatever is already stored.
        let Some(selection) = options_patch.monitor_selection.clone() else {
            return Ok(());
        };
        self.services
            .catalog
            .titles
            .replace_title_monitor_selection(&title.id, selection)
            .await
    }
}

fn anibridge_scoped_external_ids_from_mappings(
    anime_mappings: &[AnimeMapping],
    season_number_to_collection: &HashMap<i32, String>,
    episodes_by_number: &HashMap<(i32, i32), Episode>,
) -> (Vec<ScopedExternalId>, Vec<ScopedExternalId>) {
    let known_episodes_by_season = known_episode_numbers_by_season(episodes_by_number);
    let mut collection_ids = Vec::new();
    let mut episode_ids = Vec::new();
    let mut seen_collections = HashSet::new();
    let mut seen_episodes = HashSet::new();

    for mapping in anime_mappings {
        let external_ids = anime_mapping_external_ids(mapping);
        if external_ids.is_empty() {
            continue;
        }
        let source_scope = non_empty_scope(mapping.mapping_type.as_str());

        if mapping.episode_mappings.is_empty() {
            if let Some(season) = mapping.thetvdb_season
                && let Some(collection_id) = season_number_to_collection.get(&season)
            {
                push_scoped_external_ids(
                    &mut collection_ids,
                    &mut seen_collections,
                    collection_id,
                    &external_ids,
                    source_scope.as_deref(),
                );
            }
            continue;
        }

        let mut covered_by_season = HashMap::<i32, std::collections::BTreeSet<i32>>::new();
        for episode_mapping in &mapping.episode_mappings {
            if episode_mapping.episode_start > episode_mapping.episode_end {
                continue;
            }
            let Some(known_episode_numbers) =
                known_episodes_by_season.get(&episode_mapping.tvdb_season)
            else {
                continue;
            };
            for episode_number in known_episode_numbers
                .range(episode_mapping.episode_start..=episode_mapping.episode_end)
                .copied()
            {
                let Some(episode) =
                    episodes_by_number.get(&(episode_mapping.tvdb_season, episode_number))
                else {
                    continue;
                };
                push_scoped_external_ids(
                    &mut episode_ids,
                    &mut seen_episodes,
                    &episode.id,
                    &external_ids,
                    source_scope.as_deref(),
                );
                covered_by_season
                    .entry(episode_mapping.tvdb_season)
                    .or_default()
                    .insert(episode_number);
            }
        }

        for (season, covered) in covered_by_season {
            let Some(known) = known_episodes_by_season.get(&season) else {
                continue;
            };
            let Some(collection_id) = season_number_to_collection.get(&season) else {
                continue;
            };
            if !known.is_empty() && known.iter().all(|episode| covered.contains(episode)) {
                push_scoped_external_ids(
                    &mut collection_ids,
                    &mut seen_collections,
                    collection_id,
                    &external_ids,
                    source_scope.as_deref(),
                );
            }
        }
    }

    (collection_ids, episode_ids)
}
fn known_episode_numbers_by_season(
    episodes_by_number: &HashMap<(i32, i32), Episode>,
) -> HashMap<i32, std::collections::BTreeSet<i32>> {
    let mut known = HashMap::<i32, std::collections::BTreeSet<i32>>::new();
    for (season, episode_number) in episodes_by_number.keys().copied() {
        known.entry(season).or_default().insert(episode_number);
    }
    known
}

// Every argument is a distinct slice of the hydration pass; bundling them
// would only move the shape into a one-off struct.
#[allow(clippy::too_many_arguments)]
async fn sync_series_movie_links(
    app: &AppUseCase,
    title: &Title,
    anime_movies: &[&AnimeMovie],
    movie_metadata: &HashMap<i64, crate::MovieMetadata>,
    anime_mappings: &[AnimeMapping],
    season_last_aired: &std::collections::BTreeMap<i32, String>,
    episodes_by_number: &HashMap<(i32, i32), Episode>,
    monitor_selection: Option<&MonitorSelection>,
) {
    if anime_movies.is_empty() {
        if let Err(err) = app
            .services
            .catalog
            .shows
            .delete_stale_series_movie_links(&title.id, &[])
            .await
        {
            warn!(
                title_id = %title.id,
                error = %err,
                "failed to prune stale series movie links"
            );
        }
        return;
    }

    let mut mapping_episode_links: HashMap<String, Vec<(i32, i32)>> = HashMap::new();
    for mapping in anime_mappings {
        let identity_keys = anime_mapping_identity_keys(mapping);
        if identity_keys.is_empty() || mapping.episode_mappings.is_empty() {
            continue;
        }
        let mut linked_episodes = Vec::new();
        for episode_mapping in &mapping.episode_mappings {
            for episode_number in episode_mapping.episode_start..=episode_mapping.episode_end {
                linked_episodes.push((episode_mapping.tvdb_season, episode_number));
            }
        }
        for key in identity_keys {
            mapping_episode_links
                .entry(key)
                .or_default()
                .extend(linked_episodes.iter().copied());
        }
    }

    let mut movies_by_position: std::collections::BTreeMap<i32, Vec<&AnimeMovie>> =
        std::collections::BTreeMap::new();
    for movie in anime_movies {
        let after_season = if movie.placement == "specials" {
            0
        } else {
            anime_movie_after_season(movie, season_last_aired)
        };
        movies_by_position
            .entry(after_season)
            .or_default()
            .push(*movie);
    }

    let mut retained_link_ids = Vec::new();
    for (after_season, movies) in &mut movies_by_position {
        movies.sort_by(|left, right| {
            anime_movie_release_sort_key(left)
                .cmp(&anime_movie_release_sort_key(right))
                .then_with(|| left.name.cmp(&right.name))
        });

        for (seq, movie) in movies.iter().enumerate() {
            let narrative_order = format!("{}.{}", after_season, seq + 1);
            let linked_episode_id = anime_movie_identity_keys(movie)
                .iter()
                .filter_map(|key| mapping_episode_links.get(key.as_str()))
                .flatten()
                .find_map(|(season, episode_number)| {
                    episodes_by_number
                        .get(&(*season, *episode_number))
                        .map(|episode| episode.id.clone())
                });
            let mut movie_entity = movie_entity_from_anime_movie(movie);
            if let Some(metadata) = movie
                .movie_tvdb_id
                .and_then(|tvdb_id| movie_metadata.get(&tvdb_id))
            {
                movie_entity.ratings = Some(metadata.ratings.clone());
                movie_entity.credits = Some(metadata.credits.clone());
            }
            let link = series_movie_link_from_anime_movie(
                &title.id,
                movie,
                movie_entity,
                narrative_order,
                *after_season,
                linked_episode_id,
                title_policy_monitors_series_movie(
                    title,
                    movie.continuity_status.as_str(),
                    true,
                    monitor_selection,
                    &monitor_selection_external_ids_from_anime_movie(movie),
                ),
            );

            match app
                .services
                .catalog
                .shows
                .upsert_series_movie_link(link)
                .await
            {
                Ok(saved) => retained_link_ids.push(saved.id),
                Err(err) => {
                    warn!(
                        title_id = %title.id,
                        movie = %movie.name,
                        error = %err,
                        "failed to sync series movie link"
                    );
                }
            }
        }
    }

    if let Err(err) = app
        .services
        .catalog
        .shows
        .delete_stale_series_movie_links(&title.id, &retained_link_ids)
        .await
    {
        warn!(
            title_id = %title.id,
            error = %err,
            "failed to deactivate stale series movie links"
        );
    }

    match app
        .services
        .catalog
        .shows
        .list_series_movie_links_for_title(&title.id)
        .await
    {
        Ok(links) => {
            for link in links.into_iter().filter(|link| {
                link.source.as_deref() == Some("anibridge")
                    && !link.metadata_active
                    && link.monitoring_override != Some(true)
            }) {
                if let Err(err) = app
                    .services
                    .workflow
                    .acquisition_scope_states
                    .delete_acquisition_scope_states_for_series_movie_link(&link.id)
                    .await
                {
                    warn!(
                        title_id = %title.id,
                        series_movie_link_id = %link.id,
                        error = %err,
                        "failed to clear acquisition state for inactive series movie link"
                    );
                }
            }
        }
        Err(err) => warn!(
            title_id = %title.id,
            error = %err,
            "failed to load inactive series movie links after metadata sync"
        ),
    }
}

impl AppUseCase {
    #[cfg(test)]
    pub(crate) async fn create_series_seasons_and_episodes(
        &self,
        title: &Title,
        seasons: &[SeasonMetadata],
        episodes: &[EpisodeMetadata],
        anime_mappings: &[AnimeMapping],
        anime_movies: &[AnimeMovie],
    ) {
        self.create_series_seasons_and_episodes_with_movie_metadata(
            title,
            seasons,
            episodes,
            anime_mappings,
            anime_movies,
            &HashMap::new(),
        )
        .await;
    }

    pub(crate) async fn create_series_seasons_and_episodes_with_movie_metadata(
        &self,
        title: &Title,
        seasons: &[SeasonMetadata],
        episodes: &[EpisodeMetadata],
        anime_mappings: &[AnimeMapping],
        anime_movies: &[AnimeMovie],
        movie_metadata: &HashMap<i64, crate::MovieMetadata>,
    ) {
        let monitor_type = if title.monitored {
            extract_monitor_type(&title.tags)
        } else {
            "none".to_string()
        };
        // Loaded once per hydration pass: advanced monitoring reads the
        // title's stored selection for every season, episode and movie link.
        let monitor_selection = if monitor_type == MONITOR_TYPE_ADVANCED {
            match self
                .services
                .catalog
                .titles
                .get_title_monitor_selection(&title.id)
                .await
            {
                Ok(selection) => selection,
                Err(err) => {
                    warn!(
                        title_id = %title.id,
                        error = %err,
                        "failed to load advanced monitor selection; treating it as empty"
                    );
                    None
                }
            }
        } else {
            None
        };
        let monitor_selection = monitor_selection.as_ref();
        info!(
            title_id = %title.id,
            monitor_type = %monitor_type,
            tags = ?title.tags,
            episode_count = episodes.len(),
            "creating series seasons and episodes"
        );

        // Fetch existing collections so we can reuse them instead of creating
        // duplicates on every metadata refresh cycle.
        let existing_collections = self
            .services
            .catalog
            .shows
            .list_collections_for_title(&title.id)
            .await
            .unwrap_or_default();
        let mut existing_collections_by_id: std::collections::HashMap<String, Collection> =
            existing_collections
                .iter()
                .map(|collection| (collection.id.clone(), collection.clone()))
                .collect();
        let mut existing_collection_map: std::collections::HashMap<
            (CollectionType, String),
            String,
        > = existing_collections
            .iter()
            .map(|c| {
                (
                    (c.collection_type, c.collection_index.clone()),
                    c.id.clone(),
                )
            })
            .collect();
        if !existing_collection_map.contains_key(&(CollectionType::Specials, "0".to_string()))
            && let Some(legacy_specials_id) = existing_collections
                .iter()
                .find(|collection| is_logical_specials_collection(collection))
                .map(|collection| collection.id.clone())
        {
            existing_collection_map.insert(
                (CollectionType::Specials, "0".to_string()),
                legacy_specials_id,
            );
        }
        let mut existing_episode_lookup: std::collections::HashMap<(String, String), Episode> =
            self.services
                .catalog
                .shows
                .list_episodes_for_title(&title.id)
                .await
                .unwrap_or_default()
                .into_iter()
                .filter_map(|episode| {
                    let season_number = episode.season_number.clone()?;
                    let episode_number = episode.episode_number.clone()?;
                    Some(((season_number, episode_number), episode))
                })
                .collect();

        // Build a map from season number -> collection_id for episode assignment.
        // Only create one collection per season number, preferring "official" episode_type.
        let mut best_season_by_number: std::collections::HashMap<i32, &SeasonMetadata> =
            std::collections::HashMap::new();
        for season in seasons {
            let existing = best_season_by_number.get(&season.number);
            if existing.is_none() || season.episode_type == "official" {
                best_season_by_number.insert(season.number, season);
            }
        }

        let monitor_specials = if title.facet == MediaFacet::Anime {
            // Per-title tag overrides global setting
            if let Some(per_title) = extract_tag_bool(&title.tags, "scryer:monitor-specials:") {
                per_title
            } else {
                self.resolve_library_bool_setting(
                    "anime.monitor_specials",
                    Some(&title.library_id),
                    Some(title.facet.as_str()),
                    false,
                )
                .await
                .unwrap_or(false)
            }
        } else {
            false
        };

        let inter_season_movies = if title.facet == MediaFacet::Anime {
            if let Some(per_title) = extract_tag_bool(&title.tags, "scryer:inter-season-movies:") {
                per_title
            } else {
                self.resolve_library_bool_setting(
                    "anime.inter_season_movies",
                    Some(&title.library_id),
                    Some(title.facet.as_str()),
                    true,
                )
                .await
                .unwrap_or(true)
            }
        } else {
            false
        };

        // Regular seasons should auto-monitor on creation even before SMG has
        // episode rows. Specials still require episode data so empty season-0
        // shells do not become monitored unless they are backed by episodes.
        let seasons_with_episodes: std::collections::HashSet<i32> =
            episodes.iter().map(|ep| ep.season_number).collect();

        let derived_anime_movies: Vec<&AnimeMovie> =
            if title.facet == MediaFacet::Anime && inter_season_movies {
                anime_movies
                    .iter()
                    .filter(|movie| {
                        !movie.name.trim().is_empty()
                            && matches!(movie.association_confidence.as_str(), "medium" | "high")
                    })
                    .collect()
            } else {
                vec![]
            };

        let mut season_number_to_collection: std::collections::HashMap<i32, String> =
            std::collections::HashMap::new();

        for season in best_season_by_number.values() {
            let season_should_monitor = should_monitor_season(
                &monitor_type,
                season.number,
                monitor_specials,
                monitor_selection,
            );
            let season_monitored = if season.number == 0 {
                seasons_with_episodes.contains(&season.number) && season_should_monitor
            } else {
                season_should_monitor
            };
            let collection_type = if season.number == 0 {
                CollectionType::Specials
            } else {
                CollectionType::Season
            };
            let collection_index = season.number.to_string();
            if let Some(existing_id) =
                existing_collection_map.get(&(collection_type, collection_index.clone()))
            {
                // Update language-sensitive label if it changed
                if !season.label.is_empty()
                    && let Some(existing) = existing_collections_by_id.get(existing_id)
                    && existing.label.as_deref() != Some(&season.label)
                {
                    let _ = self
                        .services
                        .catalog
                        .shows
                        .update_collection(
                            existing_id,
                            CollectionUpdate {
                                label: Some(season.label.clone()),
                                ..Default::default()
                            },
                        )
                        .await;
                    if let Some(existing) = existing_collections_by_id.get_mut(existing_id) {
                        existing.label = Some(season.label.clone());
                    }
                }
                season_number_to_collection.insert(season.number, existing_id.clone());
                continue;
            }

            let collection = Collection {
                id: Id::new().0,
                title_id: title.id.clone(),
                collection_type,
                collection_index,
                label: Some(season.label.clone()),
                ordered_path: None,
                narrative_order: Some(season.number.to_string()),
                first_episode_number: None,
                last_episode_number: None,
                monitored: season_monitored,
                created_at: Utc::now(),
            };

            match self
                .services
                .catalog
                .shows
                .create_collection(collection.clone())
                .await
            {
                Ok(created) => {
                    existing_collections_by_id.insert(created.id.clone(), created.clone());
                    season_number_to_collection.insert(season.number, created.id);
                }
                Err(err) => {
                    warn!(
                        title_id = %title.id,
                        season_number = season.number,
                        error = %err,
                        "failed to create season collection"
                    );
                }
            }
        }

        // Build last-aired date per regular season from the episode data so
        // we can determine where each series movie falls narratively.
        let mut season_last_aired: std::collections::BTreeMap<i32, String> =
            std::collections::BTreeMap::new();
        for ep in episodes.iter() {
            if ep.season_number > 0 && !ep.aired.is_empty() {
                season_last_aired
                    .entry(ep.season_number)
                    .and_modify(|d| {
                        if ep.aired > *d {
                            *d = ep.aired.clone();
                        }
                    })
                    .or_insert_with(|| ep.aired.clone());
            }
        }

        // Build a lookup from season number → season episode_type for deriving episode type.
        let season_episode_types: std::collections::HashMap<i32, &str> = best_season_by_number
            .iter()
            .map(|(&num, s)| (num, s.episode_type.as_str()))
            .collect();

        let today = Utc::now().format("%Y-%m-%d").to_string();

        let skip_filler = if title.facet == MediaFacet::Anime {
            let effective = match extract_tag_string(&title.tags, "scryer:filler-policy:") {
                Some(v) => v.to_string(),
                None => self
                    .resolve_library_string_setting(
                        "anime.filler_policy",
                        Some(&title.library_id),
                        Some(title.facet.as_str()),
                        "download_all",
                    )
                    .await
                    .unwrap_or_else(|_| "download_all".to_string()),
            };
            effective == "skip_filler"
        } else {
            false
        };
        let skip_recap = if title.facet == MediaFacet::Anime {
            let effective = match extract_tag_string(&title.tags, "scryer:recap-policy:") {
                Some(v) => v.to_string(),
                None => self
                    .resolve_library_string_setting(
                        "anime.recap_policy",
                        Some(&title.library_id),
                        Some(title.facet.as_str()),
                        "download_all",
                    )
                    .await
                    .unwrap_or_else(|_| "download_all".to_string()),
            };
            effective == "skip_recap"
        } else {
            false
        };

        for ep in episodes {
            let season_number_key = ep.season_number.to_string();
            let episode_number_key = ep.episode_number.to_string();
            let collection_id = season_number_to_collection.get(&ep.season_number).cloned();

            let air_date = if ep.aired.is_empty() {
                None
            } else {
                Some(ep.aired.clone())
            };
            let episode_monitored = if (skip_filler && ep.is_filler) || (skip_recap && ep.is_recap)
            {
                false
            } else {
                should_monitor_episode(
                    &monitor_type,
                    ep.season_number,
                    air_date.as_deref(),
                    &today,
                    monitor_specials,
                    monitor_selection,
                )
            };

            let anime_media_type = if title.facet == MediaFacet::Anime {
                anime_mappings
                    .iter()
                    .find(|m| m.thetvdb_season == Some(ep.season_number))
                    .map(|m| m.anime_media_type.as_str())
            } else {
                None
            };

            let episode_type = derive_episode_type(
                ep.season_number,
                season_episode_types.get(&ep.season_number).copied(),
                anime_media_type,
            );

            // If episode already exists, update language-sensitive fields instead of skipping.
            if let Some(existing) = existing_episode_lookup
                .get(&(season_number_key.clone(), episode_number_key.clone()))
                .cloned()
            {
                let new_title = if ep.name.is_empty() {
                    None
                } else {
                    Some(ep.name.clone())
                };
                let new_overview = if ep.overview.trim().is_empty() {
                    None
                } else {
                    Some(ep.overview.clone())
                };
                // Only update if the new data differs from existing
                let title_changed = new_title.as_deref() != existing.title.as_deref();
                let overview_changed = new_overview.as_deref() != existing.overview.as_deref();
                let new_tvdb_id = if ep.tvdb_id > 0 {
                    Some(ep.tvdb_id.to_string())
                } else {
                    None
                };
                let new_image_url = normalize_episode_image_url(&ep.image_url);
                let tvdb_id_changed = new_tvdb_id.as_deref() != existing.tvdb_id.as_deref();
                let image_url_changed = new_image_url.as_deref() != existing.image_url.as_deref();
                if title_changed || overview_changed || tvdb_id_changed || image_url_changed {
                    let _ = self
                        .services
                        .catalog
                        .shows
                        .update_episode(
                            &existing.id,
                            EpisodeUpdate {
                                episode_label: if title_changed {
                                    new_title.clone()
                                } else {
                                    None
                                },
                                title: if title_changed { new_title } else { None },
                                overview: if overview_changed { new_overview } else { None },
                                tvdb_id: if tvdb_id_changed { new_tvdb_id } else { None },
                                image_url: if image_url_changed {
                                    new_image_url.clone()
                                } else {
                                    None
                                },
                                clear_image_url: image_url_changed && new_image_url.is_none(),
                                ..Default::default()
                            },
                        )
                        .await;
                }
                continue;
            }

            let episode = Episode {
                id: Id::new().0,
                title_id: title.id.clone(),
                collection_id,
                episode_type,
                episode_number: Some(episode_number_key.clone()),
                season_number: Some(season_number_key.clone()),
                episode_label: Some(ep.name.clone()),
                title: Some(ep.name.clone()),
                air_date,
                duration_seconds: if ep.runtime_minutes > 0 {
                    Some(i64::from(ep.runtime_minutes) * 60)
                } else {
                    None
                },
                has_multi_audio: false,
                has_subtitle: false,
                is_filler: ep.is_filler,
                is_recap: ep.is_recap,
                absolute_number: if ep.absolute_number.is_empty() {
                    None
                } else {
                    Some(ep.absolute_number.clone())
                },
                overview: if ep.overview.trim().is_empty() {
                    None
                } else {
                    Some(ep.overview.clone())
                },
                tvdb_id: if ep.tvdb_id > 0 {
                    Some(ep.tvdb_id.to_string())
                } else {
                    None
                },
                image_url: normalize_episode_image_url(&ep.image_url),
                monitored: episode_monitored,
                created_at: Utc::now(),
            };

            match self.services.catalog.shows.create_episode(episode).await {
                Ok(created) => {
                    existing_episode_lookup
                        .insert((season_number_key, episode_number_key), created);
                }
                Err(err) => {
                    warn!(
                        title_id = %title.id,
                        episode_number = ep.episode_number,
                        error = %err,
                        "failed to create episode"
                    );
                }
            }
        }

        if title.facet == MediaFacet::Anime {
            let episode_lookup_by_number: HashMap<(i32, i32), Episode> = existing_episode_lookup
                .values()
                .filter_map(|episode| {
                    let season = episode.season_number.as_deref()?.parse::<i32>().ok()?;
                    let episode_number = episode.episode_number.as_deref()?.parse::<i32>().ok()?;
                    Some(((season, episode_number), episode.clone()))
                })
                .collect();

            if inter_season_movies {
                sync_series_movie_links(
                    self,
                    title,
                    &derived_anime_movies,
                    movie_metadata,
                    anime_mappings,
                    &season_last_aired,
                    &episode_lookup_by_number,
                    monitor_selection,
                )
                .await;
            }

            let (collection_external_ids, episode_external_ids) =
                anibridge_scoped_external_ids_from_mappings(
                    anime_mappings,
                    &season_number_to_collection,
                    &episode_lookup_by_number,
                );
            if let Err(err) = self
                .services
                .catalog
                .shows
                .replace_anibridge_scoped_external_ids_for_title(
                    &title.id,
                    collection_external_ids,
                    episode_external_ids,
                )
                .await
            {
                warn!(
                    title_id = %title.id,
                    error = %err,
                    "failed to persist scoped anibridge external IDs"
                );
            }
        }
    }
}
impl AppUseCase {
    pub async fn list_primary_collection_summaries(
        &self,
        actor: &User,
        title_ids: &[String],
    ) -> AppResult<Vec<PrimaryCollectionSummary>> {
        let title_ids = self
            .filter_title_ids_for_permission(
                actor,
                title_ids,
                scryer_domain::LibraryPermission::View,
            )
            .await?;
        self.services
            .catalog
            .shows
            .list_primary_collection_summaries(&title_ids)
            .await
    }
}
impl AppUseCase {
    pub async fn list_title_media_size_summaries(
        &self,
        actor: &User,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleMediaSizeSummary>> {
        let title_ids = self
            .filter_title_ids_for_permission(
                actor,
                title_ids,
                scryer_domain::LibraryPermission::View,
            )
            .await?;
        self.services
            .library
            .media_files
            .list_title_media_size_summaries(&title_ids)
            .await
    }

    /// Byte size of the media file backing a single collection, keyed by the
    /// collection's `ordered_path`. Returns `None` when the actor cannot `View`
    /// the title or when nothing is indexed at that path.
    pub async fn collection_media_size_bytes(
        &self,
        actor: &User,
        title_id: &str,
        ordered_path: &str,
    ) -> AppResult<Option<i64>> {
        let title_ids = [title_id.to_string()];
        let allowed = self
            .filter_title_ids_for_permission(
                actor,
                &title_ids,
                scryer_domain::LibraryPermission::View,
            )
            .await?;
        if allowed.is_empty() {
            return Ok(None);
        }
        self.services
            .library
            .media_files
            .collection_media_size_bytes(title_id, ordered_path)
            .await
    }
}
impl AppUseCase {
    /// Batch-load media files for many titles, keyed by `title_id`. Titles the
    /// actor cannot `View` are silently dropped (their key is simply absent).
    pub async fn list_media_files_for_titles(
        &self,
        actor: &User,
        title_ids: &[String],
    ) -> AppResult<HashMap<String, Vec<crate::TitleMediaFile>>> {
        let title_ids = self
            .filter_title_ids_for_permission(
                actor,
                title_ids,
                scryer_domain::LibraryPermission::View,
            )
            .await?;
        let mut grouped: HashMap<String, Vec<crate::TitleMediaFile>> = HashMap::new();
        for file in self
            .services
            .library
            .media_files
            .list_media_files_for_titles(&title_ids)
            .await?
        {
            grouped.entry(file.title_id.clone()).or_default().push(file);
        }
        Ok(grouped)
    }
}
impl AppUseCase {
    pub async fn list_title_quality_summaries(
        &self,
        actor: &User,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleQualitySummary>> {
        let title_ids = self
            .filter_title_ids_for_permission(
                actor,
                title_ids,
                scryer_domain::LibraryPermission::View,
            )
            .await?;
        self.services
            .library
            .media_files
            .list_title_quality_summaries(&title_ids)
            .await
    }

    pub async fn list_title_movie_media_summaries(
        &self,
        actor: &User,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleMovieMediaSummary>> {
        let title_ids = self
            .filter_title_ids_for_permission(
                actor,
                title_ids,
                scryer_domain::LibraryPermission::View,
            )
            .await?;
        self.services
            .library
            .media_files
            .list_title_movie_media_summaries(&title_ids)
            .await
    }
}
impl AppUseCase {
    pub async fn list_title_episode_progress_summaries(
        &self,
        actor: &User,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleEpisodeProgressSummary>> {
        let title_ids = self
            .filter_title_ids_for_permission(
                actor,
                title_ids,
                scryer_domain::LibraryPermission::View,
            )
            .await?;
        self.services
            .library
            .media_files
            .list_title_episode_progress_summaries(&title_ids)
            .await
    }

    pub async fn list_episode_media_availability(
        &self,
        actor: &User,
        title_ids: &[String],
    ) -> AppResult<Vec<crate::types::EpisodeMediaAvailability>> {
        let title_ids = self
            .filter_title_ids_for_permission(
                actor,
                title_ids,
                scryer_domain::LibraryPermission::View,
            )
            .await?;
        self.services
            .library
            .media_files
            .list_episode_media_availability(&title_ids)
            .await
    }

    pub async fn list_collection_episode_progress_summaries(
        &self,
        actor: &User,
        title_ids: &[String],
    ) -> AppResult<Vec<CollectionEpisodeProgressSummary>> {
        let title_ids = self
            .filter_title_ids_for_permission(
                actor,
                title_ids,
                scryer_domain::LibraryPermission::View,
            )
            .await?;
        self.services
            .library
            .media_files
            .list_collection_episode_progress_summaries(&title_ids)
            .await
    }
}
impl AppUseCase {
    pub async fn list_collections(
        &self,
        actor: &User,
        title_id: &str,
    ) -> AppResult<Vec<Collection>> {
        self.require_title_permission(actor, title_id, scryer_domain::LibraryPermission::View)
            .await?;
        self.services
            .catalog
            .shows
            .list_collections_for_title(title_id)
            .await
    }

    pub async fn list_collections_for_titles(
        &self,
        actor: &User,
        titles: &[Title],
    ) -> AppResult<HashMap<String, Vec<Collection>>> {
        if titles.is_empty() {
            return Ok(HashMap::new());
        }

        let mut library_ids = HashSet::new();
        for title in titles {
            if library_ids.insert(title.library_id.as_str()) {
                self.require_library_permission(
                    actor,
                    &title.library_id,
                    scryer_domain::LibraryPermission::View,
                )
                .await?;
            }
        }

        let title_ids = titles
            .iter()
            .map(|title| title.id.clone())
            .collect::<Vec<_>>();
        self.services
            .catalog
            .shows
            .list_collections_for_titles(&title_ids)
            .await
    }

    pub async fn list_series_movie_links(
        &self,
        actor: &User,
        title_id: &str,
    ) -> AppResult<Vec<scryer_domain::SeriesMovieLink>> {
        self.require_title_permission(actor, title_id, scryer_domain::LibraryPermission::View)
            .await?;
        self.services
            .catalog
            .shows
            .list_series_movie_links_for_title(title_id)
            .await
    }

    pub async fn get_movie_entity(
        &self,
        actor: &User,
        title_id: &str,
        movie_entity_id: &str,
    ) -> AppResult<Option<scryer_domain::MovieEntity>> {
        Ok(self
            .list_series_movie_links(actor, title_id)
            .await?
            .into_iter()
            .find(|link| link.movie.id == movie_entity_id)
            .map(|link| link.movie))
    }

    pub async fn movie_entity_credits(
        &self,
        actor: &User,
        title_id: &str,
        movie_entity_id: &str,
    ) -> AppResult<Vec<crate::TitleCredit>> {
        self.get_movie_entity(actor, title_id, movie_entity_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("movie entity {movie_entity_id}")))?;
        self.services
            .catalog
            .shows
            .list_movie_entity_credits(movie_entity_id)
            .await
    }

    /// Batch variant of [`Self::list_series_movie_links`], keyed by
    /// `series_title_id`. Titles the actor cannot `View` are silently dropped.
    pub async fn list_series_movie_links_for_titles(
        &self,
        actor: &User,
        title_ids: &[String],
    ) -> AppResult<HashMap<String, Vec<scryer_domain::SeriesMovieLink>>> {
        let title_ids = self
            .filter_title_ids_for_permission(
                actor,
                title_ids,
                scryer_domain::LibraryPermission::View,
            )
            .await?;
        let mut grouped: HashMap<String, Vec<scryer_domain::SeriesMovieLink>> = HashMap::new();
        for link in self
            .services
            .catalog
            .shows
            .list_series_movie_links_for_titles(&title_ids)
            .await?
        {
            grouped
                .entry(link.series_title_id.clone())
                .or_default()
                .push(link);
        }
        Ok(grouped)
    }
}
impl AppUseCase {
    /// Batch-load collections by id, dropping any whose owning title the actor
    /// cannot `View`. Missing/forbidden ids are simply absent from the result.
    pub async fn get_collections_by_ids(
        &self,
        actor: &User,
        ids: &[String],
    ) -> AppResult<Vec<Collection>> {
        let collections = self
            .services
            .catalog
            .shows
            .get_collections_by_ids(ids)
            .await?;
        let title_ids = collections
            .iter()
            .map(|collection| collection.title_id.clone())
            .collect::<Vec<_>>();
        let visible_titles = self
            .get_titles_by_ids(actor, &title_ids)
            .await?
            .into_iter()
            .map(|title| title.id)
            .collect::<HashSet<_>>();
        Ok(collections
            .into_iter()
            .filter(|collection| visible_titles.contains(&collection.title_id))
            .collect())
    }

    /// Batch-load episodes for many collections, keyed by `collection_id`.
    /// Collections whose title the actor cannot `View` are silently dropped.
    pub async fn list_episodes_for_collections(
        &self,
        actor: &User,
        collection_ids: &[String],
    ) -> AppResult<HashMap<String, Vec<Episode>>> {
        let visible_collection_ids = self
            .get_collections_by_ids(actor, collection_ids)
            .await?
            .into_iter()
            .map(|collection| collection.id)
            .collect::<Vec<_>>();
        if visible_collection_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let mut grouped: HashMap<String, Vec<Episode>> = HashMap::new();
        for episode in self
            .services
            .catalog
            .shows
            .list_episodes_for_collections(&visible_collection_ids)
            .await?
        {
            if let Some(collection_id) = episode.collection_id.clone() {
                grouped.entry(collection_id).or_default().push(episode);
            }
        }
        Ok(grouped)
    }
}
impl AppUseCase {
    pub async fn get_collection(
        &self,
        actor: &User,
        collection_id: &str,
    ) -> AppResult<Option<Collection>> {
        let collection = self
            .services
            .catalog
            .shows
            .get_collection_by_id(collection_id)
            .await?;
        if let Some(collection) = collection.as_ref() {
            self.require_title_permission(
                actor,
                &collection.title_id,
                scryer_domain::LibraryPermission::View,
            )
            .await?;
        }
        Ok(collection)
    }
}
impl AppUseCase {
    #[expect(
        clippy::too_many_arguments,
        reason = "collection creation mirrors the editable collection fields at the application boundary"
    )]
    pub async fn create_collection(
        &self,
        actor: &User,
        title_id: String,
        collection_type: String,
        collection_index: String,
        label: Option<String>,
        ordered_path: Option<String>,
        first_episode_number: Option<String>,
        last_episode_number: Option<String>,
    ) -> AppResult<Collection> {
        self.require_title_permission(
            actor,
            &title_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;

        if collection_type.trim().is_empty() {
            return Err(AppError::Validation("collection type is required".into()));
        }
        let parsed_type = CollectionType::parse(collection_type.trim().to_lowercase().as_str())
            .ok_or_else(|| {
                AppError::Validation(format!("unknown collection type: {}", collection_type))
            })?;
        if collection_index.trim().is_empty() {
            return Err(AppError::Validation("collection index is required".into()));
        }
        let collection = Collection {
            id: Id::new().0,
            title_id,
            collection_type: parsed_type,
            collection_index: collection_index.trim().to_string(),
            label: normalize_show_text_opt(label),
            ordered_path: normalize_show_text_opt(ordered_path),
            narrative_order: None,
            first_episode_number: normalize_show_text_opt(first_episode_number),
            last_episode_number: normalize_show_text_opt(last_episode_number),
            monitored: true,
            created_at: Utc::now(),
        };

        let collection = self
            .services
            .catalog
            .shows
            .create_collection(collection)
            .await?;
        Ok(collection)
    }
}
impl AppUseCase {
    #[expect(
        clippy::too_many_arguments,
        reason = "episode creation mirrors the full editable episode form at the application boundary"
    )]
    pub async fn create_episode(
        &self,
        actor: &User,
        title_id: String,
        collection_id: Option<String>,
        episode_type: String,
        episode_number: Option<String>,
        season_number: Option<String>,
        episode_label: Option<String>,
        title: Option<String>,
        air_date: Option<String>,
        duration_seconds: Option<i64>,
        has_multi_audio: bool,
        has_subtitle: bool,
    ) -> AppResult<Episode> {
        self.require_title_permission(
            actor,
            &title_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;

        if episode_type.trim().is_empty() {
            return Err(AppError::Validation("episode type is required".into()));
        }

        let parsed_episode_type =
            scryer_domain::EpisodeType::parse(episode_type.trim().to_lowercase().as_str())
                .ok_or_else(|| {
                    AppError::Validation(format!("unknown episode type: {}", episode_type))
                })?;
        let episode = Episode {
            id: Id::new().0,
            title_id,
            collection_id,
            episode_type: parsed_episode_type,
            episode_number: normalize_show_text_opt(episode_number),
            season_number: normalize_show_text_opt(season_number),
            episode_label: normalize_show_text_opt(episode_label),
            title: normalize_show_text_opt(title),
            air_date: normalize_show_text_opt(air_date),
            duration_seconds,
            has_multi_audio,
            has_subtitle,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            image_url: None,
            monitored: true,
            created_at: Utc::now(),
        };

        let episode = self.services.catalog.shows.create_episode(episode).await?;
        Ok(episode)
    }
}
impl AppUseCase {
    pub async fn list_episodes(
        &self,
        actor: &User,
        collection_id: &str,
    ) -> AppResult<Vec<Episode>> {
        self.require_collection_permission(
            actor,
            collection_id,
            scryer_domain::LibraryPermission::View,
        )
        .await?;
        self.services
            .catalog
            .shows
            .list_episodes_for_collection(collection_id)
            .await
    }
}
impl AppUseCase {
    pub async fn get_episode(&self, actor: &User, episode_id: &str) -> AppResult<Option<Episode>> {
        let episode = self
            .services
            .catalog
            .shows
            .get_episode_by_id(episode_id)
            .await?;
        if let Some(episode) = episode.as_ref() {
            self.require_title_permission(
                actor,
                &episode.title_id,
                scryer_domain::LibraryPermission::View,
            )
            .await?;
        }
        Ok(episode)
    }

    /// Batch variant of [`Self::get_episode`]. Loads episodes by id in one query
    /// and silently drops those whose title the actor cannot `View`.
    pub async fn get_episodes_by_ids(
        &self,
        actor: &User,
        ids: &[String],
    ) -> AppResult<Vec<Episode>> {
        let episodes = self.services.catalog.shows.get_episodes_by_ids(ids).await?;
        let title_ids = episodes
            .iter()
            .map(|episode| episode.title_id.clone())
            .collect::<Vec<_>>();
        let visible_titles = self
            .get_titles_by_ids(actor, &title_ids)
            .await?
            .into_iter()
            .map(|title| title.id)
            .collect::<HashSet<_>>();
        Ok(episodes
            .into_iter()
            .filter(|episode| visible_titles.contains(&episode.title_id))
            .collect())
    }
}
impl AppUseCase {
    pub async fn list_calendar_episodes(
        &self,
        actor: &User,
        start_date: &str,
        end_date: &str,
        library_ids: Option<Vec<String>>,
    ) -> AppResult<Vec<CalendarEpisode>> {
        let authorized = self
            .authorized_library_ids(actor, None, scryer_domain::LibraryPermission::View)
            .await?
            .into_iter()
            .collect::<HashSet<_>>();
        let requested_library_ids = library_ids
            .unwrap_or_default()
            .into_iter()
            .map(|library_id| library_id.trim().to_string())
            .filter(|library_id| !library_id.is_empty())
            .collect::<HashSet<_>>();
        let visible_library_ids = if requested_library_ids.is_empty() {
            authorized
        } else {
            authorized
                .intersection(&requested_library_ids)
                .cloned()
                .collect::<HashSet<_>>()
        };
        let episodes = self
            .services
            .catalog
            .shows
            .list_episodes_in_date_range(start_date, end_date)
            .await?;
        Ok(episodes
            .into_iter()
            .filter(|episode| {
                visible_library_ids.contains(&episode.library_id)
                    && calendar_episode_is_visible(
                        episode.season_number.as_deref(),
                        episode.monitored,
                    )
            })
            .collect())
    }
}

fn calendar_episode_is_visible(season_number: Option<&str>, monitored: bool) -> bool {
    monitored || season_number.and_then(|value| value.trim().parse::<i32>().ok()) != Some(0)
}

#[cfg(test)]
mod calendar_episode_visibility_tests {
    use super::calendar_episode_is_visible;

    #[test]
    fn hides_unmonitored_season_zero_episodes() {
        assert!(!calendar_episode_is_visible(Some("0"), false));
        assert!(!calendar_episode_is_visible(Some("00"), false));
    }

    #[test]
    fn keeps_monitored_season_zero_and_regular_episodes() {
        assert!(calendar_episode_is_visible(Some("0"), true));
        assert!(calendar_episode_is_visible(Some("1"), false));
        assert!(calendar_episode_is_visible(None, false));
    }
}

fn normalize_episode_image_url(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let parsed = url::Url::parse(trimmed).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    if parsed.host_str().is_none_or(|host| host.trim().is_empty()) {
        return None;
    }

    Some(parsed.to_string())
}
/// Derive the episode type from the season number, season episode_type, and anime media type.
fn derive_episode_type(
    season_number: i32,
    season_episode_type: Option<&str>,
    anime_media_type: Option<&str>,
) -> scryer_domain::EpisodeType {
    use scryer_domain::EpisodeType;
    if season_number == 0 {
        return match anime_media_type {
            Some("OVA") => EpisodeType::Ova,
            Some("ONA") => EpisodeType::Ona,
            _ => EpisodeType::Special,
        };
    }
    match season_episode_type {
        Some("alternate") => EpisodeType::Alternate,
        _ => EpisodeType::Standard,
    }
}

use super::*;

pub(super) struct MockLibraryRepo {
    pub(super) libraries: Arc<Mutex<Vec<Library>>>,
    pub(super) app_permissions: Arc<Mutex<HashMap<String, AppPermissionMask>>>,
    pub(super) grants: Arc<Mutex<HashMap<String, Vec<LibraryGrant>>>>,
}

impl MockLibraryRepo {
    pub(super) fn with_libraries(libraries: Vec<Library>) -> Self {
        Self {
            libraries: Arc::new(Mutex::new(libraries)),
            app_permissions: Arc::new(Mutex::new(HashMap::new())),
            grants: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub(super) fn empty() -> Self {
        Self::with_libraries(Vec::new())
    }
}

impl Default for MockLibraryRepo {
    fn default() -> Self {
        Self::with_libraries(
            [MediaFacet::Movie, MediaFacet::Series, MediaFacet::Anime]
                .into_iter()
                .map(mock_default_library)
                .collect(),
        )
    }
}

pub(super) fn mock_default_library(facet: MediaFacet) -> Library {
    let now = Utc::now();
    let path = match facet {
        MediaFacet::Movie => "/data/movies",
        MediaFacet::Series => "/data/series",
        MediaFacet::Anime => "/data/anime",
    }
    .to_string();
    let library_id = scryer_domain::default_library_id_for_facet(&facet);
    Library {
        id: library_id.clone(),
        facet: facet.clone(),
        name: format!("Default {}", facet.as_str()),
        slug: scryer_domain::default_library_slug_for_facet(&facet).to_string(),
        is_default: true,
        roots: vec![scryer_domain::LibraryRoot {
            id: scryer_domain::root_folder_id_for_path(&path),
            library_id,
            path,
            is_default: true,
            created_at: now,
            updated_at: now,
        }],
        created_at: now,
        updated_at: now,
    }
}

pub(super) fn mock_library_roots(
    library_id: &str,
    roots: Vec<LibraryRootDraft>,
) -> Vec<scryer_domain::LibraryRoot> {
    let now = Utc::now();
    roots
        .into_iter()
        .map(|root| scryer_domain::LibraryRoot {
            id: Id::new().0,
            library_id: library_id.to_string(),
            path: root.path,
            is_default: root.is_default,
            created_at: now,
            updated_at: now,
        })
        .collect()
}

#[async_trait]
impl LibraryRepository for MockLibraryRepo {
    async fn list(&self, facet: Option<MediaFacet>) -> AppResult<Vec<Library>> {
        Ok(self
            .libraries
            .lock()
            .await
            .iter()
            .filter(|library| facet.as_ref().is_none_or(|facet| &library.facet == facet))
            .cloned()
            .collect())
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<Library>> {
        Ok(self
            .libraries
            .lock()
            .await
            .iter()
            .find(|library| library.id == id)
            .cloned())
    }

    async fn default_for_facet(&self, facet: MediaFacet) -> AppResult<Option<Library>> {
        Ok(self
            .libraries
            .lock()
            .await
            .iter()
            .find(|library| library.facet == facet && library.is_default)
            .cloned())
    }

    async fn create(
        &self,
        mut library: Library,
        roots: Vec<LibraryRootDraft>,
    ) -> AppResult<Library> {
        library.roots = mock_library_roots(&library.id, roots);
        self.libraries.lock().await.push(library.clone());
        Ok(library)
    }

    /// Replace-on-write of a library's root list, with the real store's
    /// identity semantics.
    ///
    /// `update_library_tx` reads the stored root ids **before** it rewrites the
    /// rows and re-keys them by normalized path, so a root whose path is
    /// resubmitted lands back on the id it already had; only a genuinely new
    /// path is allocated one (FR-078, `existing_root_ids_by_normalized_path_tx`).
    /// A mock that allocated a fresh id for every submitted root would make
    /// every `update` look like "delete every root, create new ones" — exactly
    /// the identity change synthetic root ids exist to prevent, and exactly the
    /// bug a story test over a consolidation (US5) has to be able to catch.
    ///
    /// The real store's second guard — refusing to remove a root any title still
    /// references — is not reproduced here, because this double holds no titles.
    /// The consolidation tail asks that question itself, before it calls
    /// `update`, so the story tests still exercise the rule.
    async fn update(
        &self,
        library_id: &str,
        name: String,
        slug: String,
        roots: Vec<LibraryRootDraft>,
    ) -> AppResult<Library> {
        let mut libraries = self.libraries.lock().await;
        let library = libraries
            .iter_mut()
            .find(|library| library.id == library_id)
            .ok_or_else(|| AppError::NotFound(format!("library {library_id}")))?;
        let existing_root_ids: HashMap<String, String> = library
            .roots
            .iter()
            .map(|root| {
                (
                    scryer_domain::normalize_library_root_path(&root.path),
                    root.id.clone(),
                )
            })
            .collect();
        let now = Utc::now();
        library.name = name;
        library.slug = slug;
        library.roots = roots
            .into_iter()
            .map(|root| {
                let id = existing_root_ids
                    .get(&scryer_domain::normalize_library_root_path(&root.path))
                    .cloned()
                    .unwrap_or_else(|| Id::new().0);
                scryer_domain::LibraryRoot {
                    id,
                    library_id: library_id.to_string(),
                    path: root.path,
                    is_default: root.is_default,
                    created_at: now,
                    updated_at: now,
                }
            })
            .collect();
        library.updated_at = now;
        Ok(library.clone())
    }

    /// The in-place path flip a root change performs (FR-021): the row keeps
    /// its id, its library, and its default flag, and only `path` moves. The
    /// real store's behaviour, which is the whole reason this is its own port
    /// method rather than an `update` with a rewritten root list.
    async fn set_root_path(&self, root_id: &str, path: &str) -> AppResult<Library> {
        let mut libraries = self.libraries.lock().await;
        if libraries.iter().any(|library| {
            library
                .roots
                .iter()
                .any(|root| root.id != root_id && root.path == path)
        }) {
            return Err(AppError::Validation(format!(
                "library root '{path}' is already configured"
            )));
        }
        let now = Utc::now();
        for library in libraries.iter_mut() {
            if let Some(root) = library.roots.iter_mut().find(|root| root.id == root_id) {
                root.path = path.to_string();
                root.updated_at = now;
                library.updated_at = now;
                return Ok(library.clone());
            }
        }
        Err(AppError::NotFound(format!("library root {root_id}")))
    }

    async fn delete_library(&self, library_id: &str) -> AppResult<bool> {
        let mut libraries = self.libraries.lock().await;
        let before = libraries.len();
        libraries.retain(|library| library.id != library_id || library.is_default);
        let deleted = libraries.len() != before;
        drop(libraries);
        if deleted {
            let mut grants = self.grants.lock().await;
            for user_grants in grants.values_mut() {
                user_grants.retain(|grant| grant.library_id != library_id);
            }
        }
        Ok(deleted)
    }

    async fn app_permission_mask_for_user(&self, user_id: &str) -> AppResult<AppPermissionMask> {
        Ok(self
            .app_permissions
            .lock()
            .await
            .get(user_id)
            .copied()
            .unwrap_or(AppPermissionMask::NONE))
    }

    async fn set_app_permission_mask_for_user(
        &self,
        user_id: &str,
        permissions: AppPermissionMask,
    ) -> AppResult<()> {
        self.app_permissions
            .lock()
            .await
            .insert(user_id.to_string(), permissions);
        Ok(())
    }

    async fn permission_masks_for_user(&self, user_id: &str) -> AppResult<Vec<LibraryGrant>> {
        Ok(self
            .grants
            .lock()
            .await
            .get(user_id)
            .cloned()
            .unwrap_or_default())
    }

    async fn set_grants_for_user(
        &self,
        user_id: &str,
        mut grants: Vec<LibraryGrant>,
    ) -> AppResult<()> {
        for grant in &mut grants {
            grant.user_id = user_id.to_string();
        }
        self.grants.lock().await.insert(user_id.to_string(), grants);
        Ok(())
    }

    async fn title_library_id(&self, _title_id: &str) -> AppResult<Option<String>> {
        Ok(Some(scryer_domain::default_library_id_for_facet(
            &MediaFacet::Movie,
        )))
    }
}

#[derive(Default)]
pub(super) struct MockShowRepo {
    pub(super) collections: Arc<Mutex<Vec<Collection>>>,
    pub(super) episodes: Arc<Mutex<Vec<Episode>>>,
    pub(super) series_movie_links: Arc<Mutex<Vec<scryer_domain::SeriesMovieLink>>>,
    pub(super) collection_external_ids: Arc<Mutex<Vec<ScopedExternalId>>>,
    pub(super) episode_external_ids: Arc<Mutex<Vec<ScopedExternalId>>>,
}

#[async_trait]
impl ShowRepository for MockShowRepo {
    async fn list_series_movie_links_for_title(
        &self,
        title_id: &str,
    ) -> AppResult<Vec<scryer_domain::SeriesMovieLink>> {
        let links = self.series_movie_links.lock().await;
        Ok(links
            .iter()
            .filter(|item| item.series_title_id == title_id)
            .cloned()
            .collect())
    }

    async fn list_series_movie_external_id_lookup_matches(
        &self,
        library_ids: &[String],
        lookups: &[TitleExternalIdLookup],
    ) -> AppResult<Vec<SeriesMovieExternalIdLookupMatch>> {
        if library_ids.is_empty() {
            return Ok(Vec::new());
        }

        let links = self.series_movie_links.lock().await;
        Ok(lookups
            .iter()
            .filter(|lookup| {
                let external_id = lookup.external_id.trim();
                !external_id.is_empty()
                    && links.iter().any(|link| {
                        let value = match lookup.source.trim().to_ascii_lowercase().as_str() {
                            "imdb" => link.movie.imdb_id.as_deref(),
                            "tvdb" | "tvdb_movie" => link.movie.tvdb_id.as_deref(),
                            "tmdb" | "tmdb_movie" => link.movie.tmdb_id.as_deref(),
                            "mal" | "myanimelist" => link.movie.mal_id.as_deref(),
                            "anidb" => link.movie.anidb_id.as_deref(),
                            _ => None,
                        };
                        value == Some(external_id)
                    })
            })
            .map(|lookup| SeriesMovieExternalIdLookupMatch {
                lookup_index: lookup.lookup_index,
            })
            .collect())
    }

    async fn get_series_movie_link_by_id(
        &self,
        link_id: &str,
    ) -> AppResult<Option<scryer_domain::SeriesMovieLink>> {
        let links = self.series_movie_links.lock().await;
        Ok(links.iter().find(|item| item.id == link_id).cloned())
    }

    async fn find_series_movie_link_by_legacy_collection_id(
        &self,
        collection_id: &str,
    ) -> AppResult<Option<scryer_domain::SeriesMovieLink>> {
        let links = self.series_movie_links.lock().await;
        Ok(links
            .iter()
            .find(|item| item.legacy_collection_id.as_deref() == Some(collection_id))
            .cloned())
    }

    async fn upsert_series_movie_link(
        &self,
        link: scryer_domain::SeriesMovieLink,
    ) -> AppResult<scryer_domain::SeriesMovieLink> {
        let mut links = self.series_movie_links.lock().await;
        if let Some(existing) = links.iter_mut().find(|item| item.id == link.id) {
            *existing = link.clone();
        } else {
            links.push(link.clone());
        }
        Ok(link)
    }

    async fn delete_stale_series_movie_links(
        &self,
        title_id: &str,
        retained_link_ids: &[String],
    ) -> AppResult<()> {
        let retained = retained_link_ids.iter().cloned().collect::<HashSet<_>>();
        self.series_movie_links
            .lock()
            .await
            .retain(|item| item.series_title_id != title_id || retained.contains(&item.id));
        Ok(())
    }

    async fn update_series_movie_link_user_tags(
        &self,
        link_id: &str,
        add: &[String],
        remove: &[String],
    ) -> AppResult<scryer_domain::SeriesMovieLink> {
        let mut links = self.series_movie_links.lock().await;
        let link = links
            .iter_mut()
            .find(|item| item.id == link_id)
            .ok_or_else(|| AppError::NotFound(format!("series movie link {link_id}")))?;
        // Same order as the store: removals first, reserved entries untouched,
        // and the bag assembled before it is assigned because the mock has no
        // transaction to roll a refused write back.
        let mut next_tags = link
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
        link.tags = next_tags;
        Ok(link.clone())
    }

    async fn list_collections_for_title(&self, title_id: &str) -> AppResult<Vec<Collection>> {
        let collections = self.collections.lock().await;
        Ok(collections
            .iter()
            .filter(|item| item.title_id == title_id)
            .cloned()
            .collect())
    }

    async fn list_collection_external_ids(
        &self,
        collection_id: &str,
    ) -> AppResult<Vec<ScopedExternalId>> {
        let ids = self.collection_external_ids.lock().await;
        Ok(ids
            .iter()
            .filter(|item| item.scope_id == collection_id)
            .cloned()
            .collect())
    }

    async fn list_collections_for_titles(
        &self,
        title_ids: &[String],
    ) -> AppResult<HashMap<String, Vec<Collection>>> {
        let collections = self.collections.lock().await;
        let wanted = title_ids.iter().cloned().collect::<HashSet<_>>();
        let mut grouped = HashMap::<String, Vec<Collection>>::new();
        for collection in collections.iter() {
            if wanted.contains(&collection.title_id) {
                grouped
                    .entry(collection.title_id.clone())
                    .or_default()
                    .push(collection.clone());
            }
        }
        Ok(grouped)
    }

    async fn get_collection_by_id(&self, collection_id: &str) -> AppResult<Option<Collection>> {
        let collections = self.collections.lock().await;
        Ok(collections
            .iter()
            .find(|item| item.id == collection_id)
            .cloned())
    }

    async fn get_collection_by_ordered_path(
        &self,
        ordered_path: &str,
    ) -> AppResult<Option<Collection>> {
        let collections = self.collections.lock().await;
        Ok(collections
            .iter()
            .find(|item| item.ordered_path.as_deref() == Some(ordered_path))
            .cloned())
    }

    async fn create_collection(&self, collection: Collection) -> AppResult<Collection> {
        self.collections.lock().await.push(collection.clone());
        Ok(collection)
    }

    async fn update_collection(
        &self,
        collection_id: &str,
        update: CollectionUpdate,
    ) -> AppResult<Collection> {
        let mut collections = self.collections.lock().await;
        let item = collections
            .iter_mut()
            .find(|entry| entry.id == collection_id)
            .ok_or_else(|| AppError::NotFound(format!("collection {}", collection_id)))?;

        if let Some(value) = update.collection_type {
            item.collection_type = value;
        }
        if let Some(value) = update.collection_index {
            item.collection_index = value;
        }
        if let Some(value) = update.label {
            item.label = Some(value);
        }
        if let Some(value) = update.ordered_path {
            item.ordered_path = Some(value);
        }
        if let Some(value) = update.first_episode_number {
            item.first_episode_number = Some(value);
        }
        if let Some(value) = update.last_episode_number {
            item.last_episode_number = Some(value);
        }
        if let Some(value) = update.monitored {
            item.monitored = value;
        }

        Ok(item.clone())
    }

    async fn set_collection_episodes_monitored(
        &self,
        collection_id: &str,
        monitored: bool,
    ) -> AppResult<()> {
        let mut episodes = self.episodes.lock().await;
        for episode in episodes.iter_mut() {
            if episode.collection_id.as_deref() == Some(collection_id) {
                episode.monitored = monitored;
            }
        }
        Ok(())
    }

    async fn set_collections_monitored(
        &self,
        collection_ids: &[String],
        monitored: bool,
    ) -> AppResult<()> {
        let wanted = collection_ids.iter().cloned().collect::<HashSet<_>>();
        let mut collections = self.collections.lock().await;
        for collection in collections.iter_mut() {
            if wanted.contains(&collection.id) {
                collection.monitored = monitored;
            }
        }
        Ok(())
    }

    async fn delete_collection(&self, collection_id: &str) -> AppResult<()> {
        let mut collections = self.collections.lock().await;
        let index = collections
            .iter()
            .position(|item| item.id == collection_id)
            .ok_or_else(|| AppError::NotFound(format!("collection {}", collection_id)))?;
        collections.remove(index);

        let mut episodes = self.episodes.lock().await;
        for episode in episodes.iter_mut() {
            if episode.collection_id.as_deref() == Some(collection_id) {
                episode.collection_id = None;
            }
        }
        Ok(())
    }

    async fn delete_collections_for_title(&self, title_id: &str) -> AppResult<()> {
        let mut collections = self.collections.lock().await;
        collections.retain(|item| item.title_id != title_id);

        let mut episodes = self.episodes.lock().await;
        for episode in episodes.iter_mut() {
            if episode.title_id == title_id {
                episode.collection_id = None;
            }
        }
        Ok(())
    }

    async fn list_episodes_for_collection(&self, collection_id: &str) -> AppResult<Vec<Episode>> {
        let episodes = self.episodes.lock().await;
        Ok(episodes
            .iter()
            .filter(|item| item.collection_id.as_deref() == Some(collection_id))
            .cloned()
            .collect())
    }

    async fn list_episodes_for_title(&self, title_id: &str) -> AppResult<Vec<Episode>> {
        let episodes = self.episodes.lock().await;
        Ok(episodes
            .iter()
            .filter(|item| item.title_id == title_id)
            .cloned()
            .collect())
    }

    async fn list_episode_external_ids(
        &self,
        episode_id: &str,
    ) -> AppResult<Vec<ScopedExternalId>> {
        let ids = self.episode_external_ids.lock().await;
        Ok(ids
            .iter()
            .filter(|item| item.scope_id == episode_id)
            .cloned()
            .collect())
    }

    async fn get_episode_by_id(&self, episode_id: &str) -> AppResult<Option<Episode>> {
        let episodes = self.episodes.lock().await;
        Ok(episodes.iter().find(|item| item.id == episode_id).cloned())
    }

    async fn create_episode(&self, episode: Episode) -> AppResult<Episode> {
        self.episodes.lock().await.push(episode.clone());
        Ok(episode)
    }

    async fn update_episode(&self, episode_id: &str, update: EpisodeUpdate) -> AppResult<Episode> {
        let mut episodes = self.episodes.lock().await;
        let item = episodes
            .iter_mut()
            .find(|entry| entry.id == episode_id)
            .ok_or_else(|| AppError::NotFound(format!("episode {}", episode_id)))?;

        if let Some(value) = update.episode_type {
            item.episode_type = value;
        }
        if let Some(value) = update.episode_number {
            item.episode_number = Some(value);
        }
        if let Some(value) = update.season_number {
            item.season_number = Some(value);
        }
        if let Some(value) = update.episode_label {
            item.episode_label = Some(value);
        }
        if let Some(value) = update.title {
            item.title = Some(value);
        }
        if let Some(value) = update.air_date {
            item.air_date = Some(value);
        }
        if let Some(value) = update.duration_seconds {
            item.duration_seconds = Some(value);
        }
        if let Some(value) = update.has_multi_audio {
            item.has_multi_audio = value;
        }
        if let Some(value) = update.has_subtitle {
            item.has_subtitle = value;
        }
        if let Some(value) = update.monitored {
            item.monitored = value;
        }
        if let Some(value) = update.collection_id {
            item.collection_id = Some(value);
        }
        if let Some(value) = update.overview {
            item.overview = Some(value);
        }
        if let Some(value) = update.tvdb_id {
            item.tvdb_id = Some(value);
        }
        if update.clear_image_url {
            item.image_url = None;
        } else if let Some(value) = update.image_url {
            item.image_url = Some(value);
        }

        Ok(item.clone())
    }

    async fn set_episodes_monitored(
        &self,
        episode_ids: &[String],
        monitored: bool,
    ) -> AppResult<()> {
        let wanted = episode_ids.iter().cloned().collect::<HashSet<_>>();
        let mut episodes = self.episodes.lock().await;
        for episode in episodes.iter_mut() {
            if wanted.contains(&episode.id) {
                episode.monitored = monitored;
            }
        }
        Ok(())
    }

    async fn delete_episode(&self, episode_id: &str) -> AppResult<()> {
        let mut episodes = self.episodes.lock().await;
        let index = episodes
            .iter()
            .position(|item| item.id == episode_id)
            .ok_or_else(|| AppError::NotFound(format!("episode {}", episode_id)))?;
        episodes.remove(index);
        Ok(())
    }

    async fn delete_episodes_for_title(&self, title_id: &str) -> AppResult<()> {
        let mut episodes = self.episodes.lock().await;
        episodes.retain(|item| item.title_id != title_id);
        Ok(())
    }

    async fn find_episode_by_title_and_numbers(
        &self,
        title_id: &str,
        season_number: &str,
        episode_number: &str,
    ) -> AppResult<Option<Episode>> {
        let episodes = self.episodes.lock().await;
        Ok(episodes
            .iter()
            .find(|ep| {
                ep.title_id == title_id
                    && ep.season_number.as_deref() == Some(season_number)
                    && ep.episode_number.as_deref() == Some(episode_number)
            })
            .cloned())
    }

    async fn find_episode_by_title_and_absolute_number(
        &self,
        title_id: &str,
        absolute_number: &str,
    ) -> AppResult<Option<Episode>> {
        let episodes = self.episodes.lock().await;
        Ok(episodes
            .iter()
            .find(|ep| {
                ep.title_id == title_id && ep.absolute_number.as_deref() == Some(absolute_number)
            })
            .cloned())
    }

    async fn list_primary_collection_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<PrimaryCollectionSummary>> {
        let collections = self.collections.lock().await;
        let mut out = Vec::new();
        for tid in title_ids {
            if let Some(c) = collections
                .iter()
                .filter(|c| c.title_id == *tid)
                .filter(|c| c.collection_type == CollectionType::Movie || c.collection_index == "0")
                .min_by(|left, right| {
                    let left_key = (
                        left.collection_type != CollectionType::Movie,
                        left.ordered_path
                            .as_deref()
                            .is_none_or(|path| path.trim().is_empty()),
                        left.collection_index.parse::<u32>().unwrap_or(u32::MAX),
                        left.collection_index.clone(),
                    );
                    let right_key = (
                        right.collection_type != CollectionType::Movie,
                        right
                            .ordered_path
                            .as_deref()
                            .is_none_or(|path| path.trim().is_empty()),
                        right.collection_index.parse::<u32>().unwrap_or(u32::MAX),
                        right.collection_index.clone(),
                    );
                    left_key.cmp(&right_key)
                })
            {
                out.push(PrimaryCollectionSummary {
                    title_id: tid.clone(),
                    label: c.label.clone(),
                    ordered_path: c.ordered_path.clone(),
                });
            }
        }
        Ok(out)
    }

    async fn list_episodes_in_date_range(
        &self,
        _start_date: &str,
        _end_date: &str,
    ) -> AppResult<Vec<CalendarEpisode>> {
        Ok(vec![])
    }

    async fn replace_anibridge_scoped_external_ids_for_title(
        &self,
        _title_id: &str,
        collection_ids: Vec<ScopedExternalId>,
        episode_ids: Vec<ScopedExternalId>,
    ) -> AppResult<()> {
        *self.collection_external_ids.lock().await = collection_ids;
        *self.episode_external_ids.lock().await = episode_ids;
        Ok(())
    }
}

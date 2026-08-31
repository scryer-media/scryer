const REMATCH_REPLACED_EXTERNAL_ID_SOURCES: &[&str] = &[
    "smg", "tvdb", "imdb", "tmdb", "mal", "anilist", "anidb", "kitsu",
];
const REMATCH_DERIVED_TAG_PREFIXES: &[&str] = &[
    "scryer:mal-score:",
    "scryer:anime-media-type:",
    "scryer:anime-status:",
];
fn title_external_id_value(title: &Title, source: &str) -> Option<String> {
    if source == "imdb"
        && let Some(imdb_id) = title.imdb_id.as_deref()
        && !imdb_id.trim().is_empty()
    {
        return Some(imdb_id.trim().to_string());
    }

    title
        .external_ids
        .iter()
        .find(|external_id| external_id.source == source && !external_id.value.trim().is_empty())
        .map(|external_id| external_id.value.trim().to_string())
}
fn push_title_external_id_index(
    map: &mut HashMap<String, Vec<Title>>,
    key: Option<String>,
    title: &Title,
) {
    let Some(key) = key else { return };
    map.entry(key).or_default().push(title.clone());
}
fn unique_title_match(map: &HashMap<String, Vec<Title>>, key: Option<&str>) -> Option<Title> {
    let key = key?.trim();
    if key.is_empty() {
        return None;
    }

    let matches = map.get(key)?;
    (matches.len() == 1).then(|| matches[0].clone())
}
fn anime_mapping_external_ids(mapping: &AnimeMapping) -> Vec<(&'static str, String)> {
    let mut ids = Vec::new();
    push_optional_mapping_id(&mut ids, "mal", mapping.mal_id);
    push_optional_mapping_id(&mut ids, "mal_dub", mapping.mal_dub_id);
    push_optional_mapping_id(&mut ids, "anilist", mapping.anilist_id);
    push_optional_mapping_id(&mut ids, "anidb", mapping.anidb_id);
    push_optional_mapping_id(&mut ids, "kitsu", mapping.kitsu_id);
    push_optional_mapping_id(&mut ids, "simkl", mapping.simkl_id);
    push_optional_mapping_id(&mut ids, "tvdb", mapping.thetvdb_id);
    push_optional_mapping_id(&mut ids, "tmdb", mapping.themoviedb_id);
    push_optional_mapping_id(&mut ids, "imdb", mapping.imdb_id);
    push_optional_mapping_id(&mut ids, "trakt", mapping.trakt_id);
    push_optional_mapping_id(&mut ids, "alt_tvdb", mapping.alt_tvdb_id);
    ids
}
fn push_scoped_external_ids(
    out: &mut Vec<ScopedExternalId>,
    seen: &mut HashSet<(String, String, String, String)>,
    scope_id: &str,
    external_ids: &[(&'static str, String)],
    source_scope: Option<&str>,
) {
    let scope_id = scope_id.trim();
    if scope_id.is_empty() {
        return;
    }
    let source_scope = source_scope.unwrap_or_default().trim();
    for (source, external_id) in external_ids {
        let external_id = external_id.trim();
        if external_id.is_empty() {
            continue;
        }
        let key = (
            scope_id.to_string(),
            (*source).to_string(),
            external_id.to_string(),
            source_scope.to_string(),
        );
        if seen.insert(key) {
            out.push(ScopedExternalId {
                scope_id: scope_id.to_string(),
                source: (*source).to_string(),
                external_id: external_id.to_string(),
                provenance: "anibridge".to_string(),
                source_scope: if source_scope.is_empty() {
                    None
                } else {
                    Some(source_scope.to_string())
                },
            });
        }
    }
}
impl AppUseCase {
    pub(crate) async fn metadata_language(&self) -> String {
        self.read_setting_string_value_for_scope(SETTINGS_SCOPE_SYSTEM, METADATA_LANGUAGE_KEY, None)
            .await
            .ok()
            .flatten()
            .and_then(|language| crate::normalize_metadata_language_code(&language))
            .unwrap_or_else(|| "eng".to_string())
    }

    pub async fn global_metadata_language(&self) -> String {
        self.metadata_language().await
    }

    pub(crate) async fn resolve_metadata_language_for_title(&self, title: &Title) -> String {
        let (title_override, library_override, global_language) = tokio::join!(
            self.read_setting_string_value_explicit(
                TITLE_METADATA_LANGUAGE_OVERRIDE_KEY,
                Some(&title.id),
            ),
            self.read_setting_string_value_explicit(METADATA_LANGUAGE_KEY, Some(&title.library_id)),
            self.metadata_language(),
        );
        resolve_metadata_language_overrides(
            title_override.ok().flatten().as_deref(),
            library_override.ok().flatten().as_deref(),
            &global_language,
        )
    }

    /// Resolve a collection of titles with one read per override tier.
    ///
    /// Individual resolver failures intentionally inherit from the next tier;
    /// retain that behavior for bulk hydration rather than failing an entire
    /// refresh because one settings read is unavailable.
    pub(crate) async fn resolve_metadata_languages_for_titles(
        &self,
        titles: &[Title],
    ) -> HashMap<String, String> {
        if titles.is_empty() {
            return HashMap::new();
        }

        let title_ids = titles
            .iter()
            .map(|title| title.id.clone())
            .collect::<Vec<_>>();
        let library_ids = titles
            .iter()
            .map(|title| title.library_id.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let (title_overrides, library_overrides, global_language) = tokio::join!(
            self.load_title_metadata_language_overrides(&title_ids),
            self.load_library_metadata_language_overrides(&library_ids),
            self.metadata_language(),
        );
        let title_overrides = title_overrides.unwrap_or_default();
        let library_overrides = library_overrides.unwrap_or_default();

        titles
            .iter()
            .map(|title| {
                let language = resolve_metadata_language_overrides(
                    title_overrides.get(&title.id).map(String::as_str),
                    library_overrides.get(&title.library_id).map(String::as_str),
                    &global_language,
                );
                (title.id.clone(), language)
            })
            .collect()
    }

    pub async fn title_metadata_language_override(
        &self,
        title_id: &str,
    ) -> AppResult<Option<String>> {
        self.read_setting_string_value_explicit(
            TITLE_METADATA_LANGUAGE_OVERRIDE_KEY,
            Some(title_id),
        )
        .await
        .map(|value| value.and_then(|language| crate::normalize_metadata_language_code(&language)))
    }
}

fn resolve_metadata_language_overrides(
    title_override: Option<&str>,
    library_override: Option<&str>,
    global_language: &str,
) -> String {
    title_override
        .and_then(crate::normalize_metadata_language_code)
        .or_else(|| library_override.and_then(crate::normalize_metadata_language_code))
        .or_else(|| crate::normalize_metadata_language_code(global_language))
        .unwrap_or_else(|| "eng".to_string())
}

impl AppUseCase {
    pub async fn effective_metadata_language_for_title(&self, title_id: &str) -> AppResult<String> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;
        Ok(self.resolve_metadata_language_for_title(&title).await)
    }

    pub async fn effective_use_season_folders_for_title(&self, title_id: &str) -> AppResult<bool> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;
        self.resolve_use_season_folders(&title).await
    }

    pub async fn effective_filler_policy_for_title(
        &self,
        title_id: &str,
    ) -> AppResult<Option<String>> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;
        if title.facet != MediaFacet::Anime {
            return Ok(None);
        }

        let policy = extract_tag_string(&title.tags, "scryer:filler-policy:")
            .filter(|value| matches!(*value, "download_all" | "skip_filler"))
            .map(str::to_owned);
        Ok(Some(match policy {
            Some(policy) => policy,
            None => {
                self.resolve_library_string_setting(
                    "anime.filler_policy",
                    Some(&title.library_id),
                    Some(title.facet.as_str()),
                    "download_all",
                )
                .await?
            }
        }))
    }

    pub async fn effective_recap_policy_for_title(
        &self,
        title_id: &str,
    ) -> AppResult<Option<String>> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;
        if title.facet != MediaFacet::Anime {
            return Ok(None);
        }

        let policy = extract_tag_string(&title.tags, "scryer:recap-policy:")
            .filter(|value| matches!(*value, "download_all" | "skip_recap"))
            .map(str::to_owned);
        Ok(Some(match policy {
            Some(policy) => policy,
            None => {
                self.resolve_library_string_setting(
                    "anime.recap_policy",
                    Some(&title.library_id),
                    Some(title.facet.as_str()),
                    "download_all",
                )
                .await?
            }
        }))
    }

    pub async fn set_title_metadata_language_override(
        &self,
        actor: &User,
        title_id: &str,
        language: Option<String>,
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

        let language = language
            .filter(|language| !language.trim().is_empty())
            .map(|language| {
                crate::normalize_metadata_language_code(&language).ok_or_else(|| {
                    AppError::Validation("metadata language must be one of eng, spa, fra, deu, ita, por, kor, zho, or jpn".to_string())
                })
            })
            .transpose()?;
        if let Some(language) = language {
            self.services
                .config
                .settings
                .upsert_setting_json(
                    SETTINGS_SCOPE_SYSTEM,
                    TITLE_METADATA_LANGUAGE_OVERRIDE_KEY,
                    Some(title.id.clone()),
                    serde_json::to_string(&language)
                        .map_err(|error| AppError::Repository(error.to_string()))?,
                    SETTINGS_SOURCE_TYPED_GRAPHQL,
                    Some(actor.id.clone()),
                )
                .await?;
        } else {
            self.delete_scoped_system_setting(TITLE_METADATA_LANGUAGE_OVERRIDE_KEY, &title.id)
                .await?;
        }

        self.services
            .catalog
            .titles
            .mark_title_metadata_hydration_due_now(&title.id)
            .await?;
        self.runtime.catalog.title_hydration_wake.notify_one();
        Ok(())
    }

    pub(crate) async fn queue_library_metadata_rehydration(
        &self,
        library_id: &str,
    ) -> AppResult<()> {
        let titles = self
            .services
            .catalog
            .titles
            .list_for_libraries(None, &[library_id.to_string()], None)
            .await?;
        let title_ids = titles
            .iter()
            .map(|title| title.id.clone())
            .collect::<Vec<_>>();
        let title_overrides = self
            .load_title_metadata_language_overrides(&title_ids)
            .await?;
        let rehydration_ids = titles
            .into_iter()
            .filter(|title| !title_overrides.contains_key(&title.id))
            .map(|title| title.id)
            .collect::<Vec<_>>();
        if !rehydration_ids.is_empty() {
            self.services
                .catalog
                .titles
                .mark_titles_metadata_hydration_due_now(&rehydration_ids)
                .await?;
        }
        self.runtime.catalog.title_hydration_wake.notify_one();
        Ok(())
    }

    pub(crate) async fn library_use_season_folders_override(
        &self,
        library_id: &str,
    ) -> AppResult<Option<bool>> {
        self.read_setting_bool_value_explicit(USE_SEASON_FOLDERS_KEY, Some(library_id))
            .await
    }

    pub(crate) async fn resolve_use_season_folders(&self, title: &Title) -> AppResult<bool> {
        if !matches!(title.facet, MediaFacet::Series | MediaFacet::Anime) {
            return Ok(true);
        }

        if let Some(value) = crate::import_workflow::season_folder_tag_override(title) {
            return Ok(value);
        }

        if let Some(value) = self
            .library_use_season_folders_override(&title.library_id)
            .await?
        {
            return Ok(value);
        }

        self.resolve_library_bool_setting(
            USE_SEASON_FOLDERS_KEY,
            None,
            Some(title.facet.as_str()),
            true,
        )
        .await
    }

    /// Discovery region seam. Mirrors `metadata_language` so a
    /// future preferences UI only has to write `DISCOVERY_REGION_KEY`. Defaults
    /// to "US" (the previous hardcoded value) so behavior is unchanged until set.
    pub(crate) async fn discovery_region(&self) -> String {
        self.read_setting_string_value_for_scope(SETTINGS_SCOPE_SYSTEM, DISCOVERY_REGION_KEY, None)
            .await
            .ok()
            .flatten()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| "US".to_string())
    }

    pub async fn title_ratings(
        &self,
        actor: &User,
        title_id: &str,
    ) -> AppResult<TitleRatingSummary> {
        self.get_title(actor, title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;
        self.services
            .catalog
            .titles
            .get_title_ratings(title_id)
            .await
    }

    /// Cached SMG credits for one title, optionally narrowed to a set of credit
    /// kinds (`actor`, `voice_actor`, `director`, ...) and capped at `limit`.
    ///
    /// Pure local read: the cache is refilled by metadata hydration, so this
    /// never calls SMG and never queues a refresh.
    pub async fn title_credits(
        &self,
        actor: &User,
        title_id: &str,
        kinds: Option<&[String]>,
        limit: i64,
    ) -> AppResult<Vec<TitleCredit>> {
        self.get_title(actor, title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;
        let limit = limit.clamp(0, TITLE_CREDITS_MAX_LIMIT) as usize;
        if limit == 0 {
            return Ok(Vec::new());
        }
        let credits = self
            .services
            .catalog
            .titles
            .get_title_credits(title_id)
            .await?;
        Ok(select_title_credits(credits, kinds, limit))
    }

    pub async fn list_title_ratings(
        &self,
        actor: &User,
        title_ids: &[String],
    ) -> AppResult<Vec<(String, TitleRatingSummary)>> {
        let title_ids = self
            .filter_title_ids_for_permission(
                actor,
                title_ids,
                scryer_domain::LibraryPermission::View,
            )
            .await?;
        self.services
            .catalog
            .titles
            .list_title_ratings(&title_ids)
            .await
    }
}
impl AppUseCase {
    pub(crate) async fn apply_title_metadata_update(
        &self,
        actor: impl Into<DomainEventActor>,
        id: &str,
        name: Option<String>,
        facet: Option<MediaFacet>,
        tags: Option<Vec<String>>,
    ) -> AppResult<Title> {
        let title = self
            .services
            .catalog
            .titles
            .update_metadata(id, name, facet, tags, None)
            .await?;
        self.emit_title_updated_activity(actor, &title).await;
        Ok(title)
    }
}
impl AppUseCase {
    pub async fn update_title_metadata(
        &self,
        actor: &User,
        id: &str,
        name: Option<String>,
        facet: Option<MediaFacet>,
        tags: Option<Vec<String>>,
    ) -> AppResult<Title> {
        self.update_title_metadata_with_root_folder_id(actor, id, name, facet, tags, None)
            .await
    }

    pub async fn update_title_metadata_with_root_folder_id(
        &self,
        actor: &User,
        id: &str,
        name: Option<String>,
        facet: Option<MediaFacet>,
        tags: Option<Vec<String>>,
        root_folder_id: Option<Option<String>>,
    ) -> AppResult<Title> {
        if name.is_none() && facet.is_none() && tags.is_none() && root_folder_id.is_none() {
            return Err(AppError::Validation(
                "at least one title field must be provided".into(),
            ));
        }
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {id}")))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;
        let _profile_reference_guard = self
            .runtime
            .catalog
            .quality_profile_reference_lock
            .lock()
            .await;
        if let Some(facet) = facet.as_ref()
            && facet != &title.facet
        {
            return Err(AppError::Validation(
                "changing a title facet is not supported because titles cannot move between libraries"
                    .into(),
            ));
        }
        let resolved_root_folder_id = match root_folder_id {
            Some(Some(root_folder_id)) => Some(
                self.resolve_title_root_folder_id_for_library(
                    &title.library_id,
                    Some(root_folder_id.as_str()),
                )
                .await?,
            ),
            Some(None) => Some(
                self.resolve_title_root_folder_id_for_library(&title.library_id, None)
                    .await?,
            ),
            None => None,
        };
        let mut tags = tags.map(|tags| crate::helpers::normalize_tags(&tags));
        if let Some(tags) = tags.as_mut() {
            self.canonicalize_title_quality_profile_tags(tags).await?;
        }

        let title = self
            .services
            .catalog
            .titles
            .update_metadata(id, name, facet, tags, resolved_root_folder_id)
            .await?;

        self.reconcile_series_movie_link_monitoring_for_title(&title)
            .await?;

        self.emit_title_updated_activity(actor, &title).await;
        Ok(title)
    }

    pub async fn set_primary_movie_file(
        &self,
        actor: &User,
        title_id: &str,
        file_id: &str,
    ) -> AppResult<Title> {
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

        let media_files = self
            .services
            .library
            .media_files
            .list_media_files_for_title(&title.id)
            .await?;
        let selected_file = media_files
            .iter()
            .find(|file| file.id == file_id)
            .ok_or_else(|| AppError::NotFound(format!("media file {file_id}")))?;
        if title.facet != MediaFacet::Movie {
            if let Some(episode_id) = selected_file.episode_id.as_deref() {
                let additional_file_ids = media_files
                    .iter()
                    .filter(|file| file.id != selected_file.id)
                    .filter(|file| file.episode_id.as_deref() == Some(episode_id))
                    .map(|file| file.id.clone())
                    .collect::<Vec<_>>();

                self.services
                    .library
                    .media_files
                    .set_media_file_roles_for_episode(
                        &title.id,
                        episode_id,
                        &selected_file.id,
                        &additional_file_ids,
                    )
                    .await?;
                self.emit_title_updated_activity(actor, &title).await;
                return Ok(title);
            }

            let series_movie_link_id =
                selected_file.series_movie_link_ids.first().ok_or_else(|| {
                    AppError::Validation(
                        "primary movie file can only be set for movie titles, series movie files, or episode files"
                            .to_string(),
                    )
                })?;
            let additional_file_ids = media_files
                .iter()
                .filter(|file| file.id != selected_file.id)
                .filter(|file| {
                    file.series_movie_link_ids
                        .iter()
                        .any(|link_id| link_id == series_movie_link_id)
                })
                .map(|file| file.id.clone())
                .collect::<Vec<_>>();

            self.services
                .library
                .media_files
                .set_media_file_roles_for_title(&title.id, &selected_file.id, &additional_file_ids)
                .await?;
            self.emit_title_updated_activity(actor, &title).await;
            return Ok(title);
        }
        let movie_scope =
            crate::library::movie_scan_scope::MovieScanScope::from_title_folder_or_file(
                title.folder_path.as_deref(),
                &selected_file.file_path,
            )
            .ok_or_else(|| {
                AppError::Validation(
                    "movie title does not have a canonical folder path".to_string(),
                )
            })?;
        if !movie_scope.file_is_inside_canonical_folder(&selected_file.file_path) {
            return Err(AppError::Validation(
                "selected file is outside the title's canonical movie folder".to_string(),
            ));
        }

        let additional_file_ids = media_files
            .iter()
            .filter(|file| file.id != selected_file.id)
            .filter(|file| movie_scope.file_is_inside_canonical_folder(&file.file_path))
            .map(|file| file.id.clone())
            .collect::<Vec<_>>();

        self.services
            .library
            .media_files
            .set_media_file_roles_for_title(&title.id, &selected_file.id, &additional_file_ids)
            .await?;
        self.emit_title_updated_activity(actor, &title).await;
        Ok(title)
    }
}
impl AppUseCase {
    pub async fn fix_title_match(
        &self,
        actor: &User,
        title_id: &str,
        target_tvdb_id: Option<&str>,
        target_smg_id: Option<i64>,
    ) -> AppResult<FixTitleMatchResult> {
        let target_tvdb_id = target_tvdb_id
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let target_tvdb_numeric = target_tvdb_id
            .map(|value| {
                value
                    .parse::<i64>()
                    .map_err(|_| AppError::Validation("tvdb id must be numeric".into()))
            })
            .transpose()?;

        let existing_title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;
        self.require_library_permission(
            actor,
            &existing_title.library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;

        let (replacement_identity_ids, requested_movie_ref) = match existing_title.facet {
            MediaFacet::Movie => {
                if target_smg_id.is_some_and(|id| id <= 0) {
                    return Err(AppError::Validation("smg id must be positive".into()));
                }
                let requested_ref = MovieTitleRef {
                    smg_id: target_smg_id,
                    tvdb_id: target_tvdb_numeric,
                    tmdb_id: None,
                    imdb_id: None,
                };
                if requested_ref.smg_id.is_none() && requested_ref.tvdb_id.is_none() {
                    return Err(AppError::Validation("a title identity is required".into()));
                }

                if requested_ref.smg_id.is_none() {
                    (
                        vec![ExternalId {
                            source: "tvdb".into(),
                            value: requested_ref
                                .tvdb_id
                                .expect("movie rematch reference requires an identity")
                                .to_string(),
                        }],
                        Some(requested_ref),
                    )
                } else {
                    let language = self
                        .resolve_metadata_language_for_title(&existing_title)
                        .await;
                    let movie = match self
                        .services
                        .library
                        .metadata_gateway
                        .get_movie_titles(std::slice::from_ref(&requested_ref), &language)
                        .await
                    {
                        Ok(result) => result.by_ref_index.get(&0).cloned().ok_or_else(|| {
                            AppError::NotFound("movie metadata response missing title".into())
                        })?,
                        Err(error) if movie_title_queries_not_supported(&error) => {
                            let tvdb_id = requested_ref.tvdb_id.ok_or_else(|| {
                                AppError::Repository(
                                    "legacy metadata gateway requires a tvdb id".into(),
                                )
                            })?;
                            self.services
                                .library
                                .metadata_gateway
                                .get_movie(tvdb_id, &language)
                                .await?
                        }
                        Err(error) => return Err(error),
                    };
                    let resolved_ref = MovieTitleRef {
                        smg_id: movie.smg_id.or(requested_ref.smg_id),
                        tvdb_id: movie.tvdb_id.or(requested_ref.tvdb_id),
                        tmdb_id: movie.tmdb_id,
                        imdb_id: crate::normalize::normalize_imdb_id(&movie.imdb_id),
                    };
                    let mut identity_ids = Vec::new();
                    if let Some(smg_id) = resolved_ref.smg_id {
                        identity_ids.push(ExternalId {
                            source: "smg".into(),
                            value: smg_id.to_string(),
                        });
                    }
                    if let Some(tvdb_id) = resolved_ref.tvdb_id {
                        identity_ids.push(ExternalId {
                            source: "tvdb".into(),
                            value: tvdb_id.to_string(),
                        });
                    }
                    if let Some(tmdb_id) = resolved_ref.tmdb_id {
                        identity_ids.push(ExternalId {
                            source: "tmdb".into(),
                            value: tmdb_id.to_string(),
                        });
                    }
                    if let Some(imdb_id) = resolved_ref.imdb_id.as_deref() {
                        identity_ids.push(ExternalId {
                            source: "imdb".into(),
                            value: imdb_id.to_string(),
                        });
                    }
                    (identity_ids, Some(resolved_ref))
                }
            }
            MediaFacet::Series | MediaFacet::Anime => {
                let tvdb_id = target_tvdb_id
                    .ok_or_else(|| AppError::Validation("tvdb id is required".into()))?;
                (
                    vec![ExternalId {
                        source: "tvdb".into(),
                        value: tvdb_id.to_string(),
                    }],
                    None,
                )
            }
        };

        for identity_id in &replacement_identity_ids {
            let duplicate = self
                .services
                .catalog
                .titles
                .find_by_external_id_in_library_and_facet(
                    &existing_title.library_id,
                    existing_title.facet.clone(),
                    &identity_id.source,
                    &identity_id.value,
                )
                .await?
                .filter(|title| title.id != existing_title.id);
            if let Some(duplicate) = duplicate {
                return Err(AppError::Validation(format!(
                    "{} id {} is already assigned to title {}",
                    identity_id.source, identity_id.value, duplicate.name
                )));
            }
        }

        let handler = self
            .facet_registry
            .get(&existing_title.facet)
            .ok_or_else(|| AppError::Validation("unsupported title facet".into()))?;
        let has_episodes = handler.has_episodes();

        if has_episodes {
            self.services
                .workflow
                .pending_releases
                .delete_pending_releases_for_title(&existing_title.id)
                .await?;
            self.services
                .workflow
                .acquisition_scope_states
                .delete_acquisition_scope_states_for_title(&existing_title.id)
                .await?;

            self.services
                .catalog
                .shows
                .delete_episodes_for_title(&existing_title.id)
                .await?;
            self.services
                .catalog
                .shows
                .delete_collections_for_title(&existing_title.id)
                .await?;
        }

        let replacement_external_ids = build_rematched_external_ids(
            &existing_title,
            &replacement_identity_ids,
            REMATCH_REPLACED_EXTERNAL_ID_SOURCES,
        );
        let replacement_tags =
            strip_derived_match_tags(&existing_title.tags, REMATCH_DERIVED_TAG_PREFIXES);

        let mut reset_title = {
            let _title_image_maintenance_guard = self
                .runtime
                .catalog
                .title_image_maintenance_lock
                .write()
                .await;
            self.services
                .catalog
                .titles
                .replace_match_state(
                    &existing_title.id,
                    replacement_external_ids,
                    replacement_tags,
                )
                .await?
        };

        reset_title.folder_path = existing_title.folder_path.clone();

        let mut hydration_outcome = self
            .hydrate_titles_bulk(vec![HydrationTarget {
                title: reset_title.clone(),
                requested_tvdb_id: (reset_title.facet != MediaFacet::Movie)
                    .then_some(target_tvdb_numeric)
                    .flatten(),
                requested_movie_ref,
                sync_wanted_after_completion: false,
                source: HydrationSource::Interactive,
            }])
            .await?;
        let hydrated_title = hydration_outcome
            .hydrated_titles
            .remove(&reset_title.id)
            .unwrap_or(reset_title);
        let mut warnings = Vec::new();
        if hydrated_title.metadata_fetched_at.is_none() {
            warnings.push(
                hydration_outcome
                    .failed_titles
                    .remove(&existing_title.id)
                    .unwrap_or_else(|| {
                        "Matched title metadata could not be fully refreshed.".to_string()
                    }),
            );
        }

        let mut library_scan = None;
        if has_episodes {
            match self.scan_title_library(actor, &existing_title.id).await {
                Ok(summary) => library_scan = Some(summary),
                Err(err) => warnings.push(format!("Library relink failed: {err}")),
            }
        }

        if hydrated_title.monitored {
            self.sync_title_for_immediate_acquisition(&hydrated_title)
                .await;
        }

        let refreshed_title = self
            .services
            .catalog
            .titles
            .get_by_id(&existing_title.id)
            .await?
            .unwrap_or(hydrated_title);

        let old_tvdb_id = extract_tvdb_id(&existing_title).map(|id| id.to_string());
        let new_identity_id = |source: &str| {
            refreshed_title
                .external_ids
                .iter()
                .find(|external_id| external_id.source.eq_ignore_ascii_case(source))
                .map(|external_id| external_id.value.trim())
                .filter(|value| !value.is_empty())
        };
        self.append_domain_event(new_title_domain_event(
            actor,
            &refreshed_title,
            DomainEventPayload::TitleRematched(TitleRematchedEventData {
                title: title_context_snapshot(&refreshed_title),
                old_tvdb_id,
                new_tvdb_id: new_identity_id("tvdb").unwrap_or_default().to_string(),
                smg_id: new_identity_id("smg").and_then(|value| value.parse().ok()),
                tmdb_id: new_identity_id("tmdb").and_then(|value| value.parse().ok()),
                source: "manual".to_string(),
            }),
        ))
        .await?;
        self.emit_title_updated_activity(actor, &refreshed_title)
            .await;

        Ok(FixTitleMatchResult {
            hydrated: refreshed_title.metadata_fetched_at.is_some(),
            title: refreshed_title,
            library_scan,
            warnings,
        })
    }
}
/// Extract a boolean from a `scryer:{prefix}:true/false` tag.
/// Returns `None` when no matching tag exists (caller falls back to global setting).
fn extract_tag_bool(tags: &[String], prefix: &str) -> Option<bool> {
    for tag in tags {
        if let Some(value) = tag.strip_prefix(prefix) {
            return Some(!value.trim().eq_ignore_ascii_case("false"));
        }
    }
    None
}
/// Extract a string value from a `scryer:{prefix}:{value}` tag.
/// Returns `None` when no matching tag exists (caller falls back to global setting).
fn extract_tag_string<'a>(tags: &'a [String], prefix: &str) -> Option<&'a str> {
    for tag in tags {
        if let Some(value) = tag.strip_prefix(prefix) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed);
            }
        }
    }
    None
}
/// Upper bound on one `title_credits` response. Keeps a hostile `limit` from
/// turning a rail query into a full-cast dump.
pub(crate) const TITLE_CREDITS_MAX_LIMIT: i64 = 50;

/// Narrow a title's cached credits to `kinds` (all kinds when `None`/empty),
/// order them by SMG billing order and then by the response position the cache
/// preserved, and cap the result at `limit`.
///
/// `credits` arrives in cache order (position ascending), so the stable sort by
/// billing order alone yields "billing_order asc, position asc".
pub(crate) fn select_title_credits(
    credits: Vec<TitleCredit>,
    kinds: Option<&[String]>,
    limit: usize,
) -> Vec<TitleCredit> {
    let allowed = kinds.filter(|kinds| !kinds.is_empty());
    let mut selected = credits
        .into_iter()
        .filter(|credit| allowed.is_none_or(|kinds| kinds.iter().any(|kind| kind == &credit.kind)))
        .collect::<Vec<_>>();
    selected.sort_by_key(|credit| credit.billing_order);
    selected.truncate(limit);
    selected
}

pub(crate) fn extract_tvdb_id(title: &scryer_domain::Title) -> Option<i64> {
    title
        .external_ids
        .iter()
        .find(|eid| eid.source == "tvdb")
        .and_then(|eid| eid.value.parse::<i64>().ok())
}

#[cfg(test)]
mod metadata_language_tests {
    use super::resolve_metadata_language_overrides;

    #[test]
    fn metadata_language_overrides_prefer_title_then_library_then_global() {
        assert_eq!(
            resolve_metadata_language_overrides(Some("jpn"), Some("deu"), "eng"),
            "jpn"
        );
        assert_eq!(
            resolve_metadata_language_overrides(None, Some("deu"), "eng"),
            "deu"
        );
        assert_eq!(
            resolve_metadata_language_overrides(None, None, "spa"),
            "spa"
        );
    }

    #[test]
    fn metadata_language_overrides_ignore_invalid_values() {
        assert_eq!(
            resolve_metadata_language_overrides(Some("not-a-language"), Some("jpn"), "eng"),
            "jpn"
        );
        assert_eq!(
            resolve_metadata_language_overrides(Some("not-a-language"), None, "also-invalid"),
            "eng"
        );
    }
}

pub(crate) fn extract_smg_id(title: &scryer_domain::Title) -> Option<i64> {
    title
        .external_ids
        .iter()
        .find(|external_id| external_id.source.eq_ignore_ascii_case("smg"))
        .and_then(|external_id| external_id.value.trim().parse::<i64>().ok())
}

pub(crate) fn movie_title_ref(title: &scryer_domain::Title) -> Option<crate::MovieTitleRef> {
    let mut reference = crate::MovieTitleRef::from_title(title)?;
    reference.smg_id = extract_smg_id(title);
    Some(reference)
}

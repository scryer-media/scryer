#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryPathsSettings {
    pub movie_path: String,
    pub series_path: String,
    pub anime_path: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateLibraryPaths {
    pub movie_path: String,
    pub series_path: String,
    pub anime_path: Option<String>,
}
fn library_path_key(facet: &MediaFacet) -> &'static str {
    match facet {
        MediaFacet::Movie => MOVIES_PATH_KEY,
        MediaFacet::Series => SERIES_PATH_KEY,
        MediaFacet::Anime => ANIME_PATH_KEY,
    }
}
fn root_folders_key(facet: &MediaFacet) -> &'static str {
    match facet {
        MediaFacet::Movie => MOVIES_ROOT_FOLDERS_KEY,
        MediaFacet::Series => SERIES_ROOT_FOLDERS_KEY,
        MediaFacet::Anime => ANIME_ROOT_FOLDERS_KEY,
    }
}
fn default_library_path(facet: &MediaFacet) -> &'static str {
    match facet {
        MediaFacet::Movie => DEFAULT_MOVIE_LIBRARY_PATH,
        MediaFacet::Series => DEFAULT_SERIES_LIBRARY_PATH,
        MediaFacet::Anime => DEFAULT_ANIME_LIBRARY_PATH,
    }
}
fn default_library_name(facet: &MediaFacet) -> &'static str {
    match facet {
        MediaFacet::Movie => "Movies",
        MediaFacet::Series => "Series",
        MediaFacet::Anime => "Anime",
    }
}
fn normalize_root_folders(entries: Vec<RootFolderEntry>) -> AppResult<Vec<RootFolderEntry>> {
    let mut normalized = Vec::new();
    let mut seen_paths = HashSet::new();
    let mut default_index = None;

    for entry in entries {
        let path = entry.path.trim().to_string();
        if path.is_empty() {
            return Err(AppError::Validation(
                "root folder path is required".to_string(),
            ));
        }
        if !seen_paths.insert(path.clone()) {
            continue;
        }
        if entry.is_default && default_index.is_none() {
            default_index = Some(normalized.len());
        }
        normalized.push(RootFolderEntry {
            path,
            is_default: false,
        });
    }

    if normalized.is_empty() {
        return Err(AppError::Validation(
            "at least one root folder is required".to_string(),
        ));
    }

    let default_index = default_index.unwrap_or(0);
    for (index, entry) in normalized.iter_mut().enumerate() {
        entry.is_default = index == default_index;
    }

    Ok(normalized)
}
pub(crate) fn root_folder_entries_from_library_roots(
    roots: &[scryer_domain::LibraryRoot],
) -> Vec<RootFolderEntry> {
    let mut entries = roots
        .iter()
        .filter_map(|root| {
            let path = root.path.trim();
            if path.is_empty() {
                None
            } else {
                Some(RootFolderEntry {
                    path: path.to_string(),
                    is_default: root.is_default,
                })
            }
        })
        .collect::<Vec<_>>();

    if !entries.iter().any(|entry| entry.is_default)
        && let Some(first) = entries.first_mut()
    {
        first.is_default = true;
    }

    entries
}
fn root_folder_entries_to_library_root_drafts(
    entries: &[RootFolderEntry],
) -> AppResult<Vec<LibraryRootDraft>> {
    crate::library::workflow::normalize_library_root_drafts(
        entries
            .iter()
            .map(|entry| LibraryRootDraft {
                path: entry.path.clone(),
                is_default: entry.is_default,
            })
            .collect(),
    )
}
fn default_root_folder_entry(facet: &MediaFacet) -> RootFolderEntry {
    RootFolderEntry {
        path: default_library_path(facet).to_string(),
        is_default: true,
    }
}
fn default_path_from_root_folders(facet: &MediaFacet, root_folders: &[RootFolderEntry]) -> String {
    root_folders
        .iter()
        .find(|entry| entry.is_default)
        .or_else(|| root_folders.first())
        .map(|entry| entry.path.clone())
        .unwrap_or_else(|| default_library_path(facet).to_string())
}
fn is_bootstrap_default_root_set(facet: &MediaFacet, root_folders: &[RootFolderEntry]) -> bool {
    root_folders.len() == 1
        && normalize_root_path_for_compare(&root_folders[0].path)
            == normalize_root_path_for_compare(default_library_path(facet))
}

pub fn is_bootstrap_default_library_root_set(library: &scryer_domain::Library) -> bool {
    is_bootstrap_default_root_set(
        &library.facet,
        &root_folder_entries_from_library_roots(&library.roots),
    )
}

#[cfg(test)]
mod bootstrap_default_root_tests {
    use super::*;

    #[test]
    fn exact_default_path_is_bootstrap() {
        assert!(is_bootstrap_default_root_set(
            &MediaFacet::Movie,
            &[RootFolderEntry {
                path: DEFAULT_MOVIE_LIBRARY_PATH.to_string(),
                is_default: true,
            }],
        ));
    }

    #[cfg(not(windows))]
    #[test]
    fn unix_bootstrap_path_comparison_preserves_case() {
        assert!(!is_bootstrap_default_root_set(
            &MediaFacet::Movie,
            &[RootFolderEntry {
                path: "/data/Movies".to_string(),
                is_default: true,
            }],
        ));
    }
}
pub(crate) fn effective_scan_roots_from_root_folders(
    root_folders: &[RootFolderEntry],
) -> Vec<String> {
    let mut roots = Vec::with_capacity(root_folders.len());
    let mut seen = HashSet::new();

    for entry in root_folders {
        let Some(root) = normalize_effective_scan_root(&entry.path) else {
            continue;
        };
        if seen.insert(root.clone()) {
            roots.push(root);
        }
    }

    roots
}
impl AppUseCase {
    pub(crate) async fn mirror_default_library_roots_to_legacy_settings(
        &self,
        facet: &MediaFacet,
        root_folders: &[RootFolderEntry],
        source: &str,
        actor_id: Option<String>,
    ) -> AppResult<Vec<String>> {
        let normalized = normalize_root_folders(root_folders.to_vec())?;
        let default_path = default_path_from_root_folders(facet, &normalized);

        self.services
            .config
            .settings
            .upsert_setting_json(
                SETTINGS_SCOPE_MEDIA,
                root_folders_key(facet),
                None,
                encode_setting_json(&normalized)?,
                source,
                actor_id.clone(),
            )
            .await?;
        self.services
            .config
            .settings
            .upsert_setting_json(
                SETTINGS_SCOPE_MEDIA,
                library_path_key(facet),
                None,
                encode_setting_json(&default_path)?,
                source,
                actor_id,
            )
            .await?;

        Ok(vec![
            root_folders_key(facet).to_string(),
            library_path_key(facet).to_string(),
        ])
    }
}
impl AppUseCase {
    async fn ensure_default_facet_library(&self, facet: &MediaFacet) -> AppResult<()> {
        let bootstrap_roots =
            root_folder_entries_to_library_root_drafts(&[default_root_folder_entry(facet)])?;
        let library = match self
            .services
            .catalog
            .libraries
            .default_for_facet(facet.clone())
            .await?
        {
            Some(library) => library,
            None => {
                self.validate_library_root_conflicts(None, &bootstrap_roots)
                    .await?;
                let now = chrono::Utc::now();
                let library = scryer_domain::Library {
                    id: scryer_domain::default_library_id_for_facet(facet),
                    facet: facet.clone(),
                    name: default_library_name(facet).to_string(),
                    slug: scryer_domain::default_library_slug_for_facet(facet).to_string(),
                    is_default: true,
                    roots: Vec::new(),
                    created_at: now,
                    updated_at: now,
                };
                let created = self
                    .services
                    .catalog
                    .libraries
                    .create(library, bootstrap_roots.clone())
                    .await?;
                info!(
                    facet = facet.as_str(),
                    library_id = created.id.as_str(),
                    "recreated missing default library during settings repair"
                );
                created
            }
        };

        if !library.roots.is_empty() {
            return Ok(());
        }

        self.validate_library_root_conflicts(Some(&library.id), &bootstrap_roots)
            .await?;
        self.services
            .catalog
            .libraries
            .update(
                &library.id,
                library.name.clone(),
                library.slug.clone(),
                bootstrap_roots,
            )
            .await?;
        info!(
            facet = facet.as_str(),
            library_id = library.id.as_str(),
            "restored bootstrap root for default library during settings repair"
        );
        Ok(())
    }
}
impl AppUseCase {
    async fn update_default_library_roots_from_entries(
        &self,
        facet: &MediaFacet,
        root_folders: &[RootFolderEntry],
        source: &str,
        actor_id: Option<String>,
    ) -> AppResult<Vec<String>> {
        let Some(library) = self
            .services
            .catalog
            .libraries
            .default_for_facet(facet.clone())
            .await?
        else {
            return Err(AppError::NotFound(format!(
                "default {} library",
                facet.as_str()
            )));
        };

        let roots = root_folder_entries_to_library_root_drafts(root_folders)?;
        self.validate_library_root_conflicts(Some(&library.id), &roots)
            .await?;

        let library = self
            .services
            .catalog
            .libraries
            .update(&library.id, library.name, library.slug, roots)
            .await?;
        let canonical_roots = root_folder_entries_from_library_roots(&library.roots);
        self.mirror_default_library_roots_to_legacy_settings(
            facet,
            &canonical_roots,
            source,
            actor_id,
        )
        .await
    }
}
impl AppUseCase {
    async fn read_legacy_root_folders_for_facet(
        &self,
        facet: &MediaFacet,
    ) -> AppResult<Option<Vec<RootFolderEntry>>> {
        if let Some(raw) = self
            .read_setting_string_value_for_scope_explicit(
                SETTINGS_SCOPE_MEDIA,
                root_folders_key(facet),
                None,
            )
            .await?
        {
            let trimmed = raw.trim();
            if !trimmed.is_empty() && trimmed != "[]" {
                match serde_json::from_str::<Vec<RootFolderEntry>>(trimmed) {
                    Ok(entries) if !entries.is_empty() => {
                        return normalize_root_folders(entries).map(Some);
                    }
                    Ok(_) => {}
                    Err(error) => warn!(
                        facet = facet.as_str(),
                        error = %error,
                        "failed to parse legacy root_folders setting during root reconciliation"
                    ),
                }
            }
        }

        let Some(path) = self
            .read_setting_string_value_for_scope_explicit(
                SETTINGS_SCOPE_MEDIA,
                library_path_key(facet),
                None,
            )
            .await?
        else {
            return Ok(None);
        };
        let path = path.trim();
        if path.is_empty() {
            return Ok(None);
        }

        normalize_root_folders(vec![RootFolderEntry {
            path: path.to_string(),
            is_default: true,
        }])
        .map(Some)
    }
}
impl AppUseCase {
    pub async fn reconcile_default_library_roots(&self) -> AppResult<()> {
        self.ensure_default_facet_libraries().await?;

        for facet in [MediaFacet::Movie, MediaFacet::Series, MediaFacet::Anime] {
            let library = self
                .services
                .catalog
                .libraries
                .default_for_facet(facet.clone())
                .await?
                .ok_or_else(|| AppError::NotFound(format!("default {} library", facet.as_str())))?;

            let canonical_roots = root_folder_entries_from_library_roots(&library.roots);
            let legacy_roots = self.read_legacy_root_folders_for_facet(&facet).await?;
            let canonical_is_empty_or_bootstrap = canonical_roots.is_empty()
                || is_bootstrap_default_root_set(&facet, &canonical_roots);
            let legacy_roots_are_non_bootstrap = legacy_roots.as_ref().is_some_and(|roots| {
                !roots.is_empty() && !is_bootstrap_default_root_set(&facet, roots)
            });

            if canonical_is_empty_or_bootstrap && legacy_roots_are_non_bootstrap {
                let legacy_roots = legacy_roots.expect("checked legacy root presence");
                self.update_default_library_roots_from_entries(
                    &facet,
                    &legacy_roots,
                    "startup_reconciliation",
                    None,
                )
                .await?;
                info!(
                    facet = facet.as_str(),
                    root_count = legacy_roots.len(),
                    "backfilled default library roots from legacy facet root settings"
                );
                continue;
            }

            let roots_to_mirror = if canonical_roots.is_empty() {
                vec![default_root_folder_entry(&facet)]
            } else {
                canonical_roots
            };

            if roots_to_mirror != root_folder_entries_from_library_roots(&library.roots) {
                self.update_default_library_roots_from_entries(
                    &facet,
                    &roots_to_mirror,
                    "startup_reconciliation",
                    None,
                )
                .await?;
                info!(
                    facet = facet.as_str(),
                    "initialized empty default library roots from the bootstrap default"
                );
            } else {
                self.mirror_default_library_roots_to_legacy_settings(
                    &facet,
                    &roots_to_mirror,
                    "startup_reconciliation",
                    None,
                )
                .await?;
                info!(
                    facet = facet.as_str(),
                    root_count = roots_to_mirror.len(),
                    "mirrored canonical default library roots to legacy facet settings"
                );
            }
        }

        Ok(())
    }
}
impl AppUseCase {
    pub(crate) async fn resolve_scoring_persona(
        &self,
        library_id: Option<&str>,
        scope_id: Option<&str>,
    ) -> AppResult<ScoringPersona> {
        if let Some(library_id) = library_id
            && let Some(persona) = parse_scoring_persona_setting(
                self.read_setting_string_value_explicit(SCORING_PERSONA_KEY, Some(library_id))
                    .await?,
            )
        {
            return Ok(persona);
        }

        if let Some(scope_id) = scope_id
            && let Some(persona) = parse_scoring_persona_setting(
                self.read_setting_string_value_explicit(SCORING_PERSONA_KEY, Some(scope_id))
                    .await?,
            )
        {
            return Ok(persona);
        }

        if let Some(persona) = parse_scoring_persona_setting(
            self.read_setting_string_value(SCORING_PERSONA_KEY, None)
                .await?,
        ) {
            return Ok(persona);
        }

        Ok(ScoringPersona::default())
    }
}
impl AppUseCase {
    pub(crate) async fn resolve_library_string_setting(
        &self,
        key_name: &str,
        library_id: Option<&str>,
        scope_id: Option<&str>,
        default: &str,
    ) -> AppResult<String> {
        if let Some(library_id) = library_id
            && let Some(value) = self
                .read_setting_string_value_explicit(key_name, Some(library_id))
                .await?
                .and_then(|value| normalize_optional_string(Some(value)))
        {
            return Ok(value);
        }

        if let Some(scope_id) = scope_id
            && let Some(value) = self
                .read_setting_string_value_explicit(key_name, Some(scope_id))
                .await?
                .and_then(|value| normalize_optional_string(Some(value)))
        {
            return Ok(value);
        }

        Ok(default.to_string())
    }
}
impl AppUseCase {
    pub(crate) async fn resolve_library_bool_setting(
        &self,
        key_name: &str,
        library_id: Option<&str>,
        scope_id: Option<&str>,
        default: bool,
    ) -> AppResult<bool> {
        if let Some(library_id) = library_id
            && let Some(value) = self
                .read_setting_bool_value_explicit(key_name, Some(library_id))
                .await?
        {
            return Ok(value);
        }

        if let Some(scope_id) = scope_id
            && let Some(value) = self
                .read_setting_bool_value_explicit(key_name, Some(scope_id))
                .await?
        {
            return Ok(value);
        }

        Ok(default)
    }
}
impl AppUseCase {
    pub(crate) async fn resolve_plexmatch_write_on_import(
        &self,
        library_id: Option<&str>,
        facet: &MediaFacet,
    ) -> AppResult<Option<bool>> {
        let Some(key_name) = plexmatch_write_on_import_key(facet) else {
            return Ok(None);
        };

        if let Some(library_id) = library_id
            && let Some(value) = self
                .read_setting_bool_value_explicit(key_name, Some(library_id))
                .await?
        {
            return Ok(Some(value));
        }

        Ok(Some(
            self.read_setting_bool_value(key_name, None)
                .await?
                .unwrap_or(false),
        ))
    }
}
fn normalize_request_quality_profile_ids(profile_ids: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    profile_ids
        .into_iter()
        .filter_map(|profile_id| normalize_optional_string(Some(profile_id)))
        .filter(|profile_id| seen.insert(profile_id.clone()))
        .collect()
}

fn configured_request_quality_profile_ids(
    raw_profile_ids: Vec<String>,
    profiles: &[crate::QualityProfile],
) -> AppResult<Option<Vec<String>>> {
    let profile_ids = normalize_request_quality_profile_ids(raw_profile_ids)
        .into_iter()
        .filter_map(|profile_id| {
            quality_profile_by_id(profiles, &profile_id)
                .transpose()
                .map(|result| result.map(|profile| profile.id.clone()))
        })
        .collect::<AppResult<Vec<_>>>()?;
    Ok((!profile_ids.is_empty()).then_some(profile_ids))
}

fn fallback_request_quality_profile_id(
    profile_settings: &QualityProfileSettings,
    library_quality_profile_id: &str,
) -> String {
    if let Ok(Some(profile)) =
        quality_profile_by_id(&profile_settings.profiles, library_quality_profile_id)
    {
        return profile.id.clone();
    }

    profile_settings.global_profile_id.clone()
}

impl AppUseCase {
    async fn load_library_request_quality_profile_ids_override(
        &self,
        library_id: &str,
        profiles: &[crate::QualityProfile],
    ) -> AppResult<Option<Vec<String>>> {
        self
            .read_setting_json_value::<Vec<String>>(
                REQUEST_QUALITY_PROFILE_IDS_KEY,
                Some(library_id),
            )
            .await?
            .map(|profile_ids| configured_request_quality_profile_ids(profile_ids, profiles))
            .transpose()
            .map(Option::flatten)
    }

    pub(crate) async fn effective_request_quality_profile_settings_for_library(
        &self,
        library: &Library,
    ) -> AppResult<RequestQualityProfileSettings> {
        let scope_id = library.facet.as_str();
        let quality_profile_id = self
            .resolve_quality_profile_id(Some(&library.id), Some(scope_id))
            .await?;
        let profile_settings = self.load_quality_profile_settings().await?;
        let fallback_id =
            fallback_request_quality_profile_id(&profile_settings, &quality_profile_id);
        let profile_ids = self
            .load_library_request_quality_profile_ids_override(
                &library.id,
                &profile_settings.profiles,
            )
            .await?
            .unwrap_or_else(|| vec![fallback_id.clone()]);
        let default_profile_id = profile_ids
            .first()
            .cloned()
            .unwrap_or_else(|| fallback_id.clone());

        Ok(RequestQualityProfileSettings {
            profile_ids,
            default_profile_id,
        })
    }

    pub async fn request_quality_profile_settings_for_library(
        &self,
        actor: &User,
        library_id: &str,
    ) -> AppResult<RequestQualityProfileSettings> {
        let library = self
            .services
            .catalog
            .libraries
            .get_by_id(library_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("library {library_id}")))?;

        let can_request = self
            .has_granted_library_permission(
                actor,
                &library.id,
                scryer_domain::LibraryPermission::Request,
            )
            .await?;
        let can_manage = self
            .has_granted_library_permission(
                actor,
                &library.id,
                scryer_domain::LibraryPermission::ManageTitles,
            )
            .await?;
        let can_manage_catalog = self
            .has_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;
        let can_manage_permissions = self
            .has_app_permission(actor, scryer_domain::AppPermission::ManagePermissions)
            .await?;
        if !can_request && !can_manage && !can_manage_catalog && !can_manage_permissions {
            return Err(AppError::Unauthorized(
                "You do not have access to this library".to_string(),
            ));
        }

        self.effective_request_quality_profile_settings_for_library(&library)
            .await
    }

    pub async fn title_quality_profile_id_for_library(
        &self,
        actor: &User,
        library_id: &str,
    ) -> AppResult<String> {
        let library = self
            .services
            .catalog
            .libraries
            .get_by_id(library_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("library {library_id}")))?;
        let can_manage_titles = self
            .has_granted_library_permission(
                actor,
                &library.id,
                scryer_domain::LibraryPermission::ManageTitles,
            )
            .await?;
        let can_manage_catalog = self
            .has_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;
        if !can_manage_titles && !can_manage_catalog {
            return Err(AppError::Unauthorized(
                "You do not have access to this library".to_string(),
            ));
        }

        self.resolve_quality_profile_id(Some(&library.id), Some(library.facet.as_str()))
            .await
    }

    pub async fn get_library_settings(
        &self,
        actor: &User,
        library_id: &str,
    ) -> AppResult<LibrarySettings> {
        let library = self
            .services
            .catalog
            .libraries
            .get_by_id(library_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("library {library_id}")))?;
        self.require_library_management_permission(actor, &library.id)
            .await?;

        let scope_id = library.facet.as_str();
        let required_audio_languages_override = self
            .load_facet_required_audio_languages(&library.id)
            .await?;
        let required_audio_languages_override = (!required_audio_languages_override.is_empty())
            .then_some(required_audio_languages_override);
        let required_audio_languages = self
            .resolve_required_audio_languages(None, Some(&library.id), Some(scope_id))
            .await?;
        let metadata_language_override = self
            .read_setting_string_value_explicit(METADATA_LANGUAGE_KEY, Some(&library.id))
            .await?
            .and_then(|value| normalize_optional_string(Some(value)))
            .and_then(|value| crate::normalize_metadata_language_code(&value));
        let metadata_language = match metadata_language_override.clone() {
            Some(language) => language,
            None => self.metadata_language().await,
        };
        let use_season_folders_override = if matches!(library.facet, MediaFacet::Series | MediaFacet::Anime) {
            self.read_setting_bool_value_explicit(USE_SEASON_FOLDERS_KEY, Some(&library.id))
                .await?
        } else {
            None
        };
        let use_season_folders = if matches!(library.facet, MediaFacet::Series | MediaFacet::Anime) {
            self.resolve_library_bool_setting(
                USE_SEASON_FOLDERS_KEY,
                Some(&library.id),
                Some(scope_id),
                true,
            )
            .await?
        } else {
            true
        };
        let quality_profile_id_override = self
            .read_setting_string_value_explicit(QUALITY_PROFILE_ID_KEY, Some(&library.id))
            .await?
            .and_then(|value| normalize_optional_string(Some(value)));
        let quality_profile_id = self
            .resolve_quality_profile_id(Some(&library.id), Some(scope_id))
            .await?;
        let profile_settings = self.load_quality_profile_settings().await?;
        let request_quality_profile_ids_override = self
            .load_library_request_quality_profile_ids_override(
                &library.id,
                &profile_settings.profiles,
            )
            .await?;
        let request_quality_profile_fallback_id =
            fallback_request_quality_profile_id(&profile_settings, &quality_profile_id);
        let request_quality_profile_ids = request_quality_profile_ids_override
            .clone()
            .unwrap_or_else(|| vec![request_quality_profile_fallback_id.clone()]);
        let request_quality_profile_default_id = request_quality_profile_ids
            .first()
            .cloned()
            .unwrap_or_else(|| request_quality_profile_fallback_id.clone());
        let scoring_persona_override = parse_scoring_persona_setting(
            self.read_setting_string_value_explicit(SCORING_PERSONA_KEY, Some(&library.id))
                .await?,
        );
        let scoring_persona = self
            .resolve_scoring_persona(Some(&library.id), Some(scope_id))
            .await?;
        let filler_policy_override = if library.facet == MediaFacet::Anime {
            self.read_setting_string_value_explicit(ANIME_FILLER_POLICY_KEY, Some(&library.id))
                .await?
                .and_then(|value| normalize_optional_string(Some(value)))
        } else {
            None
        };
        let filler_policy = if library.facet == MediaFacet::Anime {
            Some(
                self.resolve_library_string_setting(
                    ANIME_FILLER_POLICY_KEY,
                    Some(&library.id),
                    Some(scope_id),
                    DEFAULT_FILLER_POLICY,
                )
                .await?,
            )
        } else {
            None
        };
        let recap_policy_override = if library.facet == MediaFacet::Anime {
            self.read_setting_string_value_explicit(ANIME_RECAP_POLICY_KEY, Some(&library.id))
                .await?
                .and_then(|value| normalize_optional_string(Some(value)))
        } else {
            None
        };
        let recap_policy = if library.facet == MediaFacet::Anime {
            Some(
                self.resolve_library_string_setting(
                    ANIME_RECAP_POLICY_KEY,
                    Some(&library.id),
                    Some(scope_id),
                    DEFAULT_RECAP_POLICY,
                )
                .await?,
            )
        } else {
            None
        };
        let monitor_specials_override = if library.facet == MediaFacet::Anime {
            self.read_setting_bool_value_explicit(ANIME_MONITOR_SPECIALS_KEY, Some(&library.id))
                .await?
        } else {
            None
        };
        let monitor_specials = if library.facet == MediaFacet::Anime {
            Some(
                self.resolve_library_bool_setting(
                    ANIME_MONITOR_SPECIALS_KEY,
                    Some(&library.id),
                    Some(scope_id),
                    false,
                )
                .await?,
            )
        } else {
            None
        };
        let inter_season_movies_override = if library.facet == MediaFacet::Anime {
            self.read_setting_bool_value_explicit(ANIME_INTER_SEASON_MOVIES_KEY, Some(&library.id))
                .await?
        } else {
            None
        };
        let inter_season_movies = if library.facet == MediaFacet::Anime {
            Some(
                self.resolve_library_bool_setting(
                    ANIME_INTER_SEASON_MOVIES_KEY,
                    Some(&library.id),
                    Some(scope_id),
                    true,
                )
                .await?,
            )
        } else {
            None
        };
        let monitor_filler_movies_override = if library.facet == MediaFacet::Anime {
            self.read_setting_bool_value_explicit(
                ANIME_MONITOR_FILLER_MOVIES_KEY,
                Some(&library.id),
            )
            .await?
        } else {
            None
        };
        let monitor_filler_movies = if library.facet == MediaFacet::Anime {
            Some(
                self.resolve_library_bool_setting(
                    ANIME_MONITOR_FILLER_MOVIES_KEY,
                    Some(&library.id),
                    Some(scope_id),
                    false,
                )
                .await?,
            )
        } else {
            None
        };
        let nfo_write_on_import_override = self
            .read_setting_bool_value_explicit(
                nfo_write_on_import_key(&library.facet),
                Some(&library.id),
            )
            .await?;
        let nfo_write_on_import = self
            .resolve_nfo_write_on_import(Some(&library.id), &library.facet)
            .await?;
        let plexmatch_write_on_import_override = match plexmatch_write_on_import_key(&library.facet)
        {
            Some(key_name) => {
                self.read_setting_bool_value_explicit(key_name, Some(&library.id))
                    .await?
            }
            None => None,
        };
        let plexmatch_write_on_import = self
            .resolve_plexmatch_write_on_import(Some(&library.id), &library.facet)
            .await?;
        let import_mode_override = parse_import_mode_setting(
            self.read_setting_string_value_explicit(IMPORT_MODE_KEY, Some(&library.id))
                .await?,
        )?;
        let import_mode = self
            .resolve_import_mode(Some(&library.id), &library.facet)
            .await?;
        let set_permissions_linux_override = self
            .read_setting_bool_value_explicit(SET_PERMISSIONS_LINUX_KEY, Some(&library.id))
            .await?;
        let import_permissions = self
            .resolve_import_file_permissions(Some(&library.id), &library.facet)
            .await?;
        let file_chmod_override = normalize_chmod_setting(
            self.read_setting_string_value_explicit(FILE_CHMOD_KEY, Some(&library.id))
                .await?,
            FILE_CHMOD_KEY,
        )?;
        let folder_chmod_override = normalize_chmod_setting(
            self.read_setting_string_value_explicit(FOLDER_CHMOD_KEY, Some(&library.id))
                .await?,
            FOLDER_CHMOD_KEY,
        )?;
        let chown_group_override = normalize_chown_group_setting(
            self.read_setting_string_value_explicit(CHOWN_GROUP_KEY, Some(&library.id))
                .await?,
        )?;
        let indexer_routing_override = self.load_indexer_routing_override(&library.id).await?;
        let download_client_routing_override = self
            .load_download_client_routing_override(&library.id)
            .await?;

        Ok(LibrarySettings {
            required_audio_languages_override,
            required_audio_languages,
            metadata_language_override,
            metadata_language,
            use_season_folders_override,
            use_season_folders,
            quality_profile_id_override,
            quality_profile_id,
            request_quality_profile_ids_override,
            request_quality_profile_ids,
            request_quality_profile_default_id,
            scoring_persona_override,
            scoring_persona,
            filler_policy_override,
            filler_policy,
            recap_policy_override,
            recap_policy,
            monitor_specials_override,
            monitor_specials,
            inter_season_movies_override,
            inter_season_movies,
            monitor_filler_movies_override,
            monitor_filler_movies,
            nfo_write_on_import_override,
            nfo_write_on_import,
            plexmatch_write_on_import_override,
            plexmatch_write_on_import,
            import_mode_override,
            import_mode,
            set_permissions_linux_override,
            set_permissions_linux: import_permissions.set_permissions_linux,
            file_chmod_override,
            file_chmod: import_permissions.file_chmod,
            folder_chmod_override,
            folder_chmod: import_permissions.folder_chmod,
            chown_group_override,
            chown_group: import_permissions.chown_group,
            indexer_routing_override,
            download_client_routing_override,
        })
    }
}
impl AppUseCase {
    async fn external_import_has_effective_explicit_setting(
        &self,
        key_name: &str,
        library_id: &str,
        facet: &MediaFacet,
        include_facet: bool,
        include_global: bool,
    ) -> AppResult<bool> {
        if self
            .read_setting_string_value_explicit(key_name, Some(library_id))
            .await?
            .is_some()
        {
            return Ok(true);
        }

        if include_facet
            && self
                .read_setting_string_value_explicit(key_name, Some(facet.as_str()))
                .await?
                .is_some()
        {
            return Ok(true);
        }

        if include_global
            && self
                .read_setting_string_value_explicit(key_name, None)
                .await?
                .is_some()
        {
            return Ok(true);
        }

        Ok(false)
    }

    pub async fn apply_external_import_library_settings_auto_apply(
        &self,
        actor: &User,
        library_id: &str,
        settings: ExternalImportLibrarySettingsAutoApplyDraft,
    ) -> AppResult<ExternalImportLibrarySettingsAutoApplyResult> {
        let library = self
            .services
            .catalog
            .libraries
            .get_by_id(library_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("library {library_id}")))?;
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;
        let _profile_reference_guard = self
            .runtime
            .catalog
            .quality_profile_reference_lock
            .lock()
            .await;

        let is_anime_library = library.facet == MediaFacet::Anime;
        if !is_anime_library && settings.monitor_specials.is_some() {
            return Err(AppError::Validation(
                "monitor_specials is only valid for anime libraries".to_string(),
            ));
        }
        if library.facet == MediaFacet::Movie && settings.plexmatch_write_on_import.is_some() {
            return Err(AppError::Validation(
                "plexmatch_write_on_import is only valid for series and anime libraries"
                    .to_string(),
            ));
        }

        let mut changed_keys = Vec::new();
        let mut skipped_keys = Vec::new();
        let quality_profile_has_explicit_override = self
            .external_import_has_effective_explicit_setting(
                QUALITY_PROFILE_ID_KEY,
                &library.id,
                &library.facet,
                true,
                true,
            )
            .await?;

        if let Some(profile_id) = normalize_optional_string(settings.quality_profile_id) {
            if quality_profile_has_explicit_override {
                push_external_import_auto_apply_skip(
                    &mut skipped_keys,
                    QUALITY_PROFILE_ID_KEY,
                    "target setting already has an explicit override",
                );
            } else {
                let profile_settings = self.load_quality_profile_settings().await?;
                if !profile_settings
                    .profiles
                    .iter()
                    .any(|profile| profile.id == profile_id)
                {
                    push_external_import_auto_apply_skip(
                        &mut skipped_keys,
                        QUALITY_PROFILE_ID_KEY,
                        format!("unknown quality profile {profile_id}"),
                    );
                } else {
                    self.upsert_scoped_system_setting_json(
                        QUALITY_PROFILE_ID_KEY,
                        &library.id,
                        &profile_id,
                        Some(actor.id.clone()),
                    )
                    .await?;
                    changed_keys.push(QUALITY_PROFILE_ID_KEY.to_string());
                }
            }
        }

        if let Some(profile_ids) = settings.request_quality_profile_ids {
            if quality_profile_has_explicit_override {
                push_external_import_auto_apply_skip(
                    &mut skipped_keys,
                    REQUEST_QUALITY_PROFILE_IDS_KEY,
                    "quality profile setting already has an explicit override",
                );
            } else if self
                .external_import_has_effective_explicit_setting(
                    REQUEST_QUALITY_PROFILE_IDS_KEY,
                    &library.id,
                    &library.facet,
                    false,
                    false,
                )
                .await?
            {
                push_external_import_auto_apply_skip(
                    &mut skipped_keys,
                    REQUEST_QUALITY_PROFILE_IDS_KEY,
                    "target setting already has an explicit override",
                );
            } else {
                let normalized = normalize_request_quality_profile_ids(profile_ids);
                if !normalized.is_empty() {
                    let profile_settings = self.load_quality_profile_settings().await?;
                    let catalog_profile_ids = profile_settings
                        .profiles
                        .iter()
                        .map(|profile| profile.id.clone())
                        .collect::<HashSet<_>>();
                    if let Some(missing_profile_id) = normalized
                        .iter()
                        .find(|profile_id| !catalog_profile_ids.contains(*profile_id))
                    {
                        push_external_import_auto_apply_skip(
                            &mut skipped_keys,
                            REQUEST_QUALITY_PROFILE_IDS_KEY,
                            format!("unknown request quality profile {missing_profile_id}"),
                        );
                    } else {
                        self.upsert_scoped_system_setting_json(
                            REQUEST_QUALITY_PROFILE_IDS_KEY,
                            &library.id,
                            &normalized,
                            Some(actor.id.clone()),
                        )
                        .await?;
                        changed_keys.push(REQUEST_QUALITY_PROFILE_IDS_KEY.to_string());
                    }
                }
            }
        }

        if is_anime_library && let Some(value) = settings.monitor_specials {
            if self
                .external_import_has_effective_explicit_setting(
                    ANIME_MONITOR_SPECIALS_KEY,
                    &library.id,
                    &library.facet,
                    true,
                    false,
                )
                .await?
            {
                push_external_import_auto_apply_skip(
                    &mut skipped_keys,
                    ANIME_MONITOR_SPECIALS_KEY,
                    "target setting already has an explicit override",
                );
            } else {
                self.upsert_scoped_system_setting_json(
                    ANIME_MONITOR_SPECIALS_KEY,
                    &library.id,
                    &value,
                    Some(actor.id.clone()),
                )
                .await?;
                changed_keys.push(ANIME_MONITOR_SPECIALS_KEY.to_string());
            }
        }

        if let Some(value) = settings.nfo_write_on_import {
            let key_name = nfo_write_on_import_key(&library.facet);
            if self
                .external_import_has_effective_explicit_setting(
                    key_name,
                    &library.id,
                    &library.facet,
                    false,
                    true,
                )
                .await?
            {
                push_external_import_auto_apply_skip(
                    &mut skipped_keys,
                    key_name,
                    "target setting already has an explicit override",
                );
            } else {
                self.upsert_scoped_system_setting_json(
                    key_name,
                    &library.id,
                    &value,
                    Some(actor.id.clone()),
                )
                .await?;
                changed_keys.push(key_name.to_string());
            }
        }

        if let Some(key_name) = plexmatch_write_on_import_key(&library.facet)
            && let Some(value) = settings.plexmatch_write_on_import
        {
            if self
                .external_import_has_effective_explicit_setting(
                    key_name,
                    &library.id,
                    &library.facet,
                    false,
                    true,
                )
                .await?
            {
                push_external_import_auto_apply_skip(
                    &mut skipped_keys,
                    key_name,
                    "target setting already has an explicit override",
                );
            } else {
                self.upsert_scoped_system_setting_json(
                    key_name,
                    &library.id,
                    &value,
                    Some(actor.id.clone()),
                )
                .await?;
                changed_keys.push(key_name.to_string());
            }
        }

        if let Some(value) = settings.set_permissions_linux {
            if self
                .external_import_has_effective_explicit_setting(
                    SET_PERMISSIONS_LINUX_KEY,
                    &library.id,
                    &library.facet,
                    true,
                    true,
                )
                .await?
            {
                push_external_import_auto_apply_skip(
                    &mut skipped_keys,
                    SET_PERMISSIONS_LINUX_KEY,
                    "target setting already has an explicit override",
                );
            } else {
                self.upsert_scoped_system_setting_json(
                    SET_PERMISSIONS_LINUX_KEY,
                    &library.id,
                    &value,
                    Some(actor.id.clone()),
                )
                .await?;
                changed_keys.push(SET_PERMISSIONS_LINUX_KEY.to_string());
            }
        }

        if settings.folder_chmod.is_some() {
            if self
                .external_import_has_effective_explicit_setting(
                    FOLDER_CHMOD_KEY,
                    &library.id,
                    &library.facet,
                    true,
                    true,
                )
                .await?
            {
                push_external_import_auto_apply_skip(
                    &mut skipped_keys,
                    FOLDER_CHMOD_KEY,
                    "target setting already has an explicit override",
                );
            } else {
                match normalize_chmod_setting(settings.folder_chmod, FOLDER_CHMOD_KEY) {
                    Ok(Some(value)) => {
                        self.upsert_scoped_system_setting_json(
                            FOLDER_CHMOD_KEY,
                            &library.id,
                            &value,
                            Some(actor.id.clone()),
                        )
                        .await?;
                        changed_keys.push(FOLDER_CHMOD_KEY.to_string());
                    }
                    Ok(None) => {}
                    Err(AppError::Validation(reason)) => {
                        push_external_import_auto_apply_skip(
                            &mut skipped_keys,
                            FOLDER_CHMOD_KEY,
                            reason,
                        );
                    }
                    Err(error) => return Err(error),
                }
            }
        }

        if settings.chown_group.is_some() {
            if self
                .external_import_has_effective_explicit_setting(
                    CHOWN_GROUP_KEY,
                    &library.id,
                    &library.facet,
                    true,
                    true,
                )
                .await?
            {
                push_external_import_auto_apply_skip(
                    &mut skipped_keys,
                    CHOWN_GROUP_KEY,
                    "target setting already has an explicit override",
                );
            } else {
                match normalize_chown_group_setting(settings.chown_group) {
                    Ok(Some(value)) => {
                        self.upsert_scoped_system_setting_json(
                            CHOWN_GROUP_KEY,
                            &library.id,
                            &value,
                            Some(actor.id.clone()),
                        )
                        .await?;
                        changed_keys.push(CHOWN_GROUP_KEY.to_string());
                    }
                    Ok(None) => {}
                    Err(AppError::Validation(reason)) => {
                        push_external_import_auto_apply_skip(
                            &mut skipped_keys,
                            CHOWN_GROUP_KEY,
                            reason,
                        );
                    }
                    Err(error) => return Err(error),
                }
            }
        }

        if !changed_keys.is_empty() {
            self.emit_settings_saved(
                actor,
                "external_import_library_settings",
                Some(library.id.clone()),
                changed_keys.clone(),
            )
            .await;
        }

        Ok(ExternalImportLibrarySettingsAutoApplyResult {
            changed_keys,
            skipped_keys,
        })
    }
}
fn push_external_import_auto_apply_skip(
    skipped_keys: &mut Vec<ExternalImportSettingsAutoApplySkip>,
    key_name: &str,
    reason: impl Into<String>,
) {
    skipped_keys.push(ExternalImportSettingsAutoApplySkip {
        key_name: key_name.to_string(),
        reason: reason.into(),
    });
}
impl AppUseCase {
    pub async fn update_library_settings(
        &self,
        actor: &User,
        library_id: &str,
        settings: LibrarySettingsOverrideDraft,
    ) -> AppResult<LibrarySettings> {
        let library = self
            .services
            .catalog
            .libraries
            .get_by_id(library_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("library {library_id}")))?;
        self.require_library_management_permission(actor, &library.id)
            .await?;
        let previous_indexer_routing = self
            .resolve_indexer_routing(Some(&library.id), Some(library.facet.as_str()))
            .await;
        let _profile_reference_guard = self
            .runtime
            .catalog
            .quality_profile_reference_lock
            .lock()
            .await;
        let is_anime_library = library.facet == MediaFacet::Anime;
        let metadata_language_override = normalize_optional_string(settings.metadata_language.clone())
            .map(|value| {
                crate::normalize_metadata_language_code(&value).ok_or_else(|| {
                    AppError::Validation(
                        "metadata language must be one of eng, spa, fra, deu, ita, por, kor, zho, or jpn"
                            .to_string(),
                    )
                })
            })
            .transpose()?;
        let metadata_language_changed = self
            .read_setting_string_value_explicit(METADATA_LANGUAGE_KEY, Some(&library.id))
            .await?
            .and_then(|value| normalize_optional_string(Some(value)))
            .and_then(|value| crate::normalize_metadata_language_code(&value))
            != metadata_language_override;
        if !is_anime_library
            && (settings.filler_policy.is_some()
                || settings.recap_policy.is_some()
                || settings.monitor_specials.is_some()
                || settings.inter_season_movies.is_some()
                || settings.monitor_filler_movies.is_some())
        {
            return Err(AppError::Validation(
                "anime-specific settings require an anime library".to_string(),
            ));
        }
        if library.facet == MediaFacet::Movie && settings.use_season_folders.is_some() {
            return Err(AppError::Validation(
                "season folders are only valid for series and anime libraries".to_string(),
            ));
        }
        if library.facet == MediaFacet::Movie && settings.plexmatch_write_on_import.is_some() {
            return Err(AppError::Validation(
                "plexmatch_write_on_import is only valid for series and anime libraries"
                    .to_string(),
            ));
        }

        if let Some(profile_id) = settings
            .quality_profile_id
            .clone()
            .and_then(|profile_id| normalize_optional_string(Some(profile_id)))
        {
            self.validate_quality_profile_id(&profile_id).await?;
        }

        if let Some(languages) = settings.required_audio_languages {
            let languages = normalize_required_audio_requirements(languages);
            if languages.is_empty() {
                self.delete_scoped_system_setting(REQUIRED_AUDIO_LANGUAGES_KEY, &library.id)
                    .await?;
            } else {
                self.upsert_scoped_system_setting_json(
                    REQUIRED_AUDIO_LANGUAGES_KEY,
                    &library.id,
                    &languages,
                    Some(actor.id.clone()),
                )
                .await?;
            }
        } else {
            self.delete_scoped_system_setting(REQUIRED_AUDIO_LANGUAGES_KEY, &library.id)
                .await?;
        }

        if let Some(language) = metadata_language_override {
            self.upsert_scoped_system_setting_json(
                METADATA_LANGUAGE_KEY,
                &library.id,
                &language,
                Some(actor.id.clone()),
            )
            .await?;
        } else {
            self.delete_scoped_system_setting(METADATA_LANGUAGE_KEY, &library.id)
                .await?;
        }

        if matches!(library.facet, MediaFacet::Series | MediaFacet::Anime) {
            if let Some(value) = settings.use_season_folders {
                self.upsert_scoped_system_setting_json(
                    USE_SEASON_FOLDERS_KEY,
                    &library.id,
                    &value,
                    Some(actor.id.clone()),
                )
                .await?;
            } else {
                self.delete_scoped_system_setting(USE_SEASON_FOLDERS_KEY, &library.id)
                    .await?;
            }
        }

        if let Some(profile_id) = normalize_optional_string(settings.quality_profile_id) {
            self.upsert_scoped_system_setting_json(
                QUALITY_PROFILE_ID_KEY,
                &library.id,
                &profile_id,
                Some(actor.id.clone()),
            )
            .await?;
        } else {
            self.delete_scoped_system_setting(QUALITY_PROFILE_ID_KEY, &library.id)
                .await?;
        }

        if let Some(profile_ids) = settings.request_quality_profile_ids {
            let normalized = normalize_request_quality_profile_ids(profile_ids);
            if normalized.is_empty() {
                self.delete_scoped_system_setting(REQUEST_QUALITY_PROFILE_IDS_KEY, &library.id)
                    .await?;
            } else {
                let profile_settings = self.load_quality_profile_settings().await?;
                let catalog_profile_ids = profile_settings
                    .profiles
                    .iter()
                    .map(|profile| profile.id.clone())
                    .collect::<HashSet<_>>();
                if let Some(missing_profile_id) = normalized
                    .iter()
                    .find(|profile_id| !catalog_profile_ids.contains(*profile_id))
                {
                    return Err(AppError::Validation(format!(
                        "unknown request quality profile {missing_profile_id}"
                    )));
                }
                self.upsert_scoped_system_setting_json(
                    REQUEST_QUALITY_PROFILE_IDS_KEY,
                    &library.id,
                    &normalized,
                    Some(actor.id.clone()),
                )
                .await?;
            }
        } else {
            self.delete_scoped_system_setting(REQUEST_QUALITY_PROFILE_IDS_KEY, &library.id)
                .await?;
        }

        if let Some(persona) = settings.scoring_persona {
            let persona = global_persona_as_setting(&persona).to_string();
            self.upsert_scoped_system_setting_json(
                SCORING_PERSONA_KEY,
                &library.id,
                &persona,
                Some(actor.id.clone()),
            )
            .await?;
        } else {
            self.delete_scoped_system_setting(SCORING_PERSONA_KEY, &library.id)
                .await?;
        }

        if is_anime_library {
            if let Some(policy) = normalize_optional_string(settings.filler_policy) {
                self.upsert_scoped_system_setting_json(
                    ANIME_FILLER_POLICY_KEY,
                    &library.id,
                    &policy,
                    Some(actor.id.clone()),
                )
                .await?;
            } else {
                self.delete_scoped_system_setting(ANIME_FILLER_POLICY_KEY, &library.id)
                    .await?;
            }

            if let Some(policy) = normalize_optional_string(settings.recap_policy) {
                self.upsert_scoped_system_setting_json(
                    ANIME_RECAP_POLICY_KEY,
                    &library.id,
                    &policy,
                    Some(actor.id.clone()),
                )
                .await?;
            } else {
                self.delete_scoped_system_setting(ANIME_RECAP_POLICY_KEY, &library.id)
                    .await?;
            }

            if let Some(value) = settings.monitor_specials {
                self.upsert_scoped_system_setting_json(
                    ANIME_MONITOR_SPECIALS_KEY,
                    &library.id,
                    &value,
                    Some(actor.id.clone()),
                )
                .await?;
            } else {
                self.delete_scoped_system_setting(ANIME_MONITOR_SPECIALS_KEY, &library.id)
                    .await?;
            }

            if let Some(value) = settings.inter_season_movies {
                self.upsert_scoped_system_setting_json(
                    ANIME_INTER_SEASON_MOVIES_KEY,
                    &library.id,
                    &value,
                    Some(actor.id.clone()),
                )
                .await?;
            } else {
                self.delete_scoped_system_setting(ANIME_INTER_SEASON_MOVIES_KEY, &library.id)
                    .await?;
            }

            if let Some(value) = settings.monitor_filler_movies {
                self.upsert_scoped_system_setting_json(
                    ANIME_MONITOR_FILLER_MOVIES_KEY,
                    &library.id,
                    &value,
                    Some(actor.id.clone()),
                )
                .await?;
            } else {
                self.delete_scoped_system_setting(ANIME_MONITOR_FILLER_MOVIES_KEY, &library.id)
                    .await?;
            }
        }

        if let Some(value) = settings.nfo_write_on_import {
            self.upsert_scoped_system_setting_json(
                nfo_write_on_import_key(&library.facet),
                &library.id,
                &value,
                Some(actor.id.clone()),
            )
            .await?;
        } else {
            self.delete_scoped_system_setting(nfo_write_on_import_key(&library.facet), &library.id)
                .await?;
        }

        if let Some(key_name) = plexmatch_write_on_import_key(&library.facet) {
            if let Some(value) = settings.plexmatch_write_on_import {
                self.upsert_scoped_system_setting_json(
                    key_name,
                    &library.id,
                    &value,
                    Some(actor.id.clone()),
                )
                .await?;
            } else {
                self.delete_scoped_system_setting(key_name, &library.id)
                    .await?;
            }
        }

        if let Some(value) = settings.import_mode {
            self.upsert_scoped_system_setting_json(
                IMPORT_MODE_KEY,
                &library.id,
                &value.as_str(),
                Some(actor.id.clone()),
            )
            .await?;
        } else {
            self.delete_scoped_system_setting(IMPORT_MODE_KEY, &library.id)
                .await?;
        }

        if let Some(value) = settings.set_permissions_linux {
            self.upsert_scoped_system_setting_json(
                SET_PERMISSIONS_LINUX_KEY,
                &library.id,
                &value,
                Some(actor.id.clone()),
            )
            .await?;
        } else {
            self.delete_scoped_system_setting(SET_PERMISSIONS_LINUX_KEY, &library.id)
                .await?;
        }

        if let Some(value) = normalize_chmod_setting(settings.file_chmod, FILE_CHMOD_KEY)? {
            self.upsert_scoped_system_setting_json(
                FILE_CHMOD_KEY,
                &library.id,
                &value,
                Some(actor.id.clone()),
            )
            .await?;
        } else {
            self.delete_scoped_system_setting(FILE_CHMOD_KEY, &library.id)
                .await?;
        }

        if let Some(value) = normalize_chmod_setting(settings.folder_chmod, FOLDER_CHMOD_KEY)? {
            self.upsert_scoped_system_setting_json(
                FOLDER_CHMOD_KEY,
                &library.id,
                &value,
                Some(actor.id.clone()),
            )
            .await?;
        } else {
            self.delete_scoped_system_setting(FOLDER_CHMOD_KEY, &library.id)
                .await?;
        }

        if let Some(value) = normalize_chown_group_setting(settings.chown_group)? {
            self.upsert_scoped_system_setting_json(
                CHOWN_GROUP_KEY,
                &library.id,
                &value,
                Some(actor.id.clone()),
            )
            .await?;
        } else {
            self.delete_scoped_system_setting(CHOWN_GROUP_KEY, &library.id)
                .await?;
        }

        if let Some(entries) = settings.indexer_routing {
            let payload = indexer_routing_payload(entries)?;
            self.upsert_scoped_system_setting_json(
                INDEXER_ROUTING_SETTINGS_KEY,
                &library.id,
                &serde_json::Value::Object(payload),
                Some(actor.id.clone()),
            )
            .await?;
        } else {
            self.delete_scoped_system_setting(INDEXER_ROUTING_SETTINGS_KEY, &library.id)
                .await?;
        }

        if let Some(entries) = settings.download_client_routing {
            let entries = self
                .complete_library_download_client_routing_entries(entries)
                .await?;
            let payload = download_client_routing_payload(entries)?;
            self.upsert_scoped_system_setting_json(
                DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
                &library.id,
                &serde_json::Value::Object(payload),
                Some(actor.id.clone()),
            )
            .await?;
            self.delete_scoped_system_setting(
                LEGACY_NZBGET_CLIENT_ROUTING_SETTINGS_KEY,
                &library.id,
            )
            .await?;
        } else {
            self.delete_scoped_system_setting(DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY, &library.id)
                .await?;
            self.delete_scoped_system_setting(
                LEGACY_NZBGET_CLIENT_ROUTING_SETTINGS_KEY,
                &library.id,
            )
            .await?;
        }

        let mut changed_keys = vec![
            REQUIRED_AUDIO_LANGUAGES_KEY.to_string(),
            METADATA_LANGUAGE_KEY.to_string(),
            USE_SEASON_FOLDERS_KEY.to_string(),
            QUALITY_PROFILE_ID_KEY.to_string(),
            REQUEST_QUALITY_PROFILE_IDS_KEY.to_string(),
            SCORING_PERSONA_KEY.to_string(),
            ANIME_FILLER_POLICY_KEY.to_string(),
            ANIME_RECAP_POLICY_KEY.to_string(),
            ANIME_MONITOR_SPECIALS_KEY.to_string(),
            ANIME_INTER_SEASON_MOVIES_KEY.to_string(),
            ANIME_MONITOR_FILLER_MOVIES_KEY.to_string(),
            nfo_write_on_import_key(&library.facet).to_string(),
            IMPORT_MODE_KEY.to_string(),
            INDEXER_ROUTING_SETTINGS_KEY.to_string(),
            DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY.to_string(),
        ];
        if let Some(key_name) = plexmatch_write_on_import_key(&library.facet) {
            changed_keys.push(key_name.to_string());
        }

        self.refresh_download_client_category_admission_best_effort()
            .await;
        let updated_indexer_routing = self
            .resolve_indexer_routing(Some(&library.id), Some(library.facet.as_str()))
            .await;
        let canonical_routing = |plan: Option<IndexerRoutingPlan>| {
            plan.map(|plan| {
                plan.entries
                    .into_iter()
                    .map(|(indexer_id, entry)| {
                        let mut categories = entry.categories;
                        categories.sort();
                        categories.dedup();
                        (indexer_id, (entry.enabled, categories, entry.priority))
                    })
                    .collect::<HashMap<_, _>>()
            })
            .unwrap_or_default()
        };
        let previous_indexer_routing = canonical_routing(previous_indexer_routing);
        let updated_indexer_routing = canonical_routing(updated_indexer_routing);
        let changed_indexers = previous_indexer_routing
            .keys()
            .chain(updated_indexer_routing.keys())
            .filter(|indexer_id| {
                previous_indexer_routing.get(*indexer_id)
                    != updated_indexer_routing.get(*indexer_id)
            })
            .cloned()
            .collect::<HashSet<_>>();
        for indexer_id in changed_indexers {
            self.prune_indexer_search_learning_best_effort(
                &indexer_id,
                "library_indexer_routing_change",
            )
            .await;
        }
        if metadata_language_changed {
            self.queue_library_metadata_rehydration(&library.id).await?;
        }

        self.emit_settings_saved(
            actor,
            "library_settings",
            Some(library.id.clone()),
            changed_keys,
        )
        .await;

        self.get_library_settings(actor, &library.id).await
    }
}
impl AppUseCase {
    pub async fn get_library_paths(&self, actor: &User) -> AppResult<LibraryPathsSettings> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;
        let movie_roots = self.root_folders_for_facet(&MediaFacet::Movie).await?;
        let series_roots = self.root_folders_for_facet(&MediaFacet::Series).await?;
        let anime_roots = self.root_folders_for_facet(&MediaFacet::Anime).await?;

        Ok(LibraryPathsSettings {
            movie_path: default_path_from_root_folders(&MediaFacet::Movie, &movie_roots),
            series_path: default_path_from_root_folders(&MediaFacet::Series, &series_roots),
            anime_path: default_path_from_root_folders(&MediaFacet::Anime, &anime_roots),
        })
    }
}
impl AppUseCase {
    pub async fn update_library_paths(
        &self,
        actor: &User,
        input: UpdateLibraryPaths,
    ) -> AppResult<LibraryPathsSettings> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;
        self.ensure_default_facet_libraries().await?;
        let previous_roots = [
            (
                MediaFacet::Movie,
                self.effective_scan_roots_for_facet(&MediaFacet::Movie)
                    .await?,
            ),
            (
                MediaFacet::Series,
                self.effective_scan_roots_for_facet(&MediaFacet::Series)
                    .await?,
            ),
            (
                MediaFacet::Anime,
                self.effective_scan_roots_for_facet(&MediaFacet::Anime)
                    .await?,
            ),
        ];

        let mut changed_keys = Vec::new();
        if let Some(movie_path) = normalize_optional_string(Some(input.movie_path)) {
            let root_folders = normalize_root_folders(vec![RootFolderEntry {
                path: movie_path,
                is_default: true,
            }])?;
            changed_keys.extend(
                self.update_default_library_roots_from_entries(
                    &MediaFacet::Movie,
                    &root_folders,
                    SETTINGS_SOURCE_TYPED_GRAPHQL,
                    Some(actor.id.clone()),
                )
                .await?,
            );
        }

        if let Some(series_path) = normalize_optional_string(Some(input.series_path)) {
            let root_folders = normalize_root_folders(vec![RootFolderEntry {
                path: series_path,
                is_default: true,
            }])?;
            changed_keys.extend(
                self.update_default_library_roots_from_entries(
                    &MediaFacet::Series,
                    &root_folders,
                    SETTINGS_SOURCE_TYPED_GRAPHQL,
                    Some(actor.id.clone()),
                )
                .await?,
            );
        }

        if let Some(anime_path) = normalize_optional_string(input.anime_path) {
            let root_folders = normalize_root_folders(vec![RootFolderEntry {
                path: anime_path,
                is_default: true,
            }])?;
            changed_keys.extend(
                self.update_default_library_roots_from_entries(
                    &MediaFacet::Anime,
                    &root_folders,
                    SETTINGS_SOURCE_TYPED_GRAPHQL,
                    Some(actor.id.clone()),
                )
                .await?,
            );
        }

        if changed_keys.is_empty() {
            return self.get_library_paths(actor).await;
        }
        warn!("updateLibraryPaths is deprecated; updated default library roots instead");

        for (facet, previous) in previous_roots {
            let current = self.effective_scan_roots_for_facet(&facet).await?;
            self.clear_pending_imports_for_removed_roots(&facet, &previous, &current)
                .await?;
        }

        self.emit_settings_saved(actor, "library_paths", None, changed_keys)
            .await;
        self.get_library_paths(actor).await
    }
}

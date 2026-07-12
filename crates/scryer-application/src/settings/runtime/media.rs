#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaSettings {
    pub library_path: String,
    pub root_folders: Vec<RootFolderEntry>,
    pub required_audio_languages: Vec<String>,
    pub folder_template: String,
    pub season_folder_template: Option<String>,
    pub rename_enabled: bool,
    pub rename_template: String,
    pub rename_collision_policy: String,
    pub rename_missing_metadata_policy: String,
    pub filler_policy: Option<String>,
    pub recap_policy: Option<String>,
    pub monitor_specials: Option<bool>,
    pub inter_season_movies: Option<bool>,
    pub monitor_filler_movies: Option<bool>,
    pub nfo_write_on_import: bool,
    pub plexmatch_write_on_import: Option<bool>,
    pub import_mode: ImportMode,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateMediaSettings {
    pub library_path: Option<String>,
    pub root_folders: Option<Vec<RootFolderEntry>>,
    pub required_audio_languages: Option<Vec<String>>,
    pub folder_template: Option<String>,
    pub season_folder_template: Option<String>,
    pub rename_enabled: Option<bool>,
    pub rename_template: Option<String>,
    pub rename_collision_policy: Option<String>,
    pub rename_missing_metadata_policy: Option<String>,
    pub filler_policy: Option<String>,
    pub recap_policy: Option<String>,
    pub monitor_specials: Option<bool>,
    pub inter_season_movies: Option<bool>,
    pub monitor_filler_movies: Option<bool>,
    pub nfo_write_on_import: Option<bool>,
    pub plexmatch_write_on_import: Option<bool>,
    pub import_mode: Option<ImportMode>,
}
fn rename_template_global_key(facet: &MediaFacet) -> &'static str {
    match facet {
        MediaFacet::Movie => RENAME_TEMPLATE_MOVIE_GLOBAL_KEY,
        MediaFacet::Series => RENAME_TEMPLATE_SERIES_GLOBAL_KEY,
        MediaFacet::Anime => RENAME_TEMPLATE_ANIME_GLOBAL_KEY,
    }
}
fn default_rename_template(facet: &MediaFacet) -> &'static str {
    match facet {
        MediaFacet::Movie => DEFAULT_RENAME_TEMPLATE_MOVIE,
        MediaFacet::Series => DEFAULT_RENAME_TEMPLATE_SERIES,
        MediaFacet::Anime => DEFAULT_RENAME_TEMPLATE_ANIME,
    }
}
fn default_folder_template(facet: &MediaFacet) -> &'static str {
    match facet {
        MediaFacet::Movie => DEFAULT_FOLDER_TEMPLATE_MOVIE,
        MediaFacet::Series => DEFAULT_FOLDER_TEMPLATE_SERIES,
        MediaFacet::Anime => DEFAULT_FOLDER_TEMPLATE_ANIME,
    }
}
fn default_season_folder_template(facet: &MediaFacet) -> Option<&'static str> {
    match facet {
        MediaFacet::Movie => None,
        MediaFacet::Series => Some(DEFAULT_SEASON_FOLDER_TEMPLATE_SERIES),
        MediaFacet::Anime => Some(DEFAULT_SEASON_FOLDER_TEMPLATE_ANIME),
    }
}
fn legacy_collision_policy_global_key(facet: &MediaFacet) -> &'static str {
    match facet {
        MediaFacet::Movie => RENAME_COLLISION_POLICY_MOVIE_GLOBAL_KEY,
        MediaFacet::Series => RENAME_COLLISION_POLICY_SERIES_GLOBAL_KEY,
        MediaFacet::Anime => RENAME_COLLISION_POLICY_ANIME_GLOBAL_KEY,
    }
}
fn legacy_missing_metadata_policy_global_key(facet: &MediaFacet) -> &'static str {
    match facet {
        MediaFacet::Movie => RENAME_MISSING_METADATA_POLICY_MOVIE_GLOBAL_KEY,
        MediaFacet::Series => RENAME_MISSING_METADATA_POLICY_SERIES_GLOBAL_KEY,
        MediaFacet::Anime => RENAME_MISSING_METADATA_POLICY_ANIME_GLOBAL_KEY,
    }
}
fn nfo_write_on_import_key(facet: &MediaFacet) -> &'static str {
    match facet {
        MediaFacet::Movie => NFO_WRITE_ON_IMPORT_MOVIE_KEY,
        MediaFacet::Series => NFO_WRITE_ON_IMPORT_SERIES_KEY,
        MediaFacet::Anime => NFO_WRITE_ON_IMPORT_ANIME_KEY,
    }
}
fn parse_import_mode_setting(raw: Option<String>) -> AppResult<Option<ImportMode>> {
    raw.map(|value| {
        ImportMode::from_setting(&value).map_err(|message| {
            AppError::Validation(format!(
                "invalid {IMPORT_MODE_KEY} setting value '{}': {message}",
                value.trim()
            ))
        })
    })
    .transpose()
}
fn extract_languages_from_required_audio_rego(rego: &str) -> Vec<String> {
    for line in rego.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("_required_langs := {")
            && let Some(set_body) = rest.strip_suffix('}')
        {
            return normalize_required_audio_languages(
                set_body
                    .split(',')
                    .map(|value| value.trim().trim_matches('"').to_string()),
            );
        }
    }

    Vec::new()
}
impl AppUseCase {
    pub async fn load_facet_required_audio_languages(
        &self,
        scope_id: &str,
    ) -> AppResult<Vec<String>> {
        Ok(normalize_required_audio_languages(
            self.read_setting_json_value::<Vec<String>>(
                REQUIRED_AUDIO_LANGUAGES_KEY,
                Some(scope_id),
            )
            .await?
            .unwrap_or_default(),
        ))
    }
}
impl AppUseCase {
    pub async fn load_title_required_audio_override(
        &self,
        title_id: &str,
    ) -> AppResult<Option<Vec<String>>> {
        let raw_value = self
            .services
            .config
            .settings
            .get_setting_json(
                SETTINGS_SCOPE_SYSTEM,
                TITLE_REQUIRED_AUDIO_OVERRIDE_KEY,
                Some(title_id.to_string()),
            )
            .await?;

        let Some(raw_value) = raw_value else {
            return Ok(None);
        };

        serde_json::from_str::<Option<Vec<String>>>(&raw_value)
            .map(|value| value.map(normalize_required_audio_languages))
            .map_err(|error| {
                AppError::Repository(format!(
                    "failed to parse setting '{TITLE_REQUIRED_AUDIO_OVERRIDE_KEY}' JSON value: {error}"
                ))
            })
    }
}
impl AppUseCase {
    pub(crate) async fn resolve_required_audio_languages(
        &self,
        title_id: Option<&str>,
        library_id: Option<&str>,
        scope_id: Option<&str>,
    ) -> AppResult<Vec<String>> {
        if let Some(title_id) = title_id
            && let Some(languages) = self.load_title_required_audio_override(title_id).await?
        {
            return Ok(languages);
        }

        if let Some(library_id) = library_id {
            let languages = self.load_facet_required_audio_languages(library_id).await?;
            if !languages.is_empty() {
                return Ok(languages);
            }
        }

        if let Some(scope_id) = scope_id {
            let languages = self.load_facet_required_audio_languages(scope_id).await?;
            if !languages.is_empty() {
                return Ok(languages);
            }
        }

        Ok(Vec::new())
    }
}
impl AppUseCase {
    pub(crate) async fn resolve_rename_enabled(&self, facet: &MediaFacet) -> AppResult<bool> {
        Ok(self
            .read_setting_bool_value(RENAME_ENABLED_KEY, Some(facet.as_str()))
            .await?
            .unwrap_or(true))
    }

    pub(crate) async fn resolve_import_mode(
        &self,
        library_id: Option<&str>,
        facet: &MediaFacet,
    ) -> AppResult<ImportMode> {
        if let Some(library_id) = library_id
            && let Some(value) = parse_import_mode_setting(
                self.read_setting_string_value_explicit(IMPORT_MODE_KEY, Some(library_id))
                    .await?,
            )?
        {
            return Ok(value);
        }

        if let Some(value) = parse_import_mode_setting(
            self.read_setting_string_value_explicit(IMPORT_MODE_KEY, Some(facet.as_str()))
                .await?,
        )? {
            return Ok(value);
        }

        if let Some(value) = parse_import_mode_setting(
            self.read_setting_string_value(IMPORT_MODE_KEY, None)
                .await?,
        )? {
            return Ok(value);
        }

        Ok(ImportMode::HardlinkOrCopy)
    }
}
impl AppUseCase {
    pub(crate) async fn resolve_nfo_write_on_import(
        &self,
        library_id: Option<&str>,
        facet: &MediaFacet,
    ) -> AppResult<bool> {
        let key_name = nfo_write_on_import_key(facet);
        if let Some(library_id) = library_id
            && let Some(value) = self
                .read_setting_bool_value_explicit(key_name, Some(library_id))
                .await?
        {
            return Ok(value);
        }

        Ok(self
            .read_setting_bool_value(key_name, None)
            .await?
            .unwrap_or(false))
    }
}
impl AppUseCase {
    pub async fn set_title_required_audio_override(
        &self,
        actor: &User,
        title_id: &str,
        languages: Option<Vec<String>>,
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

        let payload = languages.map(normalize_required_audio_languages);
        self.services
            .config
            .settings
            .upsert_setting_json(
                SETTINGS_SCOPE_SYSTEM,
                TITLE_REQUIRED_AUDIO_OVERRIDE_KEY,
                Some(title_id.trim().to_string()),
                serde_json::to_string(&payload)
                    .map_err(|error| AppError::Repository(error.to_string()))?,
                SETTINGS_SOURCE_TYPED_GRAPHQL,
                Some(actor.id.clone()),
            )
            .await
    }
}
impl AppUseCase {
    pub async fn set_facet_required_audio_languages(
        &self,
        actor: &User,
        scope_id: &str,
        languages: Vec<String>,
    ) -> AppResult<()> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        let normalized = normalize_required_audio_languages(languages);
        self.services
            .config
            .settings
            .upsert_setting_json(
                SETTINGS_SCOPE_SYSTEM,
                REQUIRED_AUDIO_LANGUAGES_KEY,
                Some(scope_id.trim().to_string()),
                serde_json::to_string(&normalized)
                    .map_err(|error| AppError::Repository(error.to_string()))?,
                SETTINGS_SOURCE_TYPED_GRAPHQL,
                Some(actor.id.clone()),
            )
            .await
    }
}
impl AppUseCase {
    pub async fn get_media_settings(
        &self,
        actor: &User,
        facet: MediaFacet,
    ) -> AppResult<MediaSettings> {
        self.require_library_settings_read_permission(actor).await?;

        let root_folders = self.root_folders_for_facet(&facet).await?;
        let library_path = default_path_from_root_folders(&facet, &root_folders);
        let rename_enabled = self.resolve_rename_enabled(&facet).await?;
        let scoped_rename_template = self
            .read_setting_string_value(RENAME_TEMPLATE_KEY, Some(facet.as_str()))
            .await?;
        let global_rename_template = self
            .read_setting_string_value(rename_template_global_key(&facet), None)
            .await?;
        let folder_template = crate::normalize_title_folder_template_or_default(
            self.read_setting_string_value(FOLDER_TEMPLATE_KEY, Some(facet.as_str()))
                .await?,
            default_folder_template(&facet),
            facet.as_str(),
        );
        let season_folder_template = match default_season_folder_template(&facet) {
            Some(default_template) => Some(crate::normalize_season_folder_template_or_default(
                self.read_setting_string_value(SEASON_FOLDER_TEMPLATE_KEY, Some(facet.as_str()))
                    .await?,
                default_template,
                facet.as_str(),
            )),
            None => None,
        };
        let rename_template = scoped_rename_template
            .or(global_rename_template)
            .unwrap_or_else(|| default_rename_template(&facet).to_string());
        let scoped_collision_policy = self
            .read_setting_string_value(RENAME_COLLISION_POLICY_KEY, Some(facet.as_str()))
            .await?;
        let global_collision_policy = self
            .read_setting_string_value(RENAME_COLLISION_POLICY_GLOBAL_KEY, None)
            .await?;
        let legacy_collision_policy = self
            .read_setting_string_value(legacy_collision_policy_global_key(&facet), None)
            .await?;
        let rename_collision_policy = scoped_collision_policy
            .or(global_collision_policy)
            .or(legacy_collision_policy)
            .unwrap_or_else(|| DEFAULT_RENAME_COLLISION_POLICY.to_string());
        let scoped_missing_metadata_policy = self
            .read_setting_string_value(RENAME_MISSING_METADATA_POLICY_KEY, Some(facet.as_str()))
            .await?;
        let global_missing_metadata_policy = self
            .read_setting_string_value(RENAME_MISSING_METADATA_POLICY_GLOBAL_KEY, None)
            .await?;
        let legacy_missing_metadata_policy = self
            .read_setting_string_value(legacy_missing_metadata_policy_global_key(&facet), None)
            .await?;
        let rename_missing_metadata_policy = scoped_missing_metadata_policy
            .or(global_missing_metadata_policy)
            .or(legacy_missing_metadata_policy)
            .unwrap_or_else(|| DEFAULT_RENAME_MISSING_METADATA_POLICY.to_string());

        Ok(MediaSettings {
            library_path,
            root_folders,
            required_audio_languages: self
                .load_facet_required_audio_languages(facet.as_str())
                .await?,
            folder_template,
            season_folder_template,
            rename_enabled,
            rename_template,
            rename_collision_policy,
            rename_missing_metadata_policy,
            filler_policy: if facet == MediaFacet::Anime {
                Some(
                    self.read_setting_string_value(ANIME_FILLER_POLICY_KEY, Some(facet.as_str()))
                        .await?
                        .unwrap_or_else(|| DEFAULT_FILLER_POLICY.to_string()),
                )
            } else {
                None
            },
            recap_policy: if facet == MediaFacet::Anime {
                Some(
                    self.read_setting_string_value(ANIME_RECAP_POLICY_KEY, Some(facet.as_str()))
                        .await?
                        .unwrap_or_else(|| DEFAULT_RECAP_POLICY.to_string()),
                )
            } else {
                None
            },
            monitor_specials: if facet == MediaFacet::Anime {
                Some(
                    self.read_setting_bool_value(ANIME_MONITOR_SPECIALS_KEY, Some(facet.as_str()))
                        .await?
                        .unwrap_or(false),
                )
            } else {
                None
            },
            inter_season_movies: if facet == MediaFacet::Anime {
                Some(
                    self.read_setting_bool_value(
                        ANIME_INTER_SEASON_MOVIES_KEY,
                        Some(facet.as_str()),
                    )
                    .await?
                    .unwrap_or(true),
                )
            } else {
                None
            },
            monitor_filler_movies: if facet == MediaFacet::Anime {
                Some(
                    self.read_setting_bool_value(
                        ANIME_MONITOR_FILLER_MOVIES_KEY,
                        Some(facet.as_str()),
                    )
                    .await?
                    .unwrap_or(false),
                )
            } else {
                None
            },
            nfo_write_on_import: self
                .read_setting_bool_value(nfo_write_on_import_key(&facet), None)
                .await?
                .unwrap_or(false),
            plexmatch_write_on_import: match plexmatch_write_on_import_key(&facet) {
                Some(key) => Some(
                    self.read_setting_bool_value(key, None)
                        .await?
                        .unwrap_or(false),
                ),
                None => None,
            },
            import_mode: self.resolve_import_mode(None, &facet).await?,
        })
    }
}
impl AppUseCase {
    pub async fn update_media_settings(
        &self,
        actor: &User,
        facet: MediaFacet,
        input: UpdateMediaSettings,
    ) -> AppResult<MediaSettings> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;
        let previous_roots = self.effective_scan_roots_for_facet(&facet).await?;
        let root_folder_update = input
            .root_folders
            .clone()
            .map(normalize_root_folders)
            .transpose()?;

        let mut changed_keys = Vec::new();

        if let Some(normalized) = root_folder_update {
            changed_keys.extend(
                self.update_default_library_roots_from_entries(
                    &facet,
                    &normalized,
                    SETTINGS_SOURCE_TYPED_GRAPHQL,
                    Some(actor.id.clone()),
                )
                .await?,
            );
        } else if let Some(library_path) = normalize_optional_string(input.library_path) {
            let root_folders = normalize_root_folders(vec![RootFolderEntry {
                path: library_path,
                is_default: true,
            }])?;
            changed_keys.extend(
                self.update_default_library_roots_from_entries(
                    &facet,
                    &root_folders,
                    SETTINGS_SOURCE_TYPED_GRAPHQL,
                    Some(actor.id.clone()),
                )
                .await?,
            );
        }

        if let Some(folder_template) = normalize_optional_string(input.folder_template) {
            crate::validate_title_folder_template(&folder_template)?;
            self.services
                .config
                .settings
                .upsert_setting_json(
                    SETTINGS_SCOPE_SYSTEM,
                    FOLDER_TEMPLATE_KEY,
                    Some(facet.as_str().to_string()),
                    encode_setting_json(&folder_template)?,
                    SETTINGS_SOURCE_TYPED_GRAPHQL,
                    Some(actor.id.clone()),
                )
                .await?;
            changed_keys.push(FOLDER_TEMPLATE_KEY.to_string());
        }

        if let Some(season_folder_template) = normalize_optional_string(input.season_folder_template)
        {
            if default_season_folder_template(&facet).is_none() {
                return Err(AppError::Validation(
                    "season_folder_template is only valid for series and anime".to_string(),
                ));
            }
            crate::validate_season_folder_template(&season_folder_template)?;
            self.services
                .config
                .settings
                .upsert_setting_json(
                    SETTINGS_SCOPE_SYSTEM,
                    SEASON_FOLDER_TEMPLATE_KEY,
                    Some(facet.as_str().to_string()),
                    encode_setting_json(&season_folder_template)?,
                    SETTINGS_SOURCE_TYPED_GRAPHQL,
                    Some(actor.id.clone()),
                )
                .await?;
            changed_keys.push(SEASON_FOLDER_TEMPLATE_KEY.to_string());
        }

        if let Some(rename_enabled) = input.rename_enabled {
            self.services
                .config
                .settings
                .upsert_setting_json(
                    SETTINGS_SCOPE_SYSTEM,
                    RENAME_ENABLED_KEY,
                    Some(facet.as_str().to_string()),
                    encode_setting_json(&rename_enabled)?,
                    SETTINGS_SOURCE_TYPED_GRAPHQL,
                    Some(actor.id.clone()),
                )
                .await?;
            changed_keys.push(RENAME_ENABLED_KEY.to_string());
        }

        if let Some(rename_template) = normalize_optional_string(input.rename_template) {
            self.services
                .config
                .settings
                .upsert_setting_json(
                    SETTINGS_SCOPE_SYSTEM,
                    RENAME_TEMPLATE_KEY,
                    Some(facet.as_str().to_string()),
                    encode_setting_json(&rename_template)?,
                    SETTINGS_SOURCE_TYPED_GRAPHQL,
                    Some(actor.id.clone()),
                )
                .await?;
            changed_keys.push(RENAME_TEMPLATE_KEY.to_string());
        }

        if let Some(required_audio_languages) = input.required_audio_languages {
            self.services
                .config
                .settings
                .upsert_setting_json(
                    SETTINGS_SCOPE_SYSTEM,
                    REQUIRED_AUDIO_LANGUAGES_KEY,
                    Some(facet.as_str().to_string()),
                    encode_setting_json(&normalize_required_audio_languages(
                        required_audio_languages,
                    ))?,
                    SETTINGS_SOURCE_TYPED_GRAPHQL,
                    Some(actor.id.clone()),
                )
                .await?;
            changed_keys.push(REQUIRED_AUDIO_LANGUAGES_KEY.to_string());
        }

        if let Some(policy) = normalize_optional_string(input.rename_collision_policy) {
            self.services
                .config
                .settings
                .upsert_setting_json(
                    SETTINGS_SCOPE_SYSTEM,
                    RENAME_COLLISION_POLICY_KEY,
                    Some(facet.as_str().to_string()),
                    encode_setting_json(&policy)?,
                    SETTINGS_SOURCE_TYPED_GRAPHQL,
                    Some(actor.id.clone()),
                )
                .await?;
            changed_keys.push(RENAME_COLLISION_POLICY_KEY.to_string());
        }

        if let Some(policy) = normalize_optional_string(input.rename_missing_metadata_policy) {
            self.services
                .config
                .settings
                .upsert_setting_json(
                    SETTINGS_SCOPE_SYSTEM,
                    RENAME_MISSING_METADATA_POLICY_KEY,
                    Some(facet.as_str().to_string()),
                    encode_setting_json(&policy)?,
                    SETTINGS_SOURCE_TYPED_GRAPHQL,
                    Some(actor.id.clone()),
                )
                .await?;
            changed_keys.push(RENAME_MISSING_METADATA_POLICY_KEY.to_string());
        }

        if let Some(value) = input.nfo_write_on_import {
            self.services
                .config
                .settings
                .upsert_setting_json(
                    SETTINGS_SCOPE_SYSTEM,
                    nfo_write_on_import_key(&facet),
                    None,
                    encode_setting_json(&value)?,
                    SETTINGS_SOURCE_TYPED_GRAPHQL,
                    Some(actor.id.clone()),
                )
                .await?;
            changed_keys.push(nfo_write_on_import_key(&facet).to_string());
        }

        if let Some(value) = input.plexmatch_write_on_import {
            let Some(key) = plexmatch_write_on_import_key(&facet) else {
                return Err(AppError::Validation(
                    "plexmatch_write_on_import is only valid for series and anime".to_string(),
                ));
            };
            self.services
                .config
                .settings
                .upsert_setting_json(
                    SETTINGS_SCOPE_SYSTEM,
                    key,
                    None,
                    encode_setting_json(&value)?,
                    SETTINGS_SOURCE_TYPED_GRAPHQL,
                    Some(actor.id.clone()),
                )
                .await?;
            changed_keys.push(key.to_string());
        }

        if let Some(value) = input.import_mode {
            self.services
                .config
                .settings
                .upsert_setting_json(
                    SETTINGS_SCOPE_SYSTEM,
                    IMPORT_MODE_KEY,
                    Some(facet.as_str().to_string()),
                    encode_setting_json(&value.as_str())?,
                    SETTINGS_SOURCE_TYPED_GRAPHQL,
                    Some(actor.id.clone()),
                )
                .await?;
            changed_keys.push(IMPORT_MODE_KEY.to_string());
        }

        if facet == MediaFacet::Anime {
            if let Some(value) = normalize_optional_string(input.filler_policy) {
                self.services
                    .config
                    .settings
                    .upsert_setting_json(
                        SETTINGS_SCOPE_SYSTEM,
                        ANIME_FILLER_POLICY_KEY,
                        Some(facet.as_str().to_string()),
                        encode_setting_json(&value)?,
                        SETTINGS_SOURCE_TYPED_GRAPHQL,
                        Some(actor.id.clone()),
                    )
                    .await?;
                changed_keys.push(ANIME_FILLER_POLICY_KEY.to_string());
            }
            if let Some(value) = normalize_optional_string(input.recap_policy) {
                self.services
                    .config
                    .settings
                    .upsert_setting_json(
                        SETTINGS_SCOPE_SYSTEM,
                        ANIME_RECAP_POLICY_KEY,
                        Some(facet.as_str().to_string()),
                        encode_setting_json(&value)?,
                        SETTINGS_SOURCE_TYPED_GRAPHQL,
                        Some(actor.id.clone()),
                    )
                    .await?;
                changed_keys.push(ANIME_RECAP_POLICY_KEY.to_string());
            }
            if let Some(value) = input.monitor_specials {
                self.services
                    .config
                    .settings
                    .upsert_setting_json(
                        SETTINGS_SCOPE_SYSTEM,
                        ANIME_MONITOR_SPECIALS_KEY,
                        Some(facet.as_str().to_string()),
                        encode_setting_json(&value)?,
                        SETTINGS_SOURCE_TYPED_GRAPHQL,
                        Some(actor.id.clone()),
                    )
                    .await?;
                changed_keys.push(ANIME_MONITOR_SPECIALS_KEY.to_string());
            }
            if let Some(value) = input.inter_season_movies {
                self.services
                    .config
                    .settings
                    .upsert_setting_json(
                        SETTINGS_SCOPE_SYSTEM,
                        ANIME_INTER_SEASON_MOVIES_KEY,
                        Some(facet.as_str().to_string()),
                        encode_setting_json(&value)?,
                        SETTINGS_SOURCE_TYPED_GRAPHQL,
                        Some(actor.id.clone()),
                    )
                    .await?;
                changed_keys.push(ANIME_INTER_SEASON_MOVIES_KEY.to_string());
            }
            if let Some(value) = input.monitor_filler_movies {
                self.services
                    .config
                    .settings
                    .upsert_setting_json(
                        SETTINGS_SCOPE_SYSTEM,
                        ANIME_MONITOR_FILLER_MOVIES_KEY,
                        Some(facet.as_str().to_string()),
                        encode_setting_json(&value)?,
                        SETTINGS_SOURCE_TYPED_GRAPHQL,
                        Some(actor.id.clone()),
                    )
                    .await?;
                changed_keys.push(ANIME_MONITOR_FILLER_MOVIES_KEY.to_string());
            }
        } else if input.filler_policy.is_some()
            || input.recap_policy.is_some()
            || input.monitor_specials.is_some()
            || input.inter_season_movies.is_some()
            || input.monitor_filler_movies.is_some()
        {
            return Err(AppError::Validation(
                "anime-specific settings require scope anime".to_string(),
            ));
        }

        if changed_keys.is_empty() {
            return Err(AppError::Validation(
                "at least one media setting change is required".to_string(),
            ));
        }

        let current_roots = self.effective_scan_roots_for_facet(&facet).await?;
        self.clear_pending_imports_for_removed_roots(&facet, &previous_roots, &current_roots)
            .await?;

        self.emit_settings_saved(
            actor,
            "media_settings",
            Some(facet.as_str().to_string()),
            changed_keys,
        )
        .await;

        self.get_media_settings(actor, facet).await
    }
}

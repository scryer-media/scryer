static PEM_CERT_BLOCK_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)-----BEGIN CERTIFICATE-----.*?-----END CERTIFICATE-----")
        .expect("valid certificate PEM regex")
});
fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}
fn parse_json_object(raw_json: &str) -> Option<serde_json::Map<String, serde_json::Value>> {
    serde_json::from_str::<serde_json::Value>(raw_json)
        .ok()?
        .as_object()
        .cloned()
}
fn encode_setting_json<T: Serialize>(value: &T) -> AppResult<String> {
    serde_json::to_string(value).map_err(|error| AppError::Repository(error.to_string()))
}
fn plexmatch_write_on_import_key(facet: &MediaFacet) -> Option<&'static str> {
    match facet {
        MediaFacet::Movie => None,
        MediaFacet::Series => Some(PLEXMATCH_WRITE_ON_IMPORT_SERIES_KEY),
        MediaFacet::Anime => Some(PLEXMATCH_WRITE_ON_IMPORT_ANIME_KEY),
    }
}
fn normalize_root_path_for_compare(path: &str) -> String {
    let normalized = path.trim().trim_end_matches(['/', '\\']);

    #[cfg(windows)]
    {
        normalized.replace('/', "\\").to_ascii_lowercase()
    }
    #[cfg(not(windows))]
    {
        normalized.to_string()
    }
}
fn normalize_effective_scan_root(path: &str) -> Option<String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return None;
    }

    Some(Path::new(trimmed).to_string_lossy().trim().to_string())
}
impl AppUseCase {
    async fn effective_scan_roots_for_facet(&self, facet: &MediaFacet) -> AppResult<Vec<String>> {
        let root_folders = self.root_folders_for_facet(facet).await?;
        Ok(effective_scan_roots_from_root_folders(&root_folders))
    }
}
impl AppUseCase {
    pub(crate) async fn clear_pending_imports_for_removed_roots(
        &self,
        facet: &MediaFacet,
        previous_roots: &[String],
        current_roots: &[String],
    ) -> AppResult<()> {
        let current = current_roots.iter().cloned().collect::<HashSet<_>>();

        for removed_root in previous_roots
            .iter()
            .filter(|root| !current.contains(root.as_str()))
        {
            let count = self
                .services
                .library
                .library_scan_unmatched_items
                .count_library_scan_unmatched_items(Some(facet.clone()), Some(removed_root), None)
                .await?;
            if count <= 0 {
                continue;
            }

            let items = self
                .services
                .library
                .library_scan_unmatched_items
                .list_library_scan_unmatched_items(
                    Some(facet.clone()),
                    Some(removed_root),
                    None,
                    count,
                    0,
                )
                .await?;

            for item in items {
                self.services
                    .library
                    .library_scan_unmatched_items
                    .delete_library_scan_unmatched_item(
                        &item.library_id,
                        item.facet.clone(),
                        &item.item_path,
                    )
                    .await?;
            }
        }

        Ok(())
    }
}
impl AppUseCase {
    async fn ensure_default_facet_libraries(&self) -> AppResult<()> {
        for facet in [MediaFacet::Movie, MediaFacet::Series, MediaFacet::Anime] {
            self.ensure_default_facet_library(&facet).await?;
        }

        Ok(())
    }
}
impl AppUseCase {
    pub(crate) async fn emit_settings_saved(
        &self,
        actor: &User,
        resource_type: &str,
        resource_id: Option<String>,
        changed_keys: Vec<String>,
    ) {
        self.emit_configuration_changed_event(
            actor,
            resource_type.to_string(),
            resource_id,
            scryer_domain::ConfigurationChangeAction::Updated,
        )
        .await;

        self.publish_settings_changed(changed_keys);
    }
}
impl AppUseCase {
    pub(crate) async fn read_setting_bool_value(
        &self,
        key_name: &str,
        scope_id: Option<&str>,
    ) -> AppResult<Option<bool>> {
        Ok(self
            .read_setting_string_value(key_name, scope_id)
            .await?
            .and_then(|value| match value.trim().to_ascii_lowercase().as_str() {
                "true" | "1" | "yes" | "on" => Some(true),
                "false" | "0" | "no" | "off" => Some(false),
                _ => None,
            }))
    }
}
impl AppUseCase {
    pub(crate) async fn read_setting_bool_value_explicit(
        &self,
        key_name: &str,
        scope_id: Option<&str>,
    ) -> AppResult<Option<bool>> {
        Ok(self
            .read_setting_string_value_explicit(key_name, scope_id)
            .await?
            .and_then(|value| match value.trim().to_ascii_lowercase().as_str() {
                "true" | "1" | "yes" | "on" => Some(true),
                "false" | "0" | "no" | "off" => Some(false),
                _ => None,
            }))
    }
}
impl AppUseCase {
    pub(crate) async fn read_setting_i64_value(
        &self,
        key_name: &str,
        scope_id: Option<&str>,
    ) -> AppResult<Option<i64>> {
        Ok(self
            .read_setting_string_value(key_name, scope_id)
            .await?
            .and_then(|value| value.parse::<i64>().ok()))
    }
}
impl AppUseCase {
    pub(crate) async fn read_setting_json_value<T: DeserializeOwned>(
        &self,
        key_name: &str,
        scope_id: Option<&str>,
    ) -> AppResult<Option<T>> {
        let Some(raw_value) = self.read_setting_string_value(key_name, scope_id).await? else {
            return Ok(None);
        };
        serde_json::from_str::<T>(&raw_value)
            .map(Some)
            .map_err(|error| {
                AppError::Repository(format!(
                    "failed to parse setting '{key_name}' JSON value: {error}"
                ))
            })
    }
}
impl AppUseCase {
    pub async fn smg_version_compatibility_notice(
        &self,
    ) -> AppResult<Option<crate::SmgVersionCompatibilityNotice>> {
        self.read_setting_json_value(SMG_VERSION_COMPATIBILITY_NOTICE_KEY, None)
            .await
    }

    pub async fn smg_scryer_update_notice(
        &self,
    ) -> AppResult<Option<crate::SmgScryerUpdateNotice>> {
        self.read_setting_json_value(SMG_SCRYER_UPDATE_NOTICE_KEY, None)
            .await
    }
}
impl AppUseCase {
    pub(crate) async fn upsert_system_setting_json<T: Serialize>(
        &self,
        key_name: &str,
        value: &T,
        updated_by_user_id: Option<String>,
    ) -> AppResult<()> {
        let value_json = serde_json::to_string(value)
            .map_err(|error| AppError::Repository(error.to_string()))?;
        self.services
            .config
            .settings
            .upsert_setting_json(
                SETTINGS_SCOPE_SYSTEM,
                key_name,
                None,
                value_json,
                SETTINGS_SOURCE_TYPED_GRAPHQL,
                updated_by_user_id,
            )
            .await
    }
}
impl AppUseCase {
    async fn upsert_media_setting_json<T: Serialize>(
        &self,
        key_name: &str,
        value: &T,
        updated_by_user_id: Option<String>,
    ) -> AppResult<()> {
        let value_json = serde_json::to_string(value)
            .map_err(|error| AppError::Repository(error.to_string()))?;
        self.services
            .config
            .settings
            .upsert_setting_json(
                SETTINGS_SCOPE_MEDIA,
                key_name,
                None,
                value_json,
                SETTINGS_SOURCE_TYPED_GRAPHQL,
                updated_by_user_id,
            )
            .await
    }
}
impl AppUseCase {
    async fn delete_system_setting(&self, key_name: &str) -> AppResult<()> {
        self.services
            .config
            .settings
            .delete_setting_value(SETTINGS_SCOPE_SYSTEM, key_name, None)
            .await
    }
}
impl AppUseCase {
    async fn upsert_scoped_system_setting_json<T: Serialize>(
        &self,
        key_name: &str,
        scope_id: &str,
        value: &T,
        updated_by_user_id: Option<String>,
    ) -> AppResult<()> {
        let value_json = serde_json::to_string(value)
            .map_err(|error| AppError::Repository(error.to_string()))?;
        self.services
            .config
            .settings
            .upsert_setting_json(
                SETTINGS_SCOPE_SYSTEM,
                key_name,
                Some(scope_id.to_string()),
                value_json,
                SETTINGS_SOURCE_TYPED_GRAPHQL,
                updated_by_user_id,
            )
            .await
    }
}
impl AppUseCase {
    pub(crate) async fn delete_scoped_system_setting(
        &self,
        key_name: &str,
        scope_id: &str,
    ) -> AppResult<()> {
        self.services
            .config
            .settings
            .delete_setting_value(SETTINGS_SCOPE_SYSTEM, key_name, Some(scope_id.to_string()))
            .await
    }
}
impl AppUseCase {
    pub async fn migrate_canonical_audio_persona_settings(&self) -> AppResult<()> {
        if self
            .read_setting_bool_value(AUDIO_PERSONA_MIGRATION_SENTINEL_KEY, None)
            .await?
            == Some(true)
        {
            return Ok(());
        }

        let mut changed_keys = Vec::new();

        let existing_global_persona = parse_scoring_persona_setting(
            self.read_setting_string_value(SCORING_PERSONA_KEY, None)
                .await?,
        );
        let existing_facet_personas = {
            let mut values = HashMap::new();
            for scope_id in ["movie", "series", "anime"] {
                if let Some(persona) = parse_scoring_persona_setting(
                    self.read_setting_string_value_explicit(SCORING_PERSONA_KEY, Some(scope_id))
                        .await?,
                ) {
                    values.insert(scope_id.to_string(), persona);
                }
            }
            values
        };

        let profiles = self
            .services
            .config
            .quality_profiles
            .list_quality_profiles(SETTINGS_SCOPE_SYSTEM, None)
            .await
            .unwrap_or_default();
        let mut selected_profile_ids_by_scope = HashMap::new();
        for scope_id in ["movie", "series", "anime"] {
            selected_profile_ids_by_scope.insert(
                scope_id.to_string(),
                self.read_setting_string_value_explicit(QUALITY_PROFILE_ID_KEY, Some(scope_id))
                    .await?,
            );
        }

        let global_profile_id = self
            .read_setting_string_value(QUALITY_PROFILE_ID_KEY, None)
            .await?;
        let selected_global_profile = global_profile_id
            .as_deref()
            .and_then(|profile_id| profiles.iter().find(|profile| profile.id == profile_id))
            .or_else(|| profiles.first());
        let global_persona = existing_global_persona.unwrap_or_else(|| {
            selected_global_profile
                .map(|profile| profile.criteria.scoring_persona.clone())
                .unwrap_or_default()
        });

        self.upsert_system_setting_json(
            SCORING_PERSONA_KEY,
            &global_persona_as_setting(&global_persona),
            None,
        )
        .await?;
        changed_keys.push(SCORING_PERSONA_KEY.to_string());

        for scope_id in ["movie", "series", "anime"] {
            let selected_profile_id = selected_profile_ids_by_scope
                .get(scope_id)
                .cloned()
                .flatten();
            let profile = selected_profile_id
                .as_deref()
                .and_then(|profile_id| profiles.iter().find(|profile| profile.id == profile_id))
                .or(selected_global_profile);
            let effective_persona = existing_facet_personas
                .get(scope_id)
                .cloned()
                .or_else(|| {
                    profile.and_then(|profile| {
                        profile
                            .criteria
                            .facet_persona_overrides
                            .get(scope_id)
                            .cloned()
                            .or_else(|| Some(profile.criteria.scoring_persona.clone()))
                    })
                })
                .unwrap_or_else(|| global_persona.clone());

            if effective_persona != global_persona {
                self.services
                    .config
                    .settings
                    .upsert_setting_json(
                        SETTINGS_SCOPE_SYSTEM,
                        SCORING_PERSONA_KEY,
                        Some(scope_id.to_string()),
                        serde_json::to_string(&global_persona_as_setting(&effective_persona))
                            .map_err(|error| AppError::Repository(error.to_string()))?,
                        "startup-migration",
                        None,
                    )
                    .await?;
                if !changed_keys.iter().any(|key| key == SCORING_PERSONA_KEY) {
                    changed_keys.push(SCORING_PERSONA_KEY.to_string());
                }
            }
        }

        let managed_required_audio = self
            .services
            .customization
            .rule_sets
            .list_rule_sets_by_managed_key_prefix("convenience:required-audio:")
            .await
            .unwrap_or_default();
        let mut global_required_audio = Vec::new();
        let mut facet_required_audio = HashMap::<String, Vec<String>>::new();
        let mut title_overrides = Vec::<(String, Vec<String>)>::new();

        for rule_set in &managed_required_audio {
            let Some(managed_key) = rule_set.managed_key.as_deref() else {
                continue;
            };
            let languages = extract_languages_from_required_audio_rego(&rule_set.rego_source);
            if let Some(title_id) = managed_key.strip_prefix("convenience:required-audio:title:") {
                title_overrides.push((title_id.to_string(), languages));
            } else if let Some(scope_id) = managed_key.strip_prefix("convenience:required-audio:") {
                if scope_id == "global" {
                    global_required_audio = languages;
                } else {
                    facet_required_audio.insert(scope_id.to_string(), languages);
                }
            }
        }

        for scope_id in ["movie", "series", "anime"] {
            let current = self.load_facet_required_audio_languages(scope_id).await?;
            if !current.is_empty() {
                continue;
            }

            let migrated = facet_required_audio
                .get(scope_id)
                .cloned()
                .or_else(|| {
                    (!global_required_audio.is_empty()).then(|| global_required_audio.clone())
                })
                .or_else(|| {
                    let selected_profile_id = selected_profile_ids_by_scope
                        .get(scope_id)
                        .cloned()
                        .flatten();
                    selected_profile_id
                        .as_deref()
                        .and_then(|profile_id| {
                            profiles.iter().find(|profile| profile.id == profile_id)
                        })
                        .or(selected_global_profile)
                        .map(|profile| {
                            normalize_required_audio_requirements(
                                profile.criteria.required_audio_languages.clone(),
                            )
                        })
                })
                .unwrap_or_default();

            self.services
                .config
                .settings
                .upsert_setting_json(
                    SETTINGS_SCOPE_SYSTEM,
                    REQUIRED_AUDIO_LANGUAGES_KEY,
                    Some(scope_id.to_string()),
                    serde_json::to_string(&migrated)
                        .map_err(|error| AppError::Repository(error.to_string()))?,
                    "startup-migration",
                    None,
                )
                .await?;
            if !changed_keys
                .iter()
                .any(|key| key == REQUIRED_AUDIO_LANGUAGES_KEY)
            {
                changed_keys.push(REQUIRED_AUDIO_LANGUAGES_KEY.to_string());
            }
        }

        for (title_id, languages) in title_overrides {
            self.services
                .config
                .settings
                .upsert_setting_json(
                    SETTINGS_SCOPE_SYSTEM,
                    TITLE_REQUIRED_AUDIO_OVERRIDE_KEY,
                    Some(title_id),
                    serde_json::to_string(&Some(languages))
                        .map_err(|error| AppError::Repository(error.to_string()))?,
                    "startup-migration",
                    None,
                )
                .await?;
            if !changed_keys
                .iter()
                .any(|key| key == TITLE_REQUIRED_AUDIO_OVERRIDE_KEY)
            {
                changed_keys.push(TITLE_REQUIRED_AUDIO_OVERRIDE_KEY.to_string());
            }
        }

        for rule_set in managed_required_audio {
            self.services
                .customization
                .rule_sets
                .delete_rule_set(&rule_set.id)
                .await?;
        }

        let legacy_dual_rules = self
            .services
            .customization
            .rule_sets
            .list_rule_sets()
            .await?;
        for rule_set in legacy_dual_rules {
            let managed_key = rule_set.managed_key.as_deref().unwrap_or_default();
            let description = rule_set.description.as_str();
            let rego_source = rule_set.rego_source.as_str();
            if managed_key.starts_with("convenience:prefer-dual-audio:")
                || description.contains("legacy-prefer-dual-audio:")
                || rego_source.contains("legacy-prefer-dual-audio:")
            {
                self.services
                    .customization
                    .rule_sets
                    .delete_rule_set(&rule_set.id)
                    .await?;
            }
        }

        let scrubbed_profiles: Vec<crate::QualityProfile> = profiles
            .into_iter()
            .map(|mut profile| {
                profile.criteria.prefer_dual_audio = false;
                profile.criteria.required_audio_languages.clear();
                profile.criteria.scoring_persona = ScoringPersona::Balanced;
                profile.criteria.facet_persona_overrides.clear();
                profile
            })
            .collect();
        self.services
            .config
            .quality_profiles
            .replace_quality_profiles(SETTINGS_SCOPE_SYSTEM, None, scrubbed_profiles)
            .await?;
        if !changed_keys
            .iter()
            .any(|key| key == QUALITY_PROFILE_CATALOG_KEY)
        {
            changed_keys.push(QUALITY_PROFILE_CATALOG_KEY.to_string());
        }

        if !changed_keys.is_empty() {
            let _ = self
                .runtime
                .events
                .settings_changed_broadcast
                .send(changed_keys);
        }

        self.services
            .config
            .settings
            .upsert_setting_json(
                SETTINGS_SCOPE_SYSTEM,
                AUDIO_PERSONA_MIGRATION_SENTINEL_KEY,
                None,
                serde_json::to_string(&true)
                    .map_err(|error| AppError::Repository(error.to_string()))?,
                "startup-migration",
                None,
            )
            .await?;

        Ok(())
    }
}
impl AppUseCase {
    fn normalize_recycle_config_path(path: &Path) -> PathBuf {
        let mut normalized = PathBuf::new();
        for component in path.components() {
            match component {
                std::path::Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
                std::path::Component::RootDir => normalized.push(component.as_os_str()),
                std::path::Component::CurDir => {}
                std::path::Component::ParentDir => {
                    normalized.pop();
                }
                std::path::Component::Normal(segment) => normalized.push(segment),
            }
        }
        normalized
    }
}
impl AppUseCase {
    pub async fn queue_tvdb_movies_scan(
        &self,
        actor: &User,
        limit: i64,
        source: &str,
    ) -> AppResult<WorkflowOperationInfo> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        if limit <= 0 {
            return Err(AppError::Validation(
                "limit is required and must be greater than zero".into(),
            ));
        }

        let source = source.trim();
        if source.is_empty() {
            return Err(AppError::Validation("source is required".into()));
        }

        self.services
            .workflow
            .workflow_operations
            .create_workflow_operation(
                "tvdb_movies_scan".to_string(),
                "queued".to_string(),
                Some(actor.id.clone()),
                Some(
                    serde_json::json!({
                        "type": "tvdb_movies_scan",
                        "limit": limit,
                        "source": source,
                    })
                    .to_string(),
                ),
                None,
                None,
            )
            .await
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualityProfileSelection {
    pub facet: MediaFacet,
    pub override_profile_id: Option<String>,
    pub effective_profile_id: String,
    pub inherits_global: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FacetScoringPersonaSelection {
    pub facet: MediaFacet,
    pub override_persona: Option<ScoringPersona>,
    pub effective_persona: ScoringPersona,
    pub inherits_global: bool,
}
#[derive(Debug, Clone)]
pub struct QualityProfileSettings {
    pub profiles: Vec<crate::QualityProfile>,
    pub global_profile_id: String,
    pub global_scoring_persona: ScoringPersona,
    pub category_selections: Vec<QualityProfileSelection>,
    pub category_persona_selections: Vec<FacetScoringPersonaSelection>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateQualityProfileSelection {
    pub facet: MediaFacet,
    pub inherit_global: bool,
    pub profile_id: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateFacetScoringPersonaSelection {
    pub facet: MediaFacet,
    pub inherit_global: bool,
    pub persona: Option<ScoringPersona>,
}
#[derive(Debug, Clone)]
pub struct SaveQualityProfileSettings {
    pub profiles: Vec<crate::QualityProfile>,
    pub replace_existing: bool,
    pub global_profile_id: Option<String>,
    pub category_selections: Vec<UpdateQualityProfileSelection>,
    pub global_scoring_persona: Option<ScoringPersona>,
    pub category_persona_selections: Vec<UpdateFacetScoringPersonaSelection>,
}
fn ensure_quality_profiles_exist(
    mut profiles: Vec<crate::QualityProfile>,
) -> Vec<crate::QualityProfile> {
    if profiles.is_empty() {
        profiles.push(crate::builtin_default_quality_profile());
        profiles.push(crate::builtin_4k_profile());
    }

    profiles
}

pub(crate) fn quality_profile_ids_equal(left: &str, right: &str) -> bool {
    left.trim().eq_ignore_ascii_case(right.trim())
}

pub(crate) fn quality_profile_by_id<'a>(
    profiles: &'a [crate::QualityProfile],
    profile_id: &str,
) -> AppResult<Option<&'a crate::QualityProfile>> {
    let profile_id = profile_id.trim();
    if profile_id.is_empty() {
        return Ok(None);
    }
    if let Some(profile) = profiles.iter().find(|profile| profile.id == profile_id) {
        return Ok(Some(profile));
    }

    let mut matches = profiles
        .iter()
        .filter(|profile| quality_profile_ids_equal(&profile.id, profile_id));
    let Some(profile) = matches.next() else {
        return Ok(None);
    };
    if matches.next().is_some() {
        return Err(AppError::Validation(format!(
            "quality profile id '{profile_id}' is ambiguous because profile ids differ only by ASCII case"
        )));
    }
    Ok(Some(profile))
}

fn ensure_unique_quality_profile_ids(profiles: &[crate::QualityProfile]) -> AppResult<()> {
    let mut seen = HashMap::<String, String>::new();
    for profile in profiles {
        let profile_id = profile.id.trim();
        if profile_id.is_empty() {
            return Err(AppError::Validation(
                "quality profile id is required".to_string(),
            ));
        }
        let identity = profile_id.to_ascii_lowercase();
        if let Some(existing) = seen.insert(identity, profile_id.to_string()) {
            return Err(AppError::Validation(format!(
                "quality profile ids '{existing}' and '{profile_id}' differ only by ASCII case"
            )));
        }
    }
    Ok(())
}

fn profile_id_from_setting_json(raw: &str) -> Option<String> {
    let value = serde_json::from_str::<String>(raw)
        .unwrap_or_else(|_| raw.trim().to_string());
    normalize_optional_string(Some(value))
        .filter(|profile_id| profile_id != QUALITY_PROFILE_INHERIT_VALUE)
}

fn request_profile_ids_from_setting_json(raw: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(raw)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|profile_id| normalize_optional_string(Some(profile_id)))
        .filter(|profile_id| profile_id != QUALITY_PROFILE_INHERIT_VALUE)
        .collect()
}

fn resolve_global_profile_id(
    profiles: &[crate::QualityProfile],
    candidate: Option<String>,
) -> AppResult<String> {
    let trimmed = candidate.unwrap_or_default();
    if let Some(profile) = quality_profile_by_id(profiles, &trimmed)? {
        return Ok(profile.id.clone());
    }

    // A candidate that no longer resolves falls back to the canonical
    // built-in default when the catalog still carries it. A catalog that
    // replaced the built-ins (the setup wizard does) falls to its first
    // profile instead — the fallback must always name a real profile.
    if let Some(profile) =
        quality_profile_by_id(profiles, crate::BUILTIN_DEFAULT_QUALITY_PROFILE_ID)?
    {
        return Ok(profile.id.clone());
    }
    Ok(profiles
        .first()
        .map(|profile| profile.id.clone())
        .unwrap_or_else(|| crate::BUILTIN_DEFAULT_QUALITY_PROFILE_ID.to_string()))
}
fn merge_quality_profiles(
    existing: Vec<crate::QualityProfile>,
    updates: Vec<crate::QualityProfile>,
) -> Vec<crate::QualityProfile> {
    let mut merged = existing;
    for mut update in updates {
        if let Some(index) = merged
            .iter()
            .position(|profile| quality_profile_ids_equal(&profile.id, &update.id))
        {
            update.id = merged[index].id.clone();
            merged[index] = update;
        } else {
            merged.push(update);
        }
    }
    merged
}
fn normalize_delay_profile(mut profile: crate::DelayProfile) -> crate::DelayProfile {
    profile.id = profile.id.trim().to_string();
    profile.name = profile.name.trim().to_string();

    let mut seen_facets = HashSet::new();
    profile.applies_to_facets = profile
        .applies_to_facets
        .into_iter()
        .filter_map(|facet| MediaFacet::parse(&facet).map(|parsed| parsed.as_str().to_string()))
        .filter(|facet| seen_facets.insert(facet.clone()))
        .collect();

    let mut seen_tags = HashSet::new();
    profile.tags = profile
        .tags
        .into_iter()
        .map(|tag| tag.trim().to_string())
        .filter(|tag| !tag.is_empty())
        .filter(|tag| seen_tags.insert(tag.to_ascii_lowercase()))
        .collect();

    profile
}
fn parse_scoring_persona_setting(value: Option<String>) -> Option<ScoringPersona> {
    match value?.trim() {
        "Balanced" | "balanced" => Some(ScoringPersona::Balanced),
        "Audiophile" | "audiophile" => Some(ScoringPersona::Audiophile),
        "Efficient" | "efficient" => Some(ScoringPersona::Efficient),
        "Compatible" | "compatible" => Some(ScoringPersona::Compatible),
        _ => None,
    }
}
fn global_persona_as_setting(persona: &ScoringPersona) -> &'static str {
    match persona {
        ScoringPersona::Balanced => "balanced",
        ScoringPersona::Audiophile => "audiophile",
        ScoringPersona::Efficient => "efficient",
        ScoringPersona::Compatible => "compatible",
    }
}
impl AppUseCase {
    async fn resolve_quality_profile_id(
        &self,
        library_id: Option<&str>,
        scope_id: Option<&str>,
    ) -> AppResult<String> {
        if let Some(library_id) = library_id
            && let Some(profile_id) = self
                .read_setting_string_value_explicit(QUALITY_PROFILE_ID_KEY, Some(library_id))
                .await?
                .and_then(|value| normalize_optional_string(Some(value)))
        {
            return self.canonical_quality_profile_id(&profile_id).await;
        }
        if let Some(scope_id) = scope_id
            && let Some(profile_id) = self
                .read_setting_string_value_explicit(QUALITY_PROFILE_ID_KEY, Some(scope_id))
                .await?
                .and_then(|value| normalize_optional_string(Some(value)))
        {
            return self.canonical_quality_profile_id(&profile_id).await;
        }
        if let Some(profile_id) = self
            .read_setting_string_value(QUALITY_PROFILE_ID_KEY, None)
            .await?
            .and_then(|value| normalize_optional_string(Some(value)))
        {
            return self.canonical_quality_profile_id(&profile_id).await;
        }
        Ok(crate::BUILTIN_DEFAULT_QUALITY_PROFILE_ID.to_string())
    }
}
impl AppUseCase {
    /// Resolves the effective quality-profile label for a catalog page without
    /// fetching per-title settings or profile data.
    pub async fn list_title_effective_quality_summaries(
        &self,
        actor: &User,
        title_ids: &[String],
    ) -> AppResult<Vec<crate::TitleQualitySummary>> {
        let titles = self.get_titles_by_ids(actor, title_ids).await?;
        if titles.is_empty() {
            return Ok(Vec::new());
        }

        let settings = self.load_quality_profile_settings().await?;
        let profile_names = settings
            .profiles
            .iter()
            .map(|profile| (profile.id.to_ascii_lowercase(), profile.name.clone()))
            .collect::<HashMap<_, _>>();
        let facet_profile_ids = settings
            .category_selections
            .into_iter()
            .map(|selection| (selection.facet, selection.effective_profile_id))
            .collect::<HashMap<_, _>>();
        let library_ids = titles
            .iter()
            .map(|title| title.library_id.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let library_profile_ids = self
            .services
            .config
            .settings
            .list_setting_json_explicit_for_scope_ids(
                SETTINGS_SCOPE_SYSTEM,
                QUALITY_PROFILE_ID_KEY,
                &library_ids,
            )
            .await?
            .into_iter()
            .filter_map(|(library_id, raw_value)| {
                serde_json::from_str::<String>(&raw_value)
                    .ok()
                    .and_then(|profile_id| normalize_optional_string(Some(profile_id)))
                    .filter(|profile_id| profile_names.contains_key(&profile_id.to_ascii_lowercase()))
                    .map(|profile_id| (library_id, profile_id.to_ascii_lowercase()))
            })
            .collect::<HashMap<_, _>>();

        Ok(titles
            .into_iter()
            .filter_map(|title| {
                let title_profile_id = title
                    .tags
                    .iter()
                    .find_map(|tag| tag.strip_prefix("scryer:quality-profile:"))
                    .map(str::trim)
                    .filter(|profile_id| !profile_id.is_empty())
                    .map(str::to_ascii_lowercase)
                    .filter(|profile_id| profile_names.contains_key(profile_id));
                let profile_id = title_profile_id
                    .or_else(|| library_profile_ids.get(&title.library_id).cloned())
                    .or_else(|| {
                        facet_profile_ids
                            .get(&title.facet)
                            .map(|profile_id| profile_id.to_ascii_lowercase())
                    })
                    .unwrap_or_else(|| settings.global_profile_id.to_ascii_lowercase());
                profile_names.get(&profile_id).cloned().map(|quality_tier| {
                    crate::TitleQualitySummary {
                        title_id: title.id,
                        quality_tier,
                    }
                })
            })
            .collect())
    }
}
impl AppUseCase {
    pub(crate) async fn delay_profiles(&self) -> AppResult<Vec<crate::DelayProfile>> {
        let profiles = self
            .read_setting_json_value::<Vec<crate::DelayProfile>>(
                crate::delay_profile::DELAY_PROFILE_CATALOG_KEY,
                None,
            )
            .await?
            .unwrap_or_default()
            .into_iter()
            .map(normalize_delay_profile)
            .collect::<Vec<_>>();

        crate::validate_delay_profile_catalog(&profiles).map_err(AppError::Validation)?;

        Ok(profiles)
    }
}
impl AppUseCase {
    pub async fn get_delay_profiles(&self, actor: &User) -> AppResult<Vec<crate::DelayProfile>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;
        self.delay_profiles().await
    }
}
impl AppUseCase {
    pub(crate) async fn load_quality_profile_settings(&self) -> AppResult<QualityProfileSettings> {
        let profiles = ensure_quality_profiles_exist(
            self.services
                .config
                .quality_profiles
                .list_quality_profiles(SETTINGS_SCOPE_SYSTEM, None)
                .await?,
        );
        let global_profile_id = resolve_global_profile_id(
            &profiles,
            self.read_setting_string_value(QUALITY_PROFILE_ID_KEY, None)
                .await?,
        )?;
        let global_scoring_persona = parse_scoring_persona_setting(
            self.read_setting_string_value(SCORING_PERSONA_KEY, None)
                .await?,
        )
        .unwrap_or_default();

        let mut category_selections = Vec::with_capacity(3);
        let mut category_persona_selections = Vec::with_capacity(3);
        for facet in [MediaFacet::Movie, MediaFacet::Series, MediaFacet::Anime] {
            let override_profile_id = self
                .read_setting_string_value_explicit(QUALITY_PROFILE_ID_KEY, Some(facet.as_str()))
                .await?
                .map(|value| {
                    quality_profile_by_id(&profiles, &value)
                        .map(|profile| profile.map(|profile| profile.id.clone()))
                })
                .transpose()?
                .flatten();
            let effective_profile_id = override_profile_id
                .clone()
                .unwrap_or_else(|| global_profile_id.clone());
            category_selections.push(QualityProfileSelection {
                facet: facet.clone(),
                inherits_global: override_profile_id.is_none(),
                override_profile_id,
                effective_profile_id,
            });

            let override_persona = parse_scoring_persona_setting(
                self.read_setting_string_value_explicit(SCORING_PERSONA_KEY, Some(facet.as_str()))
                    .await?,
            );
            let effective_persona = override_persona
                .clone()
                .unwrap_or_else(|| global_scoring_persona.clone());
            category_persona_selections.push(FacetScoringPersonaSelection {
                facet,
                inherits_global: override_persona.is_none(),
                override_persona,
                effective_persona,
            });
        }

        Ok(QualityProfileSettings {
            profiles,
            global_profile_id,
            global_scoring_persona,
            category_selections,
            category_persona_selections,
        })
    }
}
impl AppUseCase {
    pub async fn get_quality_profile_settings(
        &self,
        actor: &User,
    ) -> AppResult<QualityProfileSettings> {
        let can_manage_catalog = self
            .has_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;
        let can_manage_titles = self
            .has_any_library_permission(actor, scryer_domain::LibraryPermission::ManageTitles)
            .await?;
        let can_manage_library = self
            .has_any_library_permission(
                actor,
                scryer_domain::LibraryPermission::ManageLibrary,
            )
            .await?;
        let can_request = self
            .has_any_library_permission(actor, scryer_domain::LibraryPermission::Request)
            .await?;
        if !can_manage_catalog && !can_manage_titles && !can_manage_library && !can_request {
            return Err(AppError::Unauthorized(
                "You do not have permission to view quality profiles".to_string(),
            ));
        }
        self.load_quality_profile_settings().await
    }
}
impl AppUseCase {
    pub async fn canonical_quality_profile_id(&self, profile_id: &str) -> AppResult<String> {
        let profile_id = profile_id.trim();
        if profile_id.is_empty() {
            return Err(AppError::Validation("quality profile id is required".to_string()));
        }
        let profiles = ensure_quality_profiles_exist(
            self.services
                .config
                .quality_profiles
                .list_quality_profiles(SETTINGS_SCOPE_SYSTEM, None)
                .await?,
        );
        quality_profile_by_id(&profiles, profile_id)?
            .map(|profile| profile.id.clone())
            .ok_or_else(|| AppError::Validation(format!(
                "unknown quality profile '{profile_id}'"
            )))
    }

    pub async fn validate_quality_profile_id(&self, profile_id: &str) -> AppResult<()> {
        self.canonical_quality_profile_id(profile_id).await.map(|_| ())
    }

    pub(crate) async fn canonicalize_title_quality_profile_tags(
        &self,
        tags: &mut [String],
    ) -> AppResult<()> {
        for tag in tags {
            let Some(profile_id) = tag
                .strip_prefix("scryer:quality-profile:")
                .map(str::trim)
                .filter(|profile_id| {
                    !profile_id.is_empty() && *profile_id != QUALITY_PROFILE_INHERIT_VALUE
                })
            else {
                continue;
            };
            let canonical_id = self.canonical_quality_profile_id(profile_id).await?;
            *tag = format!("scryer:quality-profile:{canonical_id}");
        }
        Ok(())
    }

    async fn ensure_quality_profiles_are_unreferenced(
        &self,
        removed_profile_ids: &HashSet<String>,
        current: &QualityProfileSettings,
    ) -> AppResult<()> {
        if removed_profile_ids.is_empty() {
            return Ok(());
        }

        let referenced = |profile_id: &str| {
            removed_profile_ids
                .iter()
                .any(|removed| quality_profile_ids_equal(removed, profile_id))
        };
        if referenced(&current.global_profile_id) {
            return Err(AppError::Validation(format!(
                "cannot remove quality profile '{}' because it is the global default",
                current.global_profile_id
            )));
        }
        if let Some(selection) = current.category_selections.iter().find(|selection| {
            selection
                .override_profile_id
                .as_deref()
                .is_some_and(referenced)
        }) {
            return Err(AppError::Validation(format!(
                "cannot remove quality profile '{}' because it is configured for {}",
                selection.override_profile_id.as_deref().unwrap_or_default(),
                selection.facet.as_str()
            )));
        }

        let libraries = self.services.catalog.libraries.list(None).await?;
        let library_ids = libraries
            .iter()
            .map(|library| library.id.clone())
            .collect::<Vec<_>>();
        let quality_overrides = self
            .services
            .config
            .settings
            .list_setting_json_explicit_for_scope_ids(
                SETTINGS_SCOPE_SYSTEM,
                QUALITY_PROFILE_ID_KEY,
                &library_ids,
            )
            .await?;
        if let Some((library_id, profile_id)) = quality_overrides
            .into_iter()
            .filter_map(|(library_id, raw)| {
                profile_id_from_setting_json(&raw).map(|profile_id| (library_id, profile_id))
            })
            .find(|(_, profile_id)| referenced(profile_id))
        {
            return Err(AppError::Validation(format!(
                "cannot remove quality profile '{profile_id}' because library '{library_id}' references it"
            )));
        }

        let request_overrides = self
            .services
            .config
            .settings
            .list_setting_json_explicit_for_scope_ids(
                SETTINGS_SCOPE_SYSTEM,
                REQUEST_QUALITY_PROFILE_IDS_KEY,
                &library_ids,
            )
            .await?;
        if let Some((library_id, profile_id)) = request_overrides
            .into_iter()
            .find_map(|(library_id, raw)| {
                request_profile_ids_from_setting_json(&raw)
                    .into_iter()
                    .find(|profile_id| referenced(profile_id))
                    .map(|profile_id| (library_id, profile_id))
            })
        {
            return Err(AppError::Validation(format!(
                "cannot remove quality profile '{profile_id}' because library '{library_id}' allows it for requests"
            )));
        }

        for profile_id in removed_profile_ids {
            let title_count = self
                .services
                .catalog
                .titles
                .count_by_quality_profile_id(profile_id)
                .await?;
            if title_count > 0 {
                return Err(AppError::Validation(format!(
                    "cannot remove quality profile '{profile_id}' because {title_count} title(s) reference it"
                )));
            }
            let request_references = self
                .services
                .catalog
                .media_requests
                .count_quality_profile_references(profile_id)
                .await?;
            if request_references.pending_requested > 0 {
                return Err(AppError::Validation(format!(
                    "cannot remove quality profile '{profile_id}' because {} pending media request(s) reference it",
                    request_references.pending_requested
                )));
            }
        }

        Ok(())
    }

    pub async fn save_quality_profile_settings(
        &self,
        actor: &User,
        input: SaveQualityProfileSettings,
    ) -> AppResult<QualityProfileSettings> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;
        let _profile_reference_guard = self
            .runtime
            .catalog
            .quality_profile_reference_lock
            .lock()
            .await;

        // GraphQL has historically treated both an omitted globalProfileId and
        // an explicit null as a partial-update no-op. Preserve that patch
        // contract; an empty string is likewise ignored for compatibility.
        let global_profile_id = input
            .global_profile_id
            .map(|profile_id| profile_id.trim().to_string())
            .filter(|profile_id| !profile_id.is_empty());

        let existing_profiles = self
            .services
            .config
            .quality_profiles
            .list_quality_profiles(SETTINGS_SCOPE_SYSTEM, None)
            .await?;
        let profiles = if input.replace_existing {
            input.profiles
        } else {
            merge_quality_profiles(existing_profiles.clone(), input.profiles)
        };
        ensure_unique_quality_profile_ids(&profiles)?;

        let current_profiles = ensure_quality_profiles_exist(if profiles.is_empty() {
            existing_profiles.clone()
        } else {
            profiles.clone()
        });
        let global_profile_id = global_profile_id
            .map(|profile_id| {
                quality_profile_by_id(&current_profiles, &profile_id)?.map_or_else(
                    || Err(AppError::Validation(format!("unknown quality profile '{profile_id}'"))),
                    |profile| Ok(profile.id.clone()),
                )
            })
            .transpose()?;
        for selection in &input.category_selections {
            if !selection.inherit_global {
                let profile_id = selection
                    .profile_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| {
                        AppError::Validation(
                            "profile_id is required when inherit_global is false".to_string(),
                        )
                    })?;
                quality_profile_by_id(&current_profiles, profile_id)?.ok_or_else(|| {
                    AppError::Validation(format!("unknown quality profile '{profile_id}'"))
                })?;
            }
        }

        let removed_profile_ids = existing_profiles
            .iter()
            .map(|profile| profile.id.clone())
            .filter(|profile_id| {
                !profiles
                    .iter()
                    .any(|profile| quality_profile_ids_equal(&profile.id, profile_id))
            })
            .collect::<HashSet<_>>();
        let mut prospective = self.load_quality_profile_settings().await?;
        let global_profile_needs_reconciliation = !current_profiles
            .iter()
            .any(|profile| quality_profile_ids_equal(&profile.id, &prospective.global_profile_id));
        if let Some(global_profile_id) = &global_profile_id {
            prospective.global_profile_id = global_profile_id.clone();
        } else if global_profile_needs_reconciliation {
            // Atomic catalog replacement may remove the previously effective
            // global profile. Reconcile that stale reference to the normal
            // fallback without assigning a destructive meaning to GraphQL null.
            prospective.global_profile_id = resolve_global_profile_id(&current_profiles, None)?;
        }
        for update in &input.category_selections {
            if let Some(selection) = prospective
                .category_selections
                .iter_mut()
                .find(|selection| selection.facet == update.facet)
            {
                selection.override_profile_id = if update.inherit_global {
                    None
                } else {
                    update.profile_id.as_deref().map(str::trim).map(|profile_id| {
                        quality_profile_by_id(&current_profiles, profile_id)
                            .expect("profile id was validated")
                            .expect("profile id exists")
                            .id
                            .clone()
                    })
                };
            }
        }
        self.ensure_quality_profiles_are_unreferenced(&removed_profile_ids, &prospective)
            .await?;

        let mut changed_keys = Vec::new();
        if !profiles.is_empty() {
            self.services
                .config
                .quality_profiles
                .replace_quality_profiles(SETTINGS_SCOPE_SYSTEM, None, profiles.clone())
                .await?;
            self.upsert_system_setting_json(
                QUALITY_PROFILE_CATALOG_KEY,
                &profiles,
                Some(actor.id.clone()),
            )
            .await?;
            changed_keys.push(QUALITY_PROFILE_CATALOG_KEY.to_string());
        }

        if let Some(global_profile_id) = global_profile_id {
            self.upsert_system_setting_json(
                QUALITY_PROFILE_ID_KEY,
                &global_profile_id,
                Some(actor.id.clone()),
            )
            .await?;
            changed_keys.push(QUALITY_PROFILE_ID_KEY.to_string());
        } else if global_profile_needs_reconciliation {
            let builtin_default_available = current_profiles.iter().any(|profile| {
                quality_profile_ids_equal(&profile.id, crate::BUILTIN_DEFAULT_QUALITY_PROFILE_ID)
            });
            if builtin_default_available {
                self.delete_system_setting(QUALITY_PROFILE_ID_KEY).await?;
            } else {
                // The replacement catalog dropped the built-in default, so
                // deleting the row would leave the definition default dangling
                // until the next boot repairs it; persist the reconciled
                // global explicitly instead (mirrors bootstrap normalization).
                self.upsert_system_setting_json(
                    QUALITY_PROFILE_ID_KEY,
                    &prospective.global_profile_id,
                    Some(actor.id.clone()),
                )
                .await?;
            }
            changed_keys.push(QUALITY_PROFILE_ID_KEY.to_string());
        }

        for selection in input.category_selections {
            let value = if selection.inherit_global {
                QUALITY_PROFILE_INHERIT_VALUE.to_string()
            } else {
                let profile_id = selection
                    .profile_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| {
                        AppError::Validation(
                            "profile_id is required when inherit_global is false".to_string(),
                        )
                    })?;
                quality_profile_by_id(&current_profiles, profile_id)?
                    .ok_or_else(|| {
                        AppError::Validation(format!("unknown quality profile '{profile_id}'"))
                    })?
                    .id
                    .clone()
            };

            self.services
                .config
                .settings
                .upsert_setting_json(
                    SETTINGS_SCOPE_SYSTEM,
                    QUALITY_PROFILE_ID_KEY,
                    Some(selection.facet.as_str().to_string()),
                    encode_setting_json(&value)?,
                    SETTINGS_SOURCE_TYPED_GRAPHQL,
                    Some(actor.id.clone()),
                )
                .await?;
            if !changed_keys.iter().any(|key| key == QUALITY_PROFILE_ID_KEY) {
                changed_keys.push(QUALITY_PROFILE_ID_KEY.to_string());
            }
        }

        if let Some(global_scoring_persona) = input.global_scoring_persona {
            self.upsert_system_setting_json(
                SCORING_PERSONA_KEY,
                &global_persona_as_setting(&global_scoring_persona),
                Some(actor.id.clone()),
            )
            .await?;
            if !changed_keys.iter().any(|key| key == SCORING_PERSONA_KEY) {
                changed_keys.push(SCORING_PERSONA_KEY.to_string());
            }
        }

        for selection in input.category_persona_selections {
            let value = if selection.inherit_global {
                QUALITY_PROFILE_INHERIT_VALUE.to_string()
            } else {
                global_persona_as_setting(&selection.persona.ok_or_else(|| {
                    AppError::Validation(
                        "persona is required when inherit_global is false".to_string(),
                    )
                })?)
                .to_string()
            };

            self.services
                .config
                .settings
                .upsert_setting_json(
                    SETTINGS_SCOPE_SYSTEM,
                    SCORING_PERSONA_KEY,
                    Some(selection.facet.as_str().to_string()),
                    encode_setting_json(&value)?,
                    SETTINGS_SOURCE_TYPED_GRAPHQL,
                    Some(actor.id.clone()),
                )
                .await?;
            if !changed_keys.iter().any(|key| key == SCORING_PERSONA_KEY) {
                changed_keys.push(SCORING_PERSONA_KEY.to_string());
            }
        }

        self.emit_configuration_changed_event(
            actor,
            "quality_profiles".to_string(),
            None,
            scryer_domain::ConfigurationChangeAction::Updated,
        )
        .await;
        if !changed_keys.is_empty() {
            self.publish_settings_changed(changed_keys);
        }

        self.load_quality_profile_settings().await
    }
}
impl AppUseCase {
    pub async fn delete_quality_profile(
        &self,
        actor: &User,
        profile_id: &str,
    ) -> AppResult<QualityProfileSettings> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;
        let _profile_reference_guard = self
            .runtime
            .catalog
            .quality_profile_reference_lock
            .lock()
            .await;

        let requested_profile_id = profile_id.trim();
        if requested_profile_id.is_empty() {
            return Err(AppError::Validation("profile_id is required".to_string()));
        }

        let current = self.load_quality_profile_settings().await?;
        let profile_id = quality_profile_by_id(&current.profiles, requested_profile_id)?
            .map(|profile| profile.id.clone())
            .ok_or_else(|| AppError::NotFound(format!("quality profile {requested_profile_id}")))?;
        self.ensure_quality_profiles_are_unreferenced(
            &HashSet::from([profile_id.clone()]),
            &current,
        )
        .await?;

        let remaining_profiles = current
            .profiles
            .into_iter()
            .filter(|profile| profile.id != profile_id)
            .collect::<Vec<_>>();
        self.services
            .config
            .quality_profiles
            .replace_quality_profiles(SETTINGS_SCOPE_SYSTEM, None, remaining_profiles.clone())
            .await?;
        self.upsert_system_setting_json(
            QUALITY_PROFILE_CATALOG_KEY,
            &remaining_profiles,
            Some(actor.id.clone()),
        )
        .await?;

        self.emit_configuration_changed_event(
            actor,
            "quality_profile".to_string(),
            Some(profile_id),
            scryer_domain::ConfigurationChangeAction::Deleted,
        )
        .await;
        self.publish_settings_changed(vec![
            QUALITY_PROFILE_CATALOG_KEY.to_string(),
            QUALITY_PROFILE_ID_KEY.to_string(),
        ]);

        self.load_quality_profile_settings().await
    }
}
impl AppUseCase {
    pub async fn upsert_delay_profile(
        &self,
        actor: &User,
        profile: crate::DelayProfile,
    ) -> AppResult<crate::DelayProfile> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        let profile = normalize_delay_profile(profile);
        if profile.id.is_empty() {
            return Err(AppError::Validation(
                "delay profile id is required".to_string(),
            ));
        }

        let mut profiles = self.delay_profiles().await?;
        if let Some(existing) = profiles
            .iter_mut()
            .find(|existing| existing.id == profile.id)
        {
            *existing = profile.clone();
        } else {
            profiles.push(profile.clone());
        }

        crate::validate_delay_profile_catalog(&profiles).map_err(AppError::Validation)?;
        self.upsert_system_setting_json(
            crate::delay_profile::DELAY_PROFILE_CATALOG_KEY,
            &profiles,
            Some(actor.id.clone()),
        )
        .await?;

        self.emit_configuration_changed_event(
            actor,
            "delay_profile",
            Some(profile.id.clone()),
            scryer_domain::ConfigurationChangeAction::Saved,
        )
        .await;
        let _ = self.runtime.events.settings_changed_broadcast.send(vec![
            crate::delay_profile::DELAY_PROFILE_CATALOG_KEY.to_string(),
        ]);
        self.runtime.acquisition.acquisition_wake.notify_one();

        Ok(profile)
    }
}
impl AppUseCase {
    pub async fn delete_delay_profile(&self, actor: &User, profile_id: &str) -> AppResult<String> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        let profile_id = profile_id.trim().to_string();
        if profile_id.is_empty() {
            return Err(AppError::Validation(
                "delay profile id is required".to_string(),
            ));
        }

        let profiles = self.delay_profiles().await?;
        if !profiles.iter().any(|profile| profile.id == profile_id) {
            return Err(AppError::NotFound(format!("delay profile {profile_id}")));
        }

        let next_profiles: Vec<crate::DelayProfile> = profiles
            .into_iter()
            .filter(|profile| profile.id != profile_id)
            .collect();
        self.upsert_system_setting_json(
            crate::delay_profile::DELAY_PROFILE_CATALOG_KEY,
            &next_profiles,
            Some(actor.id.clone()),
        )
        .await?;

        self.emit_configuration_changed_event(
            actor,
            "delay_profile",
            Some(profile_id.clone()),
            scryer_domain::ConfigurationChangeAction::Deleted,
        )
        .await;
        let _ = self.runtime.events.settings_changed_broadcast.send(vec![
            crate::delay_profile::DELAY_PROFILE_CATALOG_KEY.to_string(),
        ]);
        self.runtime.acquisition.acquisition_wake.notify_one();

        Ok(profile_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaSettings {
    pub library_path: String,
    pub root_folders: Vec<RootFolderEntry>,
    pub required_audio_languages: Vec<String>,
    /// Ranking-only audio language preferences as configured for the facet;
    /// `original` stays unexpanded.
    pub audio_language_preferences: AudioLanguagePreferenceSettings,
    pub use_season_folders: bool,
    pub folder_template: String,
    pub season_folder_template: Option<String>,
    pub specials_folder_template: Option<String>,
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
    pub set_permissions_linux: bool,
    pub file_chmod: Option<String>,
    pub folder_chmod: Option<String>,
    pub chown_group: Option<String>,
    /// The Movie facet's availability gate settings; `None` for other facets.
    pub movie_availability: Option<MovieAvailabilitySettings>,
}
/// Configured audio language preferences for one scope, as an operator wrote
/// them. [`crate::AudioLanguagePreferences`] is the per-title resolution.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AudioLanguagePreferenceSettings {
    pub preferred: Vec<String>,
    pub optional: Vec<String>,
    pub optional_keep_searching: bool,
}

/// What a new title in a library inherits for audio: the library's languages,
/// else its facet's. The `original` token stays unresolved.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LibraryAudioLanguageDefaults {
    pub required: Vec<String>,
    pub preferred: Vec<String>,
}

/// The audio language preferences one scope sets itself, where `None`
/// inherits from the next scope. A library inherits from its facet.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AudioLanguagePreferenceOverrides {
    pub preferred: Option<Vec<String>>,
    pub optional: Option<Vec<String>>,
    pub optional_keep_searching: Option<bool>,
}

impl AudioLanguagePreferenceOverrides {
    /// These overrides, with every value they leave unset taken from `inherited`.
    pub fn over(self, inherited: AudioLanguagePreferenceSettings) -> AudioLanguagePreferenceSettings {
        AudioLanguagePreferenceSettings {
            preferred: self.preferred.unwrap_or(inherited.preferred),
            optional: self.optional.unwrap_or(inherited.optional),
            optional_keep_searching: self
                .optional_keep_searching
                .unwrap_or(inherited.optional_keep_searching),
        }
    }
}

/// Normalizes a preferred or optional audio language list. Unlike required
/// languages, an unknown code is refused rather than dropped, so a typo cannot
/// silently empty the list; spellings of the same language collapse into one.
pub(crate) fn validated_preference_languages(
    label: &str,
    languages: Vec<String>,
) -> AppResult<Vec<String>> {
    if let Some(unknown) = languages.iter().find(|value| {
        !value.trim().is_empty()
            && normalize_required_audio_requirements([value.to_string()]).is_empty()
    }) {
        return Err(AppError::validation_refused_with_facts(
            crate::error_reasons::settings::AUDIO_LANGUAGE_UNKNOWN,
            format!("unknown {label} \"{}\"", unknown.trim()),
            vec![crate::RefusalFact::new(
                crate::error_reasons::settings::AUDIO_LANGUAGE_FACT,
                unknown.trim(),
            )],
        ));
    }
    Ok(normalize_required_audio_requirements(languages))
}

/// A per-title audio language override as a caller supplied it, normalized.
/// `None` and, for preferred languages, an empty list both mean the title
/// inherits; a required list may be empty on purpose, meaning the title
/// requires no language whatever its library or facet asks.
pub(crate) fn validated_title_audio_override(
    label: &str,
    languages: Option<Vec<String>>,
    empty_inherits: bool,
) -> AppResult<Option<Vec<String>>> {
    let Some(languages) = languages else {
        return Ok(None);
    };
    let languages = validated_preference_languages(label, languages)?;
    if empty_inherits && languages.is_empty() {
        return Ok(None);
    }
    Ok(Some(languages))
}

/// One audio language override: `None` inherits.
type AudioLanguageOverride = Option<Vec<String>>;

/// Normalize a title's (or a request's, or a list route's) requested audio
/// language overrides. `required` keeps the `setTitleRequiredAudio`
/// semantics: `None` inherits and an empty list requires nothing. An empty
/// `preferred` list inherits.
pub fn validated_title_audio_overrides(
    required: Option<Vec<String>>,
    preferred: Option<Vec<String>>,
) -> AppResult<(AudioLanguageOverride, AudioLanguageOverride)> {
    Ok((
        validated_title_audio_override("required audio language", required, false)?,
        validated_title_audio_override("preferred audio language", preferred, true)?,
    ))
}

/// The patch with its audio language overrides validated and normalized. A
/// preferred list that normalizes to empty becomes `Some(None)` (inherit); a
/// required list keeps an empty value (require nothing).
pub fn validated_title_audio_patch(
    mut patch: crate::TitleOptionsPatch,
) -> AppResult<crate::TitleOptionsPatch> {
    if let Some(required) = patch.required_audio_languages.take() {
        patch.required_audio_languages = Some(validated_title_audio_override(
            "required audio language",
            required,
            false,
        )?);
    }
    if let Some(preferred) = patch.preferred_audio_languages.take() {
        patch.preferred_audio_languages = Some(validated_title_audio_override(
            "preferred audio language",
            preferred,
            true,
        )?);
    }
    Ok(patch)
}

/// A stored title preferred-audio override; `null` and `[]` inherit.
fn parse_title_preferred_audio_override(raw_value: &str) -> AppResult<Option<Vec<String>>> {
    let parsed = serde_json::from_str::<Option<Vec<String>>>(raw_value).map_err(|error| {
        AppError::Repository(format!(
            "failed to parse setting '{}' JSON value: {error}",
            crate::TITLE_PREFERRED_AUDIO_OVERRIDE_KEY
        ))
    })?;
    Ok(parsed
        .map(normalize_required_audio_requirements)
        .filter(|languages| !languages.is_empty()))
}

/// Stored title preferred-audio rows, keyed by title. A title that inherits is
/// absent, and so is one whose row will not parse: one bad row is logged and
/// read as inheriting rather than failing every other title in the batch.
fn collect_title_preferred_audio_overrides(
    raw_values: Vec<(String, String)>,
) -> HashMap<String, Vec<String>> {
    let mut overrides = HashMap::with_capacity(raw_values.len());
    for (title_id, raw_value) in raw_values {
        match parse_title_preferred_audio_override(&raw_value) {
            Ok(Some(languages)) => {
                overrides.insert(title_id, languages);
            }
            Ok(None) => {}
            Err(error) => tracing::warn!(
                title_id = %title_id,
                error = %error,
                "ignoring a stored preferred audio language override that will not parse; the title inherits"
            ),
        }
    }
    overrides
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateMediaSettings {
    pub required_audio_languages: Option<Vec<String>>,
    pub preferred_audio_languages: Option<Vec<String>>,
    pub optional_audio_languages: Option<Vec<String>>,
    pub optional_audio_languages_keep_searching: Option<bool>,
    pub use_season_folders: Option<bool>,
    pub folder_template: Option<String>,
    pub season_folder_template: Option<String>,
    pub specials_folder_template: Option<String>,
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
    pub set_permissions_linux: Option<bool>,
    pub file_chmod: Option<String>,
    pub folder_chmod: Option<String>,
    pub chown_group: Option<String>,
    /// Movie availability gate settings to set on the Movie facet; unset
    /// fields are left as they are. Valid only for the Movie facet.
    pub movie_availability: MovieAvailabilityOverrides,
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
            AppError::validation_refused(crate::error_reasons::settings::SETTING_VALUE_INVALID, format!(
                "invalid {IMPORT_MODE_KEY} setting value '{}': {message}",
                value.trim()
            ))
        })
    })
    .transpose()
}

fn normalize_chmod_setting(raw: Option<String>, key_name: &str) -> AppResult<Option<String>> {
    let Some(value) = normalize_optional_string(raw) else {
        return Ok(None);
    };
    validate_chmod_setting(&value, key_name)?;
    Ok(Some(value))
}

fn validate_chmod_setting(value: &str, key_name: &str) -> AppResult<()> {
    let len = value.len();
    if !(len == 3 || len == 4) || !value.bytes().all(|byte| (b'0'..=b'7').contains(&byte)) {
        return Err(AppError::validation_refused(crate::error_reasons::settings::SETTING_VALUE_INVALID, format!(
            "invalid {key_name} setting value '{value}': expected a 3 or 4 digit octal mask"
        )));
    }
    Ok(())
}

fn normalize_chown_group_setting(raw: Option<String>) -> AppResult<Option<String>> {
    let Some(value) = normalize_optional_string(raw) else {
        return Ok(None);
    };
    if value.contains('\0') {
        return Err(AppError::validation_refused(crate::error_reasons::settings::PERMISSIONS_CHOWN_GROUP_SETTING_VALUE_INVALID, "invalid permissions.chown_group setting value: group cannot contain NUL".to_string()));
    }
    if value.chars().all(|ch| ch.is_ascii_digit()) {
        value.parse::<u32>().map_err(|_| {
            AppError::validation_refused(crate::error_reasons::settings::PERMISSIONS_CHOWN_GROUP_SETTING_VALUE_INVALID, format!(
                "invalid permissions.chown_group setting value '{value}': numeric gid is outside range"
            ))
        })?;
    }
    Ok(Some(value))
}

fn extract_languages_from_required_audio_rego(rego: &str) -> Vec<String> {
    for line in rego.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("_required_langs := {")
            && let Some(set_body) = rest.strip_suffix('}')
        {
            return normalize_required_audio_requirements(
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
        Ok(normalize_required_audio_requirements(
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
            .map(|value| value.map(normalize_required_audio_requirements))
            .map_err(|error| {
                AppError::Repository(format!(
                    "failed to parse setting '{TITLE_REQUIRED_AUDIO_OVERRIDE_KEY}' JSON value: {error}"
                ))
            })
    }

    /// Batch variant of [`Self::load_title_required_audio_override`]. Returns a
    /// `title_id -> override languages` map containing only titles that carry an
    /// explicit (non-null) override; titles without one are absent. Not
    /// actor-scoped, mirroring the singular.
    pub async fn load_title_required_audio_overrides(
        &self,
        title_ids: &[String],
    ) -> AppResult<HashMap<String, Vec<String>>> {
        if title_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let raw_values = self
            .services
            .config
            .settings
            .list_setting_json_explicit_for_scope_ids(
                SETTINGS_SCOPE_SYSTEM,
                TITLE_REQUIRED_AUDIO_OVERRIDE_KEY,
                title_ids,
            )
            .await?;

        let mut overrides = HashMap::with_capacity(raw_values.len());
        for (title_id, raw_value) in raw_values {
            let parsed =
                serde_json::from_str::<Option<Vec<String>>>(&raw_value).map_err(|error| {
                    AppError::Repository(format!(
                        "failed to parse setting '{TITLE_REQUIRED_AUDIO_OVERRIDE_KEY}' JSON value: {error}"
                    ))
                })?;
            if let Some(languages) = parsed {
                overrides.insert(title_id, normalize_required_audio_requirements(languages));
            }
        }
        Ok(overrides)
    }

    /// A title's own preferred audio languages, in order, or `None` when it
    /// inherits. A stored `null` or empty list inherits too. The `original`
    /// token stays unresolved.
    pub async fn load_title_preferred_audio_override(
        &self,
        title_id: &str,
    ) -> AppResult<Option<Vec<String>>> {
        let raw_value = self
            .services
            .config
            .settings
            .get_setting_json_explicit(
                SETTINGS_SCOPE_SYSTEM,
                crate::TITLE_PREFERRED_AUDIO_OVERRIDE_KEY,
                Some(title_id.to_string()),
            )
            .await?;
        raw_value
            .map(|raw_value| parse_title_preferred_audio_override(&raw_value))
            .transpose()
            .map(Option::flatten)
    }

    /// Batch variant of [`Self::load_title_preferred_audio_override`]: one
    /// read for every title. Titles that inherit are absent from the map.
    pub async fn load_title_preferred_audio_overrides(
        &self,
        title_ids: &[String],
    ) -> AppResult<HashMap<String, Vec<String>>> {
        if title_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let raw_values = self
            .services
            .config
            .settings
            .list_setting_json_explicit_for_scope_ids(
                SETTINGS_SCOPE_SYSTEM,
                crate::TITLE_PREFERRED_AUDIO_OVERRIDE_KEY,
                title_ids,
            )
            .await?;
        Ok(collect_title_preferred_audio_overrides(raw_values))
    }

    /// Every title holding a preferred audio language override, with it, or
    /// `None` when the store cannot list them. An id may name a title that no
    /// longer exists.
    pub(crate) async fn list_title_preferred_audio_overrides(
        &self,
    ) -> AppResult<Option<HashMap<String, Vec<String>>>> {
        Ok(self
            .services
            .config
            .settings
            .list_explicit_setting_json_values(
                SETTINGS_SCOPE_SYSTEM,
                crate::TITLE_PREFERRED_AUDIO_OVERRIDE_KEY,
            )
            .await?
            .map(collect_title_preferred_audio_overrides))
    }

    /// Whether any title may carry a preferred audio language override. A
    /// store that cannot tell answers `true`, so a gate built on this only
    /// ever does more work, never less.
    pub(crate) async fn any_title_preferred_audio_override(&self) -> AppResult<bool> {
        self.services
            .config
            .settings
            .any_explicit_setting_json_for_scope_ids(
                SETTINGS_SCOPE_SYSTEM,
                crate::TITLE_PREFERRED_AUDIO_OVERRIDE_KEY,
            )
            .await
    }

    pub async fn load_title_metadata_language_overrides(
        &self,
        title_ids: &[String],
    ) -> AppResult<HashMap<String, String>> {
        self.load_metadata_language_overrides_for_scope_ids(
            TITLE_METADATA_LANGUAGE_OVERRIDE_KEY,
            title_ids,
        )
        .await
    }

    pub async fn load_library_metadata_language_overrides(
        &self,
        library_ids: &[String],
    ) -> AppResult<HashMap<String, String>> {
        self.load_metadata_language_overrides_for_scope_ids(METADATA_LANGUAGE_KEY, library_ids)
            .await
    }

    async fn load_metadata_language_overrides_for_scope_ids(
        &self,
        key_name: &str,
        scope_ids: &[String],
    ) -> AppResult<HashMap<String, String>> {
        if scope_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let raw_values = self
            .services
            .config
            .settings
            .list_setting_json_explicit_for_scope_ids(SETTINGS_SCOPE_SYSTEM, key_name, scope_ids)
            .await?;
        let mut overrides = HashMap::with_capacity(raw_values.len());
        for (scope_id, raw_value) in raw_values {
            let language = serde_json::from_str::<serde_json::Value>(&raw_value)
                .ok()
                .and_then(|value| value.as_str().map(ToOwned::to_owned))
                .unwrap_or(raw_value);
            if let Some(language) = crate::normalize_metadata_language_code(&language) {
                overrides.insert(scope_id, language);
            }
        }
        Ok(overrides)
    }

    pub async fn load_use_season_folders_overrides(
        &self,
        scope_ids: &[String],
    ) -> AppResult<HashMap<String, bool>> {
        if scope_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let raw_values = self
            .services
            .config
            .settings
            .list_setting_json_explicit_for_scope_ids(
                SETTINGS_SCOPE_SYSTEM,
                USE_SEASON_FOLDERS_KEY,
                scope_ids,
            )
            .await?;
        let mut overrides = HashMap::with_capacity(raw_values.len());
        for (scope_id, raw_value) in raw_values {
            let value = serde_json::from_str::<serde_json::Value>(&raw_value)
                .ok()
                .and_then(|value| value.as_str().map(ToOwned::to_owned))
                .unwrap_or(raw_value);
            if let Some(value) = match value.trim().to_ascii_lowercase().as_str() {
                "true" | "1" | "yes" | "on" => Some(true),
                "false" | "0" | "no" | "off" => Some(false),
                _ => None,
            } {
                overrides.insert(scope_id, value);
            }
        }
        Ok(overrides)
    }
}
impl AppUseCase {
    pub async fn resolve_required_audio_languages(
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

    /// Configured preferred audio languages after title, library and facet
    /// inheritance, in order. An empty list at any scope inherits from the
    /// next one. The `original` token stays unresolved.
    pub async fn resolve_preferred_audio_languages(
        &self,
        title_id: Option<&str>,
        library_id: Option<&str>,
        scope_id: Option<&str>,
    ) -> AppResult<Vec<String>> {
        Ok(self
            .resolve_audio_language_preference_settings(title_id, library_id, scope_id)
            .await?
            .preferred)
    }

    /// The one title → library → facet ladder for preferred and optional
    /// audio languages. Each key resolves on its own; only preferred languages
    /// have a title level. An empty list counts as unset. The `original` token
    /// stays unresolved.
    async fn resolve_audio_language_preference_settings(
        &self,
        title_id: Option<&str>,
        library_id: Option<&str>,
        scope_id: Option<&str>,
    ) -> AppResult<AudioLanguagePreferenceSettings> {
        let title = AudioLanguagePreferenceOverrides {
            preferred: match title_id {
                Some(title_id) => self.load_title_preferred_audio_override(title_id).await?,
                None => None,
            },
            ..AudioLanguagePreferenceOverrides::default()
        };
        let library = match library_id {
            Some(library_id) => {
                self.load_audio_language_preference_overrides(library_id)
                    .await?
            }
            None => AudioLanguagePreferenceOverrides::default(),
        };
        let facet = match scope_id {
            Some(scope_id) => {
                self.load_audio_language_preference_settings(scope_id)
                    .await?
            }
            None => AudioLanguagePreferenceSettings::default(),
        };
        Ok(title.over(library.over(facet)))
    }

    pub(crate) async fn resolve_required_audio_languages_for_title(
        &self,
        title: &scryer_domain::Title,
    ) -> AppResult<Vec<String>> {
        let requirements = self
            .resolve_required_audio_languages(
                Some(&title.id),
                Some(title.library_id.as_str()),
                Some(title.facet.as_str()),
            )
            .await?;
        let title_context = title_audio_language_context(
            title.language.as_deref(),
            title.country.as_deref(),
            Some(title.facet.as_str()),
            &title.tags,
        );
        Ok(resolve_required_audio_requirements(
            &requirements,
            &title_context,
        ))
    }

    /// Preferred and optional audio languages for a title.
    ///
    /// Each key resolves on its own: the library's value when it sets one,
    /// otherwise the facet's. Preferred languages are first taken from the
    /// title's own override. An empty list counts as unset, the same rule
    /// [`Self::resolve_required_audio_languages`] applies.
    pub(crate) async fn resolve_audio_language_preferences_for_title(
        &self,
        title: &scryer_domain::Title,
    ) -> AppResult<crate::AudioLanguagePreferences> {
        let settings = self
            .resolve_audio_language_preference_settings(
                Some(&title.id),
                Some(&title.library_id),
                Some(title.facet.as_str()),
            )
            .await?;
        let title_context = title_audio_language_context(
            title.language.as_deref(),
            title.country.as_deref(),
            Some(title.facet.as_str()),
            &title.tags,
        );
        Ok(crate::AudioLanguagePreferences::resolve(
            &settings.preferred,
            &settings.optional,
            settings.optional_keep_searching,
            &title_context,
        ))
    }

    /// The audio language preferences configured on one scope (a facet name or
    /// a library id), without falling back to any other scope.
    pub(crate) async fn load_audio_language_preference_settings(
        &self,
        scope_id: &str,
    ) -> AppResult<AudioLanguagePreferenceSettings> {
        Ok(self
            .load_audio_language_preference_overrides(scope_id)
            .await?
            .over(AudioLanguagePreferenceSettings::default()))
    }

    /// What one scope sets itself; every value it leaves unset is `None`.
    pub(crate) async fn load_audio_language_preference_overrides(
        &self,
        scope_id: &str,
    ) -> AppResult<AudioLanguagePreferenceOverrides> {
        let list = |languages: Vec<String>| (!languages.is_empty()).then_some(languages);
        let preferred = self
            .first_configured_language_list(crate::PREFERRED_AUDIO_LANGUAGES_KEY, &[scope_id])
            .await?;
        let optional = self
            .first_configured_language_list(crate::OPTIONAL_AUDIO_LANGUAGES_KEY, &[scope_id])
            .await?;
        let optional_keep_searching = self
            .read_setting_bool_value_explicit(
                crate::OPTIONAL_AUDIO_LANGUAGES_KEEP_SEARCHING_KEY,
                Some(scope_id),
            )
            .await?;
        Ok(AudioLanguagePreferenceOverrides {
            preferred: list(preferred),
            optional: list(optional),
            optional_keep_searching,
        })
    }


    /// Which of `scope_ids` (library ids or facet names) set preferred or
    /// optional audio languages, so their titles can become language targets.
    pub(crate) async fn scopes_with_audio_language_preferences(
        &self,
        scope_ids: &[&str],
    ) -> AppResult<HashSet<String>> {
        let mut configured = HashSet::new();
        for scope in scope_ids {
            for key in [
                crate::PREFERRED_AUDIO_LANGUAGES_KEY,
                crate::OPTIONAL_AUDIO_LANGUAGES_KEY,
            ] {
                if !self
                    .first_configured_language_list(key, &[scope])
                    .await?
                    .is_empty()
                {
                    configured.insert((*scope).to_string());
                    break;
                }
            }
        }
        Ok(configured)
    }

    async fn first_configured_language_list(
        &self,
        key_name: &str,
        scopes: &[&str],
    ) -> AppResult<Vec<String>> {
        for scope in scopes {
            let languages = normalize_required_audio_requirements(
                self.read_setting_json_value::<Vec<String>>(key_name, Some(scope))
                    .await?
                    .unwrap_or_default(),
            );
            if !languages.is_empty() {
                return Ok(languages);
            }
        }
        Ok(Vec::new())
    }
}
impl AppUseCase {
    /// Whether renaming is turned on for titles of this facet. Rename preview
    /// and apply refuse with `renamer_disabled` when this is false.
    pub async fn resolve_rename_enabled(&self, facet: &MediaFacet) -> AppResult<bool> {
        Ok(self
            .read_setting_bool_value(RENAME_ENABLED_KEY, Some(facet.as_str()))
            .await?
            .unwrap_or(true))
    }

    pub(crate) async fn resolve_rename_template(&self, facet: &MediaFacet) -> AppResult<String> {
        let scoped = self
            .read_setting_string_value(RENAME_TEMPLATE_KEY, Some(facet.as_str()))
            .await?;
        let legacy = self
            .read_setting_string_value(rename_template_global_key(facet), None)
            .await?;
        Ok(scoped
            .or(legacy)
            .unwrap_or_else(|| default_rename_template(facet).to_string()))
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

    async fn resolve_import_permission_string_setting(
        &self,
        key_name: &str,
        library_id: Option<&str>,
        facet: &MediaFacet,
    ) -> AppResult<Option<String>> {
        if let Some(library_id) = library_id
            && let Some(value) = self
                .read_setting_string_value_explicit(key_name, Some(library_id))
                .await?
        {
            return Ok(Some(value));
        }

        if let Some(value) = self
            .read_setting_string_value_explicit(key_name, Some(facet.as_str()))
            .await?
        {
            return Ok(Some(value));
        }

        self.read_setting_string_value(key_name, None).await
    }

    pub(crate) async fn resolve_import_file_permissions(
        &self,
        library_id: Option<&str>,
        facet: &MediaFacet,
    ) -> AppResult<ImportFilePermissions> {
        let set_permissions_linux = if let Some(library_id) = library_id
            && let Some(value) = self
                .read_setting_bool_value_explicit(SET_PERMISSIONS_LINUX_KEY, Some(library_id))
                .await?
        {
            value
        } else if let Some(value) = self
            .read_setting_bool_value_explicit(SET_PERMISSIONS_LINUX_KEY, Some(facet.as_str()))
            .await?
        {
            value
        } else {
            self.read_setting_bool_value(SET_PERMISSIONS_LINUX_KEY, None)
                .await?
                .unwrap_or(false)
        };

        let file_chmod = normalize_chmod_setting(
            self.resolve_import_permission_string_setting(FILE_CHMOD_KEY, library_id, facet)
                .await?,
            FILE_CHMOD_KEY,
        )?;
        let mut folder_chmod = normalize_chmod_setting(
            self.resolve_import_permission_string_setting(FOLDER_CHMOD_KEY, library_id, facet)
                .await?,
            FOLDER_CHMOD_KEY,
        )?;
        if set_permissions_linux && folder_chmod.is_none() {
            folder_chmod = Some("755".to_string());
        }
        let chown_group = normalize_chown_group_setting(
            self.resolve_import_permission_string_setting(CHOWN_GROUP_KEY, library_id, facet)
                .await?,
        )?;

        Ok(ImportFilePermissions {
            set_permissions_linux,
            file_chmod,
            folder_chmod,
            chown_group,
        })
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
            .ok_or_else(|| AppError::not_found_reason(crate::error_reasons::catalog::TITLE_NOT_FOUND, format!("title {title_id}")))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;

        let payload = languages.map(normalize_required_audio_requirements);
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

    /// Apply the audio language overrides a title option patch carries to an
    /// existing title. Needs the same grant as
    /// [`Self::set_title_required_audio_override`]; a patch that carries
    /// neither override reads and writes nothing.
    pub async fn set_title_audio_language_overrides(
        &self,
        actor: &User,
        title_id: &str,
        patch: &crate::TitleOptionsPatch,
    ) -> AppResult<()> {
        if !patch.carries_audio_language_overrides() {
            return Ok(());
        }
        let patch = validated_title_audio_patch(patch.clone())?;
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| {
                AppError::not_found_reason(
                    crate::error_reasons::catalog::TITLE_NOT_FOUND,
                    format!("title {title_id}"),
                )
            })?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;
        self.write_title_audio_language_overrides(&title.id, &patch, Some(actor.id.clone()))
            .await
    }

    /// Store the audio language overrides a title option patch carries. The
    /// caller has already authorized the write (title creation or update, an
    /// approval, a list add) and validated the languages. A field the patch
    /// leaves `None` is not touched.
    pub(crate) async fn write_title_audio_language_overrides(
        &self,
        title_id: &str,
        patch: &crate::TitleOptionsPatch,
        updated_by_user_id: Option<String>,
    ) -> AppResult<()> {
        let title_id = title_id.trim().to_string();
        if let Some(required) = &patch.required_audio_languages {
            self.services
                .config
                .settings
                .upsert_setting_json(
                    SETTINGS_SCOPE_SYSTEM,
                    TITLE_REQUIRED_AUDIO_OVERRIDE_KEY,
                    Some(title_id.clone()),
                    serde_json::to_string(required)
                        .map_err(|error| AppError::Repository(error.to_string()))?,
                    SETTINGS_SOURCE_TYPED_GRAPHQL,
                    updated_by_user_id.clone(),
                )
                .await?;
        }
        match &patch.preferred_audio_languages {
            None => {}
            Some(Some(languages)) if !languages.is_empty() => {
                self.services
                    .config
                    .settings
                    .upsert_setting_json(
                        SETTINGS_SCOPE_SYSTEM,
                        crate::TITLE_PREFERRED_AUDIO_OVERRIDE_KEY,
                        Some(title_id),
                        serde_json::to_string(languages)
                            .map_err(|error| AppError::Repository(error.to_string()))?,
                        SETTINGS_SOURCE_TYPED_GRAPHQL,
                        updated_by_user_id,
                    )
                    .await?;
            }
            // Inheriting is the absence of a row, so the language-target gate
            // only ever finds titles that really prefer something.
            Some(_) => {
                self.services
                    .config
                    .settings
                    .delete_setting_value(
                        SETTINGS_SCOPE_SYSTEM,
                        crate::TITLE_PREFERRED_AUDIO_OVERRIDE_KEY,
                        Some(title_id),
                    )
                    .await?;
            }
        }
        Ok(())
    }

    /// The audio languages a new title in each of `library_ids` would inherit,
    /// for labelling the "inherit" choice of the add, request and list forms.
    ///
    /// Anyone who can see a library may read them: its viewers, requesters
    /// and title managers, and catalog-settings or permission managers. Each
    /// library's outcome is kept apart, so one the actor may not see fails
    /// alone. The library list and the actor's grants are read once for the
    /// whole batch; the per-library setting reads are served from the settings
    /// store's per-key cache.
    pub async fn library_audio_language_defaults(
        &self,
        actor: &User,
        library_ids: &[String],
    ) -> AppResult<HashMap<String, AppResult<LibraryAudioLanguageDefaults>>> {
        let mut outcomes = HashMap::with_capacity(library_ids.len());
        if library_ids.is_empty() {
            return Ok(outcomes);
        }
        let authorization = self.authorization_for_actor(actor).await?;
        let app_wide = authorization
            .has_app_permission(scryer_domain::AppPermission::ManageCatalogSettings)
            || authorization.has_app_permission(scryer_domain::AppPermission::ManagePermissions);
        let libraries: HashMap<String, scryer_domain::Library> = self
            .services
            .catalog
            .libraries
            .list(None)
            .await?
            .into_iter()
            .map(|library| (library.id.clone(), library))
            .collect();
        for library_id in library_ids {
            if outcomes.contains_key(library_id) {
                continue;
            }
            let visible = app_wide
                || [
                    scryer_domain::LibraryPermission::View,
                    scryer_domain::LibraryPermission::Request,
                    scryer_domain::LibraryPermission::ManageTitles,
                    scryer_domain::LibraryPermission::ManageLibrary,
                ]
                .into_iter()
                .any(|permission| {
                    crate::authorization::effective_library_permission(
                        &authorization,
                        library_id,
                        permission,
                    )
                });
            let outcome = match libraries.get(library_id) {
                Some(library) if visible => {
                    let facet = library.facet.as_str();
                    async {
                        Ok(LibraryAudioLanguageDefaults {
                            required: self
                                .resolve_required_audio_languages(
                                    None,
                                    Some(&library.id),
                                    Some(facet),
                                )
                                .await?,
                            preferred: self
                                .resolve_preferred_audio_languages(
                                    None,
                                    Some(&library.id),
                                    Some(facet),
                                )
                                .await?,
                        })
                    }
                    .await
                }
                _ => Err(AppError::unauthorized_reason(
                    crate::error_reasons::settings::LIBRARY_ACCESS_DENIED,
                    "You do not have access to this library".to_string(),
                )),
            };
            outcomes.insert(library_id.clone(), outcome);
        }
        Ok(outcomes)
    }

    /// Store the audio languages an add asks a newly created title to carry.
    ///
    /// The one rule for every add (direct, list, request approval): an add
    /// that reuses a title already in the library leaves that title's audio
    /// overrides alone. The title is already committed when this runs, so a
    /// failed write is logged and the add carries on to its events, claims and
    /// first search; the title then inherits until someone sets it again. The
    /// languages were validated before the title was created, so only a
    /// repository failure reaches the log.
    pub(crate) async fn store_new_title_audio_language_overrides(
        &self,
        created: &crate::CreateTitleOutcome,
        patch: &crate::TitleOptionsPatch,
        updated_by_user_id: Option<String>,
    ) {
        if created.reused_existing || !patch.carries_audio_language_overrides() {
            return;
        }
        if let Err(error) = self
            .write_title_audio_language_overrides(&created.title.id, patch, updated_by_user_id)
            .await
        {
            tracing::warn!(
                title_id = %created.title.id,
                error = %error,
                "storing a new title's audio language overrides failed; it inherits its library's"
            );
        }
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

        let normalized = normalize_required_audio_requirements(languages);
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
        let rename_template = self.resolve_rename_template(&facet).await?;
        let folder_template = crate::normalize_title_folder_template_or_default(
            self.read_setting_string_value(FOLDER_TEMPLATE_KEY, Some(facet.as_str()))
                .await?,
            default_folder_template(&facet),
            facet.as_str(),
        );
        let (season_folder_template, specials_folder_template) =
            if matches!(facet, MediaFacet::Series | MediaFacet::Anime) {
                (
                    Some(crate::normalize_season_folder_template_or_default(
                        self.read_setting_string_value(
                            SEASON_FOLDER_TEMPLATE_KEY,
                            Some(facet.as_str()),
                        )
                        .await?,
                    )),
                    Some(crate::normalize_specials_folder_template_or_default(
                        self.read_setting_string_value(
                            SPECIALS_FOLDER_TEMPLATE_KEY,
                            Some(facet.as_str()),
                        )
                        .await?,
                    )),
                )
            } else {
                (None, None)
            };
        let use_season_folders = if matches!(facet, MediaFacet::Series | MediaFacet::Anime) {
            self.read_setting_bool_value(USE_SEASON_FOLDERS_KEY, Some(facet.as_str()))
                .await?
                .unwrap_or(true)
        } else {
            true
        };
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
        let import_permissions = self.resolve_import_file_permissions(None, &facet).await?;

        Ok(MediaSettings {
            library_path,
            root_folders,
            required_audio_languages: self
                .load_facet_required_audio_languages(facet.as_str())
                .await?,
            audio_language_preferences: self
                .load_audio_language_preference_settings(facet.as_str())
                .await?,
            use_season_folders,
            folder_template,
            season_folder_template,
            specials_folder_template,
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
            set_permissions_linux: import_permissions.set_permissions_linux,
            file_chmod: import_permissions.file_chmod,
            folder_chmod: import_permissions.folder_chmod,
            chown_group: import_permissions.chown_group,
            movie_availability: if facet == MediaFacet::Movie {
                Some(self.movie_availability_facet_settings().await?)
            } else {
                None
            },
        })
    }
}
impl AppUseCase {
    pub async fn apply_external_import_media_settings_auto_apply(
        &self,
        actor: &User,
        facet: MediaFacet,
        rename_enabled: Option<bool>,
    ) -> AppResult<Vec<String>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        let mut changed_keys = Vec::new();
        if let Some(value) = rename_enabled
            && self
                .read_setting_string_value_explicit(RENAME_ENABLED_KEY, Some(facet.as_str()))
                .await?
                .is_none()
            && self
                .read_setting_string_value_explicit(RENAME_ENABLED_KEY, None)
                .await?
                .is_none()
        {
            self.services
                .config
                .settings
                .upsert_setting_json(
                    SETTINGS_SCOPE_SYSTEM,
                    RENAME_ENABLED_KEY,
                    Some(facet.as_str().to_string()),
                    encode_setting_json(&value)?,
                    SETTINGS_SOURCE_TYPED_GRAPHQL,
                    Some(actor.id.clone()),
                )
                .await?;
            changed_keys.push(RENAME_ENABLED_KEY.to_string());
        }

        if !changed_keys.is_empty() {
            self.emit_settings_saved(
                actor,
                "external_import_media_settings",
                Some(facet.as_str().to_string()),
                changed_keys.clone(),
            )
            .await;
        }

        Ok(changed_keys)
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
        if facet == MediaFacet::Movie
            && (input.season_folder_template.is_some()
                || input.specials_folder_template.is_some()
                || input.use_season_folders.is_some())
        {
            return Err(AppError::validation_refused(crate::error_reasons::settings::SEASON_FOLDER_TEMPLATES_ONLY_SUPPORTED_SERIES_ANIME, "season folder templates are only supported for series and anime".to_string()));
        }
        let movie_availability = input.movie_availability.clone().normalized()?;
        if facet != MediaFacet::Movie && !movie_availability.is_empty() {
            return Err(AppError::validation_refused(crate::error_reasons::settings::MOVIE_AVAILABILITY_SETTINGS_REQUIRE_SCOPE_MOVIE, "movie availability settings require scope movie".to_string()));
        }
        if let Some(template) = input.season_folder_template.as_deref() {
            crate::validate_season_folder_template(template)?;
        }
        if let Some(template) = input.specials_folder_template.as_deref() {
            crate::validate_specials_folder_template(template)?;
        }
        let preferred_audio_languages = input
            .preferred_audio_languages
            .map(|languages| validated_preference_languages("preferred audio language", languages))
            .transpose()?;
        let optional_audio_languages = input
            .optional_audio_languages
            .map(|languages| validated_preference_languages("optional audio language", languages))
            .transpose()?;
        let season_folder_template =
            normalize_optional_string(input.season_folder_template.clone());
        let specials_folder_template =
            normalize_optional_string(input.specials_folder_template.clone());
        let mut changed_keys = Vec::new();

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

        if let Some(season_folder_template) = season_folder_template {
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

        if let Some(specials_folder_template) = specials_folder_template {
            self.services
                .config
                .settings
                .upsert_setting_json(
                    SETTINGS_SCOPE_SYSTEM,
                    SPECIALS_FOLDER_TEMPLATE_KEY,
                    Some(facet.as_str().to_string()),
                    encode_setting_json(&specials_folder_template)?,
                    SETTINGS_SOURCE_TYPED_GRAPHQL,
                    Some(actor.id.clone()),
                )
                .await?;
            changed_keys.push(SPECIALS_FOLDER_TEMPLATE_KEY.to_string());
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
            crate::validate_rename_template_for_facet(&rename_template, &facet)?;
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
                    encode_setting_json(&normalize_required_audio_requirements(
                        required_audio_languages,
                    ))?,
                    SETTINGS_SOURCE_TYPED_GRAPHQL,
                    Some(actor.id.clone()),
                )
                .await?;
            changed_keys.push(REQUIRED_AUDIO_LANGUAGES_KEY.to_string());
        }

        if let Some(languages) = preferred_audio_languages {
            self.services
                .config
                .settings
                .upsert_setting_json(
                    SETTINGS_SCOPE_SYSTEM,
                    crate::PREFERRED_AUDIO_LANGUAGES_KEY,
                    Some(facet.as_str().to_string()),
                    encode_setting_json(&languages)?,
                    SETTINGS_SOURCE_TYPED_GRAPHQL,
                    Some(actor.id.clone()),
                )
                .await?;
            changed_keys.push(crate::PREFERRED_AUDIO_LANGUAGES_KEY.to_string());
        }

        if let Some(languages) = optional_audio_languages {
            self.services
                .config
                .settings
                .upsert_setting_json(
                    SETTINGS_SCOPE_SYSTEM,
                    crate::OPTIONAL_AUDIO_LANGUAGES_KEY,
                    Some(facet.as_str().to_string()),
                    encode_setting_json(&languages)?,
                    SETTINGS_SOURCE_TYPED_GRAPHQL,
                    Some(actor.id.clone()),
                )
                .await?;
            changed_keys.push(crate::OPTIONAL_AUDIO_LANGUAGES_KEY.to_string());
        }

        if let Some(keep_searching) = input.optional_audio_languages_keep_searching {
            self.services
                .config
                .settings
                .upsert_setting_json(
                    SETTINGS_SCOPE_SYSTEM,
                    crate::OPTIONAL_AUDIO_LANGUAGES_KEEP_SEARCHING_KEY,
                    Some(facet.as_str().to_string()),
                    encode_setting_json(&keep_searching)?,
                    SETTINGS_SOURCE_TYPED_GRAPHQL,
                    Some(actor.id.clone()),
                )
                .await?;
            changed_keys.push(crate::OPTIONAL_AUDIO_LANGUAGES_KEEP_SEARCHING_KEY.to_string());
        }

        if let Some(use_season_folders) = input.use_season_folders {
            self.services
                .config
                .settings
                .upsert_setting_json(
                    SETTINGS_SCOPE_SYSTEM,
                    USE_SEASON_FOLDERS_KEY,
                    Some(facet.as_str().to_string()),
                    encode_setting_json(&use_season_folders)?,
                    SETTINGS_SOURCE_TYPED_GRAPHQL,
                    Some(actor.id.clone()),
                )
                .await?;
            changed_keys.push(USE_SEASON_FOLDERS_KEY.to_string());
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
                return Err(AppError::validation_refused(crate::error_reasons::settings::PLEXMATCH_WRITE_IMPORT_ONLY_VALID_SERIES_ANIME, "plexmatch_write_on_import is only valid for series and anime".to_string()));
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

        if let Some(value) = input.set_permissions_linux {
            self.services
                .config
                .settings
                .upsert_setting_json(
                    SETTINGS_SCOPE_SYSTEM,
                    SET_PERMISSIONS_LINUX_KEY,
                    Some(facet.as_str().to_string()),
                    encode_setting_json(&value)?,
                    SETTINGS_SOURCE_TYPED_GRAPHQL,
                    Some(actor.id.clone()),
                )
                .await?;
            changed_keys.push(SET_PERMISSIONS_LINUX_KEY.to_string());
        }

        if input.file_chmod.is_some() {
            match normalize_chmod_setting(input.file_chmod, FILE_CHMOD_KEY)? {
                Some(value) => {
                    self.services
                        .config
                        .settings
                        .upsert_setting_json(
                            SETTINGS_SCOPE_SYSTEM,
                            FILE_CHMOD_KEY,
                            Some(facet.as_str().to_string()),
                            encode_setting_json(&value)?,
                            SETTINGS_SOURCE_TYPED_GRAPHQL,
                            Some(actor.id.clone()),
                        )
                        .await?;
                }
                None => {
                    self.services
                        .config
                        .settings
                        .delete_setting_value(
                            SETTINGS_SCOPE_SYSTEM,
                            FILE_CHMOD_KEY,
                            Some(facet.as_str().to_string()),
                        )
                        .await?;
                }
            }
            changed_keys.push(FILE_CHMOD_KEY.to_string());
        }

        if input.folder_chmod.is_some() {
            match normalize_chmod_setting(input.folder_chmod, FOLDER_CHMOD_KEY)? {
                Some(value) => {
                    self.services
                        .config
                        .settings
                        .upsert_setting_json(
                            SETTINGS_SCOPE_SYSTEM,
                            FOLDER_CHMOD_KEY,
                            Some(facet.as_str().to_string()),
                            encode_setting_json(&value)?,
                            SETTINGS_SOURCE_TYPED_GRAPHQL,
                            Some(actor.id.clone()),
                        )
                        .await?;
                }
                None => {
                    self.services
                        .config
                        .settings
                        .delete_setting_value(
                            SETTINGS_SCOPE_SYSTEM,
                            FOLDER_CHMOD_KEY,
                            Some(facet.as_str().to_string()),
                        )
                        .await?;
                }
            }
            changed_keys.push(FOLDER_CHMOD_KEY.to_string());
        }

        if input.chown_group.is_some() {
            match normalize_chown_group_setting(input.chown_group)? {
                Some(value) => {
                    self.services
                        .config
                        .settings
                        .upsert_setting_json(
                            SETTINGS_SCOPE_SYSTEM,
                            CHOWN_GROUP_KEY,
                            Some(facet.as_str().to_string()),
                            encode_setting_json(&value)?,
                            SETTINGS_SOURCE_TYPED_GRAPHQL,
                            Some(actor.id.clone()),
                        )
                        .await?;
                }
                None => {
                    self.services
                        .config
                        .settings
                        .delete_setting_value(
                            SETTINGS_SCOPE_SYSTEM,
                            CHOWN_GROUP_KEY,
                            Some(facet.as_str().to_string()),
                        )
                        .await?;
                }
            }
            changed_keys.push(CHOWN_GROUP_KEY.to_string());
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
            return Err(AppError::validation_refused(crate::error_reasons::settings::ANIME_SPECIFIC_SETTINGS_REQUIRE_SCOPE_ANIME, "anime-specific settings require scope anime".to_string()));
        }

        if !movie_availability.is_empty() {
            let before = self.movie_availability_policies().await?;
            changed_keys.extend(
                self.write_movie_availability_values(
                    MediaFacet::Movie.as_str(),
                    &movie_availability,
                    false,
                    &actor.id,
                )
                .await?,
            );
            let after = self.movie_availability_policies().await?;
            self.queue_movie_release_market_refresh_where_changed(&before, &after)
                .await?;
        }

        if changed_keys.is_empty() {
            return Err(AppError::validation_refused(crate::error_reasons::settings::MEDIA_SETTING_CHANGE_REQUIRED, "at least one media setting change is required".to_string()));
        }

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

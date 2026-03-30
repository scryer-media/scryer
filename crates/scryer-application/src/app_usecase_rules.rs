use super::*;
use crate::managed_rules;
use scryer_domain::RuleSet;
use scryer_rules::validation::{ValidationResult, validate_user_rule};

impl AppUseCase {
    pub async fn list_rule_sets(&self, actor: &User) -> AppResult<Vec<RuleSet>> {
        require(actor, &Entitlement::ViewCatalog)?;
        self.services.rule_sets.list_rule_sets().await
    }

    pub async fn get_rule_set(&self, actor: &User, id: &str) -> AppResult<Option<RuleSet>> {
        require(actor, &Entitlement::ViewCatalog)?;
        self.services.rule_sets.get_rule_set(id).await
    }

    pub async fn create_rule_set(
        &self,
        actor: &User,
        name: String,
        description: String,
        rego_source: String,
        applied_facets: Vec<MediaFacet>,
        priority: i32,
    ) -> AppResult<RuleSet> {
        require(actor, &Entitlement::ManageTitle)?;

        let id = Id::new_rego_safe().0;

        // Rewrite the package declaration to match the system-assigned ID.
        let rewritten_source = scryer_rules::rewrite_package_declaration(&rego_source, &id);

        // Validate the rewritten Rego source
        let validation = validate_user_rule(&rewritten_source, &id)
            .map_err(|e| AppError::Validation(format!("rule validation failed: {e}")))?;
        if !validation.valid {
            return Err(AppError::Validation(validation.errors.join("; ")));
        }

        let now = Utc::now();
        let rule_set = RuleSet {
            id,
            name,
            description,
            rego_source: rewritten_source.clone(),
            enabled: true,
            priority,
            applied_facets,
            created_at: now,
            updated_at: now,
            is_managed: false,
            managed_key: None,
        };

        self.services.rule_sets.create_rule_set(&rule_set).await?;
        self.services
            .rule_sets
            .record_rule_set_history(
                &rule_set.id,
                "created",
                Some(&rewritten_source),
                Some(&actor.id),
            )
            .await?;

        self.rebuild_user_rules_engine().await?;
        Ok(rule_set)
    }

    pub async fn update_rule_set(
        &self,
        actor: &User,
        id: String,
        name: Option<String>,
        description: Option<String>,
        rego_source: Option<String>,
        applied_facets: Option<Vec<MediaFacet>>,
        priority: Option<i32>,
    ) -> AppResult<RuleSet> {
        require(actor, &Entitlement::ManageTitle)?;

        let mut rule_set = self
            .services
            .rule_sets
            .get_rule_set(&id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("rule set {id} not found")))?;

        if rule_set.is_managed {
            return Err(AppError::Validation(
                "This rule is managed by a convenience setting. Change the setting instead of editing the rule directly.".into(),
            ));
        }

        if let Some(new_source) = &rego_source {
            // Rewrite the package declaration to match the existing rule ID.
            let rewritten = scryer_rules::rewrite_package_declaration(new_source, &rule_set.id);
            let validation = validate_user_rule(&rewritten, &rule_set.id)
                .map_err(|e| AppError::Validation(format!("rule validation failed: {e}")))?;
            if !validation.valid {
                return Err(AppError::Validation(validation.errors.join("; ")));
            }
            rule_set.rego_source = rewritten;
        }
        if let Some(n) = name {
            rule_set.name = n;
        }
        if let Some(d) = description {
            rule_set.description = d;
        }
        if let Some(f) = applied_facets {
            rule_set.applied_facets = f;
        }
        if let Some(p) = priority {
            rule_set.priority = p;
        }
        rule_set.updated_at = Utc::now();

        self.services.rule_sets.update_rule_set(&rule_set).await?;
        self.services
            .rule_sets
            .record_rule_set_history(
                &rule_set.id,
                "updated",
                Some(&rule_set.rego_source),
                Some(&actor.id),
            )
            .await?;

        self.rebuild_user_rules_engine().await?;
        Ok(rule_set)
    }

    pub async fn delete_rule_set(&self, actor: &User, id: &str) -> AppResult<()> {
        require(actor, &Entitlement::ManageTitle)?;

        if let Some(rule_set) = self.services.rule_sets.get_rule_set(id).await?
            && rule_set.is_managed
        {
            return Err(AppError::Validation(
                "This rule is managed by a convenience setting. Remove the setting instead of deleting the rule directly.".into(),
            ));
        }

        self.services.rule_sets.delete_rule_set(id).await?;
        self.services
            .rule_sets
            .record_rule_set_history(id, "deleted", None, Some(&actor.id))
            .await?;

        self.rebuild_user_rules_engine().await?;
        Ok(())
    }

    pub async fn toggle_rule_set(
        &self,
        actor: &User,
        id: &str,
        enabled: bool,
    ) -> AppResult<RuleSet> {
        require(actor, &Entitlement::ManageTitle)?;

        let mut rule_set = self
            .services
            .rule_sets
            .get_rule_set(id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("rule set {id} not found")))?;

        rule_set.enabled = enabled;
        rule_set.updated_at = Utc::now();

        self.services.rule_sets.update_rule_set(&rule_set).await?;
        let action = if enabled { "enabled" } else { "disabled" };
        self.services
            .rule_sets
            .record_rule_set_history(&rule_set.id, action, None, Some(&actor.id))
            .await?;

        self.rebuild_user_rules_engine().await?;
        Ok(rule_set)
    }

    pub async fn validate_rule_set(
        &self,
        actor: &User,
        rego_source: &str,
        rule_set_id: &str,
    ) -> AppResult<ValidationResult> {
        require(actor, &Entitlement::ViewCatalog)?;

        // Rewrite the package declaration so validation works regardless of
        // what the user typed.
        let rewritten = scryer_rules::rewrite_package_declaration(rego_source, rule_set_id);
        validate_user_rule(&rewritten, rule_set_id)
            .map_err(|e| AppError::Validation(format!("rule validation error: {e}")))
    }

    // ── Convenience settings ───────────────────────────────────────────────

    /// Set or remove a facet-level required audio language rule.
    ///
    /// `scope` is "global", "movie", "series", or "anime".
    /// Empty `languages` removes the rule.
    pub async fn set_convenience_required_audio(
        &self,
        actor: &User,
        scope: &str,
        languages: Vec<String>,
    ) -> AppResult<()> {
        require(actor, &Entitlement::ManageTitle)?;

        let key = managed_rules::managed_key_required_audio(scope);

        if languages.is_empty() {
            self.services
                .rule_sets
                .delete_rule_set_by_managed_key(&key)
                .await?;
            self.rebuild_user_rules_engine().await?;
            return Ok(());
        }

        // Find title-level exceptions for this scope
        let title_overrides = self
            .services
            .rule_sets
            .list_rule_sets_by_managed_key_prefix(
                managed_rules::MANAGED_KEY_REQUIRED_AUDIO_TITLE_PREFIX,
            )
            .await?;
        let excepted_ids: Vec<String> = title_overrides
            .iter()
            .filter_map(|rs| {
                rs.managed_key
                    .as_deref()?
                    .strip_prefix(managed_rules::MANAGED_KEY_REQUIRED_AUDIO_TITLE_PREFIX)
                    .map(String::from)
            })
            .collect();

        let rego = managed_rules::generate_required_audio_rego(&languages, &excepted_ids);
        let applied_facets = scope_to_facets(scope);

        self.upsert_managed_rule(
            &key,
            &managed_rules::managed_rule_display_name(&key),
            &rego,
            applied_facets,
        )
        .await?;

        self.rebuild_user_rules_engine().await
    }

    /// Set or remove a title-level required audio language override.
    ///
    /// `languages = Some(vec![...])` creates a title-scoped rule and adds
    /// the title to the parent facet rule's exception list.
    /// `languages = None` removes the override.
    pub async fn set_title_required_audio(
        &self,
        actor: &User,
        title_id: &str,
        facet: &str,
        languages: Option<Vec<String>>,
    ) -> AppResult<()> {
        require(actor, &Entitlement::ManageTitle)?;

        let title_key = managed_rules::managed_key_required_audio_title(title_id);

        match languages {
            Some(langs) if !langs.is_empty() => {
                // Create the title-scoped rule
                let rego = managed_rules::generate_title_required_audio_rego(title_id, &langs);
                let applied_facets = scope_to_facets(facet);
                self.upsert_managed_rule(
                    &title_key,
                    &managed_rules::managed_rule_display_name(&title_key),
                    &rego,
                    applied_facets,
                )
                .await?;
            }
            _ => {
                // Remove the title-scoped rule
                self.services
                    .rule_sets
                    .delete_rule_set_by_managed_key(&title_key)
                    .await?;
            }
        }

        // Regenerate the parent facet rule with updated exceptions
        self.regenerate_facet_required_audio(facet).await?;

        self.rebuild_user_rules_engine().await
    }

    /// Get the current state of all convenience settings by inspecting managed rules.
    pub async fn get_convenience_settings(&self, actor: &User) -> AppResult<ConvenienceSettings> {
        require(actor, &Entitlement::ViewCatalog)?;

        let all_managed = self
            .services
            .rule_sets
            .list_rule_sets_by_managed_key_prefix("convenience:")
            .await?;

        let mut required_audio = Vec::new();
        for rs in &all_managed {
            let Some(key) = rs.managed_key.as_deref() else {
                continue;
            };

            if let Some(scope) = key.strip_prefix("convenience:required-audio:title:") {
                // Title-level override — extract languages from the rego
                let languages = extract_languages_from_rego(&rs.rego_source);
                required_audio.push(ConvenienceAudioSetting {
                    scope: format!("title:{scope}"),
                    languages,
                    rule_set_id: Some(rs.id.clone()),
                });
            } else if let Some(scope) = key.strip_prefix("convenience:required-audio:") {
                let languages = extract_languages_from_rego(&rs.rego_source);
                required_audio.push(ConvenienceAudioSetting {
                    scope: scope.to_string(),
                    languages,
                    rule_set_id: Some(rs.id.clone()),
                });
            }
        }

        Ok(ConvenienceSettings { required_audio })
    }

    pub async fn migrate_legacy_persona_preferences(&self) -> AppResult<()> {
        const SYSTEM_SCOPE: &str = "system";

        let mut existing_rules = self.services.rule_sets.list_rule_sets().await?;
        let profiles = self
            .services
            .quality_profiles
            .list_quality_profiles(SYSTEM_SCOPE, None)
            .await?;

        for profile in &profiles {
            if profile.criteria.prefer_dual_audio {
                let marker = format!("legacy-prefer-dual-audio:profile:{}", profile.id);
                self.ensure_migrated_rule(
                    &mut existing_rules,
                    &marker,
                    &format!("Migrated: Prefer Multi-Audio ({})", profile.name),
                    "Auto-migrated from the deprecated dual-audio profile preference.",
                    &generate_profile_prefer_multi_audio_rego(&profile.id),
                    Vec::new(),
                )
                .await?;
            }

            if profile.criteria.scoring_persona == ScoringPersona::Audiophile {
                if !profile.criteria.atmos_preferred {
                    let marker = format!("legacy-atmos-disabled:profile:{}", profile.id);
                    self.ensure_migrated_rule(
                        &mut existing_rules,
                        &marker,
                        &format!("Migrated: Disable Atmos Persona Bias ({})", profile.name),
                        "Auto-migrated from the deprecated Atmos preference toggle.",
                        &generate_profile_cancel_atmos_rego(&profile.id, 150, 30),
                        Vec::new(),
                    )
                    .await?;
                }
            } else if profile.criteria.atmos_preferred {
                let (bonus, penalty) = legacy_atmos_rule_values(&profile.criteria.scoring_persona);
                let marker = format!("legacy-atmos-preferred:profile:{}", profile.id);
                self.ensure_migrated_rule(
                    &mut existing_rules,
                    &marker,
                    &format!("Migrated: Prefer Atmos ({})", profile.name),
                    "Auto-migrated from the deprecated Atmos preference toggle.",
                    &generate_profile_prefer_atmos_rego(&profile.id, bonus, penalty),
                    Vec::new(),
                )
                .await?;
            }
        }

        let legacy_managed = self
            .services
            .rule_sets
            .list_rule_sets_by_managed_key_prefix("convenience:prefer-dual-audio:")
            .await?;
        for rule_set in legacy_managed {
            let marker = format!(
                "legacy-convenience-prefer-dual-audio:{}",
                rule_set.managed_key.as_deref().unwrap_or(&rule_set.id)
            );
            self.ensure_migrated_rule(
                &mut existing_rules,
                &marker,
                &format!("Migrated: {}", rule_set.name),
                "Auto-migrated from the removed dual-audio convenience rule.",
                &rule_set.rego_source,
                rule_set.applied_facets.clone(),
            )
            .await?;

            self.services
                .rule_sets
                .delete_rule_set(&rule_set.id)
                .await?;
        }

        Ok(())
    }

    // ── Internal helpers ──────────────────────────────────────────────────

    /// Regenerate a facet-level required audio rule after a title override changes.
    async fn regenerate_facet_required_audio(&self, facet: &str) -> AppResult<()> {
        let facet_key = managed_rules::managed_key_required_audio(facet);
        let facet_rule = self
            .services
            .rule_sets
            .get_rule_set_by_managed_key(&facet_key)
            .await?;

        let Some(facet_rule) = facet_rule else {
            // No facet rule to regenerate
            return Ok(());
        };

        let languages = extract_languages_from_rego(&facet_rule.rego_source);
        if languages.is_empty() {
            return Ok(());
        }

        // Collect all title overrides
        let title_overrides = self
            .services
            .rule_sets
            .list_rule_sets_by_managed_key_prefix(
                managed_rules::MANAGED_KEY_REQUIRED_AUDIO_TITLE_PREFIX,
            )
            .await?;
        let excepted_ids: Vec<String> = title_overrides
            .iter()
            .filter_map(|rs| {
                rs.managed_key
                    .as_deref()?
                    .strip_prefix(managed_rules::MANAGED_KEY_REQUIRED_AUDIO_TITLE_PREFIX)
                    .map(String::from)
            })
            .collect();

        let rego = managed_rules::generate_required_audio_rego(&languages, &excepted_ids);
        self.upsert_managed_rule(
            &facet_key,
            &facet_rule.name,
            &rego,
            facet_rule.applied_facets.clone(),
        )
        .await
    }

    /// Create or update a managed rule by its managed_key.
    async fn upsert_managed_rule(
        &self,
        managed_key: &str,
        name: &str,
        rego_source: &str,
        applied_facets: Vec<MediaFacet>,
    ) -> AppResult<()> {
        let existing = self
            .services
            .rule_sets
            .get_rule_set_by_managed_key(managed_key)
            .await?;

        let id = existing
            .as_ref()
            .map(|r| r.id.clone())
            .unwrap_or_else(|| Id::new_rego_safe().0);

        let rewritten = scryer_rules::rewrite_package_declaration(rego_source, &id);
        let now = Utc::now();

        let rule_set = RuleSet {
            id: id.clone(),
            name: name.to_string(),
            description: String::new(),
            rego_source: rewritten,
            enabled: existing.as_ref().map(|r| r.enabled).unwrap_or(true),
            priority: -100, // Managed rules run before user rules
            applied_facets,
            created_at: existing.as_ref().map(|r| r.created_at).unwrap_or(now),
            updated_at: now,
            is_managed: true,
            managed_key: Some(managed_key.to_string()),
        };

        if existing.is_some() {
            self.services.rule_sets.update_rule_set(&rule_set).await?;
        } else {
            self.services.rule_sets.create_rule_set(&rule_set).await?;
        }

        Ok(())
    }

    async fn ensure_migrated_rule(
        &self,
        existing_rules: &mut Vec<RuleSet>,
        migration_key: &str,
        name: &str,
        description_prefix: &str,
        rego_source: &str,
        applied_facets: Vec<MediaFacet>,
    ) -> AppResult<()> {
        if existing_rules.iter().any(|rule| {
            rule.description.contains(migration_key) || rule.rego_source.contains(migration_key)
        }) {
            return Ok(());
        }

        let now = Utc::now();
        let id = Id::new_rego_safe().0;
        let rewritten = scryer_rules::rewrite_package_declaration(rego_source, &id);
        let rule_set = RuleSet {
            id,
            name: name.to_string(),
            description: format!("{description_prefix} [scryer-migration:{migration_key}]"),
            rego_source: rewritten,
            enabled: true,
            priority: 0,
            applied_facets,
            created_at: now,
            updated_at: now,
            is_managed: false,
            managed_key: None,
        };
        self.services.rule_sets.create_rule_set(&rule_set).await?;
        existing_rules.push(rule_set);
        Ok(())
    }

    pub async fn rebuild_user_rules_engine(&self) -> AppResult<()> {
        let enabled = self.services.rule_sets.list_enabled_rule_sets().await?;

        let mut policies: Vec<scryer_rules::UserPolicy> = enabled
            .iter()
            .map(|rs| scryer_rules::UserPolicy {
                id: rs.id.clone(),
                name: rs.name.clone(),
                rego_source: rs.rego_source.clone(),
                applied_facets: rs
                    .applied_facets
                    .iter()
                    .map(|f| format!("{:?}", f).to_lowercase())
                    .collect(),
            })
            .collect();

        let user_count = policies.len();

        // Append scoring policies from loaded WASM plugins.
        // Rewrite package declarations so the Rego package path matches the
        // system-assigned ID, same as we do for user-authored rules.
        if let Some(ref pp) = self.services.plugin_provider {
            let plugin_policies = pp.scoring_policies();
            if !plugin_policies.is_empty() {
                tracing::info!(
                    plugin_policy_count = plugin_policies.len(),
                    "including plugin-supplied scoring policies"
                );
                for mut p in plugin_policies {
                    p.rego_source =
                        scryer_rules::rewrite_package_declaration(&p.rego_source, &p.id);
                    policies.push(p);
                }
            }
        }

        let engine = scryer_rules::UserRulesEngine::build(&policies)
            .map_err(|e| AppError::Validation(format!("failed to build rules engine: {e}")))?;

        let mut guard = self
            .services
            .user_rules
            .write()
            .map_err(|e| AppError::Repository(format!("rules engine lock poisoned: {e}")))?;
        *guard = engine;

        tracing::info!(
            user_rule_count = user_count,
            total_rule_count = policies.len(),
            "user rules engine rebuilt"
        );
        Ok(())
    }
}

// ── Convenience setting types ────────────────────────────────────────────────

#[derive(Clone, Debug, serde::Serialize)]
pub struct ConvenienceSettings {
    pub required_audio: Vec<ConvenienceAudioSetting>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct ConvenienceAudioSetting {
    pub scope: String,
    pub languages: Vec<String>,
    pub rule_set_id: Option<String>,
}

// ── Helper functions ─────────────────────────────────────────────────────────

fn scope_to_facets(scope: &str) -> Vec<MediaFacet> {
    match scope {
        "movie" => vec![MediaFacet::Movie],
        "series" | "tv" => vec![MediaFacet::Series],
        "anime" => vec![MediaFacet::Anime],
        _ => vec![], // global = all facets (empty means all)
    }
}

/// Extract required language codes from generated Rego source.
///
/// Looks for the pattern `_required_langs := {"eng", "jpn"}` and extracts
/// the language codes from the set literal.
fn extract_languages_from_rego(rego: &str) -> Vec<String> {
    for line in rego.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("_required_langs := {")
            && let Some(set_body) = rest.strip_suffix('}')
        {
            return set_body
                .split(',')
                .map(|s| s.trim().trim_matches('"').to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
    }
    vec![]
}

fn generate_profile_prefer_multi_audio_rego(profile_id: &str) -> String {
    format!(
        "import rego.v1\n\n# scryer-migration:legacy-prefer-dual-audio:profile:{profile_id}\n\nscore_entry[\"migrated_prefer_multi_audio\"] := 200 if {{\n    input.profile.id == \"{profile_id}\"\n    input.release.is_dual_audio\n}}\n\nscore_entry[\"migrated_prefer_multi_audio_file\"] := 200 if {{\n    input.profile.id == \"{profile_id}\"\n    not input.release.is_dual_audio\n    input.file != null\n    input.file.has_multiaudio\n}}\n"
    )
}

fn generate_profile_prefer_atmos_rego(profile_id: &str, bonus: i32, penalty: i32) -> String {
    format!(
        "import rego.v1\n\n# scryer-migration:legacy-atmos-preferred:profile:{profile_id}\n\nscore_entry[\"migrated_atmos_match\"] := {bonus} if {{\n    input.profile.id == \"{profile_id}\"\n    input.release.is_atmos\n}}\n\nscore_entry[\"migrated_atmos_missing\"] := {penalty} if {{\n    input.profile.id == \"{profile_id}\"\n    not input.release.is_atmos\n}}\n"
    )
}

fn generate_profile_cancel_atmos_rego(
    profile_id: &str,
    match_penalty: i32,
    missing_bonus: i32,
) -> String {
    format!(
        "import rego.v1\n\n# scryer-migration:legacy-atmos-disabled:profile:{profile_id}\n\nscore_entry[\"migrated_atmos_cancel_match\"] := -{match_penalty} if {{\n    input.profile.id == \"{profile_id}\"\n    input.release.is_atmos\n}}\n\nscore_entry[\"migrated_atmos_cancel_missing\"] := {missing_bonus} if {{\n    input.profile.id == \"{profile_id}\"\n    not input.release.is_atmos\n}}\n"
    )
}

fn legacy_atmos_rule_values(persona: &ScoringPersona) -> (i32, i32) {
    match persona {
        ScoringPersona::Balanced => (100, -20),
        ScoringPersona::Audiophile => (150, -30),
        ScoringPersona::Efficient => (40, -5),
        ScoringPersona::Compatible => (50, -10),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::null_repositories::test_nulls::{
        NullDownloadClient, NullDownloadClientConfigRepository, NullEventRepository,
        NullIndexerClient, NullReleaseAttemptRepository, NullShowRepository, NullTitleRepository,
        NullUserRepository,
    };
    use async_trait::async_trait;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    #[derive(Default)]
    struct TestQualityProfileRepo {
        profiles: Vec<QualityProfile>,
    }

    #[derive(Default)]
    struct TestIndexerConfigRepo;

    #[async_trait]
    impl IndexerConfigRepository for TestIndexerConfigRepo {
        async fn list(&self, _provider_filter: Option<String>) -> AppResult<Vec<IndexerConfig>> {
            Ok(vec![])
        }

        async fn get_by_id(&self, _id: &str) -> AppResult<Option<IndexerConfig>> {
            Ok(None)
        }

        async fn touch_last_error(&self, _provider_type: &str) -> AppResult<()> {
            Ok(())
        }

        async fn create(&self, config: IndexerConfig) -> AppResult<IndexerConfig> {
            Ok(config)
        }

        async fn update(
            &self,
            _id: &str,
            _name: Option<String>,
            _provider_type: Option<String>,
            _base_url: Option<String>,
            _api_key_encrypted: Option<String>,
            _rate_limit_seconds: Option<i64>,
            _rate_limit_burst: Option<i64>,
            _is_enabled: Option<bool>,
            _enable_interactive_search: Option<bool>,
            _enable_auto_search: Option<bool>,
            _config_json: Option<String>,
        ) -> AppResult<IndexerConfig> {
            Err(AppError::Repository("not configured".into()))
        }

        async fn delete(&self, _id: &str) -> AppResult<()> {
            Ok(())
        }
    }

    #[async_trait]
    impl QualityProfileRepository for TestQualityProfileRepo {
        async fn list_quality_profiles(
            &self,
            _scope: &str,
            _scope_id: Option<String>,
        ) -> AppResult<Vec<QualityProfile>> {
            Ok(self.profiles.clone())
        }
    }

    struct TestRuleSetRepo {
        rules: Mutex<Vec<RuleSet>>,
    }

    impl TestRuleSetRepo {
        fn new(rules: Vec<RuleSet>) -> Self {
            Self {
                rules: Mutex::new(rules),
            }
        }
    }

    #[async_trait]
    impl RuleSetRepository for TestRuleSetRepo {
        async fn list_rule_sets(&self) -> AppResult<Vec<RuleSet>> {
            Ok(self.rules.lock().await.clone())
        }

        async fn list_enabled_rule_sets(&self) -> AppResult<Vec<RuleSet>> {
            Ok(self
                .rules
                .lock()
                .await
                .iter()
                .filter(|rule| rule.enabled)
                .cloned()
                .collect())
        }

        async fn get_rule_set(&self, id: &str) -> AppResult<Option<RuleSet>> {
            Ok(self
                .rules
                .lock()
                .await
                .iter()
                .find(|rule| rule.id == id)
                .cloned())
        }

        async fn create_rule_set(&self, rule_set: &RuleSet) -> AppResult<()> {
            self.rules.lock().await.push(rule_set.clone());
            Ok(())
        }

        async fn update_rule_set(&self, rule_set: &RuleSet) -> AppResult<()> {
            let mut rules = self.rules.lock().await;
            let existing = rules
                .iter_mut()
                .find(|candidate| candidate.id == rule_set.id)
                .ok_or_else(|| AppError::NotFound(rule_set.id.clone()))?;
            *existing = rule_set.clone();
            Ok(())
        }

        async fn delete_rule_set(&self, id: &str) -> AppResult<()> {
            self.rules.lock().await.retain(|rule| rule.id != id);
            Ok(())
        }

        async fn record_rule_set_history(
            &self,
            _rule_set_id: &str,
            _action: &str,
            _rego_source: Option<&str>,
            _actor_id: Option<&str>,
        ) -> AppResult<()> {
            Ok(())
        }

        async fn get_rule_set_by_managed_key(&self, key: &str) -> AppResult<Option<RuleSet>> {
            Ok(self
                .rules
                .lock()
                .await
                .iter()
                .find(|rule| rule.managed_key.as_deref() == Some(key))
                .cloned())
        }

        async fn delete_rule_set_by_managed_key(&self, key: &str) -> AppResult<()> {
            self.rules
                .lock()
                .await
                .retain(|rule| rule.managed_key.as_deref() != Some(key));
            Ok(())
        }

        async fn list_rule_sets_by_managed_key_prefix(
            &self,
            prefix: &str,
        ) -> AppResult<Vec<RuleSet>> {
            Ok(self
                .rules
                .lock()
                .await
                .iter()
                .filter(|rule| {
                    rule.managed_key
                        .as_deref()
                        .is_some_and(|key| key.starts_with(prefix))
                })
                .cloned()
                .collect())
        }
    }

    fn build_test_app(profiles: Vec<QualityProfile>, rules: Vec<RuleSet>) -> AppUseCase {
        let mut services = AppServices::with_default_channels(
            Arc::new(NullTitleRepository),
            Arc::new(NullShowRepository),
            Arc::new(NullUserRepository),
            Arc::new(NullEventRepository),
            Arc::new(TestIndexerConfigRepo),
            Arc::new(NullIndexerClient),
            Arc::new(NullDownloadClient),
            Arc::new(NullDownloadClientConfigRepository),
            Arc::new(NullReleaseAttemptRepository),
            Arc::new(crate::null_repositories::NullSettingsRepository),
            Arc::new(TestQualityProfileRepo { profiles }),
            String::new(),
        );
        services.rule_sets = Arc::new(TestRuleSetRepo::new(rules));

        AppUseCase::new(
            services,
            JwtAuthConfig {
                issuer: "scryer-test".to_string(),
                access_ttl_seconds: 3600,
                jwt_signing_salt: "test-salt".to_string(),
            },
            Arc::new(FacetRegistry::new()),
        )
    }

    fn test_profile(
        id: &str,
        name: &str,
        persona: ScoringPersona,
        atmos_preferred: bool,
        prefer_dual_audio: bool,
    ) -> QualityProfile {
        QualityProfile {
            id: id.to_string(),
            name: name.to_string(),
            criteria: QualityProfileCriteria {
                scoring_persona: persona,
                atmos_preferred,
                prefer_dual_audio,
                ..QualityProfileCriteria::default()
            },
        }
    }

    fn legacy_managed_rule(id: &str, managed_key: &str, name: &str, rego_source: &str) -> RuleSet {
        let now = Utc::now();
        RuleSet {
            id: id.to_string(),
            name: name.to_string(),
            description: String::new(),
            rego_source: rego_source.to_string(),
            enabled: true,
            priority: -100,
            applied_facets: vec![MediaFacet::Anime],
            created_at: now,
            updated_at: now,
            is_managed: true,
            managed_key: Some(managed_key.to_string()),
        }
    }

    fn multi_audio_rule_input(
        profile_id: &str,
        release_is_dual_audio: bool,
        file_has_multiaudio: bool,
    ) -> scryer_rules::UserRuleInput {
        scryer_rules::UserRuleInput {
            release: scryer_rules::ReleaseDoc {
                raw_title: "Test.Movie.2024.2160p.WEB-DL.H.265".to_string(),
                quality: Some("2160P".to_string()),
                source: Some("WEB-DL".to_string()),
                video_codec: Some("H.265".to_string()),
                audio: Some("DDP".to_string()),
                audio_codecs: vec!["DDP".to_string()],
                audio_channels: Some("5.1".to_string()),
                languages_audio: vec!["eng".to_string()],
                languages_subtitles: vec![],
                is_dual_audio: release_is_dual_audio,
                is_atmos: false,
                is_dolby_vision: false,
                detected_hdr: false,
                is_remux: false,
                is_bd_disk: false,
                is_proper_upload: false,
                is_repack: false,
                is_ai_enhanced: false,
                is_hardcoded_subs: false,
                is_hdr10plus: false,
                is_hlg: false,
                is_10bit: false,
                is_uncensored: false,
                is_dubs_only: false,
                has_release_group: true,
                is_obfuscated: false,
                is_retagged: false,
                streaming_service: None,
                edition: None,
                anime_version: None,
                episode_release_type: Some("single_episode".to_string()),
                is_season_pack: false,
                is_multi_episode: false,
                release_group: Some("TestGroup".to_string()),
                year: Some(2024),
                parse_confidence: 0.9,
                size_bytes: Some(8_000_000_000),
                age_days: Some(5),
                thumbs_up: None,
                thumbs_down: None,
                extra: Default::default(),
            },
            profile: scryer_rules::ProfileDoc {
                id: profile_id.to_string(),
                name: "Test Profile".to_string(),
                quality_tiers: vec!["2160P".to_string(), "1080P".to_string(), "720P".to_string()],
                archival_quality: Some("2160P".to_string()),
                allow_unknown_quality: false,
                source_allowlist: vec![],
                source_blocklist: vec![],
                video_codec_allowlist: vec![],
                video_codec_blocklist: vec![],
                audio_codec_allowlist: vec![],
                audio_codec_blocklist: vec![],
                atmos_preferred: false,
                dolby_vision_allowed: true,
                detected_hdr_allowed: true,
                prefer_remux: false,
                allow_bd_disk: false,
                allow_upgrades: true,
                prefer_dual_audio: false,
                required_audio_languages: vec![],
            },
            context: scryer_rules::ContextDoc {
                title_id: Some("tt1234567".to_string()),
                media_type: "movie".to_string(),
                category: "movie".to_string(),
                tags: vec![],
                has_existing_file: false,
                existing_score: None,
                search_mode: "auto".to_string(),
                runtime_minutes: Some(120),
                is_anime: false,
                is_filler: false,
            },
            builtin_score: scryer_rules::BuiltinScoreDoc {
                total: 0,
                blocked: false,
                codes: vec![],
            },
            file: Some(scryer_rules::FileDoc {
                video_codec: Some("hevc".to_string()),
                video_width: Some(3840),
                video_height: Some(2160),
                video_bitrate_kbps: Some(40000),
                video_bit_depth: Some(10),
                video_hdr_format: Some("HDR10".to_string()),
                dovi_profile: Some(8),
                dovi_bl_compat_id: Some(1),
                video_frame_rate: Some("23.976".to_string()),
                video_profile: Some("Main 10".to_string()),
                audio_codec: Some("eac3".to_string()),
                audio_channels: Some(6),
                audio_bitrate_kbps: Some(640),
                audio_languages: vec!["eng".to_string(), "jpn".to_string()],
                audio_streams: vec![scryer_rules::AudioStreamDoc {
                    codec: Some("eac3".to_string()),
                    channels: Some(6),
                    language: Some("eng".to_string()),
                    bitrate_kbps: Some(640),
                }],
                subtitle_languages: vec!["eng".to_string()],
                subtitle_codecs: vec!["subrip".to_string()],
                subtitle_streams: vec![scryer_rules::SubtitleStreamDoc {
                    codec: Some("subrip".to_string()),
                    language: Some("eng".to_string()),
                    name: Some("English".to_string()),
                    forced: false,
                    default: true,
                }],
                has_multiaudio: file_has_multiaudio,
                duration_seconds: Some(7200),
                num_chapters: Some(12),
                container_format: Some("matroska".to_string()),
            }),
        }
    }

    #[tokio::test]
    async fn migration_creates_profile_scoped_multi_audio_rule() {
        let app = build_test_app(
            vec![test_profile(
                "balanced-legacy",
                "Balanced Legacy",
                ScoringPersona::Balanced,
                false,
                true,
            )],
            vec![],
        );

        app.migrate_legacy_persona_preferences().await.unwrap();

        let rules = app.services.rule_sets.list_rule_sets().await.unwrap();
        let migrated = rules
            .iter()
            .find(|rule| rule.name == "Migrated: Prefer Multi-Audio (Balanced Legacy)")
            .expect("expected migrated multi-audio rule");
        assert!(
            migrated
                .description
                .contains("scryer-migration:legacy-prefer-dual-audio:profile:balanced-legacy")
        );
        assert!(migrated.rego_source.contains("input.release.is_dual_audio"));
        assert!(
            migrated
                .rego_source
                .contains("not input.release.is_dual_audio")
        );
        assert!(migrated.rego_source.contains("input.file.has_multiaudio"));
        assert!(!migrated.is_managed);
    }

    #[tokio::test]
    async fn migration_creates_profile_scoped_atmos_rule_for_non_audiophile_profiles() {
        let app = build_test_app(
            vec![test_profile(
                "balanced-atmos",
                "Balanced Atmos",
                ScoringPersona::Balanced,
                true,
                false,
            )],
            vec![],
        );

        app.migrate_legacy_persona_preferences().await.unwrap();

        let rules = app.services.rule_sets.list_rule_sets().await.unwrap();
        let migrated = rules
            .iter()
            .find(|rule| rule.name == "Migrated: Prefer Atmos (Balanced Atmos)")
            .expect("expected migrated atmos rule");
        assert!(
            migrated
                .description
                .contains("scryer-migration:legacy-atmos-preferred:profile:balanced-atmos")
        );
        assert!(migrated.rego_source.contains("migrated_atmos_match"));
        assert!(migrated.rego_source.contains(":= 100 if"));
        assert!(migrated.rego_source.contains(":= -20 if"));
    }

    #[tokio::test]
    async fn migration_creates_cancel_rule_for_audiophile_profiles_that_disabled_atmos() {
        let app = build_test_app(
            vec![test_profile(
                "audiophile-no-atmos",
                "Audiophile No Atmos",
                ScoringPersona::Audiophile,
                false,
                false,
            )],
            vec![],
        );

        app.migrate_legacy_persona_preferences().await.unwrap();

        let rules = app.services.rule_sets.list_rule_sets().await.unwrap();
        let migrated = rules
            .iter()
            .find(|rule| rule.name == "Migrated: Disable Atmos Persona Bias (Audiophile No Atmos)")
            .expect("expected cancel-atmos migration rule");
        assert!(
            migrated
                .description
                .contains("scryer-migration:legacy-atmos-disabled:profile:audiophile-no-atmos")
        );
        assert!(migrated.rego_source.contains("migrated_atmos_cancel_match"));
        assert!(migrated.rego_source.contains(":= -150 if"));
        assert!(migrated.rego_source.contains(":= 30 if"));
    }

    #[tokio::test]
    async fn migration_converts_legacy_managed_multi_audio_rules_once() {
        let legacy = legacy_managed_rule(
            "legacy-rule",
            "convenience:prefer-dual-audio:anime",
            "Prefer Dual Audio (Anime)",
            "import rego.v1\n\nscore_entry[\"managed_dual_audio_preferred\"] := 200 if {\n    input.release.is_dual_audio\n}\n",
        );
        let app = build_test_app(vec![], vec![legacy]);

        app.migrate_legacy_persona_preferences().await.unwrap();
        app.migrate_legacy_persona_preferences().await.unwrap();

        let rules = app.services.rule_sets.list_rule_sets().await.unwrap();
        assert_eq!(
            rules.iter().filter(|rule| rule.is_managed).count(),
            0,
            "legacy managed rule should be removed"
        );
        let migrated: Vec<_> = rules
            .iter()
            .filter(|rule| rule.name == "Migrated: Prefer Dual Audio (Anime)")
            .collect();
        assert_eq!(migrated.len(), 1, "migration should be idempotent");
        assert!(
            migrated[0].description.contains(
                "legacy-convenience-prefer-dual-audio:convenience:prefer-dual-audio:anime"
            )
        );
    }

    #[test]
    fn migrated_multi_audio_rule_scores_once_when_both_release_and_file_match() {
        let policy = scryer_rules::UserPolicy {
            id: "legacy_multi_audio".to_string(),
            name: "Legacy Multi-Audio".to_string(),
            rego_source: scryer_rules::rewrite_package_declaration(
                &generate_profile_prefer_multi_audio_rego("profile-1"),
                "legacy_multi_audio",
            ),
            applied_facets: vec![],
        };

        let engine = scryer_rules::UserRulesEngine::build(&[policy]).unwrap();
        let mut evaluator = engine.evaluator();
        let result = evaluator
            .evaluate(&multi_audio_rule_input("profile-1", true, true), "movie")
            .unwrap();

        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].delta, 200);
    }
}

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

    /// Set or remove a facet-level "prefer dual audio" rule.
    pub async fn set_convenience_prefer_dual_audio(
        &self,
        actor: &User,
        scope: &str,
        enabled: bool,
    ) -> AppResult<()> {
        require(actor, &Entitlement::ManageTitle)?;

        let key = managed_rules::managed_key_prefer_dual_audio(scope);

        if !enabled {
            self.services
                .rule_sets
                .delete_rule_set_by_managed_key(&key)
                .await?;
            self.rebuild_user_rules_engine().await?;
            return Ok(());
        }

        let rego = managed_rules::generate_prefer_dual_audio_rego();
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

    /// Get the current state of all convenience settings by inspecting managed rules.
    pub async fn get_convenience_settings(&self, actor: &User) -> AppResult<ConvenienceSettings> {
        require(actor, &Entitlement::ViewCatalog)?;

        let all_managed = self
            .services
            .rule_sets
            .list_rule_sets_by_managed_key_prefix("convenience:")
            .await?;

        let mut required_audio = Vec::new();
        let mut prefer_dual_audio = Vec::new();

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
            } else if let Some(scope) = key.strip_prefix("convenience:prefer-dual-audio:") {
                prefer_dual_audio.push(ConvenienceBoolSetting {
                    scope: scope.to_string(),
                    enabled: rs.enabled,
                    rule_set_id: Some(rs.id.clone()),
                });
            }
        }

        Ok(ConvenienceSettings {
            required_audio,
            prefer_dual_audio,
        })
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

    pub async fn rebuild_user_rules_engine(&self) -> AppResult<()> {
        let enabled = self.services.rule_sets.list_enabled_rule_sets().await?;

        let mut policies: Vec<scryer_rules::UserPolicy> = enabled
            .iter()
            .map(|rs| scryer_rules::UserPolicy {
                id: rs.id.clone(),
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
    pub prefer_dual_audio: Vec<ConvenienceBoolSetting>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct ConvenienceAudioSetting {
    pub scope: String,
    pub languages: Vec<String>,
    pub rule_set_id: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct ConvenienceBoolSetting {
    pub scope: String,
    pub enabled: bool,
    pub rule_set_id: Option<String>,
}

// ── Helper functions ─────────────────────────────────────────────────────────

fn scope_to_facets(scope: &str) -> Vec<MediaFacet> {
    match scope {
        "movie" => vec![MediaFacet::Movie],
        "series" | "tv" => vec![MediaFacet::Tv],
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

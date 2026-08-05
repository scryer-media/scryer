use super::*;
use scryer_domain::RuleSet;
use scryer_rules::validation::{ValidationResult, validate_user_rule};

fn format_rule_validation_errors(validation: &ValidationResult) -> String {
    format!(
        "Rule validation failed:\n- {}",
        validation.errors.join("\n- ")
    )
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ManagedRuleReconciliation {
    pub created: usize,
    pub updated: usize,
    pub removed: usize,
}

/// The single managed key that earlier releases shipped for French.
///
/// The redesign splits French into three mutually exclusive variants, so this key
/// is no longer in the registry and stale-key pruning removes it. Its
/// hand-picked scores rewarded French dubs — tier bonuses on French release
/// groups with no VOSTFR preference — which is MULTi VF intent, so an install
/// that had it enabled gets `trash-guides:locale:french-vf` enabled in its
/// place. The other two variants stay off; picking between VO and VOSTFR is the
/// user's call, not a migration's.
const LEGACY_FRENCH_MANAGED_KEY: &str = "trash-guides:locale:french";
const LEGACY_FRENCH_SUCCESSOR_KEY: &str = "trash-guides:locale:french-vf";

/// Managed keys of the French family, which are mutually exclusive because
/// their upstream score sets encode contradictory intent.
const FRENCH_PACK_KEY_PREFIX: &str = "trash-guides:locale:french-";

impl AppUseCase {
    pub async fn list_rule_sets(&self, actor: &User) -> AppResult<Vec<RuleSet>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;
        self.services.customization.rule_sets.list_rule_sets().await
    }

    pub async fn get_rule_set(&self, actor: &User, id: &str) -> AppResult<Option<RuleSet>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;
        self.services.customization.rule_sets.get_rule_set(id).await
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "mirrors the rule-set creation contract field for field"
    )]
    pub async fn create_rule_set(
        &self,
        actor: &User,
        name: String,
        description: String,
        rego_source: String,
        applied_facets: Vec<MediaFacet>,
        priority: i32,
        enabled: Option<bool>,
    ) -> AppResult<RuleSet> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        let id = Id::new_rego_safe().0;

        // Rewrite the package declaration to match the system-assigned ID.
        let rewritten_source = scryer_rules::rewrite_package_declaration(&rego_source, &id);

        // Validate the rewritten Rego source
        let validation = validate_user_rule(&rewritten_source, &id)
            .map_err(|e| AppError::Validation(format!("rule validation failed: {e}")))?;
        if !validation.valid {
            return Err(AppError::Validation(format_rule_validation_errors(
                &validation,
            )));
        }

        let now = Utc::now();
        let rule_set = RuleSet {
            id,
            name,
            description,
            rego_source: rewritten_source.clone(),
            enabled: enabled.unwrap_or(true),
            priority,
            applied_facets,
            created_at: now,
            updated_at: now,
            is_managed: false,
            managed_key: None,
            managed_tag_filter: None,
        };

        self.services
            .customization
            .rule_sets
            .create_rule_set(&rule_set)
            .await?;
        self.services
            .customization
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

    #[expect(
        clippy::too_many_arguments,
        reason = "the application boundary mirrors the editable rule-set fields explicitly"
    )]
    pub async fn update_rule_set(
        &self,
        actor: &User,
        id: String,
        name: Option<String>,
        description: Option<String>,
        rego_source: Option<String>,
        applied_facets: Option<Vec<MediaFacet>>,
        priority: Option<i32>,
        managed_tag_filter: Option<Vec<String>>,
    ) -> AppResult<RuleSet> {
        let edits_authored_fields = name.is_some()
            || description.is_some()
            || rego_source.is_some()
            || applied_facets.is_some()
            || priority.is_some();
        if !edits_authored_fields && managed_tag_filter.is_none() {
            return Err(AppError::Validation(
                "at least one rule set field must be provided".into(),
            ));
        }
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        let mut rule_set = self
            .services
            .customization
            .rule_sets
            .get_rule_set(&id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("rule set {id} not found")))?;

        if rule_set.is_managed {
            if edits_authored_fields {
                return Err(AppError::Validation(
                    "This rule is managed by a convenience setting. Change the setting instead of editing the rule directly.".into(),
                ));
            }
            if managed_tag_filter.is_some() {
                return Err(AppError::Validation(
                    "Managed TRaSH Guides locale packs use their predefined locale policy and cannot be filtered.".into(),
                ));
            }
        } else if managed_tag_filter.is_some() {
            return Err(AppError::Validation(
                "A tag filter only applies to managed rule sets.".into(),
            ));
        }

        if let Some(new_source) = &rego_source {
            // Rewrite the package declaration to match the existing rule ID.
            let rewritten = scryer_rules::rewrite_package_declaration(new_source, &rule_set.id);
            let validation = validate_user_rule(&rewritten, &rule_set.id)
                .map_err(|e| AppError::Validation(format!("rule validation failed: {e}")))?;
            if !validation.valid {
                return Err(AppError::Validation(format_rule_validation_errors(
                    &validation,
                )));
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

        self.services
            .customization
            .rule_sets
            .update_rule_set(&rule_set)
            .await?;
        self.services
            .customization
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
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        if let Some(rule_set) = self
            .services
            .customization
            .rule_sets
            .get_rule_set(id)
            .await?
            && rule_set.is_managed
        {
            return Err(AppError::Validation(
                "This rule is managed by a convenience setting. Remove the setting instead of deleting the rule directly.".into(),
            ));
        }

        self.services
            .customization
            .rule_sets
            .delete_rule_set(id)
            .await?;
        self.services
            .customization
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
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        let mut rule_set = self
            .services
            .customization
            .rule_sets
            .get_rule_set(id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("rule set {id} not found")))?;

        if enabled && !rule_set.enabled {
            self.ensure_no_conflicting_french_pack(&rule_set).await?;
        }

        rule_set.enabled = enabled;
        rule_set.updated_at = Utc::now();

        self.services
            .customization
            .rule_sets
            .update_rule_set(&rule_set)
            .await?;
        let action = if enabled { "enabled" } else { "disabled" };
        self.services
            .customization
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
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        // Rewrite the package declaration so validation works regardless of
        // what the user typed.
        let rewritten = scryer_rules::rewrite_package_declaration(rego_source, rule_set_id);
        validate_user_rule(&rewritten, rule_set_id)
            .map_err(|e| AppError::Validation(format!("rule validation error: {e}")))
    }

    // ── Convenience settings ───────────────────────────────────────────────
    /// Set or remove a title-level required audio language override.
    ///
    /// `languages = Some(vec![...])` stores an explicit title override.
    /// `languages = Some(vec![])` stores an explicit "no required language" override.
    /// `languages = None` removes the override and restores inheritance.
    pub async fn set_title_required_audio(
        &self,
        actor: &User,
        title_id: &str,
        facet: &str,
        languages: Option<Vec<String>>,
    ) -> AppResult<()> {
        let _ = facet;
        self.set_title_required_audio_override(actor, title_id, languages)
            .await
    }

    pub async fn migrate_legacy_persona_preferences(&self) -> AppResult<()> {
        const SYSTEM_SCOPE: &str = "system";

        let mut existing_rules = self
            .services
            .customization
            .rule_sets
            .list_rule_sets()
            .await?;
        let profiles = self
            .services
            .config
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
                    "Auto-migrated from the deprecated multi-audio preference toggle.",
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

        let legacy_dual_managed = self
            .services
            .customization
            .rule_sets
            .list_rule_sets_by_managed_key_prefix("convenience:prefer-dual-audio:")
            .await?;
        for rule_set in &legacy_dual_managed {
            let Some(managed_key) = rule_set.managed_key.as_deref() else {
                continue;
            };

            let marker = format!("legacy-convenience-prefer-dual-audio:{managed_key}");
            self.ensure_migrated_rule(
                &mut existing_rules,
                &marker,
                &format!("Migrated: {}", rule_set.name),
                "Auto-migrated from the deprecated managed convenience rule.",
                &generate_prefer_multi_audio_rego(&marker),
                rule_set.applied_facets.clone(),
            )
            .await?;
        }

        for rule_set in legacy_dual_managed {
            self.services
                .customization
                .rule_sets
                .delete_rule_set(&rule_set.id)
                .await?;
        }

        for rule_set in existing_rules {
            if is_legacy_prefer_dual_audio_cleanup_candidate(&rule_set) {
                self.services
                    .customization
                    .rule_sets
                    .delete_rule_set(&rule_set.id)
                    .await?;
            }
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
            managed_tag_filter: None,
        };
        self.services
            .customization
            .rule_sets
            .create_rule_set(&rule_set)
            .await?;
        existing_rules.push(rule_set);
        Ok(())
    }

    /// Seed, update, and prune compiled TRaSH locale packs.
    ///
    /// Callers choose whether reconciliation should rebuild the in-memory engine.
    /// Startup wiring can therefore reconcile before its normal engine rebuild.
    pub async fn reconcile_managed_trash_rule_packs(
        &self,
        rebuild_engine: bool,
    ) -> AppResult<ManagedRuleReconciliation> {
        self.reconcile_managed_trash_rule_packs_from_registry(
            managed_trash::managed_trash_rule_packs(),
            rebuild_engine,
        )
        .await
    }

    /// Load a safe engine without managed TRaSH packs before reconciliation, so
    /// neither a current failure nor partial rows from an earlier process can be
    /// activated. Successful reconciliation replaces it with the complete set.
    pub async fn reconcile_and_activate_managed_trash_rule_packs(
        &self,
    ) -> AppResult<ManagedRuleReconciliation> {
        self.rebuild_user_rules_engine_filtered(true).await?;
        let reconciliation = self.reconcile_managed_trash_rule_packs(false).await?;
        self.rebuild_user_rules_engine().await?;
        Ok(reconciliation)
    }

    async fn reconcile_managed_trash_rule_packs_from_registry(
        &self,
        packs: &[managed_trash::ManagedTrashRulePack],
        rebuild_engine: bool,
    ) -> AppResult<ManagedRuleReconciliation> {
        validate_managed_trash_rule_packs(packs)?;
        let expected_keys = packs
            .iter()
            .map(|pack| pack.key)
            .collect::<std::collections::HashSet<_>>();
        let mut reconciliation = ManagedRuleReconciliation::default();

        // The legacy French row is pruned below, so its opt-in has to be read
        // before the loop that would otherwise create its successor disabled.
        let legacy_french_enabled = self
            .services
            .customization
            .rule_sets
            .get_rule_set_by_managed_key(LEGACY_FRENCH_MANAGED_KEY)
            .await?
            .is_some_and(|legacy| legacy.enabled);

        for pack in packs {
            let inherits_legacy_french =
                legacy_french_enabled && pack.key == LEGACY_FRENCH_SUCCESSOR_KEY;
            match self
                .services
                .customization
                .rule_sets
                .get_rule_set_by_managed_key(pack.key)
                .await?
            {
                Some(mut rule_set) => {
                    // Locale packs have a predefined policy. Legacy user-defined
                    // filters are intentionally cleared during reconciliation.
                    let tag_filter = None;
                    let enabled = rule_set.enabled || inherits_legacy_french;
                    let source = scryer_rules::rewrite_package_declaration(
                        &pack.source(tag_filter.as_deref()),
                        &rule_set.id,
                    );
                    let changed = rule_set.name != pack.name
                        || rule_set.description != pack.description
                        || rule_set.rego_source != source
                        || rule_set.priority != 0
                        || rule_set.applied_facets != pack.applied_facets
                        || rule_set.managed_tag_filter != tag_filter
                        || rule_set.enabled != enabled
                        || !rule_set.is_managed;
                    if changed {
                        rule_set.name = pack.name.to_string();
                        rule_set.description = pack.description.to_string();
                        rule_set.rego_source = source;
                        rule_set.priority = 0;
                        rule_set.applied_facets = pack.applied_facets.to_vec();
                        rule_set.managed_tag_filter = tag_filter;
                        rule_set.enabled = enabled;
                        rule_set.is_managed = true;
                        rule_set.updated_at = Utc::now();
                        self.services
                            .customization
                            .rule_sets
                            .update_rule_set(&rule_set)
                            .await?;
                        self.services
                            .customization
                            .rule_sets
                            .record_rule_set_history(
                                &rule_set.id,
                                "managed_updated",
                                Some(&rule_set.rego_source),
                                None,
                            )
                            .await?;
                        reconciliation.updated += 1;
                    }
                }
                None => {
                    let now = Utc::now();
                    let id = Id::new_rego_safe().0;
                    // A pack the user has never seen ships off and ungated.
                    // Enabling it is the opt-in for its predefined locale policy.
                    let tag_filter = None;
                    let source = scryer_rules::rewrite_package_declaration(
                        &pack.source(tag_filter.as_deref()),
                        &id,
                    );
                    let rule_set = RuleSet {
                        id,
                        name: pack.name.to_string(),
                        description: pack.description.to_string(),
                        rego_source: source,
                        enabled: inherits_legacy_french,
                        priority: 0,
                        applied_facets: pack.applied_facets.to_vec(),
                        created_at: now,
                        updated_at: now,
                        is_managed: true,
                        managed_key: Some(pack.key.to_string()),
                        managed_tag_filter: tag_filter,
                    };
                    self.services
                        .customization
                        .rule_sets
                        .create_rule_set(&rule_set)
                        .await?;
                    self.services
                        .customization
                        .rule_sets
                        .record_rule_set_history(
                            &rule_set.id,
                            "managed_created",
                            Some(&rule_set.rego_source),
                            None,
                        )
                        .await?;
                    reconciliation.created += 1;
                }
            }
        }

        for stale in self
            .services
            .customization
            .rule_sets
            .list_rule_sets_by_managed_key_prefix(managed_trash::MANAGED_TRASH_KEY_PREFIX)
            .await?
            .into_iter()
            .filter(|rule_set| {
                rule_set
                    .managed_key
                    .as_deref()
                    .is_some_and(|key| !expected_keys.contains(key))
            })
        {
            self.services
                .customization
                .rule_sets
                .record_rule_set_history(&stale.id, "managed_removed", None, None)
                .await?;
            self.services
                .customization
                .rule_sets
                .delete_rule_set_by_managed_key(stale.managed_key.as_deref().unwrap_or_default())
                .await?;
            reconciliation.removed += 1;
        }

        if rebuild_engine {
            self.rebuild_user_rules_engine().await?;
        }

        Ok(reconciliation)
    }

    pub async fn rebuild_user_rules_engine(&self) -> AppResult<()> {
        self.rebuild_user_rules_engine_filtered(false).await
    }

    async fn rebuild_user_rules_engine_filtered(
        &self,
        exclude_managed_trash: bool,
    ) -> AppResult<()> {
        let enabled = self
            .services
            .customization
            .rule_sets
            .list_enabled_rule_sets()
            .await?;

        let mut policies: Vec<scryer_rules::UserPolicy> = enabled
            .iter()
            .filter(|rule_set| {
                !exclude_managed_trash
                    || !rule_set
                        .managed_key
                        .as_deref()
                        .is_some_and(|key| key.starts_with(managed_trash::MANAGED_TRASH_KEY_PREFIX))
            })
            .map(|rs| scryer_rules::UserPolicy {
                id: rs.id.clone(),
                name: rs.name.clone(),
                rego_source: rs.rego_source.clone(),
                origin: if rs.is_managed {
                    scryer_rules::PolicyOrigin::System
                } else {
                    scryer_rules::PolicyOrigin::User
                },
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
        if let Some(pp) = self.services.integrations.plugin_provider.available() {
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
            .customization
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

    /// The three French packs read score sets that contradict each
    /// other, so only one may be live at a time.
    async fn ensure_no_conflicting_french_pack(&self, rule_set: &RuleSet) -> AppResult<()> {
        let Some(key) = rule_set
            .managed_key
            .as_deref()
            .filter(|key| key.starts_with(FRENCH_PACK_KEY_PREFIX))
        else {
            return Ok(());
        };

        let conflicting = self
            .services
            .customization
            .rule_sets
            .list_rule_sets_by_managed_key_prefix(FRENCH_PACK_KEY_PREFIX)
            .await?
            .into_iter()
            .find(|candidate| {
                candidate.enabled && candidate.managed_key.as_deref().is_some_and(|k| k != key)
            });

        match conflicting {
            Some(conflicting) => Err(AppError::Validation(format!(
                "\"{}\" is already enabled. The French locale packs read contradictory TRaSH \
                 Guides score sets, so disable it before enabling another French pack.",
                conflicting.name
            ))),
            None => Ok(()),
        }
    }
}

fn validate_managed_trash_rule_packs(
    packs: &[managed_trash::ManagedTrashRulePack],
) -> AppResult<()> {
    for (index, pack) in packs.iter().enumerate() {
        let id = format!("managed_trash_validation_{index}");
        let source = scryer_rules::rewrite_package_declaration(&pack.source(None), &id);
        let validation =
            scryer_rules::validation::validate_managed_rule(&source, &id).map_err(|error| {
                AppError::Validation(format!("managed rule validation failed: {error}"))
            })?;
        if !validation.valid {
            return Err(AppError::Validation(format!(
                "managed rule pack {} is invalid: {}",
                pack.key,
                validation.errors.join("; ")
            )));
        }
    }

    Ok(())
}

// ── Helper functions ─────────────────────────────────────────────────────────

fn generate_profile_prefer_multi_audio_rego(profile_id: &str) -> String {
    format!(
        "import rego.v1\n\n# scryer-migration:legacy-prefer-dual-audio:profile:{profile_id}\n\nscore_entry[\"migrated_prefer_multi_audio\"] := 200 if {{\n    input.profile.id == \"{profile_id}\"\n    input.release.is_dual_audio\n}}\n\nscore_entry[\"migrated_prefer_multi_audio_file\"] := 200 if {{\n    input.profile.id == \"{profile_id}\"\n    not input.release.is_dual_audio\n    input.file != null\n    input.file.has_multiaudio\n}}\n"
    )
}

fn generate_prefer_multi_audio_rego(migration_key: &str) -> String {
    format!(
        "import rego.v1\n\n# scryer-migration:{migration_key}\n\nscore_entry[\"migrated_prefer_multi_audio\"] := 200 if {{\n    input.release.is_dual_audio\n}}\n\nscore_entry[\"migrated_prefer_multi_audio_file\"] := 200 if {{\n    not input.release.is_dual_audio\n    input.file != null\n    input.file.has_multiaudio\n}}\n"
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

fn is_legacy_prefer_dual_audio_cleanup_candidate(rule_set: &RuleSet) -> bool {
    let has_legacy_marker = rule_set.description.contains("legacy-prefer-dual-audio:")
        || rule_set.rego_source.contains("legacy-prefer-dual-audio:");
    let is_migrated_rule = rule_set
        .description
        .contains("scryer-migration:legacy-prefer-dual-audio:")
        || rule_set
            .rego_source
            .contains("scryer-migration:legacy-prefer-dual-audio:");

    has_legacy_marker && !is_migrated_rule
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::null_repositories::test_nulls::{
        NullDownloadClient, NullDownloadClientConfigRepository, NullIndexerClient,
        NullReleaseAttemptRepository, NullShowRepository, NullTitleRepository, NullUserRepository,
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

        async fn update(&self, _update: crate::IndexerConfigUpdate) -> AppResult<IndexerConfig> {
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

        async fn replace_quality_profiles(
            &self,
            _scope: &str,
            _scope_id: Option<String>,
            _profiles: Vec<QualityProfile>,
        ) -> AppResult<()> {
            Ok(())
        }
    }

    #[derive(Clone, Debug, Default, PartialEq, Eq)]
    struct RuleSetMutationCounts {
        creates: usize,
        updates: usize,
        deletes: usize,
    }

    struct TestRuleSetRepo {
        rules: Mutex<Vec<RuleSet>>,
        mutations: Mutex<RuleSetMutationCounts>,
        fail_on_create: Option<usize>,
    }

    impl TestRuleSetRepo {
        fn new(rules: Vec<RuleSet>) -> Self {
            Self {
                rules: Mutex::new(rules),
                mutations: Mutex::new(RuleSetMutationCounts::default()),
                fail_on_create: None,
            }
        }

        fn failing_on_create(rules: Vec<RuleSet>, create_number: usize) -> Self {
            Self {
                rules: Mutex::new(rules),
                mutations: Mutex::new(RuleSetMutationCounts::default()),
                fail_on_create: Some(create_number),
            }
        }

        async fn mutation_counts(&self) -> RuleSetMutationCounts {
            self.mutations.lock().await.clone()
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
            let create_number = {
                let mut mutations = self.mutations.lock().await;
                mutations.creates += 1;
                mutations.creates
            };
            if self.fail_on_create == Some(create_number) {
                return Err(AppError::Repository(format!(
                    "injected create failure #{create_number}"
                )));
            }
            self.rules.lock().await.push(rule_set.clone());
            Ok(())
        }

        async fn update_rule_set(&self, rule_set: &RuleSet) -> AppResult<()> {
            self.mutations.lock().await.updates += 1;
            let mut rules = self.rules.lock().await;
            let existing = rules
                .iter_mut()
                .find(|candidate| candidate.id == rule_set.id)
                .ok_or_else(|| AppError::NotFound(rule_set.id.clone()))?;
            *existing = rule_set.clone();
            Ok(())
        }

        async fn delete_rule_set(&self, id: &str) -> AppResult<()> {
            self.mutations.lock().await.deletes += 1;
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
            self.mutations.lock().await.deletes += 1;
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
        build_test_app_with_rule_repo(profiles, rules).0
    }

    fn build_test_app_with_rule_repo(
        profiles: Vec<QualityProfile>,
        rules: Vec<RuleSet>,
    ) -> (AppUseCase, Arc<TestRuleSetRepo>) {
        let rule_sets = Arc::new(TestRuleSetRepo::new(rules));
        let app = build_test_app_with_existing_rule_repo(profiles, rule_sets.clone());
        (app, rule_sets)
    }

    fn build_test_app_with_existing_rule_repo(
        profiles: Vec<QualityProfile>,
        rule_sets: Arc<TestRuleSetRepo>,
    ) -> AppUseCase {
        let services = AppServices::builder(
            Arc::new(NullTitleRepository),
            Arc::new(NullShowRepository),
            Arc::new(NullUserRepository),
            Arc::new(TestIndexerConfigRepo),
            Arc::new(NullIndexerClient),
            Arc::new(NullDownloadClient),
            Arc::new(NullDownloadClientConfigRepository),
            Arc::new(NullReleaseAttemptRepository),
            Arc::new(crate::null_repositories::NullSettingsRepository),
            Arc::new(TestQualityProfileRepo { profiles }),
            String::new(),
        )
        .with_rule_sets(rule_sets.clone())
        .build_partial_for_tests();

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
            managed_tag_filter: None,
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
                is_password_protected: None,
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
                guide_facts: vec![],
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
                library_name: Some("Movies".to_string()),
                media_type: "movie".to_string(),
                category: "movie".to_string(),
                original_language: Some("eng".to_string()),
                original_country: Some("US".to_string()),
                inferred_original_audio_language: "eng".to_string(),
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
                audio_profile: Some("Dolby Digital Plus + Dolby Atmos".to_string()),
                audio_channels: Some(6),
                audio_bitrate_kbps: Some(640),
                audio_languages: vec!["eng".to_string(), "jpn".to_string()],
                audio_streams: vec![scryer_rules::AudioStreamDoc {
                    codec: Some("eac3".to_string()),
                    profile: Some("Dolby Digital Plus + Dolby Atmos".to_string()),
                    channels: Some(6),
                    language: Some("eng".to_string()),
                    name: None,
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

        let rules = app
            .services
            .customization
            .rule_sets
            .list_rule_sets()
            .await
            .unwrap();
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

        let rules = app
            .services
            .customization
            .rule_sets
            .list_rule_sets()
            .await
            .unwrap();
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

        let rules = app
            .services
            .customization
            .rule_sets
            .list_rule_sets()
            .await
            .unwrap();
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

        let rules = app
            .services
            .customization
            .rule_sets
            .list_rule_sets()
            .await
            .unwrap();
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
    fn managed_locale_packs_match_exact_namespaced_guide_facts() {
        let cases = [
            (
                "trash-guides:locale:french-vf",
                &["fra" as &str][..],
                &[][..],
                &[
                    "trash.locale.french.group.tier1",
                    "trash.locale.french.marker.vff",
                    "trash.locale.french.marker.vfi",
                    "trash.locale.french.marker.vof",
                    "trash.locale.french.marker.vfq",
                    "trash.locale.french.marker.vq",
                    "trash.locale.french.marker.voq",
                    "trash.locale.french.marker.vostfr",
                ][..],
                // The MULTi.VF set scores `language-not-french` as
                // a veto, and this release carries English audio only.
                &[
                    ("trash_tier_1", 245),
                    ("trash_french_vostfr", 0),
                    ("trash_lang_not_french", -10_000),
                ][..],
            ),
            (
                "trash-guides:locale:french-vf",
                &["fr-fr" as &str][..],
                &[][..],
                &[
                    "trash.locale.french.marker.vff",
                    "trash.locale.french.marker.vfi",
                    "trash.locale.french.marker.vof",
                    "trash.locale.french.marker.vfq",
                    "trash.locale.french.marker.vq",
                    "trash.locale.french.marker.voq",
                ][..],
                &[
                    ("trash_french_fr_fr_reference", 40),
                    ("trash_french_fr_fr_quebec", -20),
                    ("trash_lang_not_french", -10_000),
                ][..],
            ),
            (
                "trash-guides:locale:french-vf",
                &[][..],
                &["locale:fr-ca"][..],
                &[
                    "trash.locale.french.marker.vff",
                    "trash.locale.french.marker.vfi",
                    "trash.locale.french.marker.vof",
                    "trash.locale.french.marker.vfq",
                    "trash.locale.french.marker.vq",
                    "trash.locale.french.marker.voq",
                ][..],
                &[
                    ("trash_french_fr_ca_reference", -20),
                    ("trash_french_fr_ca_quebec", 40),
                    ("trash_lang_not_french", -10_000),
                ][..],
            ),
            // The VOSTFR variant reads a score set that neutralizes the French
            // dub tiers and rewards the subbed marker instead.
            (
                "trash-guides:locale:french-vostfr",
                &["fra" as &str][..],
                &[][..],
                &[
                    "trash.locale.french.group.tier1",
                    "trash.locale.french.marker.vostfr",
                ][..],
                &[("trash_tier_1", 0), ("trash_french_vostfr", 181)][..],
            ),
            (
                "trash-guides:locale:german",
                &["deu" as &str][..],
                &[][..],
                &[
                    "trash.locale.german.group.tier2",
                    "trash.locale.german.marker.subbed",
                ][..],
                &[("trash_tier_2", 146), ("trash_german_subbed", 329)][..],
            ),
            (
                "trash-guides:locale:asian",
                &[][..],
                &["locale:asian"][..],
                &["trash.locale.asian.group.tier3"][..],
                &[("trash_tier_3", 100)][..],
            ),
        ];

        for (key, languages, tags, facts, expected) in cases {
            let pack = managed_trash::managed_trash_rule_packs()
                .iter()
                .find(|pack| pack.key == key)
                .unwrap();
            let id = key.replace([':', '-'], "_");
            let policy = scryer_rules::UserPolicy {
                id: id.clone(),
                name: pack.name.to_string(),
                rego_source: scryer_rules::rewrite_package_declaration(
                    &pack.source(None),
                    &id,
                ),
                origin: scryer_rules::PolicyOrigin::System,
                applied_facets: vec![],
            };
            let mut input = multi_audio_rule_input("locale-profile", false, false);
            input.profile.required_audio_languages =
                languages.iter().map(|value| (*value).to_string()).collect();
            input.context.tags = tags.iter().map(|value| (*value).to_string()).collect();
            input.release.guide_facts = facts.iter().map(|value| (*value).to_string()).collect();

            let mut evaluator = scryer_rules::UserRulesEngine::build(&[policy])
                .unwrap()
                .evaluator();
            let result = evaluator.evaluate(&input, "movie").unwrap();
            assert!(result.errors.is_empty(), "{key}: {result:?}");
            let mut actual = result
                .entries
                .iter()
                .map(|entry| (entry.code.as_str(), entry.delta))
                .collect::<Vec<_>>();
            actual.sort_unstable();
            let mut expected = expected.to_vec();
            expected.sort_unstable();
            assert_eq!(actual, expected, "{key}");
            assert!(
                result
                    .entries
                    .iter()
                    .all(|entry| entry.origin == scryer_rules::PolicyOrigin::System)
            );
        }

    }

    /// An enabled pack with no tag filter is the user's opt-in, so
    /// it applies wherever its facts match — the same titles the filtered pack
    /// would score, minus the locale gate.
    #[test]
    fn unfiltered_managed_locale_pack_scores_without_any_locale_intent() {
        let french = &managed_trash::managed_trash_rule_packs()[0];
        let id = "locale_open_gate";
        let policy = scryer_rules::UserPolicy {
            id: id.to_string(),
            name: french.name.to_string(),
            rego_source: scryer_rules::rewrite_package_declaration(&french.source(None), id),
            origin: scryer_rules::PolicyOrigin::System,
            applied_facets: vec![],
        };
        let mut input = multi_audio_rule_input("locale-profile", false, false);
        input.release.guide_facts = vec!["trash.locale.french.group.tier1".to_string()];
        let result = scryer_rules::UserRulesEngine::build(&[policy])
            .unwrap()
            .evaluator()
            .evaluate(&input, "movie")
            .unwrap();
        assert!(result.errors.is_empty(), "{result:?}");
        let mut entries = result
            .entries
            .iter()
            .map(|entry| (entry.code.as_str(), entry.delta))
            .collect::<Vec<_>>();
        entries.sort_unstable();
        // The open gate lets the language clauses through too, and
        // MULTi.VF vetoes a release with no French audio.
        assert_eq!(
            entries,
            vec![("trash_lang_not_french", -10_000), ("trash_tier_1", 245)]
        );
    }

    /// Upstream scores the locale LQ formats as vetoes, so the derived packs
    /// emit `BLOCK_SCORE`. The veto is admitted because the pack is
    /// opt-in: the user who enabled it asked for TRaSH's locale policy,
    /// including the part that refuses a release outright.
    #[test]
    fn managed_locale_veto_survives_the_managed_score_bound() {
        let asian = managed_trash::managed_trash_rule_packs()
            .iter()
            .find(|pack| pack.key == "trash-guides:locale:asian")
            .unwrap();
        let id = "locale_veto";
        let source = asian.source(None);
        let policy = scryer_rules::UserPolicy {
            id: id.to_string(),
            name: asian.name.to_string(),
            rego_source: scryer_rules::rewrite_package_declaration(&source, id),
            origin: scryer_rules::PolicyOrigin::System,
            applied_facets: vec![],
        };
        assert!(source.contains(r#"score_entry["trash_lq"] := -10000"#));

        let mut input = multi_audio_rule_input("locale-profile", false, false);
        input.context.tags = vec!["locale:asian".to_string()];
        input.release.guide_facts = vec!["trash.locale.asian.lq".to_string()];
        let result = scryer_rules::UserRulesEngine::build(&[policy])
            .unwrap()
            .evaluator()
            .evaluate(&input, "movie")
            .unwrap();
        assert!(result.errors.is_empty(), "{result:?}");
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].code, "trash_lq");
        assert_eq!(result.entries[0].delta, scryer_rules::BLOCK_SCORE);
    }

    #[tokio::test]
    async fn reconcile_managed_locale_packs_preserves_disabled_rows_and_prunes_stale_keys() {
        let now = Utc::now();
        let existing = RuleSet {
            id: "preserved-french".to_string(),
            name: "Old French".to_string(),
            description: "old".to_string(),
            rego_source: "old".to_string(),
            enabled: false,
            priority: 99,
            applied_facets: vec![MediaFacet::Movie],
            created_at: now,
            updated_at: now,
            is_managed: true,
            managed_key: Some("trash-guides:locale:french-vf".to_string()),
            managed_tag_filter: None,
        };
        let stale =
            legacy_managed_rule("stale-pack", "trash-guides:locale:obsolete", "Stale", "old");
        let app = build_test_app(vec![], vec![existing, stale]);

        let reconciliation = app.reconcile_managed_trash_rule_packs(false).await.unwrap();
        assert_eq!(reconciliation.created, 4);
        assert_eq!(reconciliation.updated, 1);
        assert_eq!(reconciliation.removed, 1);

        let rules = app
            .services
            .customization
            .rule_sets
            .list_rule_sets()
            .await
            .unwrap();
        assert_eq!(rules.len(), 5);
        let french = rules
            .iter()
            .find(|rule| rule.managed_key.as_deref() == Some("trash-guides:locale:french-vf"))
            .unwrap();
        assert_eq!(french.id, "preserved-french");
        assert_eq!(french.created_at, now);
        assert!(!french.enabled);
        assert!(
            french
                .rego_source
                .contains("MANAGED_TRASH_REGISTRY_VERSION=managed-trash-registry-v2")
        );
        // A disabled row was never applying, so nothing has to be preserved and
        // it is not retroactively gated.
        assert_eq!(french.managed_tag_filter, None);
        assert!(!rules.iter().any(|rule| rule.id == "stale-pack"));
    }

    /// Packs a user has never seen ship off and ungated.
    #[tokio::test]
    async fn freshly_created_managed_locale_packs_are_disabled_and_unfiltered() {
        let app = build_test_app(vec![], vec![]);

        let reconciliation = app.reconcile_managed_trash_rule_packs(false).await.unwrap();
        assert_eq!(reconciliation.created, 5);

        let rules = app
            .services
            .customization
            .rule_sets
            .list_rule_sets()
            .await
            .unwrap();
        assert_eq!(rules.len(), 5);
        assert!(rules.iter().all(|rule| !rule.enabled), "{rules:#?}");
        assert!(
            rules
                .iter()
                .all(|rule| rule.managed_tag_filter.is_none() && rule.is_managed),
            "{rules:#?}"
        );
        assert!(
            rules
                .iter()
                .all(|rule| rule.rego_source.contains("locale_intent := true")),
            "{rules:#?}"
        );
    }

    /// Upgrading clears legacy tag filters so every pack follows its
    /// predefined locale policy.
    #[tokio::test]
    async fn reconciliation_clears_legacy_locale_pack_filters() {
        let enabled_v1 = legacy_managed_rule(
            "existing-german",
            "trash-guides:locale:german",
            "Old German",
            "# MANAGED_TRASH_REGISTRY_VERSION=managed-trash-registry-v1\nlocale_intent := true",
        );
        let mut disabled_v1 = legacy_managed_rule(
            "existing-asian",
            "trash-guides:locale:asian",
            "Old Asian",
            "# MANAGED_TRASH_REGISTRY_VERSION=managed-trash-registry-v1\nlocale_intent := true",
        );
        disabled_v1.enabled = false;
        let app = build_test_app(vec![], vec![enabled_v1, disabled_v1]);

        app.reconcile_managed_trash_rule_packs(false).await.unwrap();

        let rules = app
            .services
            .customization
            .rule_sets
            .list_rule_sets()
            .await
            .unwrap();
        let german = rules
            .iter()
            .find(|rule| rule.id == "existing-german")
            .unwrap();
        assert!(german.enabled);
        assert_eq!(german.managed_tag_filter, None);
        assert!(german.rego_source.contains("locale_intent := true"));

        let asian = rules
            .iter()
            .find(|rule| rule.id == "existing-asian")
            .unwrap();
        assert!(!asian.enabled);
        assert_eq!(asian.managed_tag_filter, None);

        // Reconciliation remains a no-op after the legacy filter is cleared.
        let second = app.reconcile_managed_trash_rule_packs(false).await.unwrap();
        assert_eq!(second, ManagedRuleReconciliation::default());
    }

    /// The pre-split French pack carries forward into MULTi VF
    /// and nothing else.
    #[tokio::test]
    async fn enabled_legacy_french_pack_migrates_into_the_vf_variant() {
        let legacy = legacy_managed_rule(
            "legacy-french",
            "trash-guides:locale:french",
            "TRaSH Guides French Locale",
            "# MANAGED_TRASH_REGISTRY_VERSION=managed-trash-registry-v1\nlocale_intent := true",
        );
        let app = build_test_app(vec![], vec![legacy]);

        let reconciliation = app.reconcile_managed_trash_rule_packs(false).await.unwrap();
        assert_eq!(reconciliation.created, 5);
        assert_eq!(reconciliation.removed, 1);

        let rules = app
            .services
            .customization
            .rule_sets
            .list_rule_sets()
            .await
            .unwrap();
        assert!(
            !rules
                .iter()
                .any(|rule| rule.managed_key.as_deref() == Some("trash-guides:locale:french"))
        );
        let vf = rules
            .iter()
            .find(|rule| rule.managed_key.as_deref() == Some("trash-guides:locale:french-vf"))
            .unwrap();
        assert!(vf.enabled);
        assert_eq!(vf.managed_tag_filter, None);
        assert!(vf.rego_source.contains("locale_intent := true"));
        // Reconciliation never produces two enabled French packs.
        assert_eq!(
            rules
                .iter()
                .filter(|rule| rule.enabled
                    && rule
                        .managed_key
                        .as_deref()
                        .is_some_and(|key| key.starts_with("trash-guides:locale:french-")))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn invalid_managed_pack_prevents_all_reconciliation_mutations() {
        fn invalid_source(_tag_filter: Option<&[String]>) -> String {
            r#"score_entry["blocked"] := 1.5"#.to_string()
        }

        let (app, rule_sets) = build_test_app_with_rule_repo(vec![], vec![]);
        let packs = [managed_trash::ManagedTrashRulePack {
            key: "trash-guides:locale:invalid",
            name: "Invalid",
            description: "Invalid managed fixture.",
            applied_facets: &[],
            source: invalid_source,
        }];

        let error = app
            .reconcile_managed_trash_rule_packs_from_registry(&packs, false)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("managed rule pack"));
        assert_eq!(
            rule_sets.mutation_counts().await,
            RuleSetMutationCounts::default()
        );
    }

    #[tokio::test]
    async fn failed_startup_reconciliation_never_activates_partial_managed_packs() {
        let partial = legacy_managed_rule(
            "partial-french",
            "trash-guides:locale:french",
            "Partial French",
            "old source",
        );
        let rule_sets = Arc::new(TestRuleSetRepo::failing_on_create(vec![partial], 2));
        let app = build_test_app_with_existing_rule_repo(vec![], rule_sets.clone());

        let error = app
            .reconcile_and_activate_managed_trash_rule_packs()
            .await
            .unwrap_err();
        assert!(error.to_string().contains("injected create failure"));
        assert_eq!(rule_sets.mutation_counts().await.creates, 2);

        let mut input = multi_audio_rule_input("locale-profile", false, false);
        input.profile.required_audio_languages = vec!["fra".to_string()];
        input.release.guide_facts = vec!["trash.locale.french.group.tier1".to_string()];
        let engine = app
            .services
            .customization
            .user_rules
            .read()
            .expect("rules engine lock");
        let result = engine.evaluator().evaluate(&input, "movie").unwrap();
        assert!(
            result
                .entries
                .iter()
                .all(|entry| entry.code != "trash_tier_1")
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
            origin: scryer_rules::PolicyOrigin::User,
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

    fn config_admin() -> User {
        User {
            id: scryer_domain::Id::new().0,
            username: "config-admin".to_string(),
            password_hash: None,
            account_kind: Default::default(),
            authorization: scryer_domain::UserAuthorization {
                app: scryer_domain::AppPermissionMask::from_permissions([
                    scryer_domain::AppPermission::ManageCatalogSettings,
                ]),
                loaded: true,
                ..Default::default()
            },
        }
    }

    async fn reconciled_app() -> AppUseCase {
        let app = build_test_app(vec![], vec![]);
        app.reconcile_managed_trash_rule_packs(false).await.unwrap();
        app
    }

    async fn managed_row(app: &AppUseCase, key: &str) -> RuleSet {
        app.services
            .customization
            .rule_sets
            .get_rule_set_by_managed_key(key)
            .await
            .unwrap()
            .unwrap()
    }

    /// The French variants read contradictory score sets, so only
    /// one may be live.
    #[tokio::test]
    async fn enabling_a_second_french_pack_is_rejected() {
        let app = reconciled_app().await;
        let actor = config_admin();
        let vf = managed_row(&app, "trash-guides:locale:french-vf").await;
        let vostfr = managed_row(&app, "trash-guides:locale:french-vostfr").await;
        let german = managed_row(&app, "trash-guides:locale:german").await;

        app.toggle_rule_set(&actor, &vf.id, true).await.unwrap();

        let error = app
            .toggle_rule_set(&actor, &vostfr.id, true)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("already enabled"), "{error}");
        assert!(
            !managed_row(&app, "trash-guides:locale:french-vostfr")
                .await
                .enabled
        );

        // A different locale is unaffected, and the variant becomes available
        // once the conflicting one is off.
        app.toggle_rule_set(&actor, &german.id, true).await.unwrap();
        app.toggle_rule_set(&actor, &vf.id, false).await.unwrap();
        app.toggle_rule_set(&actor, &vostfr.id, true).await.unwrap();
        assert!(
            managed_row(&app, "trash-guides:locale:french-vostfr")
                .await
                .enabled
        );
    }

    #[tokio::test]
    async fn managed_locale_pack_tag_filters_are_rejected() {
        let app = reconciled_app().await;
        let actor = config_admin();
        let asian = managed_row(&app, "trash-guides:locale:asian").await;

        let error = app
            .update_rule_set(
                &actor,
                asian.id,
                None,
                None,
                None,
                None,
                None,
                Some(vec!["Locale:Asian".to_string()]),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("predefined locale policy"));
    }

    #[tokio::test]
    async fn managed_tag_filter_is_rejected_for_user_rule_sets() {
        let app = build_test_app(vec![], vec![]);
        let actor = config_admin();
        let rule_set = app
            .create_rule_set(
                &actor,
                "User Rule".to_string(),
                String::new(),
                r#"score_entry["bonus"] := 10"#.to_string(),
                vec![],
                0,
                Some(true),
            )
            .await
            .unwrap();

        let error = app
            .update_rule_set(
                &actor,
                rule_set.id,
                None,
                None,
                None,
                None,
                None,
                Some(vec!["locale:french".to_string()]),
            )
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("only applies to managed"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn managed_rows_still_reject_authored_field_edits() {
        let app = reconciled_app().await;
        let actor = config_admin();
        let german = managed_row(&app, "trash-guides:locale:german").await;

        let error = app
            .update_rule_set(
                &actor,
                german.id,
                Some("Renamed".to_string()),
                None,
                None,
                None,
                None,
                Some(vec!["locale:german".to_string()]),
            )
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("managed by a convenience setting"),
            "{error}"
        );
    }
}

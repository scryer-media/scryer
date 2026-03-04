use super::*;
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
            return Err(AppError::Validation(
                validation.errors.join("; "),
            ));
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
        if let Some(n) = name { rule_set.name = n; }
        if let Some(d) = description { rule_set.description = d; }
        if let Some(f) = applied_facets { rule_set.applied_facets = f; }
        if let Some(p) = priority { rule_set.priority = p; }
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
                    p.rego_source = scryer_rules::rewrite_package_declaration(&p.rego_source, &p.id);
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

use super::*;
use crate::plugins::plugins::VerifiedRulePack;
use scryer_domain::{AppPermission, Id, MediaFacet, RulePackInstallation, RulePackMember, RuleSet};
use scryer_rules::validation::{retired_release_input_fields, validate_user_rule};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RulePackPreviewChange {
    pub template_id: String,
    pub rule_set_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrackedRulePackPreview {
    pub pack_id: String,
    pub version: String,
    pub digest: String,
    pub revision: i64,
    pub added: Vec<String>,
    pub changed: Vec<String>,
    pub removed: Vec<String>,
}

impl AppUseCase {
    /// Atomically installs the bundled pack and retires saved rules that still
    /// depend on the removed release input. Existing installations are left to
    /// their explicit tracked-pack update flow.
    pub async fn bootstrap_builtin_trash_rule_pack(&self) -> AppResult<()> {
        let pack = super::builtin_trash::verified_pack()?;
        let _mutation = self.services.customization.rule_mutation_lock.lock().await;
        let existing_installation = self
            .services
            .customization
            .rule_sets
            .get_rule_pack_installation(&pack.registry.id)
            .await?;
        let existing = self
            .services
            .customization
            .rule_sets
            .list_rule_sets()
            .await?;
        if let Some(mut installation) = existing_installation {
            let retired = retired_rules(&existing)?;
            if retired.is_empty() {
                let engine = self.prospective_engine(&[], &[]).await?;
                self.swap_user_rules_engine(engine);
                return Ok(());
            }
            let expected_revision = installation.revision;
            installation.revision += 1;
            installation.last_updated = Utc::now();
            let engine = self.prospective_engine(&retired, &[]).await?;
            let actor = User::system_execution_actor();
            let history = history_for(&retired, "retired_input_disabled", &actor.id);
            if !self
                .services
                .customization
                .rule_sets
                .apply_rule_pack_installation(
                    &installation,
                    Some(expected_revision),
                    &retired,
                    &history,
                )
                .await?
            {
                return Err(stale_pack_revision());
            }
            self.swap_user_rules_engine(engine);
            return Ok(());
        }

        let AdoptedLocaleMembers {
            members: prior,
            enabled_template_ids: enabled,
            priorities,
        } = adopted_legacy_locale_members(&existing, &pack)?;
        let installation = new_installation(&pack);
        let (installation, mut changed) =
            prepare_pack_rules(&pack, installation, &prior, &enabled, &priorities)?;
        for rule in &mut changed {
            if let Some(existing) = existing.iter().find(|existing| existing.id == rule.id) {
                preserve_adopted_metadata(rule, existing);
            }
        }

        let adopted_ids = prior
            .iter()
            .map(|member| member.rule_set_id.as_str())
            .collect::<BTreeSet<_>>();
        changed.extend(retired_rules(
            &existing
                .into_iter()
                .filter(|rule| !adopted_ids.contains(rule.id.as_str()))
                .collect::<Vec<_>>(),
        )?);

        let engine = self.prospective_engine(&changed, &[]).await?;
        let actor = User::system_execution_actor();
        let mut history = history_for(&changed, "builtin_trash_seeded", &actor.id);
        for change in &mut history {
            if adopted_ids.contains(change.rule_set_id.as_str()) {
                change.action = "rule_pack_adopted".to_string();
            } else if changed.iter().any(|rule| {
                rule.id == change.rule_set_id && !rule.enabled && rule.disabled_reason.is_some()
            }) {
                change.action = "retired_input_disabled".to_string();
            }
        }
        if !self
            .services
            .customization
            .rule_sets
            .apply_rule_pack_installation(&installation, None, &changed, &history)
            .await?
        {
            return Err(AppError::Validation(
                "bundled rule pack installation conflicted with another change".to_string(),
            ));
        }
        self.swap_user_rules_engine(engine);
        Ok(())
    }

    pub async fn list_tracked_rule_packs(
        &self,
        actor: &User,
    ) -> AppResult<Vec<RulePackInstallation>> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;
        self.services
            .customization
            .rule_sets
            .list_rule_pack_installations()
            .await
    }

    pub async fn preview_tracked_rule_pack_update(
        &self,
        actor: &User,
        pack_id: &str,
    ) -> AppResult<TrackedRulePackPreview> {
        let pack = self.fetch_verified_rule_pack(actor, pack_id).await?;
        let _mutation = self.services.customization.rule_mutation_lock.lock().await;
        let installation = self
            .services
            .customization
            .rule_sets
            .get_rule_pack_installation(pack_id)
            .await?;
        let rules = self
            .services
            .customization
            .rule_sets
            .list_rule_sets()
            .await?;
        rule_pack_preview(&pack, installation.as_ref(), &rules)
    }

    pub async fn install_tracked_rule_pack(
        &self,
        actor: &User,
        pack_id: &str,
        enabled_template_ids: &[String],
    ) -> AppResult<RulePackInstallation> {
        let pack = self.fetch_verified_rule_pack(actor, pack_id).await?;
        self.install_verified_rule_pack(actor, &pack, enabled_template_ids)
            .await
    }

    pub(crate) async fn install_verified_rule_pack(
        &self,
        actor: &User,
        pack: &VerifiedRulePack,
        enabled_template_ids: &[String],
    ) -> AppResult<RulePackInstallation> {
        self.require_rule_pack_permissions(actor).await?;
        let pack_id = &pack.registry.id;
        let enabled: BTreeSet<_> = enabled_template_ids.iter().collect();
        if enabled.len() != enabled_template_ids.len()
            || enabled
                .iter()
                .any(|id| !pack.templates.iter().any(|template| &template.id == *id))
        {
            return Err(AppError::Validation(
                "enabled templates must be unique members of the selected pack".into(),
            ));
        }
        let _mutation = self.services.customization.rule_mutation_lock.lock().await;
        if self
            .services
            .customization
            .rule_sets
            .get_rule_pack_installation(pack_id)
            .await?
            .is_some()
        {
            return Err(AppError::Validation(format!(
                "rule pack {pack_id} is already installed"
            )));
        }
        let installation = new_installation(pack);
        let (installation, changed) =
            prepare_pack_rules(pack, installation, &[], enabled_template_ids, &[])?;
        let engine = self.prospective_engine(&changed, &[]).await?;
        let history = history_for(&changed, "rule_pack_installed", &actor.id);
        if !self
            .services
            .customization
            .rule_sets
            .apply_rule_pack_installation(&installation, None, &changed, &history)
            .await?
        {
            return Err(AppError::Validation(
                "rule pack installation conflicted with another change".to_string(),
            ));
        }
        self.swap_user_rules_engine(engine);
        Ok(installation)
    }

    pub async fn update_tracked_rule_pack(
        &self,
        actor: &User,
        pack_id: &str,
        version: &str,
        digest: &str,
        expected_revision: i64,
    ) -> AppResult<RulePackInstallation> {
        let pack = self
            .fetch_verified_rule_pack_version(actor, pack_id, version)
            .await?;
        if pack.registry.version != version || pack.registry.digest != digest {
            return Err(AppError::Validation(
                "rule pack version or digest did not match the verified catalog release"
                    .to_string(),
            ));
        }
        self.apply_verified_tracked_pack_update(actor, &pack, expected_revision, false)
            .await
    }

    pub async fn set_tracked_rule_pack_settings(
        &self,
        actor: &User,
        pack_id: &str,
        enabled_template_ids: &[String],
        priorities: &[(String, i32)],
        auto_update: bool,
        expected_revision: i64,
    ) -> AppResult<RulePackInstallation> {
        self.require_rule_pack_permissions(actor).await?;
        let _mutation = self.services.customization.rule_mutation_lock.lock().await;
        let mut installation = self
            .services
            .customization
            .rule_sets
            .get_rule_pack_installation(pack_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("tracked rule pack {pack_id}")))?;
        if installation.revision != expected_revision {
            return Err(stale_pack_revision());
        }
        let enabled: BTreeSet<_> = enabled_template_ids.iter().cloned().collect();
        if enabled.len() != enabled_template_ids.len() {
            return Err(AppError::Validation(
                "duplicate rule pack template ID".to_string(),
            ));
        }
        let priority_by_template: BTreeMap<_, _> = priorities.iter().cloned().collect();
        if priority_by_template.len() != priorities.len() {
            return Err(AppError::Validation(
                "duplicate rule pack priority template ID".to_string(),
            ));
        }
        let active: BTreeSet<_> = installation
            .members
            .iter()
            .filter(|member| !member.removed)
            .map(|member| member.template_id.as_str())
            .collect();
        if enabled.iter().any(|id| !active.contains(id.as_str()))
            || priority_by_template
                .keys()
                .any(|id| !active.contains(id.as_str()))
        {
            return Err(AppError::Validation(
                "rule pack settings reference an unknown or removed template".to_string(),
            ));
        }
        let all = self
            .services
            .customization
            .rule_sets
            .list_rule_sets()
            .await?;
        let mut changed = Vec::new();
        for member in &installation.members {
            if member.removed {
                continue;
            }
            let mut rule = all
                .iter()
                .find(|rule| rule.id == member.rule_set_id)
                .cloned()
                .ok_or_else(|| {
                    AppError::Validation(format!("tracked rule {} is missing", member.rule_set_id))
                })?;
            let next_enabled = enabled.contains(&member.template_id);
            let next_priority = priority_by_template
                .get(&member.template_id)
                .copied()
                .unwrap_or(rule.priority);
            if rule.enabled == next_enabled && rule.priority == next_priority {
                continue;
            }
            rule.enabled = next_enabled;
            rule.priority = next_priority;
            rule.updated_at = Utc::now();
            changed.push(rule);
        }
        installation.auto_update = auto_update;
        installation.revision += 1;
        installation.last_updated = Utc::now();
        let engine = if changed.is_empty() {
            None
        } else {
            Some(self.prospective_engine(&changed, &[]).await?)
        };
        let history = changed
            .iter()
            .map(|rule| RuleSetHistoryChange {
                rule_set_id: rule.id.clone(),
                action: "rule_pack_settings_updated".into(),
                rego_source: None,
                actor_id: Some(actor.id.clone()),
            })
            .collect::<Vec<_>>();
        if !self
            .services
            .customization
            .rule_sets
            .apply_rule_pack_installation(
                &installation,
                Some(expected_revision),
                &changed,
                &history,
            )
            .await?
        {
            return Err(stale_pack_revision());
        }
        if let Some(engine) = engine {
            self.swap_user_rules_engine(engine);
        }
        Ok(installation)
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "mirrors the custom rule authoring boundary"
    )]
    pub async fn copy_tracked_rule_pack_rule(
        &self,
        actor: &User,
        rule_set_id: &str,
        name: String,
        description: String,
        source: String,
        facets: Vec<MediaFacet>,
        priority: i32,
    ) -> AppResult<RuleSet> {
        self.require_rule_pack_permissions(actor).await?;
        let _mutation = self.services.customization.rule_mutation_lock.lock().await;
        let installation = self
            .services
            .customization
            .rule_sets
            .find_rule_pack_installation_by_rule_set_id(rule_set_id)
            .await?
            .ok_or_else(|| {
                AppError::Validation("rule set is not tracked by a rule pack".to_string())
            })?;
        if !installation.customizable {
            return Err(AppError::Validation(
                "rules from this pack cannot be copied".to_string(),
            ));
        }
        let original = self
            .services
            .customization
            .rule_sets
            .get_rule_set(rule_set_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("rule set {rule_set_id}")))?;
        let id = Id::new_rego_safe().0;
        let source = validate_pack_source(&source, &id)?;
        let now = Utc::now();
        let custom = RuleSet {
            id,
            name,
            description,
            rego_source: source,
            enabled: original.enabled,
            priority,
            evaluation_phase: original.evaluation_phase,
            exclusive_group: original.exclusive_group.clone(),
            disabled_reason: None,
            applied_facets: facets,
            created_at: now,
            updated_at: now,
            is_managed: false,
            managed_key: None,
            managed_tag_filter: original.managed_tag_filter.clone(),
        };
        let mut disabled_original = original;
        disabled_original.enabled = false;
        disabled_original.updated_at = Utc::now();
        let engine = self
            .prospective_engine(&[custom.clone(), disabled_original], &[])
            .await?;
        let mut history = history_for(
            std::slice::from_ref(&custom),
            "rule_pack_rule_copied",
            &actor.id,
        );
        history.push(RuleSetHistoryChange {
            rule_set_id: rule_set_id.to_string(),
            action: "rule_pack_rule_disabled_after_copy".to_string(),
            rego_source: None,
            actor_id: Some(actor.id.clone()),
        });
        if !self
            .services
            .customization
            .rule_sets
            .copy_rule_pack_rule_set_to_custom(
                &installation.pack_id,
                rule_set_id,
                &custom,
                installation.revision,
                &history,
            )
            .await?
        {
            return Err(stale_pack_revision());
        }
        self.swap_user_rules_engine(engine);
        Ok(custom)
    }

    pub async fn uninstall_tracked_rule_pack(
        &self,
        actor: &User,
        pack_id: &str,
        expected_revision: i64,
    ) -> AppResult<()> {
        self.require_rule_pack_permissions(actor).await?;
        let _mutation = self.services.customization.rule_mutation_lock.lock().await;
        let installation = self
            .services
            .customization
            .rule_sets
            .get_rule_pack_installation(pack_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("tracked rule pack {pack_id}")))?;
        if installation.revision != expected_revision {
            return Err(stale_pack_revision());
        }
        let removed: Vec<_> = installation
            .members
            .iter()
            .map(|member| member.rule_set_id.clone())
            .collect();
        let engine = self.prospective_engine(&[], &removed).await?;
        let history = installation
            .members
            .iter()
            .map(|member| RuleSetHistoryChange {
                rule_set_id: member.rule_set_id.clone(),
                action: "rule_pack_uninstalled".to_string(),
                rego_source: None,
                actor_id: Some(actor.id.clone()),
            })
            .collect::<Vec<_>>();
        if !self
            .services
            .customization
            .rule_sets
            .uninstall_rule_pack(pack_id, expected_revision, &history)
            .await?
        {
            return Err(stale_pack_revision());
        }
        self.swap_user_rules_engine(engine);
        Ok(())
    }

    async fn require_rule_pack_permissions(&self, actor: &User) -> AppResult<()> {
        self.require_app_permission(actor, AppPermission::ManageSystemSettings)
            .await?;
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await
    }

    pub(crate) async fn prospective_engine(
        &self,
        changed: &[RuleSet],
        removed: &[String],
    ) -> AppResult<scryer_rules::UserRulesEngine> {
        let mut rules = self
            .services
            .customization
            .rule_sets
            .list_rule_sets()
            .await?;
        rules.retain(|rule| {
            !removed.contains(&rule.id)
                && !changed.iter().any(|replacement| replacement.id == rule.id)
        });
        rules.extend_from_slice(changed);
        self.prepare_user_rules_engine(rules).await
    }

    pub(crate) async fn apply_verified_tracked_pack_update(
        &self,
        actor: &User,
        pack: &VerifiedRulePack,
        expected_revision: i64,
        automatic: bool,
    ) -> AppResult<RulePackInstallation> {
        self.require_rule_pack_permissions(actor).await?;
        let _mutation = self.services.customization.rule_mutation_lock.lock().await;
        let current = self
            .services
            .customization
            .rule_sets
            .get_rule_pack_installation(&pack.registry.id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("tracked rule pack {}", pack.registry.id)))?;
        if current.revision != expected_revision {
            return Err(stale_pack_revision());
        }
        if automatic && !current.auto_update {
            return Err(AppError::Validation(
                "tracked rule pack automatic updates are disabled".to_string(),
            ));
        }
        let next_version = parse_rule_pack_version(&pack.registry.version, "rule pack")?;
        let current_version = parse_rule_pack_version(&current.version, "installed rule pack")?;
        if !next_version.cmp_precedence(&current_version).is_gt() {
            return Err(AppError::Validation(
                "rule pack update must be a newer version".to_string(),
            ));
        }
        if automatic
            && (!next_version.pre.is_empty()
                || !current_version.pre.is_empty()
                || next_version.major != current_version.major
                || next_version.minor != current_version.minor)
        {
            return Err(AppError::Validation(
                "automatic rule pack updates require a stable patch on the installed major/minor"
                    .into(),
            ));
        }
        let existing_rules = self
            .services
            .customization
            .rule_sets
            .list_rule_sets()
            .await?;
        let mut enabled = Vec::new();
        let mut priorities = Vec::new();
        for member in &current.members {
            let rule = existing_rules
                .iter()
                .find(|rule| rule.id == member.rule_set_id)
                .ok_or_else(|| {
                    AppError::Validation(format!("tracked rule {} is missing", member.rule_set_id))
                })?;
            if !member.removed && rule.enabled {
                enabled.push(member.template_id.clone());
            }
            priorities.push((member.template_id.clone(), rule.priority));
        }
        let mut next = current.clone();
        next.name = pack.registry.name.clone();
        next.version = pack.registry.version.clone();
        next.digest = pack.registry.digest.clone();
        next.customizable = pack.registry.customizable;
        next.revision += 1;
        next.last_updated = Utc::now();
        next.last_error = None;
        let (next, mut changed) =
            prepare_pack_rules(pack, next, &current.members, &enabled, &priorities)?;
        for rule in &mut changed {
            if let Some(existing) = existing_rules
                .iter()
                .find(|existing| existing.id == rule.id)
            {
                preserve_adopted_metadata(rule, existing);
            }
        }
        for member in current.members.iter().filter(|member| {
            !member.removed
                && !pack
                    .templates
                    .iter()
                    .any(|template| template.id == member.template_id)
        }) {
            if let Some(mut rule) = existing_rules
                .iter()
                .find(|rule| rule.id == member.rule_set_id)
                .cloned()
            {
                rule.enabled = false;
                rule.updated_at = Utc::now();
                changed.push(rule);
            }
        }
        let engine = self.prospective_engine(&changed, &[]).await?;
        let history = history_for(&changed, "rule_pack_updated", &actor.id);
        if !self
            .services
            .customization
            .rule_sets
            .apply_rule_pack_installation(&next, Some(expected_revision), &changed, &history)
            .await?
        {
            return Err(stale_pack_revision());
        }
        self.swap_user_rules_engine(engine);
        Ok(next)
    }
}

fn new_installation(pack: &VerifiedRulePack) -> RulePackInstallation {
    RulePackInstallation {
        pack_id: pack.registry.id.clone(),
        name: pack.registry.name.clone(),
        version: pack.registry.version.clone(),
        digest: pack.registry.digest.clone(),
        customizable: pack.registry.customizable,
        auto_update: false,
        revision: 1,
        last_updated: Utc::now(),
        last_error: None,
        members: Vec::new(),
    }
}

fn parse_rule_pack_version(raw: &str, subject: &str) -> AppResult<semver::Version> {
    semver::Version::parse(raw.trim().trim_start_matches('v')).map_err(|_| {
        AppError::Validation(format!("{subject} version is not valid semantic version"))
    })
}

struct AdoptedLocaleMembers {
    members: Vec<RulePackMember>,
    enabled_template_ids: Vec<String>,
    priorities: Vec<(String, i32)>,
}

fn adopted_legacy_locale_members(
    existing: &[RuleSet],
    pack: &VerifiedRulePack,
) -> AppResult<AdoptedLocaleMembers> {
    let template_ids = pack
        .templates
        .iter()
        .map(|template| template.id.as_str())
        .collect::<BTreeSet<_>>();
    let mut by_template = BTreeMap::<String, &RuleSet>::new();
    for rule in existing.iter().filter(|rule| rule.is_managed) {
        let Some(template_id) = legacy_locale_template_id(rule.managed_key.as_deref()) else {
            continue;
        };
        if !template_ids.contains(template_id) {
            continue;
        }
        if by_template.insert(template_id.to_string(), rule).is_some() {
            return Err(AppError::Validation(format!(
                "cannot adopt duplicate legacy locale rules for {template_id}"
            )));
        }
    }
    let mut enabled = super::builtin_trash::default_template_ids(pack);
    let mut members = Vec::new();
    let mut priorities = Vec::new();
    for (template_id, rule) in by_template {
        if rule.enabled {
            enabled.push(template_id.clone());
        }
        priorities.push((template_id.clone(), rule.priority));
        members.push(RulePackMember {
            template_id,
            rule_set_id: rule.id.clone(),
            removed: false,
        });
    }
    Ok(AdoptedLocaleMembers {
        members,
        enabled_template_ids: enabled,
        priorities,
    })
}

fn legacy_locale_template_id(managed_key: Option<&str>) -> Option<&'static str> {
    match managed_key? {
        "trash-guides:locale:french" | "trash-guides:locale:french-vf" => {
            Some("trash-guides-french-vf")
        }
        "trash-guides:locale:french-vo" => Some("trash-guides-french-vo"),
        "trash-guides:locale:french-vostfr" => Some("trash-guides-french-vostfr"),
        "trash-guides:locale:german" => Some("trash-guides-german"),
        "trash-guides:locale:asian" => Some("trash-guides-asian"),
        _ => None,
    }
}

fn preserve_adopted_metadata(adopted: &mut RuleSet, legacy: &RuleSet) {
    adopted.created_at = legacy.created_at;
    if legacy.is_managed {
        adopted.is_managed = true;
        adopted.applied_facets = legacy.applied_facets.clone();
        adopted.managed_key = legacy.managed_key.clone();
        adopted.managed_tag_filter = legacy.managed_tag_filter.clone();
    }
}

fn retired_rules(existing: &[RuleSet]) -> AppResult<Vec<RuleSet>> {
    existing
        .iter()
        .filter_map(|rule| {
            let fields = retired_release_input_fields(&rule.rego_source)
                .map_err(|error| AppError::Validation(format!("rule validation failed: {error}")));
            match fields {
                Ok(fields) if fields.is_empty() => None,
                Err(_) if !rule.enabled => None,
                Ok(_) => {
                    const REASON: &str = "uses retired input.release.guide_facts; update the rule source before enabling it";
                    if !rule.enabled && rule.disabled_reason.as_deref().is_some_and(|reason| reason.contains(REASON)) {
                        return None;
                    }
                    let mut retired = rule.clone();
                    retired.enabled = false;
                    retired.disabled_reason = Some(match rule.disabled_reason.as_deref() {
                        Some(reason) if !reason.is_empty() => format!("{reason}; {REASON}"),
                        _ => REASON.to_string(),
                    });
                    retired.updated_at = Utc::now();
                    Some(Ok(retired))
                }
                Err(error) => Some(Err(error)),
            }
        })
        .collect()
}

fn rule_pack_preview(
    pack: &VerifiedRulePack,
    installation: Option<&RulePackInstallation>,
    rules: &[RuleSet],
) -> AppResult<TrackedRulePackPreview> {
    let installation = installation
        .ok_or_else(|| AppError::NotFound(format!("tracked rule pack {}", pack.registry.id)))?;
    let version = parse_rule_pack_version(&pack.registry.version, "rule pack")?;
    let installed = parse_rule_pack_version(&installation.version, "installed rule pack")?;
    if !version.cmp_precedence(&installed).is_gt() {
        return Err(AppError::Validation(
            "rule pack is already up to date".into(),
        ));
    }
    let known = installation
        .members
        .iter()
        .map(|m| (m.template_id.as_str(), m))
        .collect::<BTreeMap<_, _>>();
    let mut added = Vec::new();
    let mut changed = Vec::new();
    for template in &pack.templates {
        match known.get(template.id.as_str()) {
            Some(member) if !member.removed => {
                let source = validate_pack_source(&template.rego_source, &member.rule_set_id)?;
                let facets = parse_facets(&template.applied_facets)?;
                let differs = rules
                    .iter()
                    .find(|rule| rule.id == member.rule_set_id)
                    .is_none_or(|rule| {
                        rule.name != template.title
                            || rule.description != template.description
                            || rule.rego_source != source
                            || rule.applied_facets != facets
                    });
                if differs {
                    changed.push(RulePackPreviewChange {
                        template_id: template.id.clone(),
                        rule_set_id: member.rule_set_id.clone(),
                    });
                }
            }
            previous => {
                let id = previous
                    .map(|member| member.rule_set_id.clone())
                    .unwrap_or_else(|| Id::new_rego_safe().0);
                validate_pack_source(&template.rego_source, &id)?;
                parse_facets(&template.applied_facets)?;
                added.push(RulePackPreviewChange {
                    template_id: template.id.clone(),
                    rule_set_id: id,
                });
            }
        }
    }
    let removed: Vec<_> = installation
        .members
        .iter()
        .filter(|m| !m.removed && !pack.templates.iter().any(|t| t.id == m.template_id))
        .map(|m| RulePackPreviewChange {
            template_id: m.template_id.clone(),
            rule_set_id: m.rule_set_id.clone(),
        })
        .collect();
    Ok(TrackedRulePackPreview {
        pack_id: pack.registry.id.clone(),
        version: pack.registry.version.clone(),
        digest: pack.registry.digest.clone(),
        revision: installation.revision,
        added: added.into_iter().map(|v| v.template_id).collect(),
        changed: changed.into_iter().map(|v| v.template_id).collect(),
        removed: removed.into_iter().map(|v| v.template_id).collect(),
    })
}

fn prepare_pack_rules(
    pack: &VerifiedRulePack,
    mut installation: RulePackInstallation,
    prior: &[RulePackMember],
    enabled_template_ids: &[String],
    priorities: &[(String, i32)],
) -> AppResult<(RulePackInstallation, Vec<RuleSet>)> {
    let prior = prior
        .iter()
        .map(|member| (member.template_id.clone(), member.clone()))
        .collect::<BTreeMap<_, _>>();
    let enabled = enabled_template_ids.iter().collect::<BTreeSet<_>>();
    let priorities = priorities.iter().cloned().collect::<BTreeMap<_, _>>();
    let mut members = Vec::new();
    let mut rules = Vec::new();
    for template in &pack.templates {
        let old = prior.get(&template.id);
        let id = old
            .map(|m| m.rule_set_id.clone())
            .unwrap_or_else(|| Id::new_rego_safe().0);
        let source = validate_pack_source(&template.rego_source, &id)?;
        let facets = parse_facets(&template.applied_facets)?;
        let reappearing = old.is_some_and(|member| member.removed);
        rules.push(RuleSet {
            id: id.clone(),
            name: template.title.clone(),
            description: template.description.clone(),
            rego_source: source,
            enabled: old.is_some_and(|_| !reappearing && enabled.contains(&template.id))
                || old.is_none() && enabled.contains(&template.id),
            priority: priorities.get(&template.id).copied().unwrap_or(0),
            evaluation_phase: template.evaluation_phase,
            exclusive_group: template.exclusive_group.clone(),
            disabled_reason: None,
            applied_facets: facets,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            is_managed: false,
            managed_key: None,
            managed_tag_filter: None,
        });
        members.push(RulePackMember {
            template_id: template.id.clone(),
            rule_set_id: id,
            removed: false,
        });
    }
    for member in prior.values().filter(|member| {
        !pack
            .templates
            .iter()
            .any(|template| template.id == member.template_id)
    }) {
        members.push(RulePackMember {
            template_id: member.template_id.clone(),
            rule_set_id: member.rule_set_id.clone(),
            removed: true,
        });
    }
    installation.members = members;
    Ok((installation, rules))
}

fn parse_facets(values: &[String]) -> AppResult<Vec<MediaFacet>> {
    values
        .iter()
        .map(|value| {
            MediaFacet::parse(value)
                .ok_or_else(|| AppError::Validation(format!("invalid rule pack facet {value}")))
        })
        .collect()
}
fn validate_pack_source(source: &str, id: &str) -> AppResult<String> {
    let source = scryer_rules::rewrite_package_declaration(source, id);
    let result = validate_user_rule(&source, id)
        .map_err(|error| AppError::Validation(format!("rule validation failed: {error}")))?;
    if result.valid {
        Ok(source)
    } else {
        Err(AppError::Validation(format!(
            "rule validation failed: {}",
            result.errors.join("; ")
        )))
    }
}
fn history_for(rules: &[RuleSet], action: &str, actor_id: &str) -> Vec<RuleSetHistoryChange> {
    rules
        .iter()
        .map(|rule| RuleSetHistoryChange {
            rule_set_id: rule.id.clone(),
            action: action.to_string(),
            rego_source: Some(rule.rego_source.clone()),
            actor_id: Some(actor_id.to_string()),
        })
        .collect()
}
fn stale_pack_revision() -> AppError {
    AppError::Validation("tracked rule pack changed; reload and retry".to_string())
}

#[cfg(test)]
mod builtin_trash_tests {
    use super::*;

    #[test]
    fn adopting_a_managed_locale_preserves_its_scope_and_identity() {
        let now = Utc::now();
        let legacy = RuleSet {
            id: "legacy-german".to_string(),
            name: "Legacy German".to_string(),
            description: String::new(),
            rego_source: "package ignored".to_string(),
            enabled: true,
            priority: 17,
            evaluation_phase: scryer_domain::RuleEvaluationPhase::Additional,
            exclusive_group: None,
            disabled_reason: None,
            applied_facets: vec![],
            created_at: now,
            updated_at: now,
            is_managed: true,
            managed_key: Some("trash-guides:locale:german".to_string()),
            managed_tag_filter: Some(vec!["locale:german".to_string()]),
        };
        let pack = super::super::builtin_trash::verified_pack().expect("bundled pack parses");
        let AdoptedLocaleMembers {
            members,
            enabled_template_ids: enabled,
            priorities,
        } = adopted_legacy_locale_members(std::slice::from_ref(&legacy), &pack)
            .expect("legacy locale adopts");

        assert_eq!(members[0].template_id, "trash-guides-german");
        assert_eq!(members[0].rule_set_id, legacy.id);
        assert!(enabled.contains(&"trash-guides-german".to_string()));
        assert_eq!(priorities, vec![("trash-guides-german".to_string(), 17)]);

        let mut adopted = legacy.clone();
        adopted.is_managed = false;
        adopted.managed_key = None;
        adopted.managed_tag_filter = None;
        adopted.applied_facets = vec![];
        preserve_adopted_metadata(&mut adopted, &legacy);
        assert!(adopted.is_managed);
        assert_eq!(adopted.managed_key, legacy.managed_key);
        assert_eq!(adopted.managed_tag_filter, legacy.managed_tag_filter);
        assert_eq!(adopted.applied_facets, legacy.applied_facets);
    }
}

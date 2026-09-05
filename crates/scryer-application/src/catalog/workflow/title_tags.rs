// Admin-defined title tags: registry CRUD and the per-title assignment patch.
//
// Two permissions meet here and must not be confused. Deciding *which* tags
// exist is catalog configuration, exactly like a delay profile, so it needs
// `AppPermission::ManageCatalogSettings`. Deciding which titles carry them is
// title management, so it needs `LibraryPermission::ManageTitles` on each
// affected title's library. Reading the registry is unprivileged: the picker
// and the catalog filter need it for anyone who can see a title at all.

impl AppUseCase {
    /// Every defined tag with its current title count.
    ///
    /// Deliberately unauthorized beyond being a request from a user: the tag
    /// picker and the catalog filter pane both need the vocabulary, and the
    /// vocabulary reveals nothing about any particular title.
    pub async fn title_tag_definitions(
        &self,
        _actor: &User,
    ) -> AppResult<Vec<crate::TitleTagDefinitionSummary>> {
        self.services.catalog.titles.list_title_tag_definitions().await
    }

    pub async fn create_title_tag_definition(
        &self,
        actor: &User,
        label: &str,
        description: Option<String>,
    ) -> AppResult<scryer_domain::TitleTagDefinition> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;
        let label = crate::normalize_user_title_tag(label).map_err(AppError::Validation)?;
        let now = Utc::now();
        let definition = scryer_domain::TitleTagDefinition {
            id: Id::new().0,
            label,
            description: normalize_title_tag_description(description),
            created_by: Some(actor.id.clone()),
            created_at: now,
            updated_at: now,
        };
        let created = self
            .services
            .catalog
            .titles
            .create_title_tag_definition(&definition)
            .await?;

        self.emit_configuration_changed_event(
            actor,
            "title_tag",
            Some(created.id.clone()),
            scryer_domain::ConfigurationChangeAction::Saved,
        )
        .await;
        Ok(created)
    }

    /// Rename and/or re-describe a tag.
    ///
    /// A rename is a data migration, not a label edit: membership is stored by
    /// label, so the registry row, every title bag, and every delay profile's
    /// tag list all have to move together. Rego revisions are immutable and are
    /// *not* rewritten — the returned counts name how many rule sets mention
    /// the old label so the caller can warn that those rules will stop matching.
    pub async fn update_title_tag_definition(
        &self,
        actor: &User,
        id: &str,
        label: Option<String>,
        description: Option<Option<String>>,
    ) -> AppResult<crate::TitleTagDefinitionUpdate> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;
        let next_label = label
            .map(|label| crate::normalize_user_title_tag(&label).map_err(AppError::Validation))
            .transpose()?;
        let description = description.map(normalize_title_tag_description);

        let existing = self
            .services
            .catalog
            .titles
            .get_title_tag_definition(id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title tag {id}")))?;
        let previous_label = existing.label.clone();
        let renamed = next_label
            .as_ref()
            .is_some_and(|label| label != &previous_label);

        // The rule-set scan runs before the write so the warning describes the
        // label the operator is renaming away from, not what is left afterwards.
        let (maintenance_rule_sets, release_rule_sets, managed_tag_filters) = if renamed {
            self.count_rule_sets_referencing_title_tag(&previous_label)
                .await?
        } else {
            (0, 0, 0)
        };

        let (definition, titles) = self
            .services
            .catalog
            .titles
            .update_title_tag_definition(id, next_label, description, Utc::now())
            .await?;

        // Delay-profile tag lists live in the settings catalog, behind a
        // different repository, so this half cannot join the store's
        // transaction. It runs immediately after and only ever rewrites labels,
        // so a failure between the two leaves profiles pointing at a label that
        // no longer exists — which reads as "matches nothing", the same as a
        // deleted tag, rather than as a wrong match.
        let delay_profiles = if renamed {
            self.rewrite_delay_profile_tag(actor, &previous_label, Some(&definition.label))
                .await?
        } else {
            0
        };

        self.emit_configuration_changed_event(
            actor,
            "title_tag",
            Some(definition.id.clone()),
            scryer_domain::ConfigurationChangeAction::Saved,
        )
        .await;

        Ok(crate::TitleTagDefinitionUpdate {
            definition,
            counts: crate::TitleTagRewriteCounts {
                titles,
                delay_profiles,
                maintenance_rule_sets,
                release_rule_sets,
                managed_tag_filters,
            },
        })
    }

    /// Delete a tag and strip it from every title and delay profile.
    ///
    /// Rules that name the label are left alone and simply stop matching, which
    /// is the safe direction: a rule that no longer fires is visible in its run
    /// history, a rule quietly repointed at a different set of titles is not.
    pub async fn delete_title_tag_definition(
        &self,
        actor: &User,
        id: &str,
    ) -> AppResult<crate::TitleTagRewriteCounts> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        let existing = self
            .services
            .catalog
            .titles
            .get_title_tag_definition(id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title tag {id}")))?;
        let (maintenance_rule_sets, release_rule_sets, managed_tag_filters) = self
            .count_rule_sets_referencing_title_tag(&existing.label)
            .await?;

        let (definition, titles) = self
            .services
            .catalog
            .titles
            .delete_title_tag_definition(id)
            .await?;
        let delay_profiles = self
            .rewrite_delay_profile_tag(actor, &definition.label, None)
            .await?;

        self.emit_configuration_changed_event(
            actor,
            "title_tag",
            Some(definition.id.clone()),
            scryer_domain::ConfigurationChangeAction::Deleted,
        )
        .await;

        Ok(crate::TitleTagRewriteCounts {
            titles,
            delay_profiles,
            maintenance_rule_sets,
            release_rule_sets,
            managed_tag_filters,
        })
    }

    /// Add and/or remove user tags across a set of titles.
    ///
    /// Every title's library is checked before the first write, so a bulk call
    /// that touches one library the actor cannot manage changes nothing at all
    /// rather than half-applying. Reserved `scryer:` entries are untouched: the
    /// patch is applied inside the store transaction, so a concurrent options
    /// save cannot be clobbered by this write or clobber it.
    pub async fn update_title_tags(
        &self,
        actor: &User,
        title_ids: &[String],
        add: &[String],
        remove: &[String],
    ) -> AppResult<Vec<Title>> {
        let add = crate::normalize_user_title_tags(add).map_err(AppError::Validation)?;
        let remove = crate::normalize_user_title_tags(remove).map_err(AppError::Validation)?;
        if add.is_empty() && remove.is_empty() {
            return Err(AppError::Validation(
                "at least one tag to add or remove must be provided".to_string(),
            ));
        }
        // Only additions are gated on the registry. Removing a label that was
        // deleted from the registry while it was still on a title is cleanup,
        // and refusing it would strand the title with a tag nothing can clear.
        self.require_registered_title_tags(&add).await?;

        let mut seen = HashSet::new();
        let mut titles = Vec::new();
        for title_id in title_ids {
            if !seen.insert(title_id.clone()) {
                continue;
            }
            let title = self
                .services
                .catalog
                .titles
                .get_by_id(title_id)
                .await?
                .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;
            titles.push(title);
        }
        if titles.is_empty() {
            return Err(AppError::Validation(
                "at least one title must be provided".to_string(),
            ));
        }
        for title in &titles {
            self.require_library_permission(
                actor,
                &title.library_id,
                scryer_domain::LibraryPermission::ManageTitles,
            )
            .await?;
        }

        let mut updated = Vec::with_capacity(titles.len());
        for title in &titles {
            let title = self
                .services
                .catalog
                .titles
                .update_user_tags(&title.id, &add, &remove)
                .await?;
            self.emit_title_updated_activity(actor, &title).await;
            updated.push(title);
        }
        Ok(updated)
    }

    /// Refuse any unprefixed label the registry does not define.
    ///
    /// The gate lives here rather than in the store so both write paths share
    /// it: the tag patch, and the raw whole-bag `updateTitle(tags:)` write.
    pub(crate) async fn require_registered_title_tags(&self, labels: &[String]) -> AppResult<()> {
        let unprefixed = labels
            .iter()
            .filter(|label| !crate::is_reserved_title_tag(label))
            .collect::<Vec<_>>();
        if unprefixed.is_empty() {
            return Ok(());
        }
        let defined = self
            .services
            .catalog
            .titles
            .list_title_tag_definitions()
            .await?
            .into_iter()
            .map(|summary| summary.definition.label)
            .collect::<HashSet<_>>();
        for label in unprefixed {
            if !defined.contains(label) {
                return Err(AppError::Validation(format!(
                    "'{label}' is not a defined tag; an administrator has to add it in Settings before it can be applied"
                )));
            }
        }
        Ok(())
    }

    /// Rewrite `label` to `replacement` (or drop it) across every delay
    /// profile's tag list, returning how many profiles changed.
    async fn rewrite_delay_profile_tag(
        &self,
        actor: &User,
        label: &str,
        replacement: Option<&str>,
    ) -> AppResult<u64> {
        let mut profiles = self.delay_profiles().await?;
        let mut changed = 0_u64;
        for profile in profiles.iter_mut() {
            if !profile.tags.iter().any(|tag| tag == label) {
                continue;
            }
            profile.tags.retain(|tag| tag != label);
            if let Some(replacement) = replacement
                && !profile.tags.iter().any(|tag| tag == replacement)
            {
                profile.tags.push(replacement.to_string());
            }
            changed += 1;
        }
        if changed == 0 {
            return Ok(0);
        }

        self.upsert_system_setting_json(
            crate::delay_profile::DELAY_PROFILE_CATALOG_KEY,
            &profiles,
            Some(actor.id.clone()),
        )
        .await?;
        let _ = self.runtime.events.settings_changed_broadcast.send(vec![
            crate::delay_profile::DELAY_PROFILE_CATALOG_KEY.to_string(),
        ]);
        Ok(changed)
    }

    /// `(maintenance rule sets, release rule sets, managed tag filters)` that
    /// name `label`.
    ///
    /// The first two are a plain substring search over stored Rego, on purpose:
    /// this is a warning, not a rewrite. Rego revisions are immutable, so
    /// nothing here can be corrected automatically, and an over-broad match
    /// costs the operator one extra look at a rule while a missed match costs a
    /// rule that silently stops firing.
    ///
    /// The third is exact: a managed pack's `tag_filter` is a list of labels,
    /// not free text, and it is SMG-owned, so it is neither rewritten nor
    /// folded into the Rego count — the operator has to fix it somewhere else.
    /// One managed pack can therefore be counted twice, once per reason.
    ///
    /// Only each rule set's *current* revision is scanned. Older revisions are
    /// immutable history and cannot fire, so naming them would send the
    /// operator after rules that are already inert.
    async fn count_rule_sets_referencing_title_tag(
        &self,
        label: &str,
    ) -> AppResult<(u64, u64, u64)> {
        let mut maintenance = 0_u64;
        for rule_set in self
            .services
            .customization
            .maintenance_rule_sets
            .list_rule_sets()
            .await?
        {
            if let Some(revision) = self
                .services
                .customization
                .maintenance_rule_sets
                .get_revision(&rule_set.id, rule_set.current_revision_number)
                .await?
                && revision.rego_source.contains(label)
            {
                maintenance += 1;
            }
        }

        let mut release = 0_u64;
        let mut managed_tag_filters = 0_u64;
        for rule_set in self
            .services
            .customization
            .rule_sets
            .list_rule_sets()
            .await?
        {
            if rule_set.rego_source.contains(label) {
                release += 1;
            }
            if rule_set
                .managed_tag_filter
                .as_ref()
                .is_some_and(|tags| tags.iter().any(|tag| tag == label))
            {
                managed_tag_filters += 1;
            }
        }

        Ok((maintenance, release, managed_tag_filters))
    }
}

/// A description is either real text or absent; an all-whitespace one is the
/// latter, so the UI never has to distinguish `""` from `NULL`.
fn normalize_title_tag_description(description: Option<String>) -> Option<String> {
    description
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

const MANAGED_INDEXER_SCOPE_IDS: &[&str] = &["movie", "series", "anime"];
fn normalize_managed_child_routing_scopes(
    scopes: Vec<ManagedIndexerRoutingScope>,
) -> AppResult<HashMap<String, Vec<String>>> {
    let mut routing_by_scope = HashMap::new();
    for scope in scopes {
        let scope_id = scope.scope_id.trim().to_ascii_lowercase();
        if !MANAGED_INDEXER_SCOPE_IDS.contains(&scope_id.as_str()) {
            return Err(AppError::Validation(format!(
                "managed child routing scope '{}' is not supported",
                scope.scope_id
            )));
        }
        if routing_by_scope.contains_key(&scope_id) {
            return Err(AppError::Validation(format!(
                "managed child routing contains duplicate scope '{}'",
                scope_id
            )));
        }
        routing_by_scope.insert(scope_id, normalize_routing_categories(scope.categories));
    }
    Ok(routing_by_scope)
}
fn apply_managed_child_routing(
    routing_by_scope: &mut HashMap<String, Vec<IndexerRoutingSettingsEntry>>,
    indexer_id: &str,
    desired_scopes: &HashMap<String, Vec<String>>,
) {
    for scope_id in MANAGED_INDEXER_SCOPE_IDS {
        let Some(categories) = desired_scopes.get(*scope_id).cloned() else {
            if let Some(entries) = routing_by_scope.get_mut(*scope_id) {
                entries.retain(|entry| entry.indexer_id != indexer_id);
            }
            continue;
        };
        upsert_indexer_routing_entry(
            routing_by_scope.entry((*scope_id).to_string()).or_default(),
            indexer_id,
            categories,
        );
    }
}
fn remove_indexer_routing_entries(
    routing_by_scope: &mut HashMap<String, Vec<IndexerRoutingSettingsEntry>>,
    indexer_id: &str,
) {
    for scope_id in MANAGED_INDEXER_SCOPE_IDS {
        if let Some(entries) = routing_by_scope.get_mut(*scope_id) {
            entries.retain(|entry| entry.indexer_id != indexer_id);
        }
    }
}
impl AppUseCase {
    pub fn queue_managed_indexer_sync(&self, actor: &User, config_id: &str) {
        let config_id = config_id.trim().to_string();
        if config_id.is_empty() {
            return;
        }

        let app = self.clone();
        let actor = actor.clone();
        tokio::spawn(async move {
            if let Err(error) = app.sync_indexer_config(&actor, &config_id).await {
                tracing::warn!(
                    config_id = %config_id,
                    error = %error,
                    "background managed indexer sync failed"
                );
            }
        });
    }

    pub(crate) fn queue_managed_indexer_enrichment(&self, actor: &User, config_id: &str) {
        let config_id = config_id.trim().to_string();
        if config_id.is_empty() {
            return;
        }

        let app = self.clone();
        let actor = actor.clone();
        tokio::spawn(async move {
            if let Err(error) = app
                .enrich_managed_indexer_children(&actor, &config_id)
                .await
            {
                tracing::warn!(
                    config_id = %config_id,
                    error = %error,
                    "background managed indexer caps enrichment failed"
                );
            }
        });
    }

    async fn enrich_managed_indexer_children(
        &self,
        actor: &User,
        config_id: &str,
    ) -> AppResult<()> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        let started = std::time::Instant::now();
        let _sync_guard = self
            .runtime
            .integrations
            .managed_indexer_sync_lock
            .clone()
            .lock_owned()
            .await;
        let parent = self
            .services
            .integrations
            .indexer_configs
            .get_by_id(config_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("indexer config '{config_id}' not found")))?;
        let provider = self
            .services
            .integrations
            .plugin_provider
            .available()
            .ok_or_else(|| AppError::Repository("indexer provider not available".into()))?;
        let client = provider
            .management_client_for_provider(&parent)
            .ok_or_else(|| {
                AppError::Validation(format!(
                    "no indexer management client available for provider type '{}'",
                    parent.provider_type
                ))
            })?;
        let Some(plan) = client.enrichment_sync_plan(&parent.id).await? else {
            return Ok(());
        };
        let desired_children = self
            .prepare_managed_indexer_sync_plan(&parent, plan)
            .await?;
        let existing_by_key = self
            .services
            .integrations
            .indexer_configs
            .list(None)
            .await?
            .into_iter()
            .filter(|candidate| {
                candidate.managed_parent_config_id.as_deref() == Some(parent.id.as_str())
            })
            .filter_map(|candidate| {
                candidate
                    .managed_child_key
                    .clone()
                    .map(|child_key| (child_key, candidate))
            })
            .collect::<HashMap<_, _>>();

        let mut enriched = 0_usize;
        for desired in desired_children {
            let Some(caps_snapshot_json) = desired.caps_snapshot_json else {
                continue;
            };
            let Some(existing) = existing_by_key.get(&desired.child_key) else {
                continue;
            };
            let managed_metadata_json = merge_managed_child_metadata(
                existing.managed_metadata_json.as_deref(),
                desired.managed_metadata_json.as_deref(),
            )
            .or_else(|| desired.managed_metadata_json.clone())
            .or_else(|| existing.managed_metadata_json.clone());
            if existing.caps_snapshot_json.as_deref() == Some(caps_snapshot_json.as_str())
                && existing.managed_metadata_json == managed_metadata_json
            {
                continue;
            }
            let updated = self
                .services
                .integrations
                .indexer_configs
                .update(IndexerConfigUpdate {
                    id: existing.id.clone(),
                    managed_metadata_json: Some(managed_metadata_json),
                    caps_snapshot_json: Some(Some(caps_snapshot_json)),
                    ..Default::default()
                })
                .await?;
            if crate::indexer_search_identity(existing, None)
                != crate::indexer_search_identity(&updated, None)
            {
                self.prune_indexer_search_learning_best_effort(
                    &updated.id,
                    "managed_indexer_caps_change",
                )
                .await;
            }
            enriched = enriched.saturating_add(1);
        }
        if enriched > 0 {
            self.publish_indexers_changed();
        }
        tracing::info!(
            config_id = %parent.id,
            child_count = enriched,
            duration_ms = started.elapsed().as_millis(),
            "managed indexer caps enrichment completed"
        );
        Ok(())
    }
}
impl AppUseCase {
    async fn prepare_managed_indexer_sync_plan(
        &self,
        parent: &IndexerConfig,
        plan: IndexerSyncPlan,
    ) -> AppResult<Vec<PreparedManagedIndexerChild>> {
        let mut seen_child_keys = HashSet::new();
        let mut prepared = Vec::with_capacity(plan.children.len());

        for child in plan.children {
            let child_key = child.child_key.trim().to_string();
            if child_key.is_empty() {
                return Err(AppError::Validation(
                    "managed child plan entries require child_key".into(),
                ));
            }
            if !seen_child_keys.insert(child_key.clone()) {
                return Err(AppError::Validation(format!(
                    "managed child plan contains duplicate child_key '{}'",
                    child_key
                )));
            }

            let name = child.name.trim().to_string();
            if name.is_empty() {
                return Err(AppError::Validation(format!(
                    "managed child '{}' requires a name",
                    child_key
                )));
            }

            let provider_type = child.provider_type.trim().to_ascii_lowercase();
            if provider_type.is_empty() {
                return Err(AppError::Validation(format!(
                    "managed child '{}' requires provider_type",
                    child_key
                )));
            }

            let fields = self.indexer_config_fields_for_provider_type(&provider_type)?;
            let config_json =
                normalize_indexer_config_json(&fields, Some(child.config_json.as_str()), None)?;
            let base_url = derive_indexer_base_url_from_config_fields(&fields, Some(&config_json))?;
            let managed_metadata_json = child
                .managed_metadata_json
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty());
            let routing_by_scope = normalize_managed_child_routing_scopes(child.routing_scopes)?;

            prepared.push(PreparedManagedIndexerChild {
                child_key,
                name,
                provider_type,
                base_url,
                config_json,
                is_enabled: parent.is_enabled && child.is_enabled,
                enable_interactive_search: child.enable_interactive_search,
                enable_auto_search: child.enable_auto_search,
                managed_metadata_json,
                caps_snapshot_json: child
                    .caps_snapshot_json
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty()),
                routing_by_scope,
            });
        }

        Ok(prepared)
    }
}

const MANAGED_INDEXER_SCOPE_IDS: &[&str] = &["movie", "series", "anime"];
fn normalize_managed_child_routing_scopes(
    scopes: Vec<ManagedIndexerRoutingScope>,
) -> AppResult<HashMap<String, Vec<String>>> {
    let mut routing_by_scope = HashMap::new();
    for scope in scopes {
        let scope_id = scope.scope_id.trim().to_ascii_lowercase();
        if !MANAGED_INDEXER_SCOPE_IDS.contains(&scope_id.as_str()) {
            return Err(AppError::validation_refused(
                crate::error_reasons::integration::MANAGED_CHILD_ROUTING_SCOPE_UNSUPPORTED,
                format!(
                    "managed child routing scope '{}' is not supported",
                    scope.scope_id
                ),
            ));
        }
        if routing_by_scope.contains_key(&scope_id) {
            return Err(AppError::validation_refused(
                crate::error_reasons::integration::MANAGED_CHILD_ROUTING_CONTAINS_DUPLICATE_SCOPE,
                format!(
                    "managed child routing contains duplicate scope '{}'",
                    scope_id
                ),
            ));
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
                entries.retain(|entry| entry.indexer_id != indexer_id || !entry.enabled);
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
        self.restore_coordinator().spawn_task(async move {
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
        self.restore_coordinator().spawn_task(async move {
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

    /// Fetches capabilities for the managed children that need them: a new
    /// child, a child whose snapshot is missing, expired or invalidated by a
    /// connection change, and only while the child is enabled and not backed
    /// off. A pass where every snapshot is fresh sends nothing to the parent.
    pub(crate) async fn enrich_managed_indexer_children(
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
        let indexer_configs = &self.services.integrations.indexer_configs;
        let parent = indexer_configs.get_by_id(config_id).await?.ok_or_else(|| {
            AppError::not_found_reason(
                crate::error_reasons::integration::INDEXER_CONFIG_NOT_FOUND,
                format!("indexer config '{config_id}' not found"),
            )
        })?;
        let existing_by_key = indexer_configs
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
        let system_backoffs = indexer_configs.list_system_backoffs().await?;
        let now = Utc::now();
        let unanswered = &self.runtime.integrations.managed_caps_unanswered;
        let caps_due = existing_by_key
            .iter()
            .filter(|(_, child)| child.is_enabled)
            .filter(|(_, child)| {
                crate::integration::indexer_caps_cache::fresh_snapshot(child, now).is_none()
            })
            .filter(|(_, child)| {
                let backed_off = child
                    .disabled_until
                    .into_iter()
                    .chain(
                        system_backoffs
                            .get(&child.id)
                            .map(|backoff| backoff.disabled_until),
                    )
                    .any(|until| until > now);
                if backed_off {
                    tracing::debug!(
                        config_id = %child.id,
                        "skipping managed child caps fetch while the child is backed off"
                    );
                }
                !backed_off
            })
            .filter(|(_, child)| {
                let waiting = unanswered.waiting(child, now);
                if waiting {
                    tracing::debug!(
                        config_id = %child.id,
                        "skipping managed child caps fetch until its unanswered request is due again"
                    );
                }
                !waiting
            })
            .map(|(child_key, _)| child_key.clone())
            .collect::<HashSet<_>>();
        if caps_due.is_empty() {
            tracing::debug!(
                config_id = %parent.id,
                "managed indexer caps are fresh; nothing to fetch"
            );
            return Ok(());
        }

        let provider = self
            .services
            .integrations
            .plugin_provider
            .available()
            .ok_or_else(|| AppError::Repository("indexer provider not available".into()))?;
        let client = provider
            .management_client_for_provider(&parent)
            .ok_or_else(|| {
                AppError::validation_refused(crate::error_reasons::integration::NO_INDEXER_MANAGEMENT_CLIENT_AVAILABLE_PROVIDER_TYPE, format!(
                    "no indexer management client available for provider type '{}'",
                    parent.provider_type
                ))
            })?;
        let plan = client.enrichment_sync_plan(&parent.id, &caps_due).await?;
        let desired_children = match plan {
            Some(plan) => {
                self.prepare_managed_indexer_sync_plan(&parent, plan)
                    .await?
            }
            None => Vec::new(),
        };

        let mut answered = HashSet::new();
        let mut enriched = 0_usize;
        let mut changed = false;
        for desired in desired_children {
            let Some(provider_snapshot_json) = desired.caps_snapshot_json else {
                continue;
            };
            if !caps_due.contains(&desired.child_key) {
                continue;
            }
            let Some(existing) = existing_by_key.get(&desired.child_key) else {
                continue;
            };
            let caps_snapshot_json =
                match serde_json::from_str::<scryer_domain::IndexerCapsSnapshot>(
                    &provider_snapshot_json,
                )
                .map_err(|error| AppError::Repository(error.to_string()))
                .and_then(|snapshot| {
                    crate::integration::indexer_caps_cache::serialize_snapshot(
                        &snapshot,
                        Utc::now(),
                    )
                }) {
                    Ok(snapshot_json) => snapshot_json,
                    Err(error) => {
                        tracing::warn!(
                            config_id = %existing.id,
                            error = %error,
                            "managed child caps snapshot could not be stored"
                        );
                        continue;
                    }
                };
            let managed_metadata_json = merge_managed_child_metadata(
                existing.managed_metadata_json.as_deref(),
                desired.managed_metadata_json.as_deref(),
            )
            .or_else(|| desired.managed_metadata_json.clone())
            .or_else(|| existing.managed_metadata_json.clone());
            let content_changed = !crate::integration::indexer_caps_cache::same_snapshot_content(
                existing.caps_snapshot_json.as_deref(),
                Some(caps_snapshot_json.as_str()),
            ) || existing.managed_metadata_json != managed_metadata_json;
            let updated = indexer_configs
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
            changed |= content_changed;
            enriched = enriched.saturating_add(1);
            answered.insert(desired.child_key.clone());
        }
        // A due child the pass got no caps for waits before it is asked again,
        // rather than being asked on every sync.
        for child_key in &caps_due {
            let Some(child) = existing_by_key.get(child_key) else {
                continue;
            };
            if answered.contains(child_key) {
                unanswered.record_answered(&child.id);
            } else {
                let retry_at = unanswered.record_unanswered(child, now);
                tracing::debug!(
                    config_id = %child.id,
                    retry_at = %retry_at,
                    "managed child returned no caps; asking again later"
                );
            }
        }
        // A refetch that only renews the fetch time changes nothing anyone sees.
        if changed {
            self.publish_indexers_changed();
        }
        tracing::info!(
            config_id = %parent.id,
            due_count = caps_due.len(),
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
                return Err(AppError::validation_refused(
                    crate::error_reasons::integration::MANAGED_CHILD_PLAN_ENTRIES_REQUIRE_CHILD_KEY,
                    "managed child plan entries require child_key",
                ));
            }
            if !seen_child_keys.insert(child_key.clone()) {
                return Err(AppError::validation_refused(crate::error_reasons::integration::MANAGED_CHILD_PLAN_CONTAINS_DUPLICATE_CHILD_KEY, format!(
                    "managed child plan contains duplicate child_key '{}'",
                    child_key
                )));
            }

            let name = child.name.trim().to_string();
            if name.is_empty() {
                return Err(AppError::validation_refused(
                    crate::error_reasons::integration::MANAGED_CHILD_REQUIRES_NAME,
                    format!("managed child '{}' requires a name", child_key),
                ));
            }

            let provider_type = child.provider_type.trim().to_ascii_lowercase();
            if provider_type.is_empty() {
                return Err(AppError::validation_refused(
                    crate::error_reasons::integration::MANAGED_CHILD_REQUIRES_PROVIDER_TYPE,
                    format!("managed child '{}' requires provider_type", child_key),
                ));
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

#[cfg(test)]
mod routing_tests {
    use super::*;

    fn routing_entry(
        enabled: bool,
        categories: &[&str],
        priority: i32,
    ) -> IndexerRoutingSettingsEntry {
        IndexerRoutingSettingsEntry {
            indexer_id: "managed-child".to_string(),
            enabled,
            categories: categories
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
            priority,
        }
    }

    #[test]
    fn managed_routing_keeps_disabled_tombstone_until_scope_returns() {
        let mut routing_by_scope = HashMap::from([(
            "movie".to_string(),
            vec![routing_entry(false, &["2000"], 7)],
        )]);

        apply_managed_child_routing(&mut routing_by_scope, "managed-child", &HashMap::new());

        let tombstone = &routing_by_scope["movie"][0];
        assert!(!tombstone.enabled);
        assert_eq!(tombstone.categories, vec!["2000"]);
        assert_eq!(tombstone.priority, 7);

        let desired_scopes = HashMap::from([(
            "movie".to_string(),
            vec!["2010".to_string(), "2020".to_string()],
        )]);
        apply_managed_child_routing(&mut routing_by_scope, "managed-child", &desired_scopes);

        let restored = &routing_by_scope["movie"][0];
        assert!(!restored.enabled);
        assert_eq!(restored.categories, vec!["2000"]);
        assert_eq!(restored.priority, 7);
    }

    #[test]
    fn managed_routing_seeds_new_scopes_but_preserves_local_categories() {
        for categories in [vec!["2040"], vec![]] {
            let local = routing_entry(true, &categories, 7);
            let mut routing = HashMap::from([("movie".to_string(), vec![local.clone()])]);
            let desired = HashMap::from([
                (
                    "movie".to_string(),
                    vec!["2000".to_string(), "2040".to_string()],
                ),
                ("anime".to_string(), vec!["5070".to_string()]),
            ]);
            for _ in 0..2 {
                apply_managed_child_routing(&mut routing, "managed-child", &desired);
                assert_eq!(routing["movie"], vec![local.clone()]);
                assert_eq!(routing["anime"], vec![routing_entry(true, &["5070"], 1)]);
            }
        }
    }

    #[test]
    fn managed_routing_removes_enabled_entry_for_omitted_scope() {
        let mut routing_by_scope =
            HashMap::from([("movie".to_string(), vec![routing_entry(true, &["2000"], 7)])]);

        apply_managed_child_routing(&mut routing_by_scope, "managed-child", &HashMap::new());

        assert!(routing_by_scope["movie"].is_empty());
    }
}

//! Reads the instance's configuration and state for the bundle: the known
//! values the sanitizer replaces, and the summary member.
//!
//! The summary carries types, counts, codes and pseudonyms only. No name,
//! URL, path or per-title row is written into it.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::{Value, json};

use super::Omission;
use super::sanitize::KnownValues;
use crate::{AppResult, AppUseCase};

/// Release decisions read for the decision-code tally, newest first.
const RECENT_RELEASE_DECISIONS: i64 = 500;
/// Indexer errors read for the classification tally, newest first.
const RECENT_INDEXER_ERRORS: usize = 500;
/// Job runs read for the outcome tally, newest first.
const RECENT_JOB_RUNS: usize = 500;

/// Quality words a profile is often named after. A profile called `1080p`
/// must not rewrite every release name that mentions 1080p, and the word
/// identifies nobody.
const QUALITY_VOCABULARY: &[&str] = &[
    "2160p", "1080p", "1080i", "720p", "576p", "480p", "4k", "uhd", "fhd", "hd", "sd", "remux",
    "bluray", "blu-ray", "web", "webdl", "web-dl", "webrip", "hdtv", "dvd", "any", "default",
    "standard", "best", "anime", "movies", "series", "tv",
];

/// The string a serde enum renders to, for type and state labels.
pub(super) fn serde_label<T: Serialize>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(Value::String(label)) => label,
        _ => "unknown".to_string(),
    }
}

/// Counts by `(type, enabled)` as `{type: {enabled: n, disabled: n}}`.
fn count_by_type(items: impl IntoIterator<Item = (String, bool)>) -> Value {
    let mut counts: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    for (kind, enabled) in items {
        let entry = counts.entry(kind).or_default();
        if enabled {
            entry.0 += 1;
        } else {
            entry.1 += 1;
        }
    }
    Value::Object(
        counts
            .into_iter()
            .map(|(kind, (enabled, disabled))| {
                (kind, json!({ "enabled": enabled, "disabled": disabled }))
            })
            .collect(),
    )
}

fn tally(items: impl IntoIterator<Item = String>) -> Value {
    let mut counts: BTreeMap<String, u64> = BTreeMap::new();
    for item in items {
        *counts.entry(item).or_default() += 1;
    }
    json!(counts)
}

/// Records a section, or its omission.
fn section(
    sections: &mut serde_json::Map<String, Value>,
    omissions: &mut Vec<Omission>,
    name: &str,
    result: AppResult<Value>,
) {
    match result {
        Ok(value) => {
            sections.insert(name.to_string(), value);
        }
        Err(error) => {
            omissions.push(Omission::new(
                format!("summary.{name}"),
                format!("read_failed:{}", error_kind(&error)),
            ));
        }
    }
}

/// A stable, text-free label for why a read failed.
pub(super) fn error_kind(error: &crate::AppError) -> &'static str {
    match error {
        crate::AppError::Unauthorized(_) | crate::AppError::UnauthorizedReason { .. } => {
            "unauthorized"
        }
        crate::AppError::NotFound(_) | crate::AppError::NotFoundReason { .. } => "not_found",
        crate::AppError::Validation(_) | crate::AppError::ValidationRefused { .. } => "validation",
        crate::AppError::TemporaryUnavailable { .. } => "temporarily_unavailable",
        _ => "error",
    }
}

impl AppUseCase {
    /// Registers every identifying value the instance holds. Returns the
    /// sources that could not be read; logs and metrics are left out of the
    /// bundle when any are missing, since their values would go unreplaced.
    pub(super) async fn harvest_known_values(
        &self,
        actor: &crate::User,
        known: &mut KnownValues,
    ) -> Vec<String> {
        let mut incomplete = Vec::new();
        macro_rules! source {
            ($name:literal, $read:expr) => {
                match $read.await {
                    Ok(value) => Some(value),
                    Err(_) => {
                        incomplete.push($name.to_string());
                        None
                    }
                }
            };
        }

        if let Some(libraries) = source!("libraries", self.services.catalog.libraries.list(None)) {
            for library in &libraries {
                let facet = serde_label(&library.facet);
                known.protect(&facet);
                let pseudonym = known.entity("library", Some(&facet), &library.id);
                known.name(&pseudonym, &library.name);
                known.name(&pseudonym, &library.slug);
                for root in &library.roots {
                    let root_pseudonym = known.entity("root", None, &root.id);
                    known.path(&root_pseudonym, &root.path);
                }
            }
        }

        if let Some(indexers) = source!(
            "indexers",
            self.services.integrations.indexer_configs.list(None)
        ) {
            for indexer in &indexers {
                known.protect(&indexer.provider_type);
                let pseudonym = known.entity("indexer", Some(&indexer.provider_type), &indexer.id);
                known.name(&pseudonym, &indexer.name);
                known.url(&pseudonym, &indexer.base_url);
                if let Some(key) = &indexer.api_key_encrypted {
                    known.secret(key);
                }
                for raw in [
                    indexer.config_json.as_deref(),
                    indexer.managed_metadata_json.as_deref(),
                ]
                .into_iter()
                .flatten()
                {
                    known.config_json(&pseudonym, raw);
                }
            }
        }

        if let Some(clients) = source!(
            "download_clients",
            self.services
                .integrations
                .download_client_configs
                .list(None)
        ) {
            for client in &clients {
                known.protect(&client.client_type);
                let pseudonym =
                    known.entity("download-client", Some(&client.client_type), &client.id);
                known.name(&pseudonym, &client.name);
                for (index, root) in download_roots(&client.config_json).iter().enumerate() {
                    let root_pseudonym =
                        known.entity("download-root", None, &format!("{}:{index}", client.id));
                    known.path(&root_pseudonym, root);
                }
                known.config_json(&pseudonym, &client.config_json);
            }
        }

        if let Some(proxies) = source!(
            "proxies",
            self.services.integrations.proxy_configs.list(None)
        ) {
            for proxy in &proxies {
                let kind = serde_label(&proxy.provider_type);
                known.protect(&kind);
                let pseudonym = known.entity("proxy", Some(&kind), &proxy.id);
                known.name(&pseudonym, &proxy.name);
                known.url(&pseudonym, &proxy.base_url);
                for secret in [
                    &proxy.password_encrypted,
                    &proxy.private_key_encrypted,
                    &proxy.private_key_passphrase_encrypted,
                ]
                .into_iter()
                .flatten()
                {
                    known.secret(secret);
                }
                if let Some(username) = &proxy.username_encrypted {
                    known.name(&pseudonym, username);
                }
            }
        }

        if let Some(servers) = source!(
            "media_servers",
            self.services
                .integrations
                .media_server_connections
                .list(None)
        ) {
            for server in &servers {
                let kind = serde_label(&server.provider);
                known.protect(&kind);
                let pseudonym = known.entity("media-server", Some(&kind), &server.id);
                known.name(&pseudonym, &server.display_name);
                known.url(&pseudonym, &server.base_url);
                if let Some(url) = &server.external_url {
                    known.url(&pseudonym, url);
                }
                if let Some(key) = &server.api_key {
                    known.secret(key);
                }
                for id in [&server.machine_id, &server.emby_server_id]
                    .into_iter()
                    .flatten()
                {
                    known.secret(id);
                }
                for mapping in &server.path_mappings {
                    known.path(&format!("{pseudonym}-path"), &mapping.source_path);
                    known.path(&format!("{pseudonym}-path"), &mapping.destination_path);
                }
            }
        }

        if let Some(repository) = self.services.notifications.notification_channels() {
            if let Some(channels) = source!("notification_channels", repository.list_channels()) {
                for channel in &channels {
                    let kind = serde_label(&channel.channel_type);
                    known.protect(&kind);
                    let pseudonym = known.entity("notification", Some(&kind), &channel.id);
                    known.name(&pseudonym, &channel.name);
                    known.config_json(&pseudonym, &channel.config_json);
                }
            }
        }

        if let Some(repository) = self
            .services
            .integrations
            .subtitle_provider_configs
            .available()
        {
            if let Some(providers) = source!("subtitle_providers", repository.list(None)) {
                for provider in &providers {
                    known.protect(&provider.provider_type);
                    let pseudonym = known.entity(
                        "subtitle-provider",
                        Some(&provider.provider_type),
                        &provider.id,
                    );
                    known.name(&pseudonym, &provider.name);
                    known.config_json(&pseudonym, &provider.config_json);
                }
            }
        }

        for word in QUALITY_VOCABULARY {
            known.protect(word);
        }
        if let Some(settings) = source!("quality_profiles", self.load_quality_profile_settings()) {
            for profile in &settings.profiles {
                for tier in &profile.criteria.quality_tiers {
                    known.protect(tier);
                }
                let pseudonym = known.entity("quality-profile", None, &profile.id);
                known.name(&pseudonym, &profile.name);
            }
        }

        if let Some(profiles) = source!("delay_profiles", self.delay_profiles()) {
            for profile in &profiles {
                let pseudonym = known.entity("delay-profile", None, &profile.id);
                known.name(&pseudonym, &profile.name);
            }
        }

        if let Some(profiles) = source!("seeding_profiles", self.seeding_profiles()) {
            for profile in &profiles {
                let pseudonym = known.entity("seeding-profile", None, &profile.id);
                known.name(&pseudonym, &profile.name);
            }
        }

        if let Some(rule_sets) = source!(
            "rule_sets",
            self.services.customization.rule_sets.list_rule_sets()
        ) {
            for rule_set in &rule_sets {
                let pseudonym = known.entity("rule-set", None, &rule_set.id);
                if !rule_set.is_managed {
                    known.name(&pseudonym, &rule_set.name);
                    known.name(&pseudonym, &rule_set.description);
                }
                for tag in rule_set.managed_tag_filter.iter().flatten() {
                    let tag_pseudonym = known.entity("tag", None, &tag.to_lowercase());
                    known.name(&tag_pseudonym, tag);
                }
            }
        }

        if let Some(labels) = source!("tags", self.services.catalog.titles.list_title_tag_labels())
        {
            for label in &labels {
                let pseudonym = known.entity("tag", None, &label.to_lowercase());
                known.name(&pseudonym, label);
            }
        }

        if let Some(users) = source!("users", self.services.identity.users.list_all()) {
            for user in &users {
                let pseudonym = known.entity("user", None, &user.id);
                known.name(&pseudonym, &user.username);
                if let Some(keys) = source!(
                    "api_keys",
                    self.services.identity.oauth.list_api_keys(&user.id)
                ) {
                    for key in &keys {
                        let key_pseudonym = known.entity("api-key", None, &key.id);
                        known.name(&key_pseudonym, &key.label);
                    }
                }
                if let Some(accounts) = source!(
                    "list_accounts",
                    self.services.lists.accounts.list_by_user_id(&user.id)
                ) {
                    for account in &accounts {
                        let account_pseudonym = known.entity("list-account", None, &account.id);
                        known.name(&account_pseudonym, &account.username);
                        known.name(&account_pseudonym, &account.external_user_id);
                        if let Some(display_name) = &account.display_name {
                            known.name(&account_pseudonym, display_name);
                        }
                    }
                }
            }
        }

        if let Some(keys) = source!(
            "environment_api_keys",
            self.services.identity.oauth.list_environment_api_keys()
        ) {
            for key in &keys {
                let pseudonym = known.entity("api-key", None, &key.id);
                known.name(&pseudonym, &key.label);
            }
        }

        if let Some(subscriptions) = source!(
            "lists",
            self.services
                .lists
                .subscriptions
                .list(crate::lists::ListSubscriptionQuery::default())
        ) {
            for subscription in &subscriptions {
                let pseudonym = known.entity("list", None, &subscription.id);
                known.name(&pseudonym, &subscription.name);
                if let Some(url) = &subscription.provider_url {
                    known.url(&pseudonym, url);
                }
            }
        }

        let public_url = self.public_url_runtime().snapshot();
        if let Some(url) = public_url.configured_value() {
            known.url("public-url", url);
        }
        for (index, address) in self
            .trusted_proxy_runtime()
            .snapshot()
            .addresses
            .iter()
            .enumerate()
        {
            known.name(&format!("trusted-proxy-{}", index + 1), address);
        }

        if let Some(recycle) = source!("recycle_bin", self.get_recycle_bin_settings(actor)) {
            if let Some(path) = &recycle.path {
                known.path("recycle-bin", path);
            }
            for path in &recycle.effective_paths {
                known.path("recycle-bin", path);
            }
        }

        known.path(
            "backup-dir",
            &self.services.config.backup_dir.to_string_lossy(),
        );
        known.path(
            "scripts-dir",
            &self.services.config.scripts_dir.to_string_lossy(),
        );
        if let Some(secret) = &self.services.config.smg_registration_secret {
            known.secret(secret);
        }

        incomplete.sort();
        incomplete.dedup();
        incomplete
    }

    /// The summary member: configuration shape and state aggregates.
    pub(super) async fn diagnostics_summary(
        &self,
        actor: &crate::User,
        omissions: &mut Vec<Omission>,
    ) -> Value {
        let mut sections = serde_json::Map::new();

        section(
            &mut sections,
            omissions,
            "quality_profiles",
            self.summary_quality_profiles().await,
        );
        section(
            &mut sections,
            omissions,
            "indexers",
            self.services
                .integrations
                .indexer_configs
                .list(None)
                .await
                .map(|indexers| {
                    json!({
                        "total": indexers.len(),
                        "by_provider_type": count_by_type(indexers.iter().map(|indexer| {
                            (indexer.provider_type.clone(), indexer.is_enabled)
                        })),
                        "interactive_search_enabled": indexers.iter().filter(|i| i.enable_interactive_search).count(),
                        "auto_search_enabled": indexers.iter().filter(|i| i.enable_auto_search).count(),
                        "with_proxy": indexers.iter().filter(|i| i.proxy_config_id.is_some()).count(),
                        "health_status": tally(indexers.iter().map(|indexer| {
                            indexer.last_health_status.clone().unwrap_or_else(|| "unknown".to_string())
                        })),
                        "last_error_codes": tally(indexers.iter().filter_map(|indexer| indexer.last_error_code.clone())),
                    })
                }),
        );
        section(
            &mut sections,
            omissions,
            "download_clients",
            self.services
                .integrations
                .download_client_configs
                .list(None)
                .await
                .map(|clients| {
                    json!({
                        "total": clients.len(),
                        "by_client_type": count_by_type(clients.iter().map(|client| {
                            (client.client_type.clone(), client.is_enabled)
                        })),
                        "status": tally(clients.iter().map(|client| serde_label(&client.status))),
                        "with_remote_path_mappings": clients.iter().filter(|client| {
                            crate::download_client_path_mappings::parse_download_client_remote_path_mappings(&client.config_json)
                                .is_ok_and(|mappings| !mappings.is_empty())
                        }).count(),
                    })
                }),
        );
        section(
            &mut sections,
            omissions,
            "libraries",
            self.services
                .catalog
                .libraries
                .list(None)
                .await
                .map(|libraries| {
                    json!({
                        "total": libraries.len(),
                        "by_facet": tally(libraries.iter().map(|library| serde_label(&library.facet))),
                        "root_folders": libraries.iter().map(|library| library.roots.len()).sum::<usize>(),
                        "root_folders_by_facet": tally(libraries.iter().flat_map(|library| {
                            library.roots.iter().map(|_| serde_label(&library.facet))
                        })),
                    })
                }),
        );
        section(
            &mut sections,
            omissions,
            "rule_sets",
            self.summary_rule_sets().await,
        );
        section(
            &mut sections,
            omissions,
            "delay_profiles",
            self.delay_profiles().await.map(|profiles| {
                json!({
                    "total": profiles.len(),
                    "enabled": profiles.iter().filter(|profile| profile.enabled).count(),
                })
            }),
        );
        section(
            &mut sections,
            omissions,
            "seeding_profiles",
            self.seeding_profiles()
                .await
                .map(|profiles| json!({ "total": profiles.len() })),
        );
        match self.services.notifications.notification_channels() {
            Some(repository) => section(
                &mut sections,
                omissions,
                "notification_channels",
                repository.list_channels().await.map(|channels| {
                    json!({
                        "total": channels.len(),
                        "by_type": count_by_type(channels.iter().map(|channel| {
                            (serde_label(&channel.channel_type), channel.is_enabled)
                        })),
                    })
                }),
            ),
            None => {
                sections.insert(
                    "notification_channels".to_string(),
                    json!({ "available": false }),
                );
            }
        }
        section(
            &mut sections,
            omissions,
            "media_servers",
            self.services
                .integrations
                .media_server_connections
                .list(None)
                .await
                .map(|servers| {
                    json!({
                        "total": servers.len(),
                        "by_provider": count_by_type(servers.iter().map(|server| {
                            (serde_label(&server.provider), server.enabled)
                        })),
                        "with_path_mappings": servers.iter().filter(|server| !server.path_mappings.is_empty()).count(),
                    })
                }),
        );
        match self
            .services
            .integrations
            .subtitle_provider_configs
            .available()
        {
            Some(repository) => section(
                &mut sections,
                omissions,
                "subtitle_providers",
                repository.list(None).await.map(|providers| {
                    json!({
                        "total": providers.len(),
                        "by_provider_type": count_by_type(providers.iter().map(|provider| {
                            (provider.provider_type.clone(), provider.is_enabled)
                        })),
                    })
                }),
            ),
            None => {
                sections.insert(
                    "subtitle_providers".to_string(),
                    json!({ "available": false }),
                );
            }
        }
        section(
            &mut sections,
            omissions,
            "proxies",
            self.services
                .integrations
                .proxy_configs
                .list(None)
                .await
                .map(|proxies| {
                    json!({
                        "total": proxies.len(),
                        "by_provider_type": count_by_type(proxies.iter().map(|proxy| {
                            (serde_label(&proxy.provider_type), proxy.is_enabled)
                        })),
                    })
                }),
        );
        section(
            &mut sections,
            omissions,
            "lists",
            self.services
                .lists
                .subscriptions
                .list(crate::lists::ListSubscriptionQuery::default())
                .await
                .map(|subscriptions| {
                    json!({
                        "total": subscriptions.len(),
                        "enabled": subscriptions.iter().filter(|s| s.enabled).count(),
                        "by_scope": tally(subscriptions.iter().map(|s| serde_label(&s.scope))),
                    })
                }),
        );
        section(
            &mut sections,
            omissions,
            "plugins",
            self.services
                .customization
                .plugin_installations
                .list_plugin_installations()
                .await
                .map(|plugins| {
                    let mut rows = plugins
                        .iter()
                        .map(|plugin| {
                            json!({
                                "plugin_id": plugin.plugin_id,
                                "version": plugin.version,
                                "plugin_type": plugin.plugin_type,
                                "provider_type": plugin.provider_type,
                                "builtin": plugin.is_builtin,
                                "enabled": plugin.is_enabled,
                                "sdk_version": plugin.sdk_version,
                                "support_tier": serde_label(&plugin.support_tier),
                            })
                        })
                        .collect::<Vec<_>>();
                    rows.sort_by_key(|row| {
                        row["plugin_id"].as_str().unwrap_or_default().to_string()
                    });
                    Value::Array(rows)
                }),
        );
        let health = self.runtime.health.results.read().await.clone();
        sections.insert(
            "health_checks".to_string(),
            Value::Array(
                health
                    .iter()
                    .map(|check| json!({ "kind": check.source, "severity": check.status.as_str() }))
                    .collect(),
            ),
        );
        section(
            &mut sections,
            omissions,
            "job_runs",
            self.summary_job_runs().await,
        );
        section(
            &mut sections,
            omissions,
            "titles",
            self.services
                .catalog
                .titles
                .title_counts()
                .await
                .map(|counts| {
                    json!({
                        "total": counts.total,
                        "monitored": counts.monitored,
                        "movie": counts.movie,
                        "series": counts.series,
                        "anime": counts.anime,
                    })
                }),
        );
        let queue = self
            .runtime
            .acquisition
            .download_queue_snapshot
            .snapshot()
            .await;
        sections.insert(
            "download_queue".to_string(),
            json!({
                "items": queue.items.len(),
                "by_state": tally(queue.items.iter().map(|item| serde_label(&item.state))),
                "by_client_type": tally(queue.items.iter().map(|item| item.client_type.clone())),
            }),
        );
        omissions.push(Omission::new(
            "summary.wanted_items",
            "no_instance_wide_count",
        ));
        section(
            &mut sections,
            omissions,
            "release_decisions",
            self.list_release_decisions_page(
                actor,
                crate::ReleaseDecisionsQuery {
                    wanted_item_id: None,
                    title_id: None,
                    limit: RECENT_RELEASE_DECISIONS,
                    offset: 0,
                },
            )
            .await
                .map(|(decisions, total)| {
                    json!({
                        "total_rows": total,
                        "sampled": decisions.len(),
                        "by_code": tally(decisions.iter().map(|decision| decision.decision_code.clone())),
                    })
                }),
        );
        section(
            &mut sections,
            omissions,
            "indexer_errors",
            self.list_indexer_errors(None, RECENT_INDEXER_ERRORS, None)
                .await
                .map(|page| {
                    json!({
                        "sampled": page.items.len(),
                        "by_classification": tally(page.items.iter().map(|error| error.classification.as_str().to_string())),
                        "by_operation": tally(page.items.iter().map(|error| serde_label(&error.operation))),
                    })
                }),
        );
        section(
            &mut sections,
            omissions,
            "users",
            self.services
                .identity
                .users
                .list_all()
                .await
                .map(|users| {
                    json!({
                        "total": users.len(),
                        "by_account_kind": tally(users.iter().map(|user| serde_label(&user.account_kind))),
                    })
                }),
        );
        Value::Object(sections)
    }

    async fn summary_quality_profiles(&self) -> AppResult<Value> {
        let settings = self.load_quality_profile_settings().await?;
        // Pseudonyms here follow the same order as the known-value harvest,
        // so `quality-profile-N` in the summary and in the logs is the same
        // profile.
        let mut ordinal = BTreeMap::new();
        for (index, profile) in settings.profiles.iter().enumerate() {
            ordinal.insert(profile.id.clone(), format!("quality-profile-{}", index + 1));
        }
        let profiles = settings
            .profiles
            .iter()
            .map(|profile| {
                json!({
                    "pseudonym": ordinal[&profile.id],
                    "global_default": profile.id == settings.global_profile_id,
                    "criteria": serde_json::to_value(&profile.criteria).unwrap_or(Value::Null),
                })
            })
            .collect::<Vec<_>>();
        Ok(json!({
            "total": settings.profiles.len(),
            "global_scoring_persona": serde_label(&settings.global_scoring_persona),
            "profiles": profiles,
        }))
    }

    async fn summary_rule_sets(&self) -> AppResult<Value> {
        let rule_sets = self
            .services
            .customization
            .rule_sets
            .list_rule_sets()
            .await?;
        let packs = self
            .services
            .customization
            .rule_sets
            .list_rule_pack_installations()
            .await?;
        let user = rule_sets.iter().filter(|rule_set| !rule_set.is_managed);
        let managed = rule_sets.iter().filter(|rule_set| rule_set.is_managed);
        Ok(json!({
            "user": {
                "total": user.clone().count(),
                "enabled": user.clone().filter(|rule_set| rule_set.enabled).count(),
                "by_phase": tally(user.map(|rule_set| serde_label(&rule_set.evaluation_phase))),
            },
            "managed": {
                "total": managed.clone().count(),
                "enabled": managed.clone().filter(|rule_set| rule_set.enabled).count(),
                "disabled_reason_codes": tally(managed.filter_map(|rule_set| rule_set.disabled_reason_code.clone())),
            },
            "packs": packs.iter().map(|pack| json!({
                "pack_id": pack.pack_id,
                "version": pack.version,
                "revision": pack.revision,
                "auto_update": pack.auto_update,
                "customizable": pack.customizable,
                "members": pack.members.len(),
                "has_error": pack.last_error.is_some(),
            })).collect::<Vec<_>>(),
        }))
    }

    async fn summary_job_runs(&self) -> AppResult<Value> {
        let runs = self
            .services
            .events
            .job_runs
            .list_job_runs(None, RECENT_JOB_RUNS)
            .await?;
        let mut by_key: BTreeMap<
            &'static str,
            (BTreeMap<&'static str, u64>, Option<&crate::JobRunRecord>),
        > = BTreeMap::new();
        for run in &runs {
            let entry = by_key.entry(run.job_key.as_str()).or_default();
            *entry.0.entry(run.status.as_str()).or_default() += 1;
            if entry
                .1
                .is_none_or(|latest| run.started_at > latest.started_at)
            {
                entry.1 = Some(run);
            }
        }
        Ok(json!({
            "sampled": runs.len(),
            "by_job": by_key.into_iter().map(|(key, (statuses, latest))| {
                (key.to_string(), json!({
                    "statuses": statuses,
                    "latest_status": latest.map(|run| run.status.as_str()),
                    "latest_started_at": latest.map(|run| run.started_at.to_rfc3339()),
                    "latest_trigger": latest.map(|run| serde_label(&run.trigger_source)),
                }))
            }).collect::<serde_json::Map<_, _>>(),
        }))
    }
}

/// Local download roots a client's configuration names: the local side of
/// every remote path mapping, and every absolute path under a path-like key.
pub(super) fn download_roots(config_json: &str) -> Vec<String> {
    let mut roots = Vec::new();
    if let Ok(mappings) =
        crate::download_client_path_mappings::parse_download_client_remote_path_mappings(
            config_json,
        )
    {
        roots.extend(
            mappings
                .iter()
                .map(|mapping| mapping.local_root().to_string()),
        );
    }
    if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(config_json) {
        for (key, value) in map {
            if super::sanitize::classify_field(&key) != super::sanitize::FieldClass::Path {
                continue;
            }
            if let Value::String(path) = value {
                let path = path.trim();
                if path.starts_with('/') || path.get(1..3).is_some_and(|s| s == ":\\" || s == ":/")
                {
                    roots.push(path.to_string());
                }
            }
        }
    }
    roots.sort();
    roots.dedup();
    roots
}

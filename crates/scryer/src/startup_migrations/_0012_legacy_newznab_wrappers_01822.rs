use std::sync::Arc;

use scryer_application::{
    IndexerConfigRepository, IndexerConfigUpdate, PluginInstallationRepository,
};
use serde_json::Value;

fn legacy_profile_id(provider_type: &str) -> Option<&'static str> {
    if provider_type.eq_ignore_ascii_case("nzbgeek") {
        Some("nzbgeek")
    } else if provider_type.eq_ignore_ascii_case("dognzb") {
        Some("dognzb")
    } else {
        None
    }
}

/// What to do with one indexer configuration.
enum LegacyConfigOutcome {
    /// Not a legacy wrapper; leave it alone.
    NotLegacy,
    /// A legacy wrapper whose configuration converted cleanly.
    Migrate(String),
    /// A legacy wrapper whose stored configuration cannot be parsed.
    ///
    /// This migration is required at startup, so a conversion error here would
    /// abort every boot until the row is repaired by hand — a boot loop earned
    /// by one corrupt row the old plugin system never re-parsed. The row is
    /// left untouched instead: that one indexer stays broken and visible, and
    /// the application still starts.
    Skip(String),
}

fn legacy_config_outcome(provider_type: &str, config_json: Option<&str>) -> LegacyConfigOutcome {
    let Some(profile_id) = legacy_profile_id(provider_type) else {
        return LegacyConfigOutcome::NotLegacy;
    };
    let mut value: Value = match serde_json::from_str(config_json.unwrap_or("{}")) {
        Ok(value) => value,
        Err(error) => {
            return LegacyConfigOutcome::Skip(format!(
                "legacy {provider_type} configuration is invalid JSON: {error}"
            ));
        }
    };
    let Some(object) = value.as_object_mut() else {
        return LegacyConfigOutcome::Skip(format!(
            "legacy {provider_type} configuration must be a JSON object"
        ));
    };
    object.insert(
        "profile_id".to_string(),
        Value::String(profile_id.to_string()),
    );
    LegacyConfigOutcome::Migrate(value.to_string())
}

fn legacy_plugin_name(name: &str, plugin_id: &str) -> String {
    let trimmed = name.trim();
    let base = if trimmed.is_empty() {
        plugin_id.trim()
    } else {
        trimmed
    };
    if base.to_ascii_lowercase().ends_with(" - legacy") {
        base.to_string()
    } else {
        format!("{base} - Legacy")
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MigrationReport {
    pub indexer_configs: u64,
    pub plugin_installations: u64,
    /// Legacy rows whose configuration could not be parsed and were left
    /// untouched rather than aborting startup.
    pub skipped_indexer_configs: u64,
}

pub async fn migrate(
    indexer_configs: Arc<dyn IndexerConfigRepository>,
    plugin_installations: &dyn PluginInstallationRepository,
) -> Result<MigrationReport, String> {
    let configs = indexer_configs
        .list(None)
        .await
        .map_err(|error| format!("failed to list indexer configurations: {error}"))?;
    let mut report = MigrationReport::default();
    for config in configs {
        let config_json = match legacy_config_outcome(
            &config.provider_type,
            config.config_json.as_deref(),
        ) {
            LegacyConfigOutcome::NotLegacy => continue,
            LegacyConfigOutcome::Migrate(config_json) => config_json,
            LegacyConfigOutcome::Skip(error) => {
                tracing::warn!(
                    indexer_id = config.id.as_str(),
                    provider_type = config.provider_type.as_str(),
                    error = error.as_str(),
                    "legacy Newznab wrapper configuration could not be converted; leaving the row for the operator"
                );
                report.skipped_indexer_configs += 1;
                continue;
            }
        };
        indexer_configs
            .update(IndexerConfigUpdate {
                id: config.id,
                provider_type: Some("newznab".to_string()),
                config_json: Some(config_json),
                ..IndexerConfigUpdate::default()
            })
            .await
            .map_err(|error| format!("failed to migrate legacy indexer configuration: {error}"))?;
        report.indexer_configs += 1;
    }

    let installations = plugin_installations
        .list_plugin_installations()
        .await
        .map_err(|error| format!("failed to list plugin installations: {error}"))?;
    for mut installation in installations {
        if legacy_profile_id(&installation.plugin_id).is_none()
            && legacy_profile_id(&installation.provider_type).is_none()
        {
            continue;
        }
        let name = legacy_plugin_name(&installation.name, &installation.plugin_id);
        if !installation.is_enabled && installation.name == name {
            continue;
        }
        installation.name = name;
        installation.is_enabled = false;
        plugin_installations
            .update_plugin_installation(&installation, None)
            .await
            .map_err(|error| format!("failed to retire legacy plugin installation: {error}"))?;
        report.plugin_installations += 1;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_wrappers_map_to_their_newznab_profiles_and_preserve_overrides() {
        for (provider_type, expected_profile) in [("nzbgeek", "nzbgeek"), ("DogNZB", "dognzb")] {
            let LegacyConfigOutcome::Migrate(migrated) = legacy_config_outcome(
                provider_type,
                Some(
                    r#"{"additional_parameters":"attrs=poster","api_key":"secret","base_url":"https://custom.example.test","request_interval_ms":750}"#,
                ),
            ) else {
                panic!("legacy provider should produce an update");
            };
            let value: Value =
                serde_json::from_str(&migrated).expect("migrated configuration should be JSON");
            assert_eq!(value["profile_id"], expected_profile);
            assert_eq!(value["api_key"], "secret");
            assert_eq!(value["base_url"], "https://custom.example.test");
            assert_eq!(value["additional_parameters"], "attrs=poster");
            assert_eq!(value["request_interval_ms"], 750);
        }
    }

    #[test]
    fn legacy_plugin_names_gain_one_suffix() {
        assert_eq!(legacy_plugin_name("NZBGeek", "nzbgeek"), "NZBGeek - Legacy");
        assert_eq!(
            legacy_plugin_name("DogNZB - Legacy", "dognzb"),
            "DogNZB - Legacy"
        );
    }

    #[test]
    fn migration_ignores_nonlegacy_providers() {
        assert!(matches!(
            legacy_config_outcome("newznab", Some(r#"{"profile_id":"nzbgeek"}"#)),
            LegacyConfigOutcome::NotLegacy
        ));
    }

    /// A corrupt legacy row is skipped, never converted and never fatal: this
    /// migration is required at startup, and an error here would be a boot
    /// loop the operator cannot escape without database surgery.
    #[test]
    fn a_corrupt_legacy_configuration_is_skipped_not_fatal() {
        assert!(matches!(
            legacy_config_outcome("nzbgeek", Some("not-json")),
            LegacyConfigOutcome::Skip(_)
        ));
        assert!(matches!(
            legacy_config_outcome("dognzb", Some(r#"["not","an","object"]"#)),
            LegacyConfigOutcome::Skip(_)
        ));
        // An absent configuration is an empty object, not a corrupt one.
        assert!(matches!(
            legacy_config_outcome("nzbgeek", None),
            LegacyConfigOutcome::Migrate(_)
        ));
    }
}

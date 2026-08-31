use std::sync::Arc;

use scryer_application::{AppUseCase, SETTINGS_SCOPE_SYSTEM};
use scryer_infrastructure_configuration::settings::settings_store::SettingsStore;

use super::versioning::{MajorMinor, is_upgrade_from_line_to_at_least};

const ENHANCED_SUBTITLE_SYNC_PLUGIN_ID: &str = "enhanced-subtitle-sync";
pub(crate) const ENHANCED_SUBSYNC_016_MIGRATION_STATE_KEY: &str =
    "subtitles.enhanced_sync_plugin_migration_state";
const ENHANCED_SUBSYNC_016_MIGRATION_STATE_NONE: &str = "none";
const ENHANCED_SUBSYNC_016_MIGRATION_STATE_PENDING: &str = "pending";
const ENHANCED_SUBSYNC_016_MIGRATION_STATE_COMPLETED: &str = "completed";
const STARTUP_PLUGIN_MIGRATION_ACTOR_ID: &str = "system:startup-plugin-migration";

fn is_015_to_016_or_later_upgrade(previous_version: Option<&str>, current_version: &str) -> bool {
    is_upgrade_from_line_to_at_least(
        previous_version,
        current_version,
        MajorMinor::new(0, 15),
        MajorMinor::new(0, 16),
    )
}

fn should_attempt_enhanced_subsync_016_migration(
    previous_version: Option<&str>,
    current_version: &str,
    migration_state: &str,
) -> bool {
    migration_state != ENHANCED_SUBSYNC_016_MIGRATION_STATE_COMPLETED
        && (is_015_to_016_or_later_upgrade(previous_version, current_version)
            || migration_state == ENHANCED_SUBSYNC_016_MIGRATION_STATE_PENDING)
}

fn parse_bootstrap_bool_token(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "y" | "on" => Some(true),
        "0" | "false" | "no" | "n" | "off" => Some(false),
        _ => None,
    }
}

fn parse_bootstrap_bool(value_json: &str) -> Option<bool> {
    serde_json::from_str::<bool>(value_json)
        .ok()
        .or_else(|| {
            serde_json::from_str::<String>(value_json)
                .ok()
                .as_deref()
                .and_then(parse_bootstrap_bool_token)
        })
        .or_else(|| parse_bootstrap_bool_token(value_json))
}

async fn read_bootstrap_system_bool(
    settings_store: Arc<SettingsStore>,
    key: &str,
    default: bool,
) -> Result<bool, String> {
    let Some(record) = settings_store
        .get_setting_with_defaults(SETTINGS_SCOPE_SYSTEM, key, None)
        .await
        .map_err(|error| error.to_string())?
    else {
        return Ok(default);
    };
    Ok(parse_bootstrap_bool(&record.effective_value_json).unwrap_or(default))
}

async fn read_bootstrap_system_string(
    settings_store: Arc<SettingsStore>,
    key: &str,
    default: &str,
) -> Result<String, String> {
    let Some(record) = settings_store
        .get_setting_with_defaults(SETTINGS_SCOPE_SYSTEM, key, None)
        .await
        .map_err(|error| error.to_string())?
    else {
        return Ok(default.to_string());
    };
    Ok(serde_json::from_str::<String>(&record.effective_value_json)
        .unwrap_or_else(|_| record.effective_value_json.trim().to_string()))
}

async fn set_enhanced_subsync_016_migration_state(
    settings_store: Arc<SettingsStore>,
    state: &str,
) -> Result<(), String> {
    let value_json = serde_json::to_string(state).map_err(|error| error.to_string())?;
    settings_store
        .upsert_setting_value(
            SETTINGS_SCOPE_SYSTEM,
            ENHANCED_SUBSYNC_016_MIGRATION_STATE_KEY,
            None,
            value_json,
            "system",
            None,
        )
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn startup_plugin_migration_actor() -> scryer_domain::User {
    scryer_domain::User {
        id: STARTUP_PLUGIN_MIGRATION_ACTOR_ID.to_string(),
        username: "system".to_string(),
        password_hash: None,
        password_change_required: false,
        account_kind: scryer_domain::UserAccountKind::Local,
        authorization: scryer_domain::UserAuthorization::full_admin(),
    }
}

pub(crate) async fn migrate_enhanced_subsync_plugin_for_016_upgrade(
    app_use_case: &AppUseCase,
    settings_store: Arc<SettingsStore>,
    previous_version: Option<&str>,
    current_version: &str,
) -> Result<(), String> {
    let migration_state = read_bootstrap_system_string(
        settings_store.clone(),
        ENHANCED_SUBSYNC_016_MIGRATION_STATE_KEY,
        ENHANCED_SUBSYNC_016_MIGRATION_STATE_NONE,
    )
    .await?;
    if !should_attempt_enhanced_subsync_016_migration(
        previous_version,
        current_version,
        &migration_state,
    ) {
        return Ok(());
    }

    let subtitles_enabled =
        read_bootstrap_system_bool(settings_store.clone(), "subtitles.enabled", false).await?;
    let subtitle_sync_enabled =
        read_bootstrap_system_bool(settings_store.clone(), "subtitles.sync_enabled", true).await?;
    if !subtitles_enabled || !subtitle_sync_enabled {
        return Ok(());
    }

    if migration_state != ENHANCED_SUBSYNC_016_MIGRATION_STATE_PENDING {
        set_enhanced_subsync_016_migration_state(
            settings_store.clone(),
            ENHANCED_SUBSYNC_016_MIGRATION_STATE_PENDING,
        )
        .await?;
    }

    let actor = startup_plugin_migration_actor();
    app_use_case
        .refresh_plugin_catalog(&actor)
        .await
        .map_err(|error| format!("catalog refresh failed: {error}"))?;

    match app_use_case
        .install_plugin(&actor, ENHANCED_SUBTITLE_SYNC_PLUGIN_ID)
        .await
    {
        Ok(installation) => {
            tracing::info!(
                plugin_id = installation.plugin_id.as_str(),
                version = installation.version.as_str(),
                "installed enhanced subtitle sync plugin during 0.16 upgrade"
            );
            set_enhanced_subsync_016_migration_state(
                settings_store,
                ENHANCED_SUBSYNC_016_MIGRATION_STATE_COMPLETED,
            )
            .await?;
            Ok(())
        }
        Err(error) if error.to_string().contains("already installed") => {
            tracing::info!(
                plugin_id = ENHANCED_SUBTITLE_SYNC_PLUGIN_ID,
                "enhanced subtitle sync plugin already installed; completing migration marker"
            );
            set_enhanced_subsync_016_migration_state(
                settings_store,
                ENHANCED_SUBSYNC_016_MIGRATION_STATE_COMPLETED,
            )
            .await?;
            Ok(())
        }
        Err(error) => Err(format!(
            "enhanced subtitle sync plugin install failed: {error}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enhanced_subsync_migration_only_targets_015_to_016_line() {
        assert!(is_015_to_016_or_later_upgrade(Some("0.15.9"), "0.16.0"));
        assert!(is_015_to_016_or_later_upgrade(Some("v0.15.0"), "0.16.1"));
        assert!(!is_015_to_016_or_later_upgrade(Some("0.14.9"), "0.16.0"));
        assert!(!is_015_to_016_or_later_upgrade(Some("0.16.0"), "0.16.1"));
        assert!(!is_015_to_016_or_later_upgrade(None, "0.16.0"));
    }

    #[test]
    fn enhanced_subsync_migration_pending_state_retries_after_upgrade_boot() {
        assert!(should_attempt_enhanced_subsync_016_migration(
            Some("0.15.9"),
            "0.16.0",
            ENHANCED_SUBSYNC_016_MIGRATION_STATE_NONE,
        ));
        assert!(should_attempt_enhanced_subsync_016_migration(
            None,
            "0.16.0",
            ENHANCED_SUBSYNC_016_MIGRATION_STATE_PENDING,
        ));
        assert!(!should_attempt_enhanced_subsync_016_migration(
            None,
            "0.16.0",
            ENHANCED_SUBSYNC_016_MIGRATION_STATE_COMPLETED,
        ));
        assert!(!should_attempt_enhanced_subsync_016_migration(
            None,
            "0.16.0",
            ENHANCED_SUBSYNC_016_MIGRATION_STATE_NONE,
        ));
    }

    #[test]
    fn bootstrap_bool_accepts_json_and_legacy_string_values() {
        assert_eq!(parse_bootstrap_bool("true"), Some(true));
        assert_eq!(parse_bootstrap_bool("false"), Some(false));
        assert_eq!(parse_bootstrap_bool("\"on\""), Some(true));
        assert_eq!(parse_bootstrap_bool("\"off\""), Some(false));
        assert_eq!(parse_bootstrap_bool("garbage"), None);
    }
}

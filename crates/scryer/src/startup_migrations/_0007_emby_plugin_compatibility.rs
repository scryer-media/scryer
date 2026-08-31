use std::sync::Arc;

use scryer_application::{AppUseCase, SETTINGS_SCOPE_SYSTEM};
use scryer_infrastructure_configuration::settings::settings_store::SettingsStore;

pub(crate) const MIGRATION_STATE_KEY: &str = "startup_migration.emby_plugin_compatibility.state";
const LEGACY_PLUGIN_ID: &str = "mediabrowser";
const CANONICAL_PLUGIN_ID: &str = "emby";
const STATE_NONE: &str = "none";
const STATE_PENDING_INSTALL_ENABLED: &str = "pending_install_enabled";
const STATE_PENDING_INSTALL_DISABLED: &str = "pending_install_disabled";
const STATE_PENDING_EXISTING_ENABLED: &str = "pending_existing_enabled";
const STATE_PENDING_EXISTING_DISABLED: &str = "pending_existing_disabled";
const STATE_COMPLETED: &str = "completed";
const ACTOR_ID: &str = "system-startup-emby-plugin-compatibility";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MigrationState {
    None,
    PendingInstall { enabled: bool },
    PendingExisting { legacy_enabled: bool },
    Completed,
}

impl MigrationState {
    fn parse(value: &str) -> Self {
        match value {
            STATE_PENDING_INSTALL_ENABLED => Self::PendingInstall { enabled: true },
            STATE_PENDING_INSTALL_DISABLED => Self::PendingInstall { enabled: false },
            STATE_PENDING_EXISTING_ENABLED => Self::PendingExisting {
                legacy_enabled: true,
            },
            STATE_PENDING_EXISTING_DISABLED => Self::PendingExisting {
                legacy_enabled: false,
            },
            STATE_COMPLETED => Self::Completed,
            _ => Self::None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::None => STATE_NONE,
            Self::PendingInstall { enabled: true } => STATE_PENDING_INSTALL_ENABLED,
            Self::PendingInstall { enabled: false } => STATE_PENDING_INSTALL_DISABLED,
            Self::PendingExisting {
                legacy_enabled: true,
            } => STATE_PENDING_EXISTING_ENABLED,
            Self::PendingExisting {
                legacy_enabled: false,
            } => STATE_PENDING_EXISTING_DISABLED,
            Self::Completed => STATE_COMPLETED,
        }
    }

    fn initial(legacy_enabled: bool, canonical_exists: bool) -> Self {
        if canonical_exists {
            Self::PendingExisting { legacy_enabled }
        } else {
            Self::PendingInstall {
                enabled: legacy_enabled,
            }
        }
    }

    fn migration_owned_enabled_state(self) -> Option<bool> {
        match self {
            Self::PendingInstall { enabled } => Some(enabled),
            Self::None | Self::PendingExisting { .. } | Self::Completed => None,
        }
    }

    fn legacy_enabled_state(self) -> Option<bool> {
        match self {
            Self::PendingInstall { enabled } => Some(enabled),
            Self::PendingExisting { legacy_enabled } => Some(legacy_enabled),
            Self::None | Self::Completed => None,
        }
    }
}

fn migration_actor() -> scryer_domain::User {
    scryer_domain::User {
        id: ACTOR_ID.to_string(),
        username: "system".to_string(),
        password_hash: None,
        password_change_required: false,
        account_kind: scryer_domain::UserAccountKind::Local,
        authorization: scryer_domain::UserAuthorization::full_admin(),
    }
}

async fn read_state(settings_store: Arc<SettingsStore>) -> Result<MigrationState, String> {
    let value = settings_store
        .get_setting_with_defaults(SETTINGS_SCOPE_SYSTEM, MIGRATION_STATE_KEY, None)
        .await
        .map_err(|error| error.to_string())?
        .and_then(|record| serde_json::from_str::<String>(&record.effective_value_json).ok())
        .unwrap_or_else(|| STATE_NONE.to_string());
    Ok(MigrationState::parse(&value))
}

async fn write_state(
    settings_store: Arc<SettingsStore>,
    state: MigrationState,
) -> Result<(), String> {
    let value_json = serde_json::to_string(state.as_str()).map_err(|error| error.to_string())?;
    settings_store
        .upsert_setting_value(
            SETTINGS_SCOPE_SYSTEM,
            MIGRATION_STATE_KEY,
            None,
            value_json,
            "system",
            None,
        )
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

async fn rollback_new_canonical_installation(app: &AppUseCase, actor: &scryer_domain::User) {
    if let Err(error) = app.uninstall_plugin(actor, CANONICAL_PLUGIN_ID).await {
        tracing::warn!(
            plugin_id = CANONICAL_PLUGIN_ID,
            error = %error,
            "failed to roll back incomplete Emby compatibility installation"
        );
    }
}

async fn restore_legacy_enabled_state(
    app: &AppUseCase,
    actor: &scryer_domain::User,
    enabled: bool,
) {
    let installation = match app.get_plugin_installation(actor, LEGACY_PLUGIN_ID).await {
        Ok(Some(installation)) => installation,
        Ok(None) => return,
        Err(error) => {
            tracing::warn!(error = %error, "failed to inspect legacy Emby plugin during rollback");
            return;
        }
    };
    if installation.is_enabled == enabled {
        return;
    }
    if let Err(error) = app.toggle_plugin(actor, LEGACY_PLUGIN_ID, enabled).await {
        tracing::warn!(error = %error, "failed to restore legacy Emby plugin enabled state");
    }
}

async fn rollback_attempt(
    app: &AppUseCase,
    actor: &scryer_domain::User,
    remove_canonical: bool,
    legacy_enabled: bool,
) {
    if remove_canonical {
        rollback_new_canonical_installation(app, actor).await;
    }
    restore_legacy_enabled_state(app, actor, legacy_enabled).await;
}

pub(crate) async fn migrate_emby_plugin_compatibility(
    app: &AppUseCase,
    settings_store: Arc<SettingsStore>,
) -> Result<(), String> {
    let state = read_state(settings_store.clone()).await?;
    if state == MigrationState::Completed {
        return Ok(());
    }

    let actor = migration_actor();
    let legacy = app
        .get_plugin_installation(&actor, LEGACY_PLUGIN_ID)
        .await
        .map_err(|error| format!("legacy installation lookup failed: {error}"))?;
    let Some(legacy) = legacy else {
        write_state(settings_store, MigrationState::Completed).await?;
        return Ok(());
    };

    let canonical = app
        .get_plugin_installation(&actor, CANONICAL_PLUGIN_ID)
        .await
        .map_err(|error| format!("canonical installation lookup failed: {error}"))?;
    let state = if state == MigrationState::None {
        let state = MigrationState::initial(legacy.is_enabled, canonical.is_some());
        write_state(settings_store.clone(), state).await?;
        state
    } else {
        state
    };

    let migration_owns_canonical = matches!(state, MigrationState::PendingInstall { .. });
    let legacy_enabled = state.legacy_enabled_state().unwrap_or(legacy.is_enabled);

    if matches!(state, MigrationState::PendingExisting { .. }) && canonical.is_none() {
        return Err("operator removed the canonical Emby plugin".to_string());
    }

    if !migration_owns_canonical
        && canonical
            .as_ref()
            .is_some_and(|installation| !installation.is_enabled)
        && legacy_enabled
    {
        return Err("canonical Emby plugin is operator-disabled".to_string());
    }

    if canonical.is_none() {
        if legacy.is_enabled != legacy_enabled {
            restore_legacy_enabled_state(app, &actor, legacy_enabled).await;
        }
        app.refresh_plugin_catalog(&actor)
            .await
            .map_err(|error| format!("catalog refresh failed: {error}"))?;
    }

    let canonical_enabled = state
        .migration_owned_enabled_state()
        .or_else(|| {
            canonical
                .as_ref()
                .map(|installation| installation.is_enabled)
        })
        .unwrap_or(legacy_enabled);
    if canonical_enabled {
        restore_legacy_enabled_state(app, &actor, false).await;
    }

    let mut canonical = match canonical {
        Some(installation) => installation,
        None => match app.install_plugin(&actor, CANONICAL_PLUGIN_ID).await {
            Ok(installation) => installation,
            Err(error) => {
                restore_legacy_enabled_state(app, &actor, legacy_enabled).await;
                return Err(format!(
                    "canonical Emby plugin installation failed: {error}"
                ));
            }
        },
    };

    if let Some(enabled) = state.migration_owned_enabled_state()
        && canonical.is_enabled != enabled
    {
        match app
            .toggle_plugin(&actor, CANONICAL_PLUGIN_ID, enabled)
            .await
        {
            Ok(installation) => canonical = installation,
            Err(error) => {
                rollback_attempt(app, &actor, migration_owns_canonical, legacy_enabled).await;
                return Err(format!("enabled state could not be preserved: {error}"));
            }
        }
    }

    let alias_declared = match app
        .plugin_declares_provider_alias(&actor, CANONICAL_PLUGIN_ID, LEGACY_PLUGIN_ID)
        .await
    {
        Ok(declared) => declared,
        Err(error) => {
            rollback_attempt(app, &actor, migration_owns_canonical, legacy_enabled).await;
            return Err(format!("provider alias verification failed: {error}"));
        }
    };
    let alias_active = !canonical.is_enabled
        || app
            .available_notification_provider_types()
            .iter()
            .any(|provider| provider.eq_ignore_ascii_case(LEGACY_PLUGIN_ID));
    if !alias_declared || !alias_active {
        rollback_attempt(app, &actor, migration_owns_canonical, legacy_enabled).await;
        return Err(format!(
            "legacy provider alias is unavailable (declared={alias_declared}, active={alias_active})"
        ));
    }

    if let Err(error) = app.uninstall_plugin(&actor, LEGACY_PLUGIN_ID).await {
        let uninstall_error = error.to_string();
        match app.get_plugin_installation(&actor, LEGACY_PLUGIN_ID).await {
            Ok(Some(_)) => {
                rollback_attempt(app, &actor, migration_owns_canonical, legacy_enabled).await;
            }
            Ok(None) => {
                tracing::warn!(
                    "legacy plugin row was removed; preserving canonical Emby installation"
                );
                if let Err(reload_error) = app.reload_plugin_providers().await {
                    tracing::warn!(
                        error = %reload_error,
                        "failed to reload canonical Emby provider after partial legacy uninstall"
                    );
                }
            }
            Err(lookup_error) => {
                tracing::warn!(
                    error = %lookup_error,
                    "legacy plugin state is unknown; preserving canonical Emby installation"
                );
            }
        }
        return Err(format!(
            "legacy Emby plugin uninstall failed: {uninstall_error}"
        ));
    }
    write_state(settings_store, MigrationState::Completed).await?;
    tracing::info!(
        legacy_plugin_id = LEGACY_PLUGIN_ID,
        canonical_plugin_id = CANONICAL_PLUGIN_ID,
        "completed Emby plugin compatibility migration"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_only_installations_preserve_enabled_state() {
        assert_eq!(
            MigrationState::initial(true, false),
            MigrationState::PendingInstall { enabled: true }
        );
        assert_eq!(
            MigrationState::initial(false, false),
            MigrationState::PendingInstall { enabled: false }
        );
    }

    #[test]
    fn dual_installations_preserve_the_existing_canonical_state() {
        let state = MigrationState::initial(true, true);
        assert_eq!(
            state,
            MigrationState::PendingExisting {
                legacy_enabled: true
            }
        );
        assert_eq!(state.migration_owned_enabled_state(), None);
        assert_eq!(state.legacy_enabled_state(), Some(true));
    }

    #[test]
    fn pending_install_state_survives_restart_for_retry() {
        for state in [
            MigrationState::PendingInstall { enabled: true },
            MigrationState::PendingInstall { enabled: false },
            MigrationState::PendingExisting {
                legacy_enabled: true,
            },
            MigrationState::PendingExisting {
                legacy_enabled: false,
            },
            MigrationState::Completed,
        ] {
            assert_eq!(MigrationState::parse(state.as_str()), state);
        }
    }
}

use std::collections::BTreeMap;

use crate::{AppError, AppResult, AppUseCase};
use scryer_domain::{AppPermission, ConfigurationChangeAction, PluginInstallation, User};
use scryer_plugin_sdk::{ConfigFieldType, PluginDescriptor};

pub(crate) const PLUGIN_CONFIG_KEY: &str = "plugins.config";

pub type PluginSettingsView = serde_json::Value;

impl AppUseCase {
    async fn plugin_settings_installation(&self, plugin_id: &str) -> AppResult<PluginInstallation> {
        self.services
            .customization
            .plugin_installations
            .get_plugin_installation(plugin_id)
            .await?
            .ok_or_else(|| AppError::NotFound("plugin is not installed".into()))
    }

    pub(crate) async fn stored_plugin_settings(
        &self,
        installation: &PluginInstallation,
    ) -> AppResult<BTreeMap<String, String>> {
        self.read_setting_json_value(PLUGIN_CONFIG_KEY, Some(&installation.id))
            .await
            .map(|values| values.unwrap_or_default())
    }

    pub(crate) async fn runtime_plugin_settings(
        &self,
        installation: &PluginInstallation,
        descriptor: &PluginDescriptor,
    ) -> AppResult<BTreeMap<String, String>> {
        let values = self.stored_plugin_settings(installation).await?;
        validated_effective_settings(descriptor, &values)
    }

    /// Field declarations and nonsecret values. Secret values are write-only.
    pub async fn installed_plugin_settings(
        &self,
        actor: &User,
        plugin_id: &str,
    ) -> AppResult<serde_json::Value> {
        self.require_app_permission(actor, AppPermission::ManageSystemSettings)
            .await?;
        let installation = self.plugin_settings_installation(plugin_id).await?;
        let descriptor = settings_descriptor(&installation)?;
        let values = self.stored_plugin_settings(&installation).await?;
        Ok(settings_view(&descriptor, &values))
    }

    pub async fn update_installed_plugin_settings(
        &self,
        actor: &User,
        plugin_id: &str,
        changes: BTreeMap<String, Option<String>>,
    ) -> AppResult<serde_json::Value> {
        self.require_app_permission(actor, AppPermission::ManageSystemSettings)
            .await?;
        let _settings_guard = self.runtime.plugins.settings_write_lock.lock().await;
        let installation = self.plugin_settings_installation(plugin_id).await?;
        let descriptor = settings_descriptor(&installation)?;
        let mut values = self.stored_plugin_settings(&installation).await?;
        for (key, value) in changes {
            descriptor
                .settings
                .iter()
                .find(|field| field.field.key == key)
                .ok_or_else(|| AppError::Validation("unknown plugin setting".into()))?;
            match value {
                Some(value) => {
                    values.insert(key, value);
                }
                None => {
                    values.remove(&key);
                }
            }
        }
        validated_effective_settings(&descriptor, &values)?;
        self.upsert_scoped_system_setting_json(
            PLUGIN_CONFIG_KEY,
            &installation.id,
            &values,
            Some(actor.id.clone()),
        )
        .await?;
        drop(_settings_guard);
        if installation.is_enabled && !descriptor.settings.is_empty() {
            self.reload_plugin_providers().await?;
        }
        self.emit_configuration_changed_event(
            actor,
            "plugin_settings",
            Some(installation.plugin_id),
            ConfigurationChangeAction::Updated,
        )
        .await;
        Ok(settings_view(&descriptor, &values))
    }

}

fn settings_descriptor(installation: &PluginInstallation) -> AppResult<PluginDescriptor> {
    serde_json::from_str(installation.descriptor_json.as_deref().unwrap_or("{}")).map_err(|_| {
        AppError::Validation("installed plugin settings descriptor is unavailable".into())
    })
}

fn settings_view(
    descriptor: &PluginDescriptor,
    values: &BTreeMap<String, String>,
) -> serde_json::Value {
    let effective = effective_settings(descriptor, values);
    serde_json::json!({
        "pluginId": descriptor.id,
        "fields": descriptor.settings.iter().map(|definition| {
            let sensitive = definition.sensitive || definition.field.field_type == ConfigFieldType::Password;
            let field = &definition.field;
            let (visible, required) = field_conditions(field, &effective);
            let reset_condition = |condition: Option<&scryer_plugin_sdk::FieldCondition>| {
                condition.is_some_and(|condition| {
                    let mut reset = BTreeMap::new();
                    if let Some(value) = descriptor.settings.iter()
                        .find(|setting| setting.field.key == condition.key)
                        .and_then(|setting| setting.field.default_value.clone()) {
                        reset.insert(condition.key.clone(), value);
                    }
                    condition_holds(condition, &reset)
                })
            };
            let mut declaration = serde_json::to_value(definition).expect("serializable plugin setting");
            if sensitive { declaration.as_object_mut().expect("setting declaration").remove("default_value"); }
            serde_json::json!({
                "definition": declaration,
                "sensitive": sensitive,
                "isSet": effective.contains_key(&field.key),
                "hasDefault": field.default_value.is_some(),
                "visible": visible,
                "required": required,
                "visibleWhenReset": reset_condition(field.visible_when.as_ref()),
                "requiredWhenReset": reset_condition(field.required_when.as_ref()),
                "value": if sensitive { None } else { effective.get(&field.key) },
            })
        }).collect::<Vec<_>>()
    })
}

fn effective_settings(
    descriptor: &PluginDescriptor,
    values: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    descriptor
        .settings
        .iter()
        .filter_map(|definition| {
            let field = &definition.field;
            values
                .get(&field.key)
                .or(field.default_value.as_ref())
                .map(|value| (field.key.clone(), value.clone()))
        })
        .collect()
}

fn condition_holds(
    condition: &scryer_plugin_sdk::FieldCondition,
    values: &BTreeMap<String, String>,
) -> bool {
    use scryer_domain::ConditionOp as Domain;
    use scryer_plugin_sdk::ConditionOp as Sdk;
    scryer_domain::FieldCondition {
        key: condition.key.clone(),
        op: match condition.op {
            Sdk::Eq => Domain::Eq,
            Sdk::Ne => Domain::Ne,
            Sdk::In => Domain::In,
            Sdk::NotIn => Domain::NotIn,
            Sdk::NonEmpty => Domain::NonEmpty,
        },
        values: condition.values.clone(),
    }
    .holds(values.get(&condition.key).map(String::as_str))
}

fn field_conditions(
    field: &scryer_plugin_sdk::ConfigFieldDef,
    values: &BTreeMap<String, String>,
) -> (bool, bool) {
    let visible = field
        .visible_when
        .as_ref()
        .is_none_or(|condition| condition_holds(condition, values));
    let required = visible
        && (field.required
            || field
                .required_when
                .as_ref()
                .is_some_and(|condition| condition_holds(condition, values)));
    (visible, required)
}

fn validated_effective_settings(
    descriptor: &PluginDescriptor,
    values: &BTreeMap<String, String>,
) -> AppResult<BTreeMap<String, String>> {
    let effective = effective_settings(descriptor, values);
    for definition in &descriptor.settings {
        let field = &definition.field;
        let (_, required) = field_conditions(field, &effective);
        if required
            && effective
                .get(&field.key)
                .is_none_or(|value| value.trim().is_empty())
        {
            return Err(AppError::Validation(
                "required plugin setting is missing".into(),
            ));
        }
        if let Some(value) = effective.get(&field.key) {
            let valid = match field.field_type {
                ConfigFieldType::Bool => matches!(value.as_str(), "true" | "false"),
                ConfigFieldType::Number => value.parse::<f64>().is_ok_and(f64::is_finite),
                ConfigFieldType::Select | ConfigFieldType::FilteredSelect => {
                    field.options.iter().any(|option| option.value == *value)
                }
                _ => true,
            };
            if !valid {
                return Err(AppError::Validation("invalid plugin setting value".into()));
            }
        }
    }
    Ok(effective)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor(settings: serde_json::Value) -> PluginDescriptor {
        serde_json::from_value(serde_json::json!({
            "id": "synthetic.settings", "name": "Synthetic settings", "version": "1.0.0", "sdk_version": "3.0.0",
            "provider": {"kind": "indexer", "provider_type": "synthetic"}, "settings": settings,
        })).unwrap()
    }

    #[test]
    fn plugin_settings_validate_effective_defaults_and_conditional_requirements() {
        let descriptor = descriptor(serde_json::json!([
            {"key":"enabled", "label":"Enabled", "field_type":"bool", "required":true, "default_value":"false"},
            {"key":"required_when_enabled", "label":"Value", "field_type":"string", "required_when":{"key":"enabled","op":"eq","values":["true"]}},
            {"key":"hidden_required", "label":"Hidden", "field_type":"string", "required":true, "visible_when":{"key":"enabled","op":"eq","values":["true"]}}
        ]));
        assert_eq!(
            validated_effective_settings(&descriptor, &BTreeMap::new()).unwrap()["enabled"],
            "false"
        );
        let mut values = BTreeMap::from([("enabled".into(), "true".into())]);
        assert!(validated_effective_settings(&descriptor, &values).is_err());
        values.insert("required_when_enabled".into(), "synthetic".into());
        assert!(validated_effective_settings(&descriptor, &values).is_err());
        values.insert("hidden_required".into(), "synthetic".into());
        assert!(validated_effective_settings(&descriptor, &values).is_ok());
        let view = settings_view(&descriptor, &BTreeMap::new());
        assert_eq!(view["fields"][0]["value"], "false");
        assert_eq!(view["fields"][1]["required"], false);
        assert_eq!(view["fields"][2]["visible"], false);
    }

    #[test]
    fn plugin_settings_reject_invalid_defaults_and_redact_sensitive_default_metadata() {
        for (field_type, value) in [
            ("bool", "yes"),
            ("number", "NaN"),
            ("select", "missing-option"),
        ] {
            let descriptor = descriptor(serde_json::json!([
                {"key":"value", "label":"Value", "field_type":field_type, "default_value":value}
            ]));
            assert!(validated_effective_settings(&descriptor, &BTreeMap::new()).is_err());
        }
        let descriptor = descriptor(serde_json::json!([
            {"key":"password", "label":"Password", "field_type":"multiline", "sensitive":true, "default_value":"synthetic-secret"}
        ]));
        let view = settings_view(&descriptor, &BTreeMap::new());
        assert!(!view.to_string().contains("synthetic-secret"));
        assert_eq!(view["fields"][0]["isSet"], true);
        assert_eq!(view["fields"][0]["hasDefault"], true);
    }

    #[test]
    fn sensitive_default_reset_conditions_do_not_reuse_override_state() {
        let descriptor = descriptor(serde_json::json!([
            {"key":"secret", "label":"Secret", "field_type":"password", "default_value":"synthetic-default"},
            {"key":"dependent", "label":"Dependent", "field_type":"string",
             "visible_when":{"key":"secret", "op":"ne", "values":["synthetic-override"]},
             "required_when":{"key":"secret", "op":"ne", "values":["synthetic-override"]}}
        ]));
        let values = BTreeMap::from([("secret".into(), "synthetic-override".into())]);
        let view = settings_view(&descriptor, &values);
        assert_eq!(view["fields"][1]["visible"], false);
        assert_eq!(view["fields"][1]["required"], false);
        assert_eq!(view["fields"][1]["visibleWhenReset"], true);
        assert_eq!(view["fields"][1]["requiredWhenReset"], true);
        assert!(!view.to_string().contains("synthetic-default"));
    }
}

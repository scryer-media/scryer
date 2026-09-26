//! Server-wide settings for list providers.
//!
//! A list provider may declare config fields that belong to the whole server,
//! such as an instance API key. They are stored as one encrypted system
//! setting per provider, keyed by the provider type, and handed to the
//! provider on every fetch and preview. Only fields the provider declares and
//! the operator supplies are stored here: host-bound values come from the host
//! at load time, and a member's credential stays on that member's linked
//! account.

use std::collections::BTreeMap;

use scryer_domain::{AppPermission, ConfigurationChangeAction, User};
use scryer_plugin_sdk::{
    ConfigFieldDef, ConfigFieldType, ConfigFieldValueSource, PluginDescriptor,
};

use super::plugin::ListPluginProvider;
use crate::{AppError, AppResult, AppUseCase};

/// The sensitive system setting that holds each provider's values, scoped by
/// provider type.
pub(crate) const LIST_PROVIDER_CONFIG_KEY: &str = "lists.provider_config";

/// Every provider's server-wide values, keyed by provider type.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ListProviderConfigs(BTreeMap<String, BTreeMap<String, String>>);

impl ListProviderConfigs {
    pub fn insert(&mut self, provider_type: &str, values: BTreeMap<String, String>) {
        self.0.insert(provider_type.to_ascii_lowercase(), values);
    }

    /// The values for `provider`, which may be the provider type or one of
    /// its aliases. Empty when nothing is stored.
    pub fn for_provider(
        &self,
        plugins: &dyn ListPluginProvider,
        provider: &str,
    ) -> BTreeMap<String, String> {
        let key = list_descriptor_for(plugins, provider)
            .and_then(|descriptor| provider_type_of(&descriptor))
            .unwrap_or_else(|| provider.to_ascii_lowercase());
        self.0.get(&key).cloned().unwrap_or_default()
    }
}

/// One server-wide field as an operator sees it. A secret's value is never
/// returned; `is_set` says whether one is stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListProviderSettingField {
    pub key: String,
    pub label: String,
    pub help_text: Option<String>,
    pub field_type: scryer_domain::ConfigFieldType,
    pub required: bool,
    pub secret: bool,
    pub is_set: bool,
    pub value: Option<String>,
}

/// A provider's server-wide fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListProviderSettings {
    pub provider_type: String,
    pub fields: Vec<ListProviderSettingField>,
}

fn list_descriptor_for(
    plugins: &dyn ListPluginProvider,
    provider: &str,
) -> Option<PluginDescriptor> {
    plugins.descriptors().into_iter().find(|descriptor| {
        descriptor.list_provider().is_some_and(|list| {
            list.provider_type.eq_ignore_ascii_case(provider)
                || list
                    .provider_aliases
                    .iter()
                    .any(|alias| alias.eq_ignore_ascii_case(provider))
        })
    })
}

fn provider_type_of(descriptor: &PluginDescriptor) -> Option<String> {
    descriptor
        .list_provider()
        .map(|list| list.provider_type.to_ascii_lowercase())
}

/// The fields an operator may set for the whole server.
fn server_fields(descriptor: &PluginDescriptor) -> Vec<&ConfigFieldDef> {
    descriptor
        .config_fields()
        .iter()
        .filter(|field| field.value_source == ConfigFieldValueSource::User)
        .collect()
}

fn is_secret(field: &ConfigFieldDef) -> bool {
    matches!(field.field_type, ConfigFieldType::Password)
}

fn domain_field_type(value: ConfigFieldType) -> scryer_domain::ConfigFieldType {
    use scryer_domain::ConfigFieldType as Domain;
    match value {
        ConfigFieldType::String => Domain::String,
        ConfigFieldType::Password => Domain::Password,
        ConfigFieldType::Multiline => Domain::Multiline,
        ConfigFieldType::Bool => Domain::Bool,
        ConfigFieldType::Select => Domain::Select,
        ConfigFieldType::FilteredSelect => Domain::FilteredSelect,
        ConfigFieldType::Number => Domain::Number,
        ConfigFieldType::Path => Domain::Path,
        ConfigFieldType::Tag => Domain::Tag,
    }
}

fn field_view(field: &ConfigFieldDef, stored_value: Option<&String>) -> ListProviderSettingField {
    let secret = is_secret(field);
    ListProviderSettingField {
        key: field.key.clone(),
        label: field.label.clone(),
        help_text: field.help_text.clone(),
        field_type: domain_field_type(field.field_type),
        required: field.required,
        secret,
        is_set: stored_value.is_some(),
        value: if secret { None } else { stored_value.cloned() },
    }
}

/// A provider's server-wide fields with nothing stored: the shape the
/// provider catalog shows before stored values are known.
pub(crate) fn declared_server_fields(
    descriptor: &PluginDescriptor,
) -> Vec<ListProviderSettingField> {
    server_fields(descriptor)
        .into_iter()
        .map(|field| field_view(field, None))
        .collect()
}

fn settings_view(
    provider_type: String,
    descriptor: &PluginDescriptor,
    stored: &BTreeMap<String, String>,
) -> ListProviderSettings {
    let fields = server_fields(descriptor)
        .into_iter()
        .map(|field| field_view(field, stored.get(&field.key)))
        .collect();
    ListProviderSettings {
        provider_type,
        fields,
    }
}

/// Apply `changes` to `stored`. A key left out keeps its value; a blank or
/// `None` value clears it. Keys the provider does not declare as a server-wide
/// field are refused.
pub(crate) fn merge_provider_settings(
    descriptor: &PluginDescriptor,
    stored: &BTreeMap<String, String>,
    changes: &BTreeMap<String, Option<String>>,
) -> AppResult<BTreeMap<String, String>> {
    let fields = server_fields(descriptor);
    let mut merged = stored
        .iter()
        .filter(|(key, _)| fields.iter().any(|field| &field.key == *key))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect::<BTreeMap<_, _>>();
    for (key, value) in changes {
        if !fields.iter().any(|field| &field.key == key) {
            return Err(AppError::Validation(format!(
                "'{key}' is not a server-wide setting of this list provider"
            )));
        }
        match value
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            Some(value) => {
                merged.insert(key.clone(), value.to_string());
            }
            None => {
                merged.remove(key);
            }
        }
    }
    Ok(merged)
}

impl AppUseCase {
    pub(crate) async fn stored_list_provider_config(
        &self,
        provider_type: &str,
    ) -> AppResult<BTreeMap<String, String>> {
        Ok(self
            .read_setting_json_value::<BTreeMap<String, String>>(
                LIST_PROVIDER_CONFIG_KEY,
                Some(provider_type),
            )
            .await?
            .unwrap_or_default())
    }

    /// Every installed provider's stored values. A provider whose values
    /// cannot be read gets none, so its fetch reports the missing key itself.
    pub(crate) async fn load_list_provider_configs(&self) -> ListProviderConfigs {
        let mut configs = ListProviderConfigs::default();
        for descriptor in self.services.lists.plugins.descriptors() {
            let Some(provider_type) = provider_type_of(&descriptor) else {
                continue;
            };
            if server_fields(&descriptor).is_empty() {
                continue;
            }
            match self.stored_list_provider_config(&provider_type).await {
                Ok(values) => configs.insert(&provider_type, values),
                Err(error) => tracing::warn!(
                    provider_type = provider_type.as_str(),
                    error = %error,
                    "could not read a list provider's server-wide settings"
                ),
            }
        }
        configs
    }

    /// One provider's stored values, for a single fetch such as a preview.
    pub(crate) async fn list_provider_config_for(
        &self,
        provider: &str,
    ) -> BTreeMap<String, String> {
        let Some(provider_type) =
            list_descriptor_for(self.services.lists.plugins.as_ref(), provider)
                .and_then(|descriptor| provider_type_of(&descriptor))
        else {
            return BTreeMap::new();
        };
        match self.stored_list_provider_config(&provider_type).await {
            Ok(values) => values,
            Err(error) => {
                tracing::warn!(
                    provider_type = provider_type.as_str(),
                    error = %error,
                    "could not read a list provider's server-wide settings"
                );
                BTreeMap::new()
            }
        }
    }

    /// The server-wide fields of every installed provider that declares any.
    pub async fn list_provider_settings(
        &self,
        actor: &User,
    ) -> AppResult<Vec<ListProviderSettings>> {
        self.require_app_permission(actor, AppPermission::ManageLists)
            .await?;
        let mut settings = Vec::new();
        for descriptor in self.services.lists.plugins.descriptors() {
            let Some(provider_type) = provider_type_of(&descriptor) else {
                continue;
            };
            if server_fields(&descriptor).is_empty() {
                continue;
            }
            let stored = self.stored_list_provider_config(&provider_type).await?;
            settings.push(settings_view(provider_type, &descriptor, &stored));
        }
        settings.sort_by(|left, right| left.provider_type.cmp(&right.provider_type));
        Ok(settings)
    }

    /// Change a provider's server-wide values. See [`merge_provider_settings`]
    /// for how `changes` apply.
    pub async fn update_list_provider_settings(
        &self,
        actor: &User,
        provider: &str,
        changes: BTreeMap<String, Option<String>>,
    ) -> AppResult<ListProviderSettings> {
        self.require_app_permission(actor, AppPermission::ManageLists)
            .await?;
        let descriptor = list_descriptor_for(self.services.lists.plugins.as_ref(), provider)
            .ok_or_else(|| AppError::NotFound(format!("list provider '{provider}'")))?;
        let provider_type = provider_type_of(&descriptor)
            .ok_or_else(|| AppError::NotFound(format!("list provider '{provider}'")))?;
        let stored = self.stored_list_provider_config(&provider_type).await?;
        let merged = merge_provider_settings(&descriptor, &stored, &changes)?;
        self.upsert_scoped_system_setting_json(
            LIST_PROVIDER_CONFIG_KEY,
            &provider_type,
            &merged,
            Some(actor.id.clone()),
        )
        .await?;
        self.emit_configuration_changed_event(
            actor,
            "list_provider_settings",
            Some(provider_type.clone()),
            ConfigurationChangeAction::Updated,
        )
        .await;
        Ok(settings_view(provider_type, &descriptor, &merged))
    }
}

#[cfg(test)]
#[path = "provider_settings_tests.rs"]
mod tests;

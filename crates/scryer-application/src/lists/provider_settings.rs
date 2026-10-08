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

/// The config key a list provider reads its OAuth app client id from.
pub(crate) const CLIENT_ID_CONFIG_KEY: &str = "client_id";

/// List providers SMG may issue a public OAuth client id for at enrollment.
/// Each has a declared [`gateway_list_client_id_setting_key`] setting.
pub const GATEWAY_LIST_CLIENT_ID_PROVIDERS: [&str; 4] = ["anilist", "mal", "simkl", "trakt"];

/// The system setting holding the public OAuth client id SMG issued for
/// `provider_type` at enrollment, such as `lists.trakt.client_id`.
pub fn gateway_list_client_id_setting_key(provider_type: &str) -> String {
    format!(
        "lists.{}.{CLIENT_ID_CONFIG_KEY}",
        provider_type.trim().to_ascii_lowercase()
    )
}

/// `values` with the gateway-issued client id under `client_id`, unless the
/// operator already set one. A blank gateway id adds nothing.
pub(crate) fn with_gateway_client_id(
    mut values: BTreeMap<String, String>,
    gateway_client_id: Option<&str>,
) -> BTreeMap<String, String> {
    let Some(id) = gateway_client_id.map(str::trim).filter(|id| !id.is_empty()) else {
        return values;
    };
    let operator_set = values
        .get(CLIENT_ID_CONFIG_KEY)
        .is_some_and(|value| !value.trim().is_empty());
    if !operator_set {
        values.insert(CLIENT_ID_CONFIG_KEY.to_string(), id.to_string());
    }
    values
}

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
    /// The choices of a select field, in the provider's order.
    pub options: Vec<scryer_domain::ConfigFieldOption>,
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
        options: field
            .options
            .iter()
            .map(|option| scryer_domain::ConfigFieldOption {
                value: option.value.clone(),
                label: option.label.clone(),
                config_overrides: option.config_overrides.clone(),
            })
            .collect(),
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

    /// The public OAuth client id SMG issued for `provider_type`, if any.
    async fn gateway_list_client_id(&self, provider_type: &str) -> Option<String> {
        let key = gateway_list_client_id_setting_key(provider_type);
        match self.read_setting_string_value(&key, None).await {
            Ok(value) => value.filter(|value| !value.trim().is_empty()),
            Err(error) => {
                tracing::warn!(
                    provider_type,
                    error = %error,
                    "could not read a list provider's gateway client id"
                );
                None
            }
        }
    }

    /// A provider's operator-entered values, with the gateway client id
    /// filled in where the operator set none.
    async fn provider_config_with_gateway_client_id(
        &self,
        provider_type: &str,
        has_server_fields: bool,
    ) -> BTreeMap<String, String> {
        let stored = if has_server_fields {
            match self.stored_list_provider_config(provider_type).await {
                Ok(values) => values,
                Err(error) => {
                    tracing::warn!(
                        provider_type,
                        error = %error,
                        "could not read a list provider's server-wide settings"
                    );
                    BTreeMap::new()
                }
            }
        } else {
            BTreeMap::new()
        };
        let gateway_id = self.gateway_list_client_id(provider_type).await;
        with_gateway_client_id(stored, gateway_id.as_deref())
    }

    /// Every installed provider's stored values. A provider whose values
    /// cannot be read gets none, so its fetch reports the missing key itself.
    pub(crate) async fn load_list_provider_configs(&self) -> ListProviderConfigs {
        let mut configs = ListProviderConfigs::default();
        for descriptor in self.services.lists.plugins.descriptors() {
            let Some(provider_type) = provider_type_of(&descriptor) else {
                continue;
            };
            let has_server_fields = !server_fields(&descriptor).is_empty();
            let values = self
                .provider_config_with_gateway_client_id(&provider_type, has_server_fields)
                .await;
            if has_server_fields || !values.is_empty() {
                configs.insert(&provider_type, values);
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
        self.provider_config_with_gateway_client_id(&provider_type, true)
            .await
    }

    /// The server-wide fields of every installed provider that declares any.
    pub async fn list_provider_settings(
        &self,
        actor: &User,
    ) -> AppResult<Vec<ListProviderSettings>> {
        self.require_app_permission(actor, AppPermission::ManageLists)
            .await?;
        self.require_lists_enabled().await?;
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
        self.require_lists_enabled().await?;
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

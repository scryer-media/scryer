use super::{CatalogRefreshStateValue, Long, MediaFacetValue};
use async_graphql::{Enum, ID, InputObject, SimpleObject};
use chrono::{DateTime, Utc};
use scryer_domain::{
    ConditionOp, ConfigFieldRole, ConfigFieldType, ConfigFieldValueSource, FieldCondition,
};

// ── Plugins ────────────────────────────────────────────────────────────────

#[derive(SimpleObject, Clone)]
/// Plugin registry entry with trust, installation, update, and progress state.
pub struct RegistryPluginPayload {
    /// Registry plugin ID.
    pub id: ID,
    /// Plugin name.
    pub name: String,
    /// Plugin description.
    pub description: String,
    /// Registry version.
    pub version: String,
    /// Latest known version, or null when unavailable.
    pub latest_version: Option<String>,
    /// Registry classification of the plugin.
    pub plugin_type: String,
    /// Provider type exposed by the plugin.
    pub provider_type: String,
    /// Publisher or author name.
    pub author: String,
    /// Whether the plugin is official.
    pub official: bool,
    /// Publisher identity, or null when unavailable.
    pub publisher: Option<String>,
    /// Trust or support tier.
    pub support_tier: String,
    /// Current registry or installation status, or null when unavailable.
    pub status: Option<String>,
    /// Documentation URL, or null when unavailable.
    pub docs_url: Option<String>,
    /// Source repository URL, or null when unavailable.
    pub source_repo: Option<String>,
    /// Whether the plugin is built into the service.
    pub builtin: bool,
    /// Download source URL, or null when unavailable.
    pub source_url: Option<String>,
    /// Source kind, or null when unavailable.
    pub source_kind: Option<String>,
    /// Trust-block reason, or null when not blocked.
    pub blocked_reason: Option<String>,
    /// Artifact size in bytes, or null when unavailable.
    pub bytes: Option<Long>,
    /// Whether the plugin is installed.
    pub is_installed: bool,
    /// Whether the installed plugin is enabled.
    pub is_enabled: bool,
    /// Installed version, or null when not installed.
    pub installed_version: Option<String>,
    /// Whether a newer version is available.
    pub update_available: bool,
    /// Whether install or update work is currently running.
    pub install_in_progress: bool,
    /// Default provider base URL, or null when not applicable.
    pub default_base_url: Option<String>,
}

// ── Rule Packs ────────────────────────────────────────────────────────────

#[derive(SimpleObject, Clone)]
/// Rule-pack registry entry.
pub struct RulePackRegistryEntryPayload {
    /// Registry rule-pack ID.
    pub id: String,
    /// Rule-pack name.
    pub name: String,
    /// Rule-pack description.
    pub description: String,
    /// Rule-pack author.
    pub author: String,
    /// Rule-pack version.
    pub version: String,
}

#[derive(SimpleObject, Clone)]
/// Template supplied by a rule pack.
pub struct RulePackTemplatePayload {
    /// Template ID.
    pub id: String,
    /// Template title.
    pub title: String,
    /// Template description.
    pub description: String,
    /// Template category.
    pub category: String,
    /// Rego source for the template.
    pub rego_source: String,
    /// Facets to which the template applies.
    pub applied_facets: Vec<String>,
}

#[derive(SimpleObject, Clone)]
/// Installed plugin identity, trust metadata, artifact digests, and timestamps.
pub struct PluginInstallationPayload {
    /// Installation record ID.
    pub id: ID,
    /// Registry plugin ID.
    pub plugin_id: ID,
    /// Installed plugin name.
    pub name: String,
    /// Installed plugin description.
    pub description: String,
    /// Installed plugin version.
    pub version: String,
    /// Plugin SDK version.
    pub sdk_version: String,
    /// SDK compatibility constraint.
    pub sdk_constraint: String,
    /// Manifest classification of the installed plugin.
    pub plugin_type: String,
    /// Provider type exposed by the plugin.
    pub provider_type: String,
    /// Whether the plugin is enabled.
    pub is_enabled: bool,
    /// Whether the plugin is built in.
    pub is_builtin: bool,
    /// Installation source kind.
    pub source_kind: String,
    /// Installation source URL, or null when unavailable.
    pub source_url: Option<String>,
    /// Publisher identity, or null when unavailable.
    pub publisher: Option<String>,
    /// Trust or support tier.
    pub support_tier: String,
    /// Documentation URL, or null when unavailable.
    pub docs_url: Option<String>,
    /// Source repository URL, or null when unavailable.
    pub source_repo: Option<String>,
    /// Manifest URL, or null when unavailable.
    pub manifest_url: Option<String>,
    /// Verified WASM digest, or null when unavailable.
    pub wasm_digest: Option<String>,
    /// Verified artifact digest, or null when unavailable.
    pub artifact_digest: Option<String>,
    /// UTC installation time.
    pub installed_at: DateTime<Utc>,
    /// UTC last-update time.
    pub updated_at: DateTime<Utc>,
}

#[derive(SimpleObject)]
/// Plugin catalog refresh state, trust warnings, and blocked actions.
pub struct PluginCatalogStatusPayload {
    /// Catalog refresh lifecycle state.
    pub refresh_state: CatalogRefreshStateValue,
    /// Whether the remote catalog source is reachable.
    pub github_available: bool,
    /// UTC time of the last catalog check, or null when never checked.
    pub last_checked_at: Option<DateTime<Utc>>,
    /// Current outage message, or null when no outage is reported.
    pub outage_message: Option<String>,
    /// Actions blocked by trust or catalog state.
    pub blocked_actions: Vec<String>,
    /// Restore warnings that require operator attention.
    pub restore_warnings: Vec<String>,
    /// Last catalog error, or null when none is recorded.
    pub last_error: Option<String>,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Background plugin installation operation kind.
pub enum PluginInstallOperationKindValue {
    /// New plugin installation.
    Install,
    /// Existing plugin upgrade.
    Upgrade,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Lifecycle state of a plugin installation operation.
pub enum PluginInstallStateValue {
    /// Artifact is downloading.
    Downloading,
    /// Artifact signatures or digests are being verified.
    Verifying,
    /// Artifact is being installed.
    Installing,
    /// Installation completed successfully.
    Succeeded,
    /// Installation failed.
    Failed,
}

#[derive(SimpleObject, Clone)]
/// Progress snapshot for a background plugin installation or upgrade.
pub struct PluginInstallProgressPayload {
    /// Plugin ID being installed or upgraded.
    pub plugin_id: ID,
    /// Installation operation kind.
    pub operation_kind: PluginInstallOperationKindValue,
    /// Current installation state.
    pub state: PluginInstallStateValue,
    /// Current progress label.
    pub label: String,
    /// Current step number, from 1 through the total step count.
    pub step_index: i32,
    /// Total step count.
    pub step_count: i32,
    /// Progress message, or null when none is available.
    pub message: Option<String>,
    /// Error message, or null when the operation has not failed.
    pub error: Option<String>,
}

#[derive(SimpleObject)]
/// Preview of a manually supplied plugin repository.
pub struct ManualPluginPreviewPayload {
    /// GitHub repository URL used for the preview.
    pub github_repo_url: String,
    /// Resolved registry metadata.
    pub plugin: RegistryPluginPayload,
}

#[derive(InputObject)]
/// GitHub repository source for a manual plugin preview or install.
pub struct ManualPluginRepoInput {
    /// GitHub repository URL.
    pub github_repo_url: String,
}

#[derive(InputObject)]
/// Uploaded plugin artifact and explicit risk acknowledgement.
pub struct ManualPluginUploadInput {
    /// Uploaded file name.
    pub file_name: String,
    /// Base64-encoded WASM artifact.
    pub wasm_base64: String,
    /// Must be true to acknowledge manual artifact risk.
    pub acknowledge_risk: bool,
}

#[derive(SimpleObject, Clone)]
/// Identifier returned after uninstalling a plugin.
pub struct UninstallPluginPayload {
    /// Uninstalled plugin ID.
    pub plugin_id: async_graphql::ID,
}

#[derive(InputObject)]
/// Enables or disables one installed plugin.
pub struct TogglePluginInput {
    /// ID of the installed plugin to enable or disable.
    pub plugin_id: ID,
    /// Desired enabled state.
    pub enabled: bool,
}

// ── Provider Type Config Schema ─────────────────────────────────────────

#[derive(SimpleObject, Clone)]
/// One field value supplied by a selected plugin configuration preset.
pub struct PluginConfigFieldOverridePayload {
    /// Configuration field key to populate.
    pub key: String,
    /// Configuration field value to populate.
    pub value: String,
}

#[derive(SimpleObject, Clone)]
/// One selectable value for a plugin configuration field.
pub struct PluginConfigFieldOptionPayload {
    /// Machine-readable option value.
    pub value: String,
    /// Display label for the option.
    pub label: String,
    /// Other configuration fields populated when this option is selected.
    pub config_overrides: Vec<PluginConfigFieldOverridePayload>,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Plugin configuration field rendering and validation type.
pub enum PluginConfigFieldTypeValue {
    /// Single-line string.
    String,
    /// Secret or password string.
    Password,
    /// Multiline text.
    Multiline,
    /// Boolean value.
    Bool,
    /// Enumerated selection.
    Select,
    /// Enumerated selection rendered with a filter box.
    FilteredSelect,
    /// Numeric value.
    Number,
    /// Filesystem path.
    Path,
    /// Tag value.
    Tag,
}

impl PluginConfigFieldTypeValue {
    pub fn from_domain(value: ConfigFieldType) -> Self {
        match value {
            ConfigFieldType::String => Self::String,
            ConfigFieldType::Password => Self::Password,
            ConfigFieldType::Multiline => Self::Multiline,
            ConfigFieldType::Bool => Self::Bool,
            ConfigFieldType::Select => Self::Select,
            ConfigFieldType::FilteredSelect => Self::FilteredSelect,
            ConfigFieldType::Number => Self::Number,
            ConfigFieldType::Path => Self::Path,
            ConfigFieldType::Tag => Self::Tag,
        }
    }
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Source of a provider configuration value.
pub enum PluginConfigValueSourceValue {
    /// Value supplied by the user.
    User,
    /// Value supplied by a host binding.
    HostBinding,
}

impl PluginConfigValueSourceValue {
    pub fn from_domain(value: ConfigFieldValueSource) -> Self {
        match value {
            ConfigFieldValueSource::User => Self::User,
            ConfigFieldValueSource::HostBinding => Self::HostBinding,
        }
    }
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Semantic role assigned to a provider configuration field.
pub enum PluginConfigFieldRoleValue {
    /// Field contains a connection URL.
    ConnectionUrl,
}

impl PluginConfigFieldRoleValue {
    pub fn from_domain(value: ConfigFieldRole) -> Self {
        match value {
            ConfigFieldRole::ConnectionUrl => Self::ConnectionUrl,
        }
    }
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Comparison a provider configuration field condition applies.
pub enum PluginConditionOpValue {
    /// Referenced value equals the first condition value.
    Eq,
    /// Referenced value differs from the first condition value.
    Ne,
    /// Referenced value is one of the condition values.
    In,
    /// Referenced value is none of the condition values.
    NotIn,
    /// Referenced value is non-blank.
    NonEmpty,
}

impl PluginConditionOpValue {
    pub fn from_domain(value: ConditionOp) -> Self {
        match value {
            ConditionOp::Eq => Self::Eq,
            ConditionOp::Ne => Self::Ne,
            ConditionOp::In => Self::In,
            ConditionOp::NotIn => Self::NotIn,
            ConditionOp::NonEmpty => Self::NonEmpty,
        }
    }
}

#[derive(SimpleObject, Clone)]
/// Predicate over another configuration field's current value.
pub struct PluginFieldConditionPayload {
    /// Key of the field whose value is tested.
    pub key: String,
    /// Comparison applied to that field's value.
    pub op: PluginConditionOpValue,
    /// Values compared against; empty for NON_EMPTY.
    pub values: Vec<String>,
}

impl PluginFieldConditionPayload {
    pub fn from_domain(condition: FieldCondition) -> Self {
        Self {
            key: condition.key,
            op: PluginConditionOpValue::from_domain(condition.op),
            values: condition.values,
        }
    }
}

#[derive(SimpleObject, Clone)]
/// Provider configuration field schema.
pub struct PluginConfigFieldPayload {
    /// Stable field key.
    pub key: String,
    /// Human-readable field label.
    pub label: String,
    /// Value type that controls field validation and presentation.
    pub field_type: PluginConfigFieldTypeValue,
    /// Whether a value is required.
    pub required: bool,
    /// Default value, or null when none is defined.
    pub default_value: Option<String>,
    /// Source of the effective value.
    pub value_source: PluginConfigValueSourceValue,
    /// Optional semantic role.
    pub role: Option<PluginConfigFieldRoleValue>,
    /// Host binding name, or null when not applicable.
    pub host_binding: Option<String>,
    /// Enumerated options, empty for non-select fields.
    pub options: Vec<PluginConfigFieldOptionPayload>,
    /// Help text, or null when unavailable.
    pub help_text: Option<String>,
    /// Condition gating visibility, or null when always shown.
    pub visible_when: Option<PluginFieldConditionPayload>,
    /// Condition that makes the field required, or null when it never applies.
    pub required_when: Option<PluginFieldConditionPayload>,
    /// Whether the field belongs behind the form's advanced disclosure.
    pub advanced: bool,
}

#[derive(SimpleObject, Clone)]
/// Provider type metadata and configuration schema.
pub struct ProviderTypePayload {
    /// Provider implementation identifier.
    pub provider_type: String,
    /// Provider display name.
    pub name: String,
    /// Configuration field schemas.
    pub config_fields: Vec<PluginConfigFieldPayload>,
    /// Default base URL, or null when not applicable.
    pub default_base_url: Option<String>,
    /// Host binding names accepted as configuration value sources.
    pub available_host_bindings: Vec<String>,
    /// Recommended media facets.
    pub recommended_facets: Vec<MediaFacetValue>,
    /// Supported notification events.
    pub supported_events: Vec<String>,
    /// Whether a connection test is supported.
    pub supports_test: bool,
}

#[derive(SimpleObject, Clone)]
/// Provider connection validation result.
pub struct ProviderValidationPayload {
    /// Machine-readable validation status.
    pub status: String,
    /// Diagnostic message, or null when unavailable.
    pub message: Option<String>,
    /// Retry delay in seconds, or null when no retry delay applies.
    pub retry_after_seconds: Option<i64>,
}

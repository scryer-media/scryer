use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

#[cfg(all(not(target_arch = "wasm32"), feature = "host-runtime"))]
use extism::{Function, Manifest, PluginBuilder, UserData, ValType, Wasm, host_fn};
use schemars::{JsonSchema, schema_for};
use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize, Serializer};

pub mod http;
pub mod indexer;
pub mod net;
pub mod notification;
pub mod torrent;
pub use indexer::{
    IndexerCategoryDescriptor, IndexerCategoryModel, IndexerCategoryValueKind, IndexerFeedMode,
    IndexerLimitCapabilities, IndexerProtocol, IndexerResponseFeatures, IndexerSearchInput,
    IndexerTorrentCapabilities, PluginSearchContext, PluginSearchOrigin, PluginSearchQueryKind,
    PluginSearchRequestKind, PluginSearchSubjectKind, derive_indexer_flags,
    indexer_capability_fixtures, normalize_external_id_key, normalize_external_ids,
    normalize_info_hash as normalize_indexer_info_hash, torrent_result, usenet_result,
};
pub use net::{
    SocketCloseRequest, SocketCloseResponse, SocketError, SocketErrorCode, SocketOpenRequest,
    SocketOpenResponse, SocketPermission, SocketReadRequest, SocketReadResponse, SocketResponse,
    SocketStartTlsRequest, SocketStartTlsResponse, SocketTlsMode, SocketWriteRequest,
    SocketWriteResponse,
};
pub use notification::{
    NOTIFICATION_REQUEST_SCHEMA_VERSION, NotificationDeliveryMode, NotificationEventOptions,
    NotificationMediaUpdateBatch, NotificationPayloadFormat, NotificationRichEmbed,
    NotificationRichEmbedField, NotificationSeverity, PluginNotificationActor,
    PluginNotificationTargetResult, coalesce_media_updates, rich_embed_from_request,
    to_script_environment, to_webhook_json,
};

pub const SDK_VERSION: &str = "3.1.0";

pub fn current_sdk_constraint() -> String {
    legacy_sdk_constraint(SDK_VERSION)
}

pub fn sdk_constraint_or_legacy(sdk_version: &str, sdk_constraint: &str) -> String {
    let explicit = sdk_constraint.trim();
    if !explicit.is_empty() {
        return explicit.to_string();
    }

    legacy_sdk_constraint(sdk_version)
}

pub fn effective_host_sdk_constraint(sdk_version: Option<&str>, sdk_constraint: &str) -> String {
    let explicit = sdk_constraint.trim();
    if explicit.is_empty() {
        return sdk_version.map_or_else(|| ">=0.0.0".to_string(), legacy_sdk_constraint);
    }

    if let Some(sdk_version) = sdk_version {
        if sdk_minor_line_constraint(sdk_version)
            .as_deref()
            .is_some_and(|minor_line| {
                normalize_constraint_literal(explicit) == normalize_constraint_literal(minor_line)
            })
        {
            return legacy_sdk_constraint(sdk_version);
        }
    } else if let Some(abi_line) = abi_major_constraint_from_generated_minor_line(explicit) {
        return abi_line;
    }

    explicit.to_string()
}

pub fn plugin_descriptor_sdk_constraint(descriptor: &PluginDescriptor) -> String {
    sdk_constraint_or_legacy(&descriptor.sdk_version, &descriptor.sdk_constraint)
}

pub fn host_version_matches_constraint(
    current_version: &str,
    constraint: &str,
) -> Result<bool, String> {
    let current = Version::parse(current_version.trim())
        .map_err(|error| format!("invalid host version {current_version}: {error}"))?;
    let required = VersionReq::parse(constraint.trim())
        .map_err(|error| format!("invalid host constraint {constraint}: {error}"))?;
    Ok(required.matches(&current))
}

pub fn validate_sdk_contract(
    subject: &str,
    sdk_version: &str,
    sdk_constraint: &str,
    host_sdk_version: &str,
) -> Result<(), String> {
    let host_version = Version::parse(host_sdk_version)
        .map_err(|error| format!("invalid host sdk_version {host_sdk_version}: {error}"))?;
    let descriptor_version = Version::parse(sdk_version.trim())
        .map_err(|error| format!("{subject}: invalid sdk_version {sdk_version}: {error}"))?;
    let descriptor_constraint = sdk_constraint_or_legacy(sdk_version, sdk_constraint);
    let descriptor_req = VersionReq::parse(descriptor_constraint.trim()).map_err(|error| {
        format!("{subject}: invalid sdk_constraint {descriptor_constraint}: {error}")
    })?;
    if !descriptor_req.matches(&descriptor_version) {
        return Err(format!(
            "{subject}: sdk_version {sdk_version} does not satisfy sdk_constraint {descriptor_constraint}"
        ));
    }
    let host_constraint = effective_host_sdk_constraint(Some(sdk_version), sdk_constraint);
    let host_req = VersionReq::parse(host_constraint.trim())
        .map_err(|error| format!("{subject}: invalid sdk_constraint {host_constraint}: {error}"))?;
    if !host_req.matches(&host_version) {
        return Err(format!(
            "{subject}: host sdk_version {host_sdk_version} does not satisfy sdk_constraint {host_constraint}"
        ));
    }
    Ok(())
}

pub fn validate_plugin_descriptor_sdk_contract(
    descriptor: &PluginDescriptor,
    host_sdk_version: &str,
) -> Result<(), String> {
    validate_sdk_contract(
        &descriptor.id,
        &descriptor.sdk_version,
        &descriptor.sdk_constraint,
        host_sdk_version,
    )
}

#[cfg(not(target_arch = "wasm32"))]
pub fn allowed_host_pattern_is_valid(host: &str) -> bool {
    let host = host.trim();
    if host.is_empty()
        || host == "*"
        || host.contains("://")
        || host.contains('/')
        || host.contains('?')
        || host.contains('#')
        || host.contains(':')
    {
        return false;
    }

    if let Some(suffix) = host.strip_prefix("*.") {
        return !suffix.is_empty() && !suffix.contains('*') && allowed_host_name_is_valid(suffix);
    }

    !host.contains('*') && allowed_host_name_is_valid(host)
}

#[cfg(not(target_arch = "wasm32"))]
fn allowed_host_name_is_valid(host: &str) -> bool {
    if host.parse::<std::net::Ipv4Addr>().is_ok() {
        return true;
    }

    host.split('.').all(|label| {
        !label.is_empty()
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
    })
}

#[cfg(not(target_arch = "wasm32"))]
pub fn socket_host_pattern_is_valid(host: &str) -> bool {
    allowed_host_pattern_is_valid(host) || socket_host_pattern_config_key(host).is_some()
}

pub fn socket_host_pattern_config_key(host: &str) -> Option<&str> {
    let key = host
        .trim()
        .strip_prefix("${")
        .and_then(|value| value.strip_suffix('}'))?;
    if key.is_empty()
        || !key
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    {
        return None;
    }
    Some(key)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn validate_plugin_descriptor_host_permissions(
    descriptor: &PluginDescriptor,
) -> Result<(), String> {
    for host in descriptor.allowed_hosts() {
        if !allowed_host_pattern_is_valid(host) {
            return Err(format!(
                "{}: invalid network permission pattern {}",
                descriptor.id, host
            ));
        }
    }
    for permission in &descriptor.socket_permissions {
        if !socket_host_pattern_is_valid(&permission.host_pattern) {
            return Err(format!(
                "{}: invalid socket permission host pattern {}",
                descriptor.id, permission.host_pattern
            ));
        }
        if permission.ports.is_empty() {
            return Err(format!(
                "{}: socket permission for {} must include at least one port",
                descriptor.id, permission.host_pattern
            ));
        }
        if permission.tls_modes.is_empty() {
            return Err(format!(
                "{}: socket permission for {} must include at least one TLS mode",
                descriptor.id, permission.host_pattern
            ));
        }
    }
    Ok(())
}

fn legacy_sdk_constraint(version: &str) -> String {
    let parsed = Version::parse(version.trim()).ok();
    let Some(version) = parsed else {
        return ">=0.0.0".to_string();
    };
    let (lower_major, lower_minor) = if version.major == 1 {
        (1, 0)
    } else {
        (version.major, version.minor)
    };
    let upper_major = if version.major == 1 {
        // SDK 2 kept the SDK-v1 guest ABI loadable; SDK 3 is the hard boundary.
        3
    } else {
        version.major + 1
    };
    format!(">={lower_major}.{lower_minor}.0, <{upper_major}.0.0")
}

fn sdk_minor_line_constraint(version: &str) -> Option<String> {
    let version = Version::parse(version.trim()).ok()?;
    Some(format!(
        ">={}.{}.0, <{}.{}.0",
        version.major,
        version.minor,
        version.major,
        version.minor + 1
    ))
}

fn normalize_constraint_literal(raw: &str) -> String {
    raw.split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

fn abi_major_constraint_from_generated_minor_line(constraint: &str) -> Option<String> {
    let normalized = normalize_constraint_literal(constraint);
    let mut parts = normalized.split(", ");
    let lower = parts.next()?.strip_prefix(">=")?.trim();
    let upper = parts.next()?.strip_prefix('<')?.trim();
    if parts.next().is_some() {
        return None;
    }

    let lower = Version::parse(lower).ok()?;
    let upper = Version::parse(upper).ok()?;
    if !lower.pre.is_empty()
        || !lower.build.is_empty()
        || !upper.pre.is_empty()
        || !upper.build.is_empty()
        || lower.patch != 0
        || upper.patch != 0
        || lower.major != upper.major
        || upper.minor != lower.minor + 1
    {
        return None;
    }

    Some(legacy_sdk_constraint(&lower.to_string()))
}

pub const EXPORT_DESCRIBE: &str = "scryer_describe";
pub const EXPORT_VALIDATE_CONFIG: &str = "scryer_validate_config";
pub const EXPORT_INDEXER_SEARCH: &str = "scryer_indexer_search";
pub const EXPORT_INDEXER_ACTION: &str = "scryer_indexer_action";
pub const EXPORT_DOWNLOAD_ADD: &str = "scryer_download_add";
pub const EXPORT_DOWNLOAD_LIST_QUEUE: &str = "scryer_download_list_queue";
pub const EXPORT_DOWNLOAD_LIST_HISTORY: &str = "scryer_download_list_history";
pub const EXPORT_DOWNLOAD_LIST_COMPLETED: &str = "scryer_download_list_completed";
pub const EXPORT_DOWNLOAD_LIST_RECENT_COMPLETED: &str = "scryer_download_list_recent_completed";
pub const EXPORT_DOWNLOAD_CONTROL: &str = "scryer_download_control";
pub const EXPORT_DOWNLOAD_MARK_IMPORTED: &str = "scryer_download_mark_imported";
pub const EXPORT_DOWNLOAD_STATUS: &str = "scryer_download_status";
pub const EXPORT_DOWNLOAD_TEST_CONNECTION: &str = "scryer_download_test_connection";
pub const EXPORT_NOTIFICATION_SEND: &str = "scryer_notification_send";
pub const EXPORT_NOTIFICATION_ACTION: &str = "scryer_notification_action";
pub const EXPORT_SUBTITLE_SEARCH: &str = "scryer_subtitle_search";
pub const EXPORT_SUBTITLE_DOWNLOAD: &str = "scryer_subtitle_download";
pub const EXPORT_SUBTITLE_GENERATE: &str = "scryer_subtitle_generate";
pub const EXPORT_SUBSYNC_ALIGN: &str = "scryer_subsync_align";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PluginKind {
    Indexer,
    DownloadClient,
    Notification,
    SubtitleProvider,
}

impl PluginKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Indexer => "indexer",
            Self::DownloadClient => "download_client",
            Self::Notification => "notification",
            Self::SubtitleProvider => "subtitle_provider",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IndexerSourceKind {
    #[default]
    Generic,
    Usenet,
    Torrent,
}

impl IndexerSourceKind {
    pub fn plugin_type(self) -> &'static str {
        match self {
            Self::Generic => "indexer",
            Self::Usenet => "usenet_indexer",
            Self::Torrent => "torrent_indexer",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PluginDescriptor {
    pub id: String,
    pub name: String,
    pub version: String,
    pub sdk_version: String,
    #[serde(default)]
    pub sdk_constraint: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub socket_permissions: Vec<SocketPermission>,
    pub provider: ProviderDescriptor,
}

impl PluginDescriptor {
    pub fn kind(&self) -> PluginKind {
        match &self.provider {
            ProviderDescriptor::Indexer(_) => PluginKind::Indexer,
            ProviderDescriptor::DownloadClient(_) => PluginKind::DownloadClient,
            ProviderDescriptor::Notification(_) => PluginKind::Notification,
            ProviderDescriptor::Subtitle(_) => PluginKind::SubtitleProvider,
        }
    }

    pub fn plugin_type(&self) -> &'static str {
        match &self.provider {
            ProviderDescriptor::Indexer(indexer) => indexer.source_kind.plugin_type(),
            ProviderDescriptor::DownloadClient(_) => PluginKind::DownloadClient.as_str(),
            ProviderDescriptor::Notification(_) => PluginKind::Notification.as_str(),
            ProviderDescriptor::Subtitle(_) => PluginKind::SubtitleProvider.as_str(),
        }
    }

    pub fn provider_type(&self) -> &str {
        match &self.provider {
            ProviderDescriptor::Indexer(provider) => provider.provider_type.as_str(),
            ProviderDescriptor::DownloadClient(provider) => provider.provider_type.as_str(),
            ProviderDescriptor::Notification(provider) => provider.provider_type.as_str(),
            ProviderDescriptor::Subtitle(provider) => provider.provider_type.as_str(),
        }
    }

    pub fn provider_aliases(&self) -> &[String] {
        match &self.provider {
            ProviderDescriptor::Indexer(provider) => provider.provider_aliases.as_slice(),
            ProviderDescriptor::DownloadClient(provider) => provider.provider_aliases.as_slice(),
            ProviderDescriptor::Notification(provider) => provider.provider_aliases.as_slice(),
            ProviderDescriptor::Subtitle(provider) => provider.provider_aliases.as_slice(),
        }
    }

    pub fn config_fields(&self) -> &[ConfigFieldDef] {
        match &self.provider {
            ProviderDescriptor::Indexer(provider) => provider.config_fields.as_slice(),
            ProviderDescriptor::DownloadClient(provider) => provider.config_fields.as_slice(),
            ProviderDescriptor::Notification(provider) => provider.config_fields.as_slice(),
            ProviderDescriptor::Subtitle(provider) => provider.config_fields.as_slice(),
        }
    }

    pub fn config_fields_mut(&mut self) -> &mut Vec<ConfigFieldDef> {
        match &mut self.provider {
            ProviderDescriptor::Indexer(provider) => &mut provider.config_fields,
            ProviderDescriptor::DownloadClient(provider) => &mut provider.config_fields,
            ProviderDescriptor::Notification(provider) => &mut provider.config_fields,
            ProviderDescriptor::Subtitle(provider) => &mut provider.config_fields,
        }
    }

    pub fn allowed_hosts(&self) -> &[String] {
        match &self.provider {
            ProviderDescriptor::Indexer(provider) => provider.allowed_hosts.as_slice(),
            ProviderDescriptor::DownloadClient(provider) => provider.allowed_hosts.as_slice(),
            ProviderDescriptor::Notification(provider) => provider.allowed_hosts.as_slice(),
            ProviderDescriptor::Subtitle(provider) => provider.allowed_hosts.as_slice(),
        }
    }

    pub fn default_base_url(&self) -> Option<&str> {
        match &self.provider {
            ProviderDescriptor::Indexer(provider) => provider
                .config_fields
                .iter()
                .find(|field| field.role == Some(ConfigFieldRole::ConnectionUrl))
                .and_then(|field| field.default_value.as_deref()),
            ProviderDescriptor::DownloadClient(provider) => provider.default_base_url.as_deref(),
            ProviderDescriptor::Notification(provider) => provider.default_base_url.as_deref(),
            ProviderDescriptor::Subtitle(provider) => provider.default_base_url.as_deref(),
        }
    }

    pub fn set_default_base_url(&mut self, value: Option<String>) {
        match &mut self.provider {
            ProviderDescriptor::Indexer(provider) => {
                if let Some(field) = provider
                    .config_fields
                    .iter_mut()
                    .find(|field| field.role == Some(ConfigFieldRole::ConnectionUrl))
                {
                    field.default_value = value;
                }
            }
            ProviderDescriptor::DownloadClient(provider) => provider.default_base_url = value,
            ProviderDescriptor::Notification(provider) => provider.default_base_url = value,
            ProviderDescriptor::Subtitle(provider) => provider.default_base_url = value,
        }
    }

    pub fn indexer(&self) -> Option<&IndexerDescriptor> {
        match &self.provider {
            ProviderDescriptor::Indexer(provider) => Some(provider),
            _ => None,
        }
    }

    pub fn notification(&self) -> Option<&NotificationDescriptor> {
        match &self.provider {
            ProviderDescriptor::Notification(provider) => Some(provider),
            _ => None,
        }
    }

    pub fn download_client(&self) -> Option<&DownloadClientDescriptor> {
        match &self.provider {
            ProviderDescriptor::DownloadClient(provider) => Some(provider),
            _ => None,
        }
    }

    pub fn subtitle(&self) -> Option<&SubtitleDescriptor> {
        match &self.provider {
            ProviderDescriptor::Subtitle(provider) => Some(provider),
            _ => None,
        }
    }
}

#[cfg(all(not(target_arch = "wasm32"), feature = "host-runtime"))]
fn required_exports_for_descriptor(descriptor: &PluginDescriptor) -> Vec<&'static str> {
    let mut exports = vec![EXPORT_DESCRIBE];
    match &descriptor.provider {
        ProviderDescriptor::Indexer(_) => {
            exports.push(EXPORT_INDEXER_SEARCH);
        }
        ProviderDescriptor::DownloadClient(_) => exports.extend([
            EXPORT_DOWNLOAD_ADD,
            EXPORT_DOWNLOAD_LIST_QUEUE,
            EXPORT_DOWNLOAD_LIST_HISTORY,
            EXPORT_DOWNLOAD_LIST_COMPLETED,
            EXPORT_DOWNLOAD_CONTROL,
            EXPORT_DOWNLOAD_MARK_IMPORTED,
            EXPORT_DOWNLOAD_STATUS,
            EXPORT_DOWNLOAD_TEST_CONNECTION,
        ]),
        ProviderDescriptor::Notification(_) => exports.push(EXPORT_NOTIFICATION_SEND),
        ProviderDescriptor::Subtitle(subtitle) => {
            exports.push(EXPORT_VALIDATE_CONFIG);
            match subtitle.capabilities.mode {
                SubtitleProviderMode::Catalog => {
                    exports.extend([EXPORT_SUBTITLE_SEARCH, EXPORT_SUBTITLE_DOWNLOAD]);
                }
                SubtitleProviderMode::Generator => exports.push(EXPORT_SUBTITLE_GENERATE),
            }
        }
    }
    exports
}

#[cfg(all(not(target_arch = "wasm32"), feature = "host-runtime"))]
const SOCKET_HOST_NAMESPACE: &str = "extism:host/user";

#[cfg(all(not(target_arch = "wasm32"), feature = "host-runtime"))]
#[derive(Clone)]
struct DisabledDescriptorSocketHost {
    state: UserData<()>,
}

#[cfg(all(not(target_arch = "wasm32"), feature = "host-runtime"))]
impl DisabledDescriptorSocketHost {
    fn new() -> Self {
        Self {
            state: UserData::new(()),
        }
    }

    fn functions(&self) -> Vec<Function> {
        let params = || [ValType::I64];
        let results = || [ValType::I64];

        vec![
            Function::new(
                "scryer_socket_open",
                params(),
                results(),
                self.state.clone(),
                descriptor_socket_open,
            )
            .with_namespace(SOCKET_HOST_NAMESPACE),
            Function::new(
                "scryer_socket_read",
                params(),
                results(),
                self.state.clone(),
                descriptor_socket_read,
            )
            .with_namespace(SOCKET_HOST_NAMESPACE),
            Function::new(
                "scryer_socket_write",
                params(),
                results(),
                self.state.clone(),
                descriptor_socket_write,
            )
            .with_namespace(SOCKET_HOST_NAMESPACE),
            Function::new(
                "scryer_socket_starttls",
                params(),
                results(),
                self.state.clone(),
                descriptor_socket_starttls,
            )
            .with_namespace(SOCKET_HOST_NAMESPACE),
            Function::new(
                "scryer_socket_close",
                params(),
                results(),
                self.state.clone(),
                descriptor_socket_close,
            )
            .with_namespace(SOCKET_HOST_NAMESPACE),
        ]
    }
}

#[cfg(all(not(target_arch = "wasm32"), feature = "host-runtime"))]
fn disabled_descriptor_socket_response() -> String {
    serde_json::to_string(&SocketResponse::<()>::error(
        SocketErrorCode::Unsupported,
        "socket host functions are unavailable while loading plugin descriptors",
    ))
    .unwrap_or_else(|_| "{\"ok\":false}".to_string())
}

#[cfg(all(not(target_arch = "wasm32"), feature = "host-runtime"))]
host_fn!(descriptor_socket_open(state: (); _input: String) -> String {
    let _ = state.get()?;
    Ok(disabled_descriptor_socket_response())
});

#[cfg(all(not(target_arch = "wasm32"), feature = "host-runtime"))]
host_fn!(descriptor_socket_read(state: (); _input: String) -> String {
    let _ = state.get()?;
    Ok(disabled_descriptor_socket_response())
});

#[cfg(all(not(target_arch = "wasm32"), feature = "host-runtime"))]
host_fn!(descriptor_socket_write(state: (); _input: String) -> String {
    let _ = state.get()?;
    Ok(disabled_descriptor_socket_response())
});

#[cfg(all(not(target_arch = "wasm32"), feature = "host-runtime"))]
host_fn!(descriptor_socket_starttls(state: (); _input: String) -> String {
    let _ = state.get()?;
    Ok(disabled_descriptor_socket_response())
});

#[cfg(all(not(target_arch = "wasm32"), feature = "host-runtime"))]
host_fn!(descriptor_socket_close(state: (); _input: String) -> String {
    let _ = state.get()?;
    Ok(disabled_descriptor_socket_response())
});

#[cfg(all(not(target_arch = "wasm32"), feature = "host-runtime"))]
pub fn load_plugin_descriptor_from_wasm_bytes(
    wasm_bytes: &[u8],
) -> Result<PluginDescriptor, String> {
    let manifest = Manifest::new([Wasm::data(wasm_bytes.to_vec())])
        .with_timeout(std::time::Duration::from_secs(10));
    let socket_host = DisabledDescriptorSocketHost::new();
    let mut plugin = PluginBuilder::new(manifest)
        .with_wasi(true)
        .with_http_response_headers(true)
        .with_functions(socket_host.functions())
        .build()
        .map_err(|error| format!("failed to instantiate WASM: {error}"))?;

    if !plugin.function_exists(EXPORT_DESCRIBE) {
        return Err(format!(
            "plugin is missing required export {EXPORT_DESCRIBE}"
        ));
    }

    let output: String = plugin
        .call::<&str, String>(EXPORT_DESCRIBE, "")
        .map_err(|error| format!("{EXPORT_DESCRIBE}() failed: {error}"))?;
    let descriptor: PluginDescriptor = serde_json::from_str(&output)
        .map_err(|error| format!("describe() returned invalid JSON: {error}"))?;

    let missing = required_exports_for_descriptor(&descriptor)
        .into_iter()
        .filter(|export| !plugin.function_exists(export))
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(format!(
            "{} ({}) is missing required export(s): {}",
            descriptor.id,
            descriptor.plugin_type(),
            missing.join(", ")
        ));
    }

    Ok(descriptor)
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProviderDescriptor {
    Indexer(IndexerDescriptor),
    DownloadClient(DownloadClientDescriptor),
    Notification(NotificationDescriptor),
    Subtitle(SubtitleDescriptor),
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct IndexerDescriptor {
    pub provider_type: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_aliases: Vec<String>,
    #[serde(default)]
    pub source_kind: IndexerSourceKind,
    #[serde(default)]
    pub capabilities: IndexerCapabilities,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scoring_policies: Vec<PluginScoringPolicy>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub config_fields: Vec<ConfigFieldDef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_hosts: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_limit_seconds: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DownloadClientDescriptor {
    pub provider_type: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_aliases: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub config_fields: Vec<ConfigFieldDef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_hosts: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accepted_inputs: Vec<DownloadInputKind>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub isolation_modes: Vec<DownloadIsolationMode>,
    #[serde(default)]
    pub capabilities: DownloadClientCapabilities,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NotificationDescriptor {
    pub provider_type: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_aliases: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub config_fields: Vec<ConfigFieldDef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_hosts: Vec<String>,
    #[serde(default)]
    pub capabilities: NotificationCapabilities,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SubtitleDescriptor {
    pub provider_type: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_aliases: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub config_fields: Vec<ConfigFieldDef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_hosts: Vec<String>,
    #[serde(default)]
    pub capabilities: SubtitleCapabilities,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PluginScoringPolicy {
    pub name: String,
    pub rego_source: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub applied_facets: Vec<String>,
}

fn default_true() -> bool {
    true
}

fn serialize_ordered_string_map<S, T>(
    map: &HashMap<String, T>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
    T: Serialize,
{
    let ordered = map.iter().collect::<BTreeMap<_, _>>();
    ordered.serialize(serializer)
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct IndexerCapabilities {
    #[serde(default = "default_true")]
    pub rss: bool,
    #[serde(default, serialize_with = "serialize_ordered_string_map")]
    pub supported_ids: HashMap<String, Vec<String>>,
    #[serde(default)]
    pub deduplicates_aliases: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub season_param: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub episode_param: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_param: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supported_query_facets: Vec<String>,
    #[serde(default)]
    pub search: bool,
    #[serde(default)]
    pub imdb_search: bool,
    #[serde(default)]
    pub tvdb_search: bool,
    #[serde(default)]
    pub anidb_search: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub protocols: Vec<IndexerProtocol>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub feed_modes: Vec<IndexerFeedMode>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub search_inputs: Vec<IndexerSearchInput>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supported_external_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category_model: Option<IndexerCategoryModel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limits: Option<IndexerLimitCapabilities>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub torrent: Option<IndexerTorrentCapabilities>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_features: Option<IndexerResponseFeatures>,
}

impl Default for IndexerCapabilities {
    fn default() -> Self {
        Self {
            rss: true,
            supported_ids: HashMap::new(),
            deduplicates_aliases: false,
            season_param: None,
            episode_param: None,
            query_param: None,
            supported_query_facets: Vec::new(),
            search: false,
            imdb_search: false,
            tvdb_search: false,
            anidb_search: false,
            protocols: Vec::new(),
            feed_modes: Vec::new(),
            search_inputs: Vec::new(),
            supported_external_ids: Vec::new(),
            category_model: None,
            limits: None,
            torrent: None,
            response_features: None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct NotificationCapabilities {
    #[serde(default)]
    pub supports_rich_text: bool,
    #[serde(default)]
    pub supports_images: bool,
    #[serde(default)]
    pub supports_test: bool,
    #[serde(default)]
    pub supports_batch: bool,
    #[serde(default)]
    pub supports_coalescing: bool,
    #[serde(default)]
    pub requires_host_filesystem: bool,
    #[serde(default)]
    pub requires_host_process: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub delivery_modes: Vec<NotificationDeliveryMode>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub payload_formats: Vec<NotificationPayloadFormat>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supported_events: Vec<NotificationEventType>,
    #[serde(default)]
    pub event_options: NotificationEventOptions,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct DownloadClientCapabilities {
    #[serde(default)]
    pub pause: bool,
    #[serde(default)]
    pub resume: bool,
    #[serde(default)]
    pub remove: bool,
    #[serde(default)]
    pub remove_with_data: bool,
    #[serde(default)]
    pub mark_imported: bool,
    #[serde(default)]
    pub prepare_for_import: bool,
    #[serde(default)]
    pub client_status: bool,
    #[serde(default)]
    pub queue_priority: bool,
    #[serde(default)]
    pub seed_limits: bool,
    #[serde(default)]
    pub start_paused: bool,
    #[serde(default)]
    pub force_start: bool,
    #[serde(default)]
    pub per_download_directory: bool,
    #[serde(default)]
    pub host_fs_required: bool,
    #[serde(default)]
    pub test_connection: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub torrent: Option<DownloadTorrentCapabilities>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct DownloadTorrentCapabilities {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supported_sources: Vec<DownloadInputKind>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub preferred_sources: Vec<DownloadInputKind>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub isolation_modes: Vec<DownloadIsolationMode>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub post_import_isolation_modes: Vec<DownloadIsolationMode>,
    #[serde(default)]
    pub supports_seed_ratio_limit: bool,
    #[serde(default)]
    pub supports_seed_time_limit: bool,
    #[serde(default)]
    pub removes_on_seed_limit: bool,
    #[serde(default)]
    pub supports_start_paused: bool,
    #[serde(default)]
    pub supports_stopped: bool,
    #[serde(default)]
    pub supports_force_start: bool,
    #[serde(default)]
    pub supports_queue_placement: bool,
    #[serde(default)]
    pub supports_priority_hint: bool,
    #[serde(default)]
    pub supports_sequential_download: bool,
    #[serde(default)]
    pub supports_first_last_piece_priority: bool,
    #[serde(default)]
    pub supports_content_layout: bool,
    #[serde(default)]
    pub supports_skip_checking: bool,
    #[serde(default)]
    pub supports_auto_management: bool,
    #[serde(default)]
    pub supports_safe_seeding: bool,
    #[serde(default)]
    pub supports_anonymity_hops: bool,
    #[serde(default)]
    pub supports_selected_files: bool,
    #[serde(default)]
    pub supports_post_import_isolation: bool,
    #[serde(default)]
    pub reports_content_paths: bool,
    #[serde(default)]
    pub reports_metadata_only: bool,
    #[serde(default)]
    pub host_fs_required: bool,
    #[serde(default)]
    pub client_id_may_differ_from_info_hash: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SubtitleProviderMode {
    #[default]
    Catalog,
    Generator,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct SubtitleCapabilities {
    pub mode: SubtitleProviderMode,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supported_media_kinds: Vec<SubtitleQueryMediaKind>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recommended_facets: Vec<String>,
    #[serde(default)]
    pub supports_hash_lookup: bool,
    #[serde(default)]
    pub supports_forced: bool,
    #[serde(default)]
    pub supports_hearing_impaired: bool,
    #[serde(default)]
    pub supports_ai_translated: bool,
    #[serde(default)]
    pub supports_machine_translated: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supported_languages: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConfigFieldType {
    #[default]
    String,
    #[serde(alias = "secret")]
    Password,
    Multiline,
    Bool,
    Select,
    Number,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConfigFieldValueSource {
    #[default]
    User,
    HostBinding,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConfigFieldRole {
    ConnectionUrl,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub enum PluginHostBindingId {
    #[serde(rename = "smg.opensubtitles_api_key")]
    SmgOpenSubtitlesApiKey,
}

impl PluginHostBindingId {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SmgOpenSubtitlesApiKey => "smg.opensubtitles_api_key",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct ConfigFieldDef {
    pub key: String,
    pub label: String,
    pub field_type: ConfigFieldType,
    #[serde(default)]
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_value: Option<String>,
    #[serde(default)]
    pub value_source: ConfigFieldValueSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<ConfigFieldRole>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_binding: Option<PluginHostBindingId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<ConfigFieldOption>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help_text: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct ConfigFieldOption {
    pub value: String,
    pub label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PluginErrorCode {
    InvalidConfig,
    AuthFailed,
    RateLimited,
    UpstreamUnavailable,
    Unsupported,
    Temporary,
    Permanent,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PluginError {
    pub code: PluginErrorCode,
    pub public_message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub debug_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_seconds: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PluginResult<T> {
    Ok(T),
    Err(PluginError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DownloadInputKind {
    Nzb,
    NzbUrl,
    TorrentFile,
    TorrentUrl,
    TorrentBytes,
    MagnetUri,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DownloadIsolationMode {
    Category,
    Tag,
    Label,
    View,
    Directory,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PluginDownloadIsolation {
    pub mode: DownloadIsolationMode,
    pub value: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PluginTorrentInitialState {
    #[default]
    Default,
    Started,
    Paused,
    Stopped,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PluginTorrentQueuePlacement {
    #[default]
    Default,
    First,
    Last,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PluginTorrentContentLayout {
    #[default]
    Default,
    Original,
    Subfolder,
    NoSubfolder,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PluginDownloadOutputKind {
    File,
    Directory,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DownloadItemState {
    Queued,
    Downloading,
    Verifying,
    Repairing,
    Extracting,
    Paused,
    Completed,
    ImportPending,
    Failed,
    Error,
    Warning,
    Seeding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DownloadControlAction {
    Pause,
    Resume,
    Remove,
    ForceStart,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NotificationEventType {
    Grab,
    Download,
    Upgrade,
    ImportComplete,
    ImportRejected,
    Rename,
    TitleAdded,
    TitleDeleted,
    FileDeleted,
    FileDeletedForUpgrade,
    PostProcessingCompleted,
    SubtitleDownloaded,
    SubtitleSearchFailed,
    MediaRequestSubmitted,
    MediaRequestApproved,
    MediaRequestRejected,
    MediaRequestCanceled,
    HealthIssue,
    HealthRestored,
    ApplicationUpdate,
    ManualInteractionRequired,
    Test,
}

impl NotificationEventType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Grab => "grab",
            Self::Download => "download",
            Self::Upgrade => "upgrade",
            Self::ImportComplete => "import_complete",
            Self::ImportRejected => "import_rejected",
            Self::Rename => "rename",
            Self::TitleAdded => "title_added",
            Self::TitleDeleted => "title_deleted",
            Self::FileDeleted => "file_deleted",
            Self::FileDeletedForUpgrade => "file_deleted_for_upgrade",
            Self::PostProcessingCompleted => "post_processing_completed",
            Self::SubtitleDownloaded => "subtitle_downloaded",
            Self::SubtitleSearchFailed => "subtitle_search_failed",
            Self::MediaRequestSubmitted => "media_request_submitted",
            Self::MediaRequestApproved => "media_request_approved",
            Self::MediaRequestRejected => "media_request_rejected",
            Self::MediaRequestCanceled => "media_request_canceled",
            Self::HealthIssue => "health_issue",
            Self::HealthRestored => "health_restored",
            Self::ApplicationUpdate => "application_update",
            Self::ManualInteractionRequired => "manual_interaction_required",
            Self::Test => "test",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SubtitleValidateConfigStatus {
    Valid,
    InvalidConfig,
    AuthFailed,
    RateLimited,
    Unreachable,
    Unsupported,
    MissingHostBinding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SubtitleMatchHintKind {
    Hash,
    ImdbId,
    SeriesImdbId,
    ExternalId,
    AbsoluteEpisode,
    Release,
    Title,
    SeasonEpisode,
    Language,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SubtitleMatchHint {
    pub kind: SubtitleMatchHintKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SubtitleQueryMediaKind {
    Movie,
    Episode,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SubtitlePluginSearchRequest {
    pub media_kind: SubtitleQueryMediaKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub facet: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imdb_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub series_imdb_id: Option<String>,
    pub title: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub title_aliases: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub title_candidates: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub season: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub episode: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub absolute_episode: Option<i32>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub external_ids: BTreeMap<String, Vec<String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub languages: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_codec: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_codec: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hearing_impaired: Option<bool>,
    #[serde(default)]
    pub include_ai_translated: bool,
    #[serde(default)]
    pub include_machine_translated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SubtitlePluginCandidate {
    pub provider_file_id: String,
    pub language: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_info: Option<String>,
    #[serde(default)]
    pub hearing_impaired: bool,
    #[serde(default)]
    pub forced: bool,
    #[serde(default)]
    pub ai_translated: bool,
    #[serde(default)]
    pub machine_translated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uploader: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_count: Option<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub match_hints: Vec<SubtitleMatchHint>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct SubtitlePluginSearchResponse {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub results: Vec<SubtitlePluginCandidate>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SubtitlePluginDownloadRequest {
    pub provider_file_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SubtitlePluginDownloadResponse {
    pub content_base64: String,
    pub format: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct SubtitlePluginValidateConfigRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_instance_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SubtitlePluginValidateConfigResponse {
    pub status: SubtitleValidateConfigStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_seconds: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SubtitleGeneratorInputRef {
    pub path: PathBuf,
    pub mime_type: String,
    pub duration_seconds: i64,
    pub size_bytes: i64,
    pub checksum: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SubtitlePluginGenerateRequest {
    pub media_kind: SubtitleQueryMediaKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub facet: Option<String>,
    pub input: SubtitleGeneratorInputRef,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub languages: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SubtitlePluginGenerateResponse {
    pub content_base64: String,
    pub format: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SubtitleSyncAudioCodec {
    Ac3,
    Eac3,
    Dts,
    DtsHdMaCore,
    TrueHd,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AudioStreamSelector {
    Default,
    StreamIndex { index: u32 },
    Language { language: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SubtitleSyncAlignSkipReason {
    AudioDecodeFailed,
    NotEnoughReferenceSpans,
    WeakAlignment,
    LowAlignmentConsistency,
    OffsetExceedsMaximum,
    OffsetTooSmall,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SubtitleTimingSpan {
    pub start_ms: i64,
    pub end_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SubtitleSyncAlignInputRef {
    pub path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SubtitleSyncInputSubtitle {
    pub content_base64: String,
    pub format: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encoding_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SubtitleSyncReferenceSubtitle {
    pub content_base64: String,
    pub format: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encoding_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SubtitleSyncAlignRequest {
    pub input: SubtitleSyncAlignInputRef,
    pub subtitle: SubtitleSyncInputSubtitle,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference_subtitle: Option<SubtitleSyncReferenceSubtitle>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subtitle_spans: Vec<SubtitleTimingSpan>,
    pub max_offset_seconds: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync_options: Option<SubtitleSyncOptions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector: Option<AudioStreamSelector>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_codec: Option<SubtitleSyncAudioCodec>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SubtitleSyncOptions {
    #[serde(default)]
    pub start_seconds: u32,
    #[serde(default = "default_max_subtitle_duration_ms")]
    pub max_subtitle_duration_ms: u64,
    #[serde(default = "default_precise_framerate_search")]
    pub precise_framerate_search: bool,
    #[serde(default = "default_output_encoding")]
    pub output_encoding: String,
}

impl Default for SubtitleSyncOptions {
    fn default() -> Self {
        Self {
            start_seconds: 0,
            max_subtitle_duration_ms: default_max_subtitle_duration_ms(),
            precise_framerate_search: default_precise_framerate_search(),
            output_encoding: default_output_encoding(),
        }
    }
}

fn default_max_subtitle_duration_ms() -> u64 {
    10_000
}

fn default_precise_framerate_search() -> bool {
    true
}

fn default_output_encoding() -> String {
    "same".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SubtitleSyncRewrittenSubtitle {
    pub content_base64: String,
    pub format: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SubtitleSyncAlignResponse {
    pub applied: bool,
    pub offset_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rewritten_subtitle: Option<SubtitleSyncRewrittenSubtitle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_framerate_ratio: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consistency_ratio: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nosplit_score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub split_score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skipped_reason: Option<SubtitleSyncAlignSkipReason>,
    pub backend: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PluginDownloadClientAddRequest {
    pub source: PluginDownloadSource,
    pub release: PluginDownloadRelease,
    pub title: PluginDownloadTitle,
    pub routing: PluginDownloadRouting,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub torrent: Option<PluginTorrentOptions>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PluginDownloadListRecentCompletedRequest {
    pub limit: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PluginDownloadSource {
    pub kind: DownloadInputKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub magnet_uri: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub torrent_bytes_base64: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub torrent_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub torrent_file_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub torrent_content_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nzb_bytes_base64: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nzb_file_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nzb_content_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_password: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct PluginTorrentOptions {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_preference: Vec<DownloadInputKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed_goal_ratio: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed_goal_seconds: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_state: Option<PluginTorrentInitialState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue_placement: Option<PluginTorrentQueuePlacement>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sequential_download: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_last_piece_priority: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_layout: Option<PluginTorrentContentLayout>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skip_checking: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_management: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub force_start: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub safe_seeding: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anonymity_hops: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub selected_file_indices: Vec<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct PluginDownloadRelease {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub import_purpose: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_recent: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub season_pack: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub indexer_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info_hash_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info_hash_v1: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info_hash_v2: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed_goal_ratio: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed_goal_seconds: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PluginDownloadTitle {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_id: Option<String>,
    pub title_name: String,
    pub media_facet: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct PluginDownloadRouting {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub isolation_value: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub isolation: Vec<PluginDownloadIsolation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub post_import_isolation: Vec<PluginDownloadIsolation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue_priority: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_directory: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PluginDownloadClientAddResponse {
    pub client_item_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PluginDownloadItem {
    pub client_item_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info_hash: Option<String>,
    pub title: String,
    pub state: DownloadItemState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_output_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub torrent: Option<PluginTorrentItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_size_bytes: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remaining_size_bytes: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eta_seconds: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress_percent: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub can_move_files: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub can_remove: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub removed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct PluginTorrentItem {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info_hash_v1: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info_hash_v2: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_native_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub views: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub save_path: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uploaded_bytes: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub downloaded_bytes: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upload_rate_bytes_per_second: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_rate_bytes_per_second: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed_ratio: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed_time_seconds: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata_only: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_encrypted: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_private: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PluginCompletedDownload {
    pub client_item_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info_hash: Option<String>,
    pub name: String,
    pub dest_dir: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_kind: Option<PluginDownloadOutputKind>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parameters: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PluginDownloadClientControlRequest {
    pub action: DownloadControlAction,
    pub client_item_id: String,
    #[serde(default)]
    pub remove_data: bool,
    #[serde(default)]
    pub is_history: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PluginDownloadClientMarkImportedRequest {
    pub client_item_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub post_import_isolation: Vec<PluginDownloadIsolation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imported_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_path: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct PluginDownloadClientStatus {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_localhost: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remote_output_roots: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub removes_completed_downloads: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sorting_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct PluginNotificationExternalIds {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tmdb_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imdb_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tvdb_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anidb_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tvmaze_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub anilist_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mal_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kitsu_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub by_source: BTreeMap<String, Vec<String>>,
}

impl PluginNotificationExternalIds {
    fn is_empty(&self) -> bool {
        self.tmdb_id.is_none()
            && self.imdb_id.is_none()
            && self.tvdb_id.is_none()
            && self.anidb_id.is_none()
            && self.tvmaze_id.is_none()
            && self.anilist_ids.is_empty()
            && self.mal_ids.is_empty()
            && self.kitsu_ids.is_empty()
            && self.by_source.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PluginNotificationApp {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PluginNotificationTitle {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub name: String,
    pub facet: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overview: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poster_url: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub genres: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_country: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "PluginNotificationExternalIds::is_empty"
    )]
    pub external_ids: PluginNotificationExternalIds,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct PluginNotificationEpisode {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub episode_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_file_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_file_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collection_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub season_number: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub episode_number: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub absolute_number: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overview: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub air_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub air_date_utc: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub episode_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finale_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tvdb_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct PluginNotificationRelease {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub indexer: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub languages: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub custom_scores: BTreeMap<String, i32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct PluginNotificationDownload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress_percent: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_path: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct PluginNotificationImport {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub import_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_system: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dest_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imported_count: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skipped_count: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rejected_count: Option<i32>,
    #[serde(default)]
    pub upgrade: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deleted_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub replaced_paths: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct PluginNotificationHealth {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub severity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum NotificationMediaUpdateType {
    Created,
    Modified,
    Deleted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PluginNotificationMediaUpdate {
    pub path: String,
    pub update_type: NotificationMediaUpdateType,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PluginNotificationFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_path: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub media_updates: Vec<PluginNotificationMediaUpdate>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PluginNotificationMediaFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recycle_bin_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene_name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub audio_languages: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subtitle_languages: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_codec: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_codec: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_channels: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_width: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_height: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_bit_depth: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_hdr_format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_frame_rate: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container_format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edition: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct PluginNotificationApplicationUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct PluginNotificationManualInteraction {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct PluginNotificationMediaRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub library_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub facet: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_quality_profile_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_quality_profile_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_monitor_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved_quality_profile_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved_quality_profile_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_title_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PluginNotificationRequest {
    #[serde(default = "default_notification_request_schema_version")]
    pub schema_version: u32,
    pub event_type: NotificationEventType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurred_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<PluginNotificationActor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub severity: Option<NotificationSeverity>,
    #[serde(default)]
    pub is_test: bool,
    pub summary_title: String,
    pub summary_message: String,
    pub app: PluginNotificationApp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<PluginNotificationTitle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub episode: Option<PluginNotificationEpisode>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub episodes: Vec<PluginNotificationEpisode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release: Option<PluginNotificationRelease>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download: Option<PluginNotificationDownload>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub import: Option<PluginNotificationImport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health: Option<PluginNotificationHealth>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<PluginNotificationFile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub media_files: Vec<PluginNotificationMediaFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub application_update: Option<PluginNotificationApplicationUpdate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manual_interaction: Option<PluginNotificationManualInteraction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_request: Option<PluginNotificationMediaRequest>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PluginNotificationResponse {
    pub success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_seconds: Option<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub target_results: Vec<PluginNotificationTargetResult>,
}

fn default_notification_request_schema_version() -> u32 {
    NOTIFICATION_REQUEST_SCHEMA_VERSION
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct PluginSearchRequest {
    pub query: String,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub ids: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub facet: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<String>,
    #[serde(default)]
    pub limit: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub season: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub episode: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub absolute_episode: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tagged_aliases: Vec<TaggedAlias>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<PluginSearchContext>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct PluginSearchResponse {
    #[serde(default)]
    pub results: Vec<PluginSearchResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_current: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_max: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grab_current: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grab_max: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct PluginSearchResult {
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grabs: Option<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub languages: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumbs_up: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumbs_down: Option<i32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subtitles: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protected: Option<bool>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub provider_extra: HashMap<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_kind: Option<IndexerSourceKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol: Option<IndexerProtocol>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub external_ids: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_categories: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub magnet_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info_hash_v1: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info_hash_v2: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seeders: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peers: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub leechers: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_volume_factor: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upload_volume_factor: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codec: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub indexer_flags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum_seed_ratio: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum_seed_time_minutes: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub season_pack_seed_ratio: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub season_pack_seed_time_minutes: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TaggedAlias {
    pub name: String,
    pub language: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct PluginSdkSchemaDocument {
    descriptor: PluginDescriptor,
    indexer_search_request: PluginSearchRequest,
    indexer_search_response: PluginSearchResponse,
    subtitle_search_request: SubtitlePluginSearchRequest,
    subtitle_search_result: PluginResult<SubtitlePluginSearchResponse>,
    subtitle_download_request: SubtitlePluginDownloadRequest,
    subtitle_download_result: PluginResult<SubtitlePluginDownloadResponse>,
    subtitle_validate_config_request: SubtitlePluginValidateConfigRequest,
    subtitle_validate_config_result: PluginResult<SubtitlePluginValidateConfigResponse>,
    subtitle_generate_request: SubtitlePluginGenerateRequest,
    subtitle_generate_result: PluginResult<SubtitlePluginGenerateResponse>,
    subtitle_sync_align_request: SubtitleSyncAlignRequest,
    subtitle_sync_align_result: PluginResult<SubtitleSyncAlignResponse>,
    download_add_request: PluginDownloadClientAddRequest,
    download_add_result: PluginResult<PluginDownloadClientAddResponse>,
    download_queue_result: PluginResult<Vec<PluginDownloadItem>>,
    download_history_result: PluginResult<Vec<PluginCompletedDownload>>,
    download_completed_result: PluginResult<Vec<PluginCompletedDownload>>,
    download_recent_completed_request: PluginDownloadListRecentCompletedRequest,
    download_control_request: PluginDownloadClientControlRequest,
    download_control_result: PluginResult<()>,
    download_mark_imported_request: PluginDownloadClientMarkImportedRequest,
    download_mark_imported_result: PluginResult<()>,
    download_status_result: PluginResult<PluginDownloadClientStatus>,
    notification_request: PluginNotificationRequest,
    notification_result: PluginResult<PluginNotificationResponse>,
}

pub fn plugin_sdk_schema_json() -> String {
    serde_json::to_string_pretty(&schema_for!(PluginSdkSchemaDocument)).unwrap() + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::indexer::{
        indexer_capability_fixtures, normalize_external_ids, torrent_result, usenet_result,
    };
    use crate::torrent::{
        choose_source_kind, decode_torrent_bytes, normalize_info_hash_pair, seed_seconds_to_minutes,
    };

    #[test]
    fn current_sdk_constraint_uses_current_v3_minor_floor() {
        assert_eq!(current_sdk_constraint(), ">=3.1.0, <4.0.0");
    }

    #[test]
    fn effective_host_sdk_constraint_widens_legacy_minor_line() {
        assert_eq!(
            effective_host_sdk_constraint(Some("1.5.0"), ">=1.5.0, <1.6.0"),
            ">=1.0.0, <3.0.0"
        );
        assert_eq!(
            effective_host_sdk_constraint(None, ">=1.5.0, <1.6.0"),
            ">=1.0.0, <3.0.0"
        );
    }

    #[test]
    fn effective_host_sdk_constraint_keeps_non_generated_explicit_pin() {
        assert_eq!(
            effective_host_sdk_constraint(Some("1.5.0"), ">=1.5.0, <1.5.2"),
            ">=1.5.0, <1.5.2"
        );
    }

    #[test]
    fn validate_sdk_contract_rejects_legacy_minor_line_plugin_on_sdk3_host() {
        let err = validate_sdk_contract("legacy-plugin", "1.5.0", ">=1.5.0, <1.6.0", SDK_VERSION)
            .expect_err("legacy minor-line plugin should not load across SDK 3 boundary");
        assert!(err.contains("host sdk_version 3.1.0"));
    }

    #[test]
    fn validate_sdk_contract_rejects_sdk2_plugin_on_sdk3_host() {
        let err = validate_sdk_contract("sdk2-plugin", "2.3.0", ">=2.3.0, <3.0.0", SDK_VERSION)
            .expect_err("SDK 2 plugin should not load on SDK 3 host");
        assert!(err.contains("host sdk_version 3.1.0"));
    }

    #[test]
    fn tagged_descriptor_round_trips() {
        let descriptor = PluginDescriptor {
            id: "newznab".into(),
            name: "Newznab".into(),
            version: "1.0.0".into(),
            sdk_version: SDK_VERSION.into(),
            sdk_constraint: current_sdk_constraint(),
            socket_permissions: vec![],
            provider: ProviderDescriptor::Indexer(IndexerDescriptor {
                provider_type: "newznab".into(),
                provider_aliases: vec![],
                source_kind: IndexerSourceKind::Usenet,
                capabilities: IndexerCapabilities::default(),
                scoring_policies: vec![],
                config_fields: vec![],
                allowed_hosts: vec![],
                rate_limit_seconds: None,
            }),
        };

        let json = serde_json::to_string(&descriptor).unwrap();
        let parsed: PluginDescriptor = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, "newznab");
        assert_eq!(parsed.provider_type(), "newznab");
        assert_eq!(parsed.plugin_type(), "usenet_indexer");
    }

    #[test]
    fn unknown_download_state_is_rejected() {
        let json = r#"{"client_item_id":"1","title":"x","state":"mystery"}"#;
        assert!(serde_json::from_str::<PluginDownloadItem>(json).is_err());
    }

    #[test]
    fn socket_permission_host_patterns_are_validated() {
        assert!(socket_host_pattern_is_valid("smtp.example.com"));
        assert!(socket_host_pattern_is_valid("*.example.com"));
        assert!(socket_host_pattern_is_valid("${smtp_host}"));
        assert!(!socket_host_pattern_is_valid("*"));
        assert!(!socket_host_pattern_is_valid("https://smtp.example.com"));
        assert!(!socket_host_pattern_is_valid("smtp.example.com:587"));
    }

    #[test]
    fn socket_permission_requires_ports_and_tls_modes() {
        let mut descriptor = PluginDescriptor {
            id: "email".into(),
            name: "Email".into(),
            version: "1.0.0".into(),
            sdk_version: SDK_VERSION.into(),
            sdk_constraint: current_sdk_constraint(),
            socket_permissions: vec![SocketPermission {
                host_pattern: "${smtp_host}".into(),
                ports: vec![],
                tls_modes: vec![SocketTlsMode::Starttls],
            }],
            provider: ProviderDescriptor::Notification(NotificationDescriptor {
                provider_type: "email".into(),
                provider_aliases: vec![],
                default_base_url: None,
                allowed_hosts: vec![],
                capabilities: NotificationCapabilities::default(),
                config_fields: vec![],
            }),
        };

        assert!(validate_plugin_descriptor_host_permissions(&descriptor).is_err());
        descriptor.socket_permissions[0].ports = vec![587];
        descriptor.socket_permissions[0].tls_modes = vec![];
        assert!(validate_plugin_descriptor_host_permissions(&descriptor).is_err());
        descriptor.socket_permissions[0].tls_modes = vec![SocketTlsMode::Starttls];
        assert!(validate_plugin_descriptor_host_permissions(&descriptor).is_ok());
    }

    #[test]
    fn v1_download_client_descriptor_defaults_v11_fields() {
        let json = r#"{
            "id":"qbittorrent",
            "name":"qBittorrent",
            "version":"0.1.0",
            "sdk_version":"1.0.0",
            "provider":{
                "kind":"download_client",
                "provider_type":"qbittorrent",
                "accepted_inputs":["magnet_uri","torrent_file"],
                "isolation_modes":["category","tag","directory"],
                "capabilities":{
                    "pause":true,
                    "resume":true,
                    "remove":true,
                    "mark_imported":true
                }
            }
        }"#;

        let parsed: PluginDescriptor = serde_json::from_str(json).unwrap();
        let ProviderDescriptor::DownloadClient(provider) = parsed.provider else {
            panic!("expected download client descriptor");
        };

        assert_eq!(
            provider.accepted_inputs,
            vec![DownloadInputKind::MagnetUri, DownloadInputKind::TorrentFile]
        );
        assert!(provider.capabilities.pause);
        assert!(provider.capabilities.torrent.is_none());
    }

    #[test]
    fn v11_notification_descriptor_defaults_v12_fields() {
        let json = r#"{
            "id":"webhook",
            "name":"Webhook",
            "version":"0.1.0",
            "sdk_version":"1.1.0",
            "provider":{
                "kind":"notification",
                "provider_type":"webhook",
                "capabilities":{
                    "supports_rich_text":false,
                    "supports_images":false,
                    "supported_events":["test"]
                }
            }
        }"#;

        let parsed: PluginDescriptor = serde_json::from_str(json).unwrap();
        let ProviderDescriptor::Notification(provider) = parsed.provider else {
            panic!("expected notification descriptor");
        };

        assert_eq!(provider.capabilities.delivery_modes, Vec::new());
        assert!(!provider.capabilities.supports_coalescing);
        assert!(!provider.capabilities.event_options.supports_upgrade_filter);
    }

    #[test]
    fn v12_indexer_descriptor_defaults_v13_fields() {
        let json = r#"{
            "id":"torznab",
            "name":"Torznab",
            "version":"0.1.0",
            "sdk_version":"1.2.0",
            "provider":{
                "kind":"indexer",
                "provider_type":"torznab",
                "source_kind":"torrent",
                "capabilities":{
                    "rss":true,
                    "search":true,
                    "supported_ids":{"series":["tvdb_id"]},
                    "query_param":"q"
                }
            }
        }"#;

        let parsed: PluginDescriptor = serde_json::from_str(json).unwrap();
        let ProviderDescriptor::Indexer(provider) = parsed.provider else {
            panic!("expected indexer descriptor");
        };

        assert!(provider.capabilities.protocols.is_empty());
        assert!(provider.capabilities.feed_modes.is_empty());
        assert!(provider.capabilities.search_inputs.is_empty());
        assert!(provider.capabilities.supported_external_ids.is_empty());
        assert!(provider.capabilities.category_model.is_none());
        assert!(provider.capabilities.limits.is_none());
        assert!(provider.capabilities.torrent.is_none());
        assert!(provider.capabilities.response_features.is_none());
    }

    #[test]
    fn indexer_supported_ids_serialize_in_stable_key_order() {
        let descriptor = PluginDescriptor {
            id: "newznab".into(),
            name: "Newznab".into(),
            version: "1.0.0".into(),
            sdk_version: SDK_VERSION.into(),
            sdk_constraint: current_sdk_constraint(),
            socket_permissions: vec![],
            provider: ProviderDescriptor::Indexer(IndexerDescriptor {
                provider_type: "newznab".into(),
                provider_aliases: vec![],
                source_kind: IndexerSourceKind::Usenet,
                capabilities: IndexerCapabilities {
                    supported_ids: HashMap::from([
                        ("series".into(), vec!["tvdb_id".into()]),
                        ("anime".into(), vec!["anidb_id".into()]),
                        ("movie".into(), vec!["imdb_id".into()]),
                    ]),
                    ..IndexerCapabilities::default()
                },
                scoring_policies: vec![],
                config_fields: vec![],
                allowed_hosts: vec![],
                rate_limit_seconds: None,
            }),
        };

        let json = serde_json::to_string(&descriptor).unwrap();
        let anime = json.find("\"anime\"").unwrap();
        let movie = json.find("\"movie\"").unwrap();
        let series = json.find("\"series\"").unwrap();

        assert!(anime < movie);
        assert!(movie < series);
    }

    #[test]
    fn torrent_capability_fixtures_round_trip() {
        let fixtures = torrent_capability_fixtures();
        for descriptor in fixtures {
            let json = serde_json::to_string(&descriptor).unwrap();
            let parsed: PluginDescriptor = serde_json::from_str(&json).unwrap();
            let ProviderDescriptor::DownloadClient(provider) = parsed.provider else {
                panic!("expected download client descriptor");
            };
            let torrent = provider
                .capabilities
                .torrent
                .expect("missing torrent capability");
            assert!(!torrent.supported_sources.is_empty());
        }
    }

    #[test]
    fn torrent_helpers_choose_best_source_and_normalize_hashes() {
        let request = PluginDownloadClientAddRequest {
            source: PluginDownloadSource {
                kind: DownloadInputKind::TorrentUrl,
                download_url: Some("https://tracker.example/release.torrent".to_string()),
                magnet_uri: Some(
                    "magnet:?xt=urn:btih:ABCDEF0123456789ABCDEF0123456789ABCDEF01".to_string(),
                ),
                torrent_bytes_base64: Some("dG9ycmVudA==".to_string()),
                torrent_url: Some("https://tracker.example/release.torrent".to_string()),
                torrent_file_name: Some("release.torrent".to_string()),
                torrent_content_type: Some("application/x-bittorrent".to_string()),
                nzb_bytes_base64: None,
                nzb_file_name: None,
                nzb_content_type: None,
                source_title: None,
                source_password: None,
            },
            release: PluginDownloadRelease {
                info_hash_hint: Some("abcdef0123456789abcdef0123456789abcdef01".to_string()),
                info_hash_v1: None,
                info_hash_v2: Some(
                    "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string(),
                ),
                seed_goal_seconds: Some(121),
                ..PluginDownloadRelease::default()
            },
            title: PluginDownloadTitle {
                title_id: Some("title-1".to_string()),
                title_name: "Example".to_string(),
                media_facet: "series".to_string(),
                tags: Vec::new(),
            },
            routing: PluginDownloadRouting::default(),
            torrent: Some(PluginTorrentOptions {
                source_preference: vec![
                    DownloadInputKind::MagnetUri,
                    DownloadInputKind::TorrentBytes,
                    DownloadInputKind::TorrentUrl,
                ],
                ..PluginTorrentOptions::default()
            }),
        };

        let chosen = choose_source_kind(None, &request);
        assert_eq!(chosen, Some(DownloadInputKind::MagnetUri));
        assert_eq!(
            decode_torrent_bytes(&request.source).unwrap(),
            Some(b"torrent".to_vec())
        );
        assert_eq!(
            normalize_info_hash_pair(&request.release),
            (
                Some("abcdef0123456789abcdef0123456789abcdef01".to_string()),
                Some(
                    "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string()
                )
            )
        );
        assert_eq!(
            seed_seconds_to_minutes(request.release.seed_goal_seconds),
            Some(3)
        );
    }

    #[test]
    fn torrent_helpers_choose_nzb_url_source() {
        let request = PluginDownloadClientAddRequest {
            source: PluginDownloadSource {
                kind: DownloadInputKind::NzbUrl,
                download_url: Some("https://indexer.example/release.nzb".to_string()),
                magnet_uri: None,
                torrent_bytes_base64: None,
                torrent_url: None,
                torrent_file_name: None,
                torrent_content_type: None,
                nzb_bytes_base64: None,
                nzb_file_name: Some("release.nzb".to_string()),
                nzb_content_type: Some("application/x-nzb".to_string()),
                source_title: None,
                source_password: None,
            },
            release: PluginDownloadRelease::default(),
            title: PluginDownloadTitle {
                title_id: Some("title-1".to_string()),
                title_name: "Example".to_string(),
                media_facet: "series".to_string(),
                tags: Vec::new(),
            },
            routing: PluginDownloadRouting::default(),
            torrent: None,
        };
        let capabilities = DownloadTorrentCapabilities {
            supported_sources: vec![DownloadInputKind::NzbUrl],
            ..DownloadTorrentCapabilities::default()
        };

        assert_eq!(
            choose_source_kind(Some(&capabilities), &request),
            Some(DownloadInputKind::NzbUrl)
        );
        assert_eq!(
            choose_source_kind(None, &request),
            Some(DownloadInputKind::NzbUrl)
        );
    }

    #[test]
    fn torrent_helpers_choose_nzb_bytes_source() {
        let request = PluginDownloadClientAddRequest {
            source: PluginDownloadSource {
                kind: DownloadInputKind::Nzb,
                download_url: Some("https://indexer.example/release.nzb".to_string()),
                magnet_uri: None,
                torrent_bytes_base64: None,
                torrent_url: None,
                torrent_file_name: None,
                torrent_content_type: None,
                nzb_bytes_base64: Some("bmti".to_string()),
                nzb_file_name: Some("release.nzb".to_string()),
                nzb_content_type: Some("application/x-nzb".to_string()),
                source_title: None,
                source_password: None,
            },
            release: PluginDownloadRelease::default(),
            title: PluginDownloadTitle {
                title_id: Some("title-1".to_string()),
                title_name: "Example".to_string(),
                media_facet: "series".to_string(),
                tags: Vec::new(),
            },
            routing: PluginDownloadRouting::default(),
            torrent: Some(PluginTorrentOptions {
                source_preference: vec![DownloadInputKind::Nzb, DownloadInputKind::NzbUrl],
                ..PluginTorrentOptions::default()
            }),
        };

        assert_eq!(
            choose_source_kind(None, &request),
            Some(DownloadInputKind::Nzb)
        );
    }

    #[test]
    fn torrent_helpers_do_not_treat_torrent_download_url_as_nzb_url() {
        let request = PluginDownloadClientAddRequest {
            source: PluginDownloadSource {
                kind: DownloadInputKind::TorrentUrl,
                download_url: Some("https://tracker.example/release.torrent".to_string()),
                magnet_uri: None,
                torrent_bytes_base64: None,
                torrent_url: None,
                torrent_file_name: Some("release.torrent".to_string()),
                torrent_content_type: Some("application/x-bittorrent".to_string()),
                nzb_bytes_base64: None,
                nzb_file_name: None,
                nzb_content_type: None,
                source_title: None,
                source_password: None,
            },
            release: PluginDownloadRelease::default(),
            title: PluginDownloadTitle {
                title_id: Some("title-1".to_string()),
                title_name: "Example".to_string(),
                media_facet: "series".to_string(),
                tags: Vec::new(),
            },
            routing: PluginDownloadRouting::default(),
            torrent: Some(PluginTorrentOptions {
                source_preference: vec![DownloadInputKind::NzbUrl],
                ..PluginTorrentOptions::default()
            }),
        };

        assert_eq!(
            choose_source_kind(None, &request),
            Some(DownloadInputKind::TorrentUrl)
        );
    }

    #[test]
    fn notification_capability_fixtures_round_trip() {
        let fixtures = notification_capability_fixtures();
        for descriptor in fixtures {
            let json = serde_json::to_string(&descriptor).unwrap();
            let parsed: PluginDescriptor = serde_json::from_str(&json).unwrap();
            let ProviderDescriptor::Notification(provider) = parsed.provider else {
                panic!("expected notification descriptor");
            };
            assert!(!provider.capabilities.delivery_modes.is_empty());
            assert!(!provider.capabilities.payload_formats.is_empty());
        }
    }

    #[test]
    fn indexer_capability_fixtures_round_trip() {
        let fixtures = indexer_capability_fixtures();
        for descriptor in fixtures {
            let json = serde_json::to_string(&descriptor).unwrap();
            let parsed: PluginDescriptor = serde_json::from_str(&json).unwrap();
            let ProviderDescriptor::Indexer(provider) = parsed.provider else {
                panic!("expected indexer descriptor");
            };
            assert!(!provider.capabilities.protocols.is_empty());
            assert!(!provider.capabilities.feed_modes.is_empty());
            assert!(!provider.capabilities.search_inputs.is_empty());
        }
    }

    #[test]
    fn indexer_capabilities_round_trip_supported_query_facets() {
        let capabilities = IndexerCapabilities {
            query_param: Some("q".to_string()),
            supported_query_facets: vec!["movie".to_string(), "anime".to_string()],
            ..IndexerCapabilities::default()
        };

        let json = serde_json::to_string(&capabilities).unwrap();
        assert!(json.contains("supported_query_facets"));

        let parsed: IndexerCapabilities = serde_json::from_str(&json).unwrap();
        assert_eq!(
            parsed.supported_query_facets,
            vec!["movie".to_string(), "anime".to_string()]
        );
    }

    #[test]
    fn indexer_helpers_normalize_ids_and_build_results() {
        let ids =
            normalize_external_ids([("imdb", "tt1234567"), ("tvdbid", "987"), ("aid", "18220")]);
        assert_eq!(ids.get("imdb_id"), Some(&"tt1234567".to_string()));
        assert_eq!(ids.get("tvdb_id"), Some(&"987".to_string()));
        assert_eq!(ids.get("anidb_id"), Some(&"18220".to_string()));

        let torrent = torrent_result(
            "Example.Torrent.Release",
            Some("magnet:?xt=urn:btih:abcdef0123456789abcdef0123456789abcdef01".to_string()),
        );
        assert_eq!(torrent.source_kind, Some(IndexerSourceKind::Torrent));
        assert_eq!(torrent.protocol, Some(IndexerProtocol::Torrent));

        let usenet = usenet_result(
            "Example.Usenet.Release",
            Some("https://example.invalid/release.nzb".to_string()),
        );
        assert_eq!(usenet.source_kind, Some(IndexerSourceKind::Usenet));
        assert_eq!(usenet.protocol, Some(IndexerProtocol::Usenet));
    }

    #[test]
    fn notification_helpers_emit_provider_neutral_shapes() {
        let request = sample_notification_request();

        let env = to_script_environment(&request);
        assert_eq!(
            env.get("SCRYER_NOTIFICATION_EVENT_TYPE"),
            Some(&"import_complete".to_string())
        );
        assert_eq!(
            env.get("SCRYER_TITLE_NAME"),
            Some(&"Example Show".to_string())
        );

        let webhook = to_webhook_json(&request);
        assert_eq!(webhook["event_type"], "import_complete");
        assert_eq!(webhook["title"]["name"], "Example Show");

        let embed = rich_embed_from_request(&request);
        assert_eq!(embed.title, "Import complete");
        assert_eq!(
            embed.image_url,
            Some("https://example.invalid/poster.jpg".to_string())
        );

        let second = PluginNotificationRequest {
            event_id: Some("evt-2".to_string()),
            file: Some(PluginNotificationFile {
                primary_path: Some("/library/Example Show/S01E02.mkv".to_string()),
                media_updates: vec![PluginNotificationMediaUpdate {
                    path: "/library/Example Show/S01E02.mkv".to_string(),
                    update_type: NotificationMediaUpdateType::Created,
                }],
            }),
            media_files: vec![PluginNotificationMediaFile {
                path: "/library/Example Show/S01E02.mkv".to_string(),
                ..PluginNotificationMediaFile::default()
            }],
            ..sample_notification_request()
        };
        let batches = coalesce_media_updates([&request, &second]);
        assert_eq!(batches.len(), 1);
        assert_eq!(
            batches[0].event_ids,
            vec!["evt-1".to_string(), "evt-2".to_string()]
        );
        assert_eq!(batches[0].media_files.len(), 2);
    }

    #[test]
    fn notification_media_request_payload_round_trips() {
        let request = PluginNotificationRequest {
            event_type: NotificationEventType::MediaRequestApproved,
            media_request: Some(PluginNotificationMediaRequest {
                request_id: Some("request-1".to_string()),
                library_id: Some("library-1".to_string()),
                status: Some("approved".to_string()),
                facet: Some("movie".to_string()),
                requested_quality_profile_id: Some("quality-requested".to_string()),
                requested_quality_profile_name: Some("Requested HD".to_string()),
                requested_monitor_type: None,
                approved_quality_profile_id: Some("quality-approved".to_string()),
                approved_quality_profile_name: Some("Approved HD".to_string()),
                created_title_id: Some("title-1".to_string()),
            }),
            ..sample_notification_request()
        };

        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"event_type\":\"media_request_approved\""));
        assert!(json.contains("\"media_request\""));

        let parsed: PluginNotificationRequest = serde_json::from_str(&json).unwrap();
        let media_request = parsed.media_request.expect("media request payload");
        assert_eq!(media_request.status.as_deref(), Some("approved"));
        assert_eq!(media_request.created_title_id.as_deref(), Some("title-1"));
    }

    #[test]
    fn committed_schema_matches_generated_types() {
        let schema_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("schemas/plugin-sdk-v3.schema.json");
        let expected = std::fs::read_to_string(schema_path).unwrap();
        assert_eq!(expected, plugin_sdk_schema_json());
    }

    #[test]
    fn notification_schema_stays_provider_neutral() {
        let schema = plugin_sdk_schema_json();
        assert!(!schema.contains("Sonarr"));
    }

    #[test]
    fn notification_schema_includes_media_request_events() {
        let schema = plugin_sdk_schema_json();
        for event_type in [
            "media_request_submitted",
            "media_request_approved",
            "media_request_rejected",
            "media_request_canceled",
        ] {
            assert!(schema.contains(event_type));
        }
        assert!(schema.contains("PluginNotificationMediaRequest"));
    }

    fn torrent_capability_fixtures() -> Vec<PluginDescriptor> {
        vec![
            download_fixture(
                "qbittorrent",
                DownloadTorrentCapabilities {
                    supported_sources: vec![
                        DownloadInputKind::MagnetUri,
                        DownloadInputKind::TorrentUrl,
                        DownloadInputKind::TorrentBytes,
                        DownloadInputKind::TorrentFile,
                    ],
                    preferred_sources: vec![
                        DownloadInputKind::MagnetUri,
                        DownloadInputKind::TorrentBytes,
                        DownloadInputKind::TorrentUrl,
                    ],
                    isolation_modes: vec![
                        DownloadIsolationMode::Category,
                        DownloadIsolationMode::Tag,
                        DownloadIsolationMode::Directory,
                    ],
                    post_import_isolation_modes: vec![DownloadIsolationMode::Category],
                    supports_seed_ratio_limit: true,
                    supports_seed_time_limit: true,
                    supports_start_paused: true,
                    supports_force_start: true,
                    host_fs_required: false,
                    ..DownloadTorrentCapabilities::default()
                },
            ),
            download_fixture(
                "transmission",
                DownloadTorrentCapabilities {
                    supported_sources: vec![
                        DownloadInputKind::MagnetUri,
                        DownloadInputKind::TorrentUrl,
                        DownloadInputKind::TorrentBytes,
                    ],
                    preferred_sources: vec![DownloadInputKind::MagnetUri],
                    isolation_modes: vec![
                        DownloadIsolationMode::Label,
                        DownloadIsolationMode::Directory,
                    ],
                    supports_seed_ratio_limit: true,
                    supports_seed_time_limit: true,
                    supports_start_paused: true,
                    host_fs_required: false,
                    ..DownloadTorrentCapabilities::default()
                },
            ),
            download_fixture(
                "deluge",
                DownloadTorrentCapabilities {
                    supported_sources: vec![
                        DownloadInputKind::MagnetUri,
                        DownloadInputKind::TorrentUrl,
                        DownloadInputKind::TorrentBytes,
                    ],
                    isolation_modes: vec![
                        DownloadIsolationMode::Label,
                        DownloadIsolationMode::Directory,
                    ],
                    post_import_isolation_modes: vec![DownloadIsolationMode::Label],
                    supports_seed_ratio_limit: true,
                    supports_seed_time_limit: true,
                    removes_on_seed_limit: true,
                    supports_start_paused: true,
                    ..DownloadTorrentCapabilities::default()
                },
            ),
            download_fixture(
                "rtorrent",
                DownloadTorrentCapabilities {
                    supported_sources: vec![
                        DownloadInputKind::MagnetUri,
                        DownloadInputKind::TorrentUrl,
                        DownloadInputKind::TorrentBytes,
                    ],
                    isolation_modes: vec![
                        DownloadIsolationMode::Category,
                        DownloadIsolationMode::View,
                    ],
                    post_import_isolation_modes: vec![DownloadIsolationMode::View],
                    supports_start_paused: true,
                    supports_queue_placement: true,
                    supports_priority_hint: true,
                    client_id_may_differ_from_info_hash: true,
                    ..DownloadTorrentCapabilities::default()
                },
            ),
            download_fixture(
                "flood",
                DownloadTorrentCapabilities {
                    supported_sources: vec![
                        DownloadInputKind::MagnetUri,
                        DownloadInputKind::TorrentUrl,
                        DownloadInputKind::TorrentBytes,
                    ],
                    isolation_modes: vec![
                        DownloadIsolationMode::Tag,
                        DownloadIsolationMode::Directory,
                    ],
                    post_import_isolation_modes: vec![DownloadIsolationMode::Tag],
                    reports_content_paths: true,
                    ..DownloadTorrentCapabilities::default()
                },
            ),
            download_fixture(
                "aria2",
                DownloadTorrentCapabilities {
                    supported_sources: vec![
                        DownloadInputKind::MagnetUri,
                        DownloadInputKind::TorrentUrl,
                        DownloadInputKind::TorrentBytes,
                    ],
                    reports_content_paths: true,
                    reports_metadata_only: true,
                    client_id_may_differ_from_info_hash: true,
                    ..DownloadTorrentCapabilities::default()
                },
            ),
            download_fixture(
                "freebox",
                DownloadTorrentCapabilities {
                    supported_sources: vec![
                        DownloadInputKind::MagnetUri,
                        DownloadInputKind::TorrentUrl,
                        DownloadInputKind::TorrentBytes,
                    ],
                    isolation_modes: vec![
                        DownloadIsolationMode::Category,
                        DownloadIsolationMode::Directory,
                    ],
                    supports_seed_ratio_limit: true,
                    supports_start_paused: true,
                    supports_queue_placement: true,
                    ..DownloadTorrentCapabilities::default()
                },
            ),
            download_fixture(
                "downloadstation",
                DownloadTorrentCapabilities {
                    supported_sources: vec![
                        DownloadInputKind::MagnetUri,
                        DownloadInputKind::TorrentUrl,
                        DownloadInputKind::TorrentBytes,
                    ],
                    isolation_modes: vec![
                        DownloadIsolationMode::Category,
                        DownloadIsolationMode::Directory,
                    ],
                    supports_seed_ratio_limit: true,
                    supports_start_paused: true,
                    supports_queue_placement: true,
                    ..DownloadTorrentCapabilities::default()
                },
            ),
            download_fixture(
                "rqbit",
                DownloadTorrentCapabilities {
                    supported_sources: vec![
                        DownloadInputKind::MagnetUri,
                        DownloadInputKind::TorrentUrl,
                        DownloadInputKind::TorrentBytes,
                    ],
                    reports_metadata_only: true,
                    client_id_may_differ_from_info_hash: true,
                    ..DownloadTorrentCapabilities::default()
                },
            ),
            download_fixture(
                "hadouken",
                DownloadTorrentCapabilities {
                    supported_sources: vec![
                        DownloadInputKind::MagnetUri,
                        DownloadInputKind::TorrentUrl,
                        DownloadInputKind::TorrentBytes,
                    ],
                    isolation_modes: vec![DownloadIsolationMode::Label],
                    supports_seed_ratio_limit: true,
                    supports_seed_time_limit: true,
                    ..DownloadTorrentCapabilities::default()
                },
            ),
            download_fixture(
                "tribler",
                DownloadTorrentCapabilities {
                    supported_sources: vec![DownloadInputKind::MagnetUri],
                    preferred_sources: vec![DownloadInputKind::MagnetUri],
                    supports_safe_seeding: true,
                    supports_anonymity_hops: true,
                    supports_selected_files: true,
                    reports_metadata_only: true,
                    ..DownloadTorrentCapabilities::default()
                },
            ),
            download_fixture(
                "blackhole",
                DownloadTorrentCapabilities {
                    supported_sources: vec![
                        DownloadInputKind::MagnetUri,
                        DownloadInputKind::TorrentUrl,
                        DownloadInputKind::TorrentBytes,
                        DownloadInputKind::TorrentFile,
                    ],
                    preferred_sources: vec![
                        DownloadInputKind::TorrentBytes,
                        DownloadInputKind::TorrentFile,
                    ],
                    host_fs_required: true,
                    client_id_may_differ_from_info_hash: true,
                    ..DownloadTorrentCapabilities::default()
                },
            ),
            download_fixture(
                "utorrent",
                DownloadTorrentCapabilities {
                    supported_sources: vec![
                        DownloadInputKind::MagnetUri,
                        DownloadInputKind::TorrentUrl,
                        DownloadInputKind::TorrentBytes,
                    ],
                    isolation_modes: vec![
                        DownloadIsolationMode::Label,
                        DownloadIsolationMode::Category,
                    ],
                    supports_seed_ratio_limit: true,
                    supports_seed_time_limit: true,
                    ..DownloadTorrentCapabilities::default()
                },
            ),
            download_fixture(
                "vuze",
                DownloadTorrentCapabilities {
                    supported_sources: vec![
                        DownloadInputKind::MagnetUri,
                        DownloadInputKind::TorrentUrl,
                        DownloadInputKind::TorrentBytes,
                    ],
                    isolation_modes: vec![DownloadIsolationMode::Category],
                    supports_seed_ratio_limit: true,
                    supports_seed_time_limit: true,
                    ..DownloadTorrentCapabilities::default()
                },
            ),
        ]
    }

    fn download_fixture(
        provider_type: &str,
        torrent: DownloadTorrentCapabilities,
    ) -> PluginDescriptor {
        PluginDescriptor {
            id: provider_type.to_string(),
            name: provider_type.to_string(),
            version: "0.1.0".to_string(),
            sdk_version: SDK_VERSION.to_string(),
            sdk_constraint: current_sdk_constraint(),
            socket_permissions: vec![],
            provider: ProviderDescriptor::DownloadClient(DownloadClientDescriptor {
                provider_type: provider_type.to_string(),
                provider_aliases: Vec::new(),
                config_fields: Vec::new(),
                default_base_url: None,
                allowed_hosts: Vec::new(),
                accepted_inputs: torrent.supported_sources.clone(),
                isolation_modes: torrent.isolation_modes.clone(),
                capabilities: DownloadClientCapabilities {
                    remove: true,
                    mark_imported: true,
                    host_fs_required: torrent.host_fs_required,
                    torrent: Some(torrent),
                    ..DownloadClientCapabilities::default()
                },
            }),
        }
    }

    fn notification_capability_fixtures() -> Vec<PluginDescriptor> {
        vec![
            notification_fixture(
                "generic_webhook",
                NotificationCapabilities {
                    delivery_modes: vec![NotificationDeliveryMode::Webhook],
                    payload_formats: vec![NotificationPayloadFormat::StructuredJson],
                    supports_test: true,
                    ..NotificationCapabilities::default()
                },
            ),
            notification_fixture(
                "custom_script",
                NotificationCapabilities {
                    delivery_modes: vec![NotificationDeliveryMode::CustomScript],
                    payload_formats: vec![NotificationPayloadFormat::ScriptEnvironment],
                    requires_host_process: true,
                    supports_test: true,
                    ..NotificationCapabilities::default()
                },
            ),
            notification_fixture(
                "discord_chat",
                NotificationCapabilities {
                    delivery_modes: vec![NotificationDeliveryMode::Chat],
                    payload_formats: vec![
                        NotificationPayloadFormat::StructuredJson,
                        NotificationPayloadFormat::RichEmbed,
                    ],
                    supports_rich_text: true,
                    supports_images: true,
                    supports_test: true,
                    ..NotificationCapabilities::default()
                },
            ),
            notification_fixture(
                "email",
                NotificationCapabilities {
                    delivery_modes: vec![NotificationDeliveryMode::Email],
                    payload_formats: vec![
                        NotificationPayloadFormat::PlainText,
                        NotificationPayloadFormat::Html,
                    ],
                    supports_rich_text: true,
                    ..NotificationCapabilities::default()
                },
            ),
            notification_fixture(
                "push",
                NotificationCapabilities {
                    delivery_modes: vec![NotificationDeliveryMode::Push],
                    payload_formats: vec![NotificationPayloadFormat::PlainText],
                    supports_test: true,
                    ..NotificationCapabilities::default()
                },
            ),
            notification_fixture(
                "media_server_update",
                NotificationCapabilities {
                    delivery_modes: vec![NotificationDeliveryMode::MediaServerUpdate],
                    payload_formats: vec![NotificationPayloadFormat::StructuredJson],
                    supports_batch: true,
                    supports_coalescing: true,
                    ..NotificationCapabilities::default()
                },
            ),
            notification_fixture(
                "external_sync",
                NotificationCapabilities {
                    delivery_modes: vec![NotificationDeliveryMode::ExternalSync],
                    payload_formats: vec![NotificationPayloadFormat::StructuredJson],
                    supports_test: true,
                    ..NotificationCapabilities::default()
                },
            ),
            notification_fixture(
                "aggregator",
                NotificationCapabilities {
                    delivery_modes: vec![NotificationDeliveryMode::Aggregator],
                    payload_formats: vec![NotificationPayloadFormat::StructuredJson],
                    supports_batch: true,
                    supports_test: true,
                    ..NotificationCapabilities::default()
                },
            ),
        ]
    }

    fn notification_fixture(
        provider_type: &str,
        capabilities: NotificationCapabilities,
    ) -> PluginDescriptor {
        PluginDescriptor {
            id: provider_type.to_string(),
            name: provider_type.to_string(),
            version: "0.1.0".to_string(),
            sdk_version: SDK_VERSION.to_string(),
            sdk_constraint: current_sdk_constraint(),
            socket_permissions: vec![],
            provider: ProviderDescriptor::Notification(NotificationDescriptor {
                provider_type: provider_type.to_string(),
                provider_aliases: Vec::new(),
                config_fields: Vec::new(),
                default_base_url: None,
                allowed_hosts: Vec::new(),
                capabilities,
            }),
        }
    }

    fn sample_notification_request() -> PluginNotificationRequest {
        PluginNotificationRequest {
            schema_version: NOTIFICATION_REQUEST_SCHEMA_VERSION,
            event_type: NotificationEventType::ImportComplete,
            event_id: Some("evt-1".to_string()),
            occurred_at: Some("2026-04-29T12:00:00Z".to_string()),
            correlation_id: Some("corr-1".to_string()),
            actor: Some(PluginNotificationActor {
                user_id: Some("user-1".to_string()),
            }),
            severity: Some(NotificationSeverity::Info),
            is_test: false,
            summary_title: "Import complete".to_string(),
            summary_message: "Imported Example Show.".to_string(),
            app: PluginNotificationApp {
                name: "Scryer".to_string(),
                version: "test".to_string(),
            },
            title: Some(PluginNotificationTitle {
                id: Some("title-1".to_string()),
                name: "Example Show".to_string(),
                facet: "series".to_string(),
                year: Some(2024),
                slug: Some("example-show".to_string()),
                path: Some("/library/Example Show".to_string()),
                overview: Some("Overview".to_string()),
                sort_title: Some("Example Show".to_string()),
                background_url: None,
                poster_url: Some("https://example.invalid/poster.jpg".to_string()),
                genres: vec!["Drama".to_string()],
                tags: vec!["tag-1".to_string()],
                aliases: vec!["Example Alias".to_string()],
                original_language: Some("ja".to_string()),
                original_country: Some("JP".to_string()),
                external_ids: PluginNotificationExternalIds {
                    tvdb_id: Some("tvdb-1".to_string()),
                    by_source: BTreeMap::from([("tvdb".to_string(), vec!["tvdb-1".to_string()])]),
                    ..PluginNotificationExternalIds::default()
                },
            }),
            episode: Some(PluginNotificationEpisode {
                episode_ids: vec!["episode-1".to_string()],
                display: Some("S01E01".to_string()),
                ..PluginNotificationEpisode::default()
            }),
            episodes: vec![PluginNotificationEpisode {
                id: Some("episode-1".to_string()),
                episode_ids: vec!["episode-1".to_string()],
                display: Some("S01E01".to_string()),
                season_number: Some("1".to_string()),
                episode_number: Some("1".to_string()),
                title: Some("Pilot".to_string()),
                ..PluginNotificationEpisode::default()
            }],
            release: Some(PluginNotificationRelease {
                source_title: Some("Example.Show.S01E01.1080p.WEB-DL".to_string()),
                quality: Some("1080p".to_string()),
                provider: Some("RSS".to_string()),
                ..PluginNotificationRelease::default()
            }),
            download: Some(PluginNotificationDownload {
                download_id: Some("download-1".to_string()),
                client_name: Some("qBittorrent".to_string()),
                status: Some("completed".to_string()),
                ..PluginNotificationDownload::default()
            }),
            import: Some(PluginNotificationImport {
                import_id: Some("import-1".to_string()),
                source_path: Some("/downloads/Example.Show.S01E01.mkv".to_string()),
                dest_path: Some("/library/Example Show/S01E01.mkv".to_string()),
                imported_count: Some(1),
                status: Some("completed".to_string()),
                ..PluginNotificationImport::default()
            }),
            health: None,
            file: Some(PluginNotificationFile {
                primary_path: Some("/library/Example Show/S01E01.mkv".to_string()),
                media_updates: vec![PluginNotificationMediaUpdate {
                    path: "/library/Example Show/S01E01.mkv".to_string(),
                    update_type: NotificationMediaUpdateType::Created,
                }],
            }),
            media_files: vec![PluginNotificationMediaFile {
                id: Some("file-1".to_string()),
                path: "/library/Example Show/S01E01.mkv".to_string(),
                quality: Some("1080p".to_string()),
                video_codec: Some("h264".to_string()),
                audio_codec: Some("aac".to_string()),
                ..PluginNotificationMediaFile::default()
            }],
            application_update: None,
            manual_interaction: None,
            media_request: None,
        }
    }
}

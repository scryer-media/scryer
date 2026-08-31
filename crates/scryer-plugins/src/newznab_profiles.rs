use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use chrono::NaiveDate;
use scryer_plugin_sdk::{
    IndexerDescriptor, IndexerProviderEndpointAliasStatus, IndexerProviderProfile,
    PluginNewznabAttributeValueKind, PluginNewznabCanonicalAttribute, PluginNewznabProfile,
    PluginProviderProfile,
};
use serde_json::{Map, Value};
use url::Url;

const SUPPORTED_PROFILE_SCHEMA_VERSION: u32 = 1;
const CUSTOM_PROFILE_ID: &str = "custom";
const GENERIC_REQUEST_INTERVAL_MS: u64 = 2_000;
const GENERIC_RETRY_DEFAULT_MS: u64 = 1_000;
const GENERIC_RETRY_MAX_MS: u64 = 300_000;
const GENERIC_PAGE_SIZE: u32 = 100;
const GENERIC_PAGE_CEILING: u32 = 30;

#[derive(Clone, Debug)]
struct NewznabProfileCatalog<'a> {
    descriptor: &'a IndexerDescriptor,
    profile_ids: BTreeMap<String, usize>,
    legacy_aliases: BTreeMap<String, usize>,
    compatible_endpoints: BTreeMap<String, usize>,
    invalid_endpoints: BTreeMap<String, usize>,
    invalid_origins: BTreeMap<String, usize>,
}

impl<'a> NewznabProfileCatalog<'a> {
    fn from_descriptor(descriptor: &'a IndexerDescriptor) -> Result<Self, NewznabProfileError> {
        let mut catalog = Self {
            descriptor,
            profile_ids: BTreeMap::new(),
            legacy_aliases: BTreeMap::new(),
            compatible_endpoints: BTreeMap::new(),
            invalid_endpoints: BTreeMap::new(),
            invalid_origins: BTreeMap::new(),
        };
        catalog.build_indexes()?;
        Ok(catalog)
    }

    fn profile(&self, index: usize) -> &IndexerProviderProfile {
        &self.descriptor.provider_profiles[index]
    }

    fn runtime_profile(&self, index: usize) -> &PluginNewznabProfile {
        let PluginProviderProfile::Newznab(profile) = &self.profile(index).runtime_profile;
        profile
    }

    fn resolve(
        &self,
        provider_type: &str,
        config_json: Option<&str>,
    ) -> Result<PluginNewznabProfile, NewznabProfileError> {
        let config = parse_config_json(config_json)?;
        let configured_base_url = config_string(&config, "base_url");
        let configured_api_path = config_string(&config, "api_path");
        let endpoint_candidates = configured_endpoint_candidates(
            configured_base_url.as_deref(),
            configured_api_path.as_deref(),
        )?;
        let configured_origin = configured_base_url
            .as_deref()
            .map(normalize_endpoint_origin)
            .transpose()?;

        let invalid_profile_index = endpoint_candidates
            .iter()
            .find_map(|endpoint| self.invalid_endpoints.get(endpoint).copied())
            .or_else(|| {
                configured_origin
                    .as_ref()
                    .and_then(|origin| self.invalid_origins.get(origin).copied())
            });
        if let Some(profile_index) = invalid_profile_index {
            let profile = self.runtime_profile(profile_index);
            return Err(NewznabProfileError::InvalidKnownEndpoint {
                configured: configured_base_url.clone().unwrap_or_default(),
                profile_id: profile.profile_id.clone(),
                canonical: canonical_api_endpoint(profile)?,
            });
        }

        let selected = if let Some(profile_id) = config_string(&config, "profile_id") {
            let key = normalize_provider_key(&profile_id);
            if key == CUSTOM_PROFILE_ID {
                None
            } else {
                Some(
                    *self
                        .profile_ids
                        .get(&key)
                        .ok_or(NewznabProfileError::UnknownProfileId(profile_id))?,
                )
            }
        } else if let Some(index) = self
            .legacy_aliases
            .get(&normalize_provider_key(provider_type))
        {
            Some(*index)
        } else {
            let mut matches = endpoint_candidates
                .iter()
                .filter_map(|endpoint| self.compatible_endpoints.get(endpoint).copied())
                .collect::<BTreeSet<_>>();
            if matches.len() > 1 {
                return Err(NewznabProfileError::AmbiguousEndpoint(
                    configured_base_url.unwrap_or_default(),
                ));
            }
            matches.pop_first()
        };

        self.resolve_effective(selected, &config, configured_base_url, configured_api_path)
    }

    fn resolve_effective(
        &self,
        selected: Option<usize>,
        config: &Map<String, Value>,
        configured_base_url: Option<String>,
        configured_api_path: Option<String>,
    ) -> Result<PluginNewznabProfile, NewznabProfileError> {
        let defaults = selected.map(|index| self.runtime_profile(index));
        let mut resolved = defaults.cloned().unwrap_or_else(generic_profile);

        if let Some(base_url) = configured_base_url {
            validate_base_url(&base_url, "Newznab base URL")
                .map_err(NewznabProfileError::InvalidConfig)?;
            resolved.canonical_base_url = base_url;
        }
        if let Some(api_path) = configured_api_path {
            validate_api_path(&api_path).map_err(NewznabProfileError::InvalidConfig)?;
            resolved.api_path = api_path;
        }
        if let Some(api_key_parameter) = config_string(config, "api_key_parameter") {
            validate_query_key(&api_key_parameter, "API key parameter")
                .map_err(NewznabProfileError::InvalidConfig)?;
            resolved.api_key_parameter = api_key_parameter;
        }

        if let Some(value) = config_u64(config, "request_interval_ms", true)? {
            resolved.request_interval_ms = value;
        }
        if let Some(value) = config_u32(config, "hourly_request_limit", true)? {
            resolved.hourly_limit = Some(value);
        }
        if let Some(value) = config_u32(config, "daily_request_limit", true)? {
            resolved.daily_limit = Some(value);
        }
        if let Some(value) = config_u64(config, "retry_default_delay_ms", true)? {
            resolved.retry_default_ms = value;
        }
        if let Some(value) = config_u64(config, "retry_max_delay_ms", true)? {
            resolved.retry_max_ms = value;
        }
        if let Some(value) = config_u32(config, "retry_max_attempts", true)? {
            resolved.retry_max_attempts = value;
        }
        if let Some(value) = config_u64(config, "retry_total_budget_ms", false)? {
            resolved.retry_total_budget_ms = value;
        }
        if let Some(value) = config_u32(config, "page_size", true)? {
            resolved.page_size = value;
        }
        if let Some(value) = config_u32(config, "max_pages", true)? {
            resolved.page_ceiling = Some(value);
        }

        if let Some(additional_params) = config_string(config, "additional_params") {
            for (key, value) in url::form_urlencoded::parse(
                additional_params.trim_start_matches(['?', '&']).as_bytes(),
            ) {
                if key.trim().is_empty() {
                    continue;
                }
                validate_query_key(&key, "additional request parameter")
                    .map_err(NewznabProfileError::InvalidConfig)?;
                if looks_like_secret_key(&key) {
                    return Err(NewznabProfileError::InvalidConfig(format!(
                        "Newznab additional request parameters cannot contain secret key '{key}'"
                    )));
                }
                resolved
                    .default_request_parameters
                    .insert(key.into_owned(), value.into_owned());
            }
        }

        validate_runtime_profile(
            &resolved,
            &self
                .descriptor
                .scoring_policies
                .iter()
                .map(|policy| policy.name.as_str())
                .collect(),
        )
        .map_err(NewznabProfileError::InvalidConfig)?;
        Ok(resolved)
    }

    fn build_indexes(&mut self) -> Result<(), NewznabProfileError> {
        let scoring_policy_ids = self
            .descriptor
            .scoring_policies
            .iter()
            .map(|policy| policy.name.as_str())
            .collect::<BTreeSet<_>>();

        for (index, definition) in self.descriptor.provider_profiles.iter().enumerate() {
            validate_definition(definition, &scoring_policy_ids)
                .map_err(NewznabProfileError::InvalidDescriptor)?;
            let PluginProviderProfile::Newznab(profile) = &definition.runtime_profile;
            insert_unique(
                &mut self.profile_ids,
                normalize_provider_key(&profile.profile_id),
                index,
                "profile id",
            )?;
            for alias in &definition.legacy_provider_type_aliases {
                insert_unique(
                    &mut self.legacy_aliases,
                    normalize_provider_key(alias),
                    index,
                    "legacy provider alias",
                )?;
            }

            insert_unique(
                &mut self.compatible_endpoints,
                normalize_endpoint(&canonical_api_endpoint(profile)?)?,
                index,
                "compatible endpoint",
            )?;
            for alias in &definition.endpoint_aliases {
                let endpoint = normalize_endpoint(&alias.url)?;
                match alias.status {
                    IndexerProviderEndpointAliasStatus::Compatible => insert_unique(
                        &mut self.compatible_endpoints,
                        endpoint,
                        index,
                        "compatible endpoint",
                    )?,
                    IndexerProviderEndpointAliasStatus::Invalid => {
                        insert_unique(
                            &mut self.invalid_endpoints,
                            endpoint,
                            index,
                            "invalid endpoint",
                        )?;
                        insert_unique(
                            &mut self.invalid_origins,
                            normalize_endpoint_origin(&alias.url)?,
                            index,
                            "invalid endpoint origin",
                        )?;
                    }
                }
            }
        }

        for endpoint in self.compatible_endpoints.keys() {
            if self.invalid_endpoints.contains_key(endpoint) {
                return Err(NewznabProfileError::InvalidDescriptor(format!(
                    "endpoint '{endpoint}' is declared both compatible and invalid"
                )));
            }
        }
        for (alias, profile_index) in &self.legacy_aliases {
            if let Some(id_index) = self.profile_ids.get(alias)
                && id_index != profile_index
            {
                return Err(NewznabProfileError::InvalidDescriptor(format!(
                    "legacy provider alias '{alias}' collides with another profile id"
                )));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NewznabProfileError {
    InvalidDescriptor(String),
    InvalidConfig(String),
    UnknownProfileId(String),
    InvalidKnownEndpoint {
        configured: String,
        profile_id: String,
        canonical: String,
    },
    AmbiguousEndpoint(String),
}

impl fmt::Display for NewznabProfileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDescriptor(message) | Self::InvalidConfig(message) => {
                formatter.write_str(message)
            }
            Self::UnknownProfileId(profile_id) => {
                write!(formatter, "unknown Newznab provider profile '{profile_id}'")
            }
            Self::InvalidKnownEndpoint {
                configured,
                profile_id,
                canonical,
            } => write!(
                formatter,
                "Newznab endpoint '{configured}' is not the API endpoint for profile '{profile_id}'; use '{canonical}'"
            ),
            Self::AmbiguousEndpoint(endpoint) => write!(
                formatter,
                "Newznab endpoint '{endpoint}' matches more than one provider profile"
            ),
        }
    }
}

impl std::error::Error for NewznabProfileError {}

pub fn validate_newznab_profiles(
    descriptor: &IndexerDescriptor,
) -> Result<(), NewznabProfileError> {
    NewznabProfileCatalog::from_descriptor(descriptor).map(|_| ())
}

pub fn resolve_newznab_profile(
    descriptor: &IndexerDescriptor,
    provider_type: &str,
    config_json: Option<&str>,
) -> Result<PluginNewznabProfile, NewznabProfileError> {
    NewznabProfileCatalog::from_descriptor(descriptor)?.resolve(provider_type, config_json)
}

pub fn resolve_newznab_profile_bytes(
    descriptor: &IndexerDescriptor,
    provider_type: &str,
    config_json: Option<&str>,
) -> Result<Vec<u8>, NewznabProfileError> {
    let profile = resolve_newznab_profile(descriptor, provider_type, config_json)?;
    serde_json::to_vec(&PluginProviderProfile::Newznab(profile)).map_err(|error| {
        NewznabProfileError::InvalidDescriptor(format!(
            "failed to serialize resolved Newznab profile: {error}"
        ))
    })
}

fn generic_profile() -> PluginNewznabProfile {
    PluginNewznabProfile {
        profile_id: CUSTOM_PROFILE_ID.to_string(),
        canonical_base_url: String::new(),
        api_path: "/api".to_string(),
        api_key_parameter: "apikey".to_string(),
        request_interval_ms: GENERIC_REQUEST_INTERVAL_MS,
        hourly_limit: None,
        daily_limit: None,
        retry_default_ms: GENERIC_RETRY_DEFAULT_MS,
        retry_max_ms: GENERIC_RETRY_MAX_MS,
        retry_max_attempts: 1,
        retry_total_budget_ms: 0,
        page_size: GENERIC_PAGE_SIZE,
        page_ceiling: Some(GENERIC_PAGE_CEILING),
        default_request_parameters: BTreeMap::from([("extended".to_string(), "1".to_string())]),
        allowed_response_formats: vec!["xml".to_string(), "json".to_string()],
        response_attribute_mappings: Vec::new(),
        quirks: Vec::new(),
        scoring_policy_ids: Vec::new(),
    }
}

fn validate_definition(
    definition: &IndexerProviderProfile,
    scoring_policy_ids: &BTreeSet<&str>,
) -> Result<(), String> {
    if definition.schema_version != SUPPORTED_PROFILE_SCHEMA_VERSION {
        return Err(format!(
            "provider profile uses unsupported schema version {}",
            definition.schema_version
        ));
    }
    if definition.display_name.trim().is_empty() {
        return Err("provider profile has an empty display name".to_string());
    }
    let PluginProviderProfile::Newznab(profile) = &definition.runtime_profile;
    reject_credential_marker(
        &definition.display_name,
        "display name",
        &profile.profile_id,
    )?;
    for alias in &definition.legacy_provider_type_aliases {
        validate_identifier(alias, "legacy provider alias")?;
    }
    for alias in &definition.endpoint_aliases {
        validate_base_url(&alias.url, "endpoint alias")?;
        if let Some(reason) = &alias.reason {
            reject_credential_marker(reason, "endpoint alias reason", &profile.profile_id)?;
        }
        if alias.status == IndexerProviderEndpointAliasStatus::Invalid
            && alias
                .reason
                .as_deref()
                .is_none_or(|reason| reason.trim().is_empty())
        {
            return Err(format!(
                "profile '{}' invalid endpoint '{}' requires a reason",
                profile.profile_id, alias.url
            ));
        }
    }
    validate_base_url(&definition.provenance_url, "provenance URL")?;
    reject_credential_marker(
        &definition.provenance_url,
        "provenance URL",
        &profile.profile_id,
    )?;
    NaiveDate::parse_from_str(&definition.reviewed_on, "%Y-%m-%d").map_err(|error| {
        format!(
            "profile '{}' has invalid reviewed_on date '{}': {error}",
            profile.profile_id, definition.reviewed_on
        )
    })?;
    if profile.profile_id == CUSTOM_PROFILE_ID {
        return Err("'custom' is reserved and cannot be a descriptor profile id".to_string());
    }
    validate_runtime_profile(profile, scoring_policy_ids)
}

fn validate_runtime_profile(
    profile: &PluginNewznabProfile,
    scoring_policy_ids: &BTreeSet<&str>,
) -> Result<(), String> {
    validate_identifier(&profile.profile_id, "profile id")?;
    if !profile.canonical_base_url.is_empty() {
        validate_base_url(&profile.canonical_base_url, "canonical API base URL")?;
        canonical_api_endpoint(profile).map_err(|error| error.to_string())?;
    }
    validate_api_path(&profile.api_path)?;
    validate_query_key(&profile.api_key_parameter, "authentication query parameter")?;
    if profile.request_interval_ms == 0 {
        return Err("request_interval_ms must be positive".to_string());
    }
    if profile.hourly_limit == Some(0) || profile.daily_limit == Some(0) {
        return Err("request limits must be positive when present".to_string());
    }
    if profile.retry_max_attempts == 0 {
        return Err("retry_max_attempts must be positive".to_string());
    }
    if profile.retry_max_ms < profile.retry_default_ms {
        return Err("retry_max_ms cannot be less than retry_default_ms".to_string());
    }
    if profile.page_size == 0 || profile.page_ceiling == Some(0) {
        return Err("page_size and page_ceiling must be positive".to_string());
    }

    for (key, value) in &profile.default_request_parameters {
        validate_query_key(key, "default request parameter")?;
        if looks_like_secret_key(key) {
            return Err(format!(
                "profile '{}' default request parameters cannot contain secret key '{key}'",
                profile.profile_id
            ));
        }
        if value.contains(['\r', '\n']) {
            return Err(format!(
                "profile '{}' default request parameter '{key}' contains a newline",
                profile.profile_id
            ));
        }
        reject_credential_marker(
            value,
            "default request parameter value",
            &profile.profile_id,
        )?;
    }

    let mut response_formats = BTreeSet::new();
    for format in &profile.allowed_response_formats {
        let normalized = format.trim().to_ascii_lowercase();
        if !matches!(normalized.as_str(), "xml" | "json") {
            return Err(format!(
                "profile '{}' has unsupported response format '{format}'",
                profile.profile_id
            ));
        }
        if !response_formats.insert(normalized) {
            return Err(format!(
                "profile '{}' contains duplicate response format '{format}'",
                profile.profile_id
            ));
        }
    }
    if profile.allowed_response_formats.is_empty() {
        return Err(format!(
            "profile '{}' must allow at least one response format",
            profile.profile_id
        ));
    }

    let mut canonical_fields = BTreeSet::new();
    let mut attribute_names = BTreeMap::new();
    for mapping in &profile.response_attribute_mappings {
        let valid_value_kind = match mapping.canonical_field {
            PluginNewznabCanonicalAttribute::ThumbsUp
            | PluginNewznabCanonicalAttribute::ThumbsDown
            | PluginNewznabCanonicalAttribute::Grabs
            | PluginNewznabCanonicalAttribute::Rating
            | PluginNewznabCanonicalAttribute::Comments => {
                mapping.value_kind == PluginNewznabAttributeValueKind::Integer
            }
            PluginNewznabCanonicalAttribute::Languages
            | PluginNewznabCanonicalAttribute::Subtitles
            | PluginNewznabCanonicalAttribute::Genres => matches!(
                mapping.value_kind,
                PluginNewznabAttributeValueKind::DashSeparatedList
                    | PluginNewznabAttributeValueKind::CommaSeparatedList
            ),
            PluginNewznabCanonicalAttribute::Password => {
                mapping.value_kind == PluginNewznabAttributeValueKind::PasswordMetadata
            }
        };
        if !valid_value_kind {
            return Err(format!(
                "profile '{}' maps canonical response field {:?} with incompatible value kind {:?}",
                profile.profile_id, mapping.canonical_field, mapping.value_kind
            ));
        }
        if mapping.provider_names.is_empty() {
            return Err(format!(
                "profile '{}' has a response mapping with no provider names",
                profile.profile_id
            ));
        }
        if !canonical_fields.insert(mapping.canonical_field) {
            return Err(format!(
                "profile '{}' maps canonical response field {:?} more than once",
                profile.profile_id, mapping.canonical_field
            ));
        }
        for provider_name in &mapping.provider_names {
            let normalized = normalize_attribute_name(provider_name);
            if normalized.is_empty() {
                return Err(format!(
                    "profile '{}' has an empty response attribute name",
                    profile.profile_id
                ));
            }
            if let Some(existing) =
                attribute_names.insert(normalized.clone(), mapping.canonical_field)
                && existing != mapping.canonical_field
            {
                return Err(format!(
                    "profile '{}' maps response attribute '{normalized}' to multiple fields",
                    profile.profile_id
                ));
            }
        }
    }

    let mut quirks = BTreeSet::new();
    for quirk in &profile.quirks {
        validate_identifier(quirk, "provider quirk")?;
        if !quirks.insert(quirk) {
            return Err(format!(
                "profile '{}' contains duplicate provider quirk '{quirk}'",
                profile.profile_id
            ));
        }
    }
    let mut referenced_policies = BTreeSet::new();
    for policy_id in &profile.scoring_policy_ids {
        validate_identifier(policy_id, "scoring policy id")?;
        if !referenced_policies.insert(policy_id) {
            return Err(format!(
                "profile '{}' contains duplicate scoring policy id '{policy_id}'",
                profile.profile_id
            ));
        }
        if !scoring_policy_ids.contains(policy_id.as_str()) {
            return Err(format!(
                "profile '{}' references unknown descriptor scoring policy '{policy_id}'",
                profile.profile_id
            ));
        }
    }
    Ok(())
}

fn validate_identifier(value: &str, label: &str) -> Result<(), String> {
    if value.is_empty()
        || value != value.trim()
        || value != value.to_ascii_lowercase()
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
        })
    {
        return Err(format!(
            "{label} '{value}' must contain only lowercase ASCII letters, digits, '-' or '_'"
        ));
    }
    Ok(())
}

fn validate_query_key(value: &str, label: &str) -> Result<(), String> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(format!("{label} '{value}' is invalid"));
    }
    Ok(())
}

fn validate_api_path(api_path: &str) -> Result<(), String> {
    if !api_path.starts_with('/') || api_path.contains(['?', '#', '\r', '\n']) {
        return Err(format!(
            "API path '{api_path}' must be an absolute URL path"
        ));
    }
    Ok(())
}

fn validate_base_url(raw: &str, label: &str) -> Result<(), String> {
    let url = Url::parse(raw).map_err(|error| format!("invalid {label} '{raw}': {error}"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(format!("{label} '{raw}' must be an HTTP(S) URL"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(format!("{label} '{raw}' cannot contain credentials"));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(format!(
            "{label} '{raw}' cannot contain a query or fragment"
        ));
    }
    Ok(())
}

fn insert_unique(
    index: &mut BTreeMap<String, usize>,
    key: String,
    profile_index: usize,
    label: &str,
) -> Result<(), NewznabProfileError> {
    if let Some(existing) = index.insert(key.clone(), profile_index) {
        return Err(NewznabProfileError::InvalidDescriptor(format!(
            "duplicate {label} '{key}' in profiles {existing} and {profile_index}"
        )));
    }
    Ok(())
}

fn normalize_provider_key(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn normalize_attribute_name(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn looks_like_secret_key(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().replace(['-', '.'], "_").as_str(),
        "api_key" | "apikey" | "password" | "passwd" | "secret" | "token" | "access_token"
    )
}

fn reject_credential_marker(value: &str, label: &str, profile_id: &str) -> Result<(), String> {
    let normalized = value.to_ascii_lowercase().replace(char::is_whitespace, "");
    if [
        "apikey=",
        "api_key=",
        "token=",
        "access_token=",
        "password=",
        "passwd=",
        "secret=",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
    {
        return Err(format!(
            "profile '{profile_id}' {label} cannot contain credential material"
        ));
    }
    Ok(())
}

fn parse_config_json(raw: Option<&str>) -> Result<Map<String, Value>, NewznabProfileError> {
    let Some(raw) = raw.map(str::trim).filter(|raw| !raw.is_empty()) else {
        return Ok(Map::new());
    };
    serde_json::from_str::<Value>(raw)
        .map_err(|error| {
            NewznabProfileError::InvalidConfig(format!("invalid Newznab config JSON: {error}"))
        })?
        .as_object()
        .cloned()
        .ok_or_else(|| {
            NewznabProfileError::InvalidConfig("Newznab config JSON must be an object".to_string())
        })
}

fn config_string(config: &Map<String, Value>, key: &str) -> Option<String> {
    config
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn config_u64(
    config: &Map<String, Value>,
    key: &str,
    positive: bool,
) -> Result<Option<u64>, NewznabProfileError> {
    let Some(value) = config.get(key) else {
        return Ok(None);
    };
    let parsed = match value {
        Value::Number(value) => value.as_u64(),
        Value::String(value) => value.trim().parse::<u64>().ok(),
        _ => None,
    }
    .filter(|value| !positive || *value > 0)
    .ok_or_else(|| {
        let requirement = if positive {
            "a positive integer"
        } else {
            "a non-negative integer"
        };
        NewznabProfileError::InvalidConfig(format!(
            "Newznab config field '{key}' must be {requirement}"
        ))
    })?;
    Ok(Some(parsed))
}

fn config_u32(
    config: &Map<String, Value>,
    key: &str,
    positive: bool,
) -> Result<Option<u32>, NewznabProfileError> {
    config_u64(config, key, positive)?
        .map(|value| {
            u32::try_from(value).map_err(|_| {
                NewznabProfileError::InvalidConfig(format!(
                    "Newznab config field '{key}' exceeds the supported range"
                ))
            })
        })
        .transpose()
}

fn canonical_api_endpoint(profile: &PluginNewznabProfile) -> Result<String, NewznabProfileError> {
    join_api_endpoint(&profile.canonical_base_url, &profile.api_path)
}

fn join_api_endpoint(base_url: &str, api_path: &str) -> Result<String, NewznabProfileError> {
    validate_api_path(api_path).map_err(NewznabProfileError::InvalidConfig)?;
    let mut url = Url::parse(base_url).map_err(|error| {
        NewznabProfileError::InvalidConfig(format!(
            "invalid Newznab base URL '{base_url}': {error}"
        ))
    })?;
    let current_path = normalize_url_path(url.path());
    let api_path = normalize_url_path(api_path);
    if current_path != api_path && !current_path.ends_with(&api_path) {
        let combined = if current_path == "/" {
            api_path
        } else {
            format!(
                "{}/{}",
                current_path.trim_end_matches('/'),
                api_path.trim_start_matches('/')
            )
        };
        url.set_path(&combined);
    }
    Ok(url.to_string())
}

fn configured_endpoint_candidates(
    base_url: Option<&str>,
    api_path: Option<&str>,
) -> Result<Vec<String>, NewznabProfileError> {
    let Some(base_url) = base_url else {
        return Ok(Vec::new());
    };
    let mut candidates = BTreeSet::from([normalize_endpoint(base_url)?]);
    let endpoint = join_api_endpoint(base_url, api_path.unwrap_or("/api"))?;
    candidates.insert(normalize_endpoint(&endpoint)?);
    Ok(candidates.into_iter().collect())
}

fn normalize_endpoint(raw: &str) -> Result<String, NewznabProfileError> {
    validate_base_url(raw, "Newznab endpoint").map_err(NewznabProfileError::InvalidConfig)?;
    let url = Url::parse(raw).map_err(|error| {
        NewznabProfileError::InvalidConfig(format!("invalid Newznab endpoint '{raw}': {error}"))
    })?;
    Ok(format!(
        "{}{}",
        normalized_url_origin(&url),
        normalize_url_path(url.path())
    ))
}

fn normalize_endpoint_origin(raw: &str) -> Result<String, NewznabProfileError> {
    validate_base_url(raw, "Newznab endpoint").map_err(NewznabProfileError::InvalidConfig)?;
    Url::parse(raw)
        .map(|url| normalized_url_origin(&url))
        .map_err(|error| {
            NewznabProfileError::InvalidConfig(format!("invalid Newznab endpoint '{raw}': {error}"))
        })
}

fn normalized_url_origin(url: &Url) -> String {
    let scheme = url.scheme().to_ascii_lowercase();
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    let port = match (scheme.as_str(), url.port()) {
        ("http", Some(80)) | ("https", Some(443)) | (_, None) => String::new(),
        (_, Some(port)) => format!(":{port}"),
    };
    format!("{scheme}://{host}{port}")
}

fn normalize_url_path(path: &str) -> String {
    let segments = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    if segments.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", segments.join("/"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::OnceLock;

    use scryer_application::PluginDescriptorLoader;
    use scryer_plugin_sdk::{PluginDescriptor, ProviderDescriptor};

    fn descriptor() -> PluginDescriptor {
        static DESCRIPTOR: OnceLock<PluginDescriptor> = OnceLock::new();
        DESCRIPTOR
            .get_or_init(|| {
                let wasm = crate::builtins::decode_builtin_wasm(crate::builtins::NEWZNAB)
                    .expect("materialized Newznab component");
                let descriptor = crate::WasmPluginDescriptorLoader
                    .load_descriptor_from_wasm_bytes(&wasm)
                    .expect("Newznab component descriptor");
                assert_eq!(descriptor.id, "newznab");
                descriptor
            })
            .clone()
    }

    fn indexer_descriptor(descriptor: &PluginDescriptor) -> &IndexerDescriptor {
        let ProviderDescriptor::Indexer(indexer) = &descriptor.provider else {
            panic!("expected indexer descriptor");
        };
        indexer
    }

    #[test]
    fn descriptor_profiles_validate_with_provenance_and_policy_references() {
        let descriptor = descriptor();
        let indexer = indexer_descriptor(&descriptor);
        validate_newznab_profiles(indexer).expect("descriptor profiles");
        assert!(!indexer.provider_profiles.is_empty());
        assert!(indexer.provider_profiles.iter().all(|profile| {
            !profile.provenance_url.is_empty()
                && NaiveDate::parse_from_str(&profile.reviewed_on, "%Y-%m-%d").is_ok()
        }));
    }

    #[test]
    fn resolution_precedence_is_explicit_alias_endpoint_then_custom() {
        let descriptor = descriptor();
        let indexer = indexer_descriptor(&descriptor);
        let second_definition = indexer
            .provider_profiles
            .iter()
            .find(|definition| !definition.legacy_provider_type_aliases.is_empty())
            .expect("profile with a legacy alias");
        let PluginProviderProfile::Newznab(second) = &second_definition.runtime_profile;
        let first = indexer
            .provider_profiles
            .iter()
            .find_map(|definition| match &definition.runtime_profile {
                PluginProviderProfile::Newznab(profile)
                    if profile.profile_id != second.profile_id =>
                {
                    Some(profile)
                }
                _ => None,
            })
            .expect("second profile");
        let second_alias = second_definition
            .legacy_provider_type_aliases
            .first()
            .expect("legacy alias");
        let explicit_config = serde_json::json!({
            "profile_id": first.profile_id,
            "base_url": "https://custom.example"
        })
        .to_string();
        let explicit = resolve_newznab_profile(indexer, second_alias, Some(&explicit_config))
            .expect("explicit profile");
        assert_eq!(explicit.profile_id, first.profile_id);
        assert_eq!(explicit.canonical_base_url, "https://custom.example");

        let alias = resolve_newznab_profile(indexer, second_alias, None).expect("legacy alias");
        assert_eq!(alias.profile_id, second.profile_id);

        let endpoint_config = serde_json::json!({
            "base_url": format!("{}/", first.canonical_base_url.trim_end_matches('/')),
            "api_path": format!("{}/", first.api_path.trim_end_matches('/'))
        })
        .to_string();
        let endpoint =
            resolve_newznab_profile(indexer, &indexer.provider_type, Some(&endpoint_config))
                .expect("normalized endpoint");
        assert_eq!(endpoint.profile_id, first.profile_id);

        let custom = resolve_newznab_profile(
            indexer,
            &indexer.provider_type,
            Some(r#"{"base_url":"https://custom.example/nab","api_path":"/nab"}"#),
        )
        .expect("custom profile");
        assert_eq!(custom.profile_id, CUSTOM_PROFILE_ID);
    }

    #[test]
    fn explicit_values_override_descriptor_defaults_and_merge_parameters() {
        let descriptor = descriptor();
        let indexer = indexer_descriptor(&descriptor);
        let PluginProviderProfile::Newznab(profile) = &indexer.provider_profiles[0].runtime_profile;
        let config = serde_json::json!({
            "profile_id": profile.profile_id,
            "base_url": "https://proxy.example",
            "api_path": "/nabapi",
            "request_interval_ms": "750",
            "page_size": 50,
            "max_pages": 4,
            "additional_params": "extended=0&attrs=poster"
        })
        .to_string();
        let resolved = resolve_newznab_profile(indexer, &indexer.provider_type, Some(&config))
            .expect("profile");
        assert_eq!(resolved.canonical_base_url, "https://proxy.example");
        assert_eq!(resolved.api_path, "/nabapi");
        assert_eq!(resolved.request_interval_ms, 750);
        assert_eq!(resolved.page_size, 50);
        assert_eq!(resolved.page_ceiling, Some(4));
        assert_eq!(
            resolved
                .default_request_parameters
                .get("extended")
                .map(String::as_str),
            Some("0")
        );
        assert_eq!(
            resolved
                .default_request_parameters
                .get("attrs")
                .map(String::as_str),
            Some("poster")
        );
    }

    #[test]
    fn invalid_web_endpoint_names_the_canonical_api_endpoint() {
        let descriptor = descriptor();
        let indexer = indexer_descriptor(&descriptor);
        let definition = indexer
            .provider_profiles
            .iter()
            .find(|definition| {
                definition
                    .endpoint_aliases
                    .iter()
                    .any(|alias| alias.status == IndexerProviderEndpointAliasStatus::Invalid)
            })
            .expect("profile with invalid endpoint alias");
        let alias = definition
            .endpoint_aliases
            .iter()
            .find(|alias| alias.status == IndexerProviderEndpointAliasStatus::Invalid)
            .expect("invalid endpoint alias");
        let PluginProviderProfile::Newznab(profile) = &definition.runtime_profile;
        let config = serde_json::json!({"base_url": alias.url}).to_string();
        let error = resolve_newznab_profile(indexer, &indexer.provider_type, Some(&config))
            .expect_err("known invalid endpoint");
        assert!(matches!(
            error,
            NewznabProfileError::InvalidKnownEndpoint { .. }
        ));
        assert!(
            error
                .to_string()
                .contains(&canonical_api_endpoint(profile).expect("canonical endpoint"))
        );
    }

    #[test]
    fn component_profile_bytes_are_typed_and_credential_free() {
        let descriptor = descriptor();
        let indexer = indexer_descriptor(&descriptor);
        let PluginProviderProfile::Newznab(expected) =
            &indexer.provider_profiles[0].runtime_profile;
        let bytes = resolve_newznab_profile_bytes(
            indexer,
            &expected.profile_id,
            Some(
                &serde_json::json!({
                    "profile_id": expected.profile_id,
                    "api_key": "do-not-copy",
                    "additional_params": "attrs=poster"
                })
                .to_string(),
            ),
        )
        .expect("component profile");
        let payload = String::from_utf8(bytes).expect("UTF-8");
        assert!(!payload.contains("do-not-copy"));
        let PluginProviderProfile::Newznab(profile) =
            serde_json::from_str(&payload).expect("typed provider profile");
        assert_eq!(profile.profile_id, expected.profile_id);
        assert_eq!(
            profile.response_attribute_mappings.len(),
            expected.response_attribute_mappings.len()
        );
    }

    #[test]
    fn selector_options_are_descriptor_driven() {
        let descriptor = descriptor();
        let indexer = indexer_descriptor(&descriptor);
        let selector = indexer
            .config_fields
            .iter()
            .find(|field| field.key == "profile_id")
            .expect("profile selector");
        assert_eq!(selector.options.len(), indexer.provider_profiles.len() + 1);
        for definition in &indexer.provider_profiles {
            let PluginProviderProfile::Newznab(profile) = &definition.runtime_profile;
            let option = selector
                .options
                .iter()
                .find(|option| option.value == profile.profile_id)
                .expect("profile option");
            assert_eq!(option.label, definition.display_name);
        }
    }

    #[test]
    fn descriptor_rejects_secrets_and_unknown_scoring_policies() {
        let mut descriptor = descriptor();
        let indexer = match &mut descriptor.provider {
            ProviderDescriptor::Indexer(indexer) => indexer,
            _ => panic!("expected indexer"),
        };
        let PluginProviderProfile::Newznab(profile) =
            &mut indexer.provider_profiles[0].runtime_profile;
        profile
            .default_request_parameters
            .insert("apikey".to_string(), "secret".to_string());
        assert!(validate_newznab_profiles(indexer).is_err());

        let PluginProviderProfile::Newznab(profile) =
            &mut indexer.provider_profiles[0].runtime_profile;
        profile.default_request_parameters.remove("apikey");
        profile.scoring_policy_ids = vec!["missing_policy".to_string()];
        assert!(validate_newznab_profiles(indexer).is_err());
    }
}

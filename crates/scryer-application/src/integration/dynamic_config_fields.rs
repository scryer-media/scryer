//! Config fields a plugin adds for the values entered so far.
//!
//! A plugin opts in by naming trigger keys in its descriptor. The host sends it
//! the current values of those keys through the reserved `config_fields`
//! action, validates what comes back as strictly as a descriptor's static
//! fields, and merges it after them. Saved values of these fields are ordinary
//! config entries; every path that reads or writes them (the form query, the
//! connection probe, save normalization, and the payload redaction) works from
//! the same merged list, so a dynamic password field is restored and redacted
//! exactly like a static one.

use super::*;
use std::collections::{BTreeMap, HashMap, HashSet};

/// Answers kept per process. Each entry is one (plugin build, trigger values)
/// pair; a full table is dropped wholesale rather than tracked for recency.
const MAX_CACHED_ANSWERS: usize = 512;
const MAX_DYNAMIC_FIELDS: usize = 256;
const MAX_DYNAMIC_FIELD_KEY_CHARS: usize = 128;

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct CacheKey {
    provider_type: String,
    plugin_version: Option<String>,
    plugin_sdk_version: Option<String>,
    values: BTreeMap<String, String>,
}

/// Validated answers, keyed by plugin build and trigger values. Errors are
/// never cached, so a plugin that failed once is asked again next time.
#[derive(Default)]
pub struct DynamicConfigFieldsCache {
    answers: std::sync::Mutex<HashMap<CacheKey, PluginDynamicConfigFields>>,
}

impl DynamicConfigFieldsCache {
    fn get(&self, key: &CacheKey) -> Option<PluginDynamicConfigFields> {
        self.answers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(key)
            .cloned()
    }

    fn insert(&self, key: CacheKey, value: PluginDynamicConfigFields) {
        let mut answers = self
            .answers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if answers.len() >= MAX_CACHED_ANSWERS && !answers.contains_key(&key) {
            answers.clear();
        }
        answers.insert(key, value);
    }
}

impl AppUseCase {
    /// The plugin's trigger keys that name a static, non-secret field. Empty
    /// when the provider declares no dynamic config fields.
    pub fn indexer_dynamic_config_trigger_keys(&self, provider_type: &str) -> Vec<String> {
        let normalized = provider_type.trim().to_lowercase();
        let Some(provider) = self.services.integrations.plugin_provider.available() else {
            return Vec::new();
        };
        let declared = provider.dynamic_config_trigger_keys_for_provider(&normalized);
        if declared.is_empty() {
            return Vec::new();
        }
        let fields = provider.config_fields_for_provider(&normalized);
        let mut keys = Vec::new();
        for key in declared {
            let key = key.trim().to_string();
            let names_plain_static_field = fields.iter().any(|field| {
                field.key == key && field.field_type != scryer_domain::ConfigFieldType::Password
            });
            if names_plain_static_field && !keys.contains(&key) {
                keys.push(key);
            }
        }
        keys
    }

    /// The config fields the plugin adds for `values` and the static fields it
    /// hides, for the settings form. Empty when the plugin declares no dynamic
    /// fields, answers `unsupported`, or is blocked on this Scryer.
    pub async fn indexer_dynamic_config_fields(
        &self,
        actor: &User,
        provider_type: &str,
        values: &BTreeMap<String, String>,
    ) -> AppResult<PluginDynamicConfigFields> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let normalized = provider_type.trim().to_lowercase();
        let static_fields = self.indexer_config_fields_for_provider_type(&normalized)?;
        let trigger_keys = self.indexer_dynamic_config_trigger_keys(&normalized);
        if trigger_keys.is_empty() {
            return Ok(PluginDynamicConfigFields::default());
        }
        let trigger_values = trigger_values_from_map(&trigger_keys, &static_fields, values);
        self.resolve_dynamic_config_fields(
            &normalized,
            &static_fields,
            &trigger_keys,
            trigger_values,
        )
        .await
    }

    /// The fields that govern a stored or submitted indexer config: the static
    /// fields followed by the plugin's dynamic fields for the trigger values in
    /// `sources` (earlier sources win). Static fields the plugin hides are
    /// never required.
    ///
    /// When the plugin cannot be asked, every key in `sources` that names no
    /// static field is treated as an optional secret, so a stored dynamic
    /// secret is still redacted and kept rather than shown or dropped.
    pub async fn effective_indexer_config_fields(
        &self,
        provider_type: &str,
        sources: &[Option<&str>],
    ) -> AppResult<Vec<scryer_domain::ConfigFieldDef>> {
        let normalized = provider_type.trim().to_lowercase();
        let static_fields = self.indexer_config_fields_for_provider_type(&normalized)?;
        let trigger_keys = self.indexer_dynamic_config_trigger_keys(&normalized);
        if trigger_keys.is_empty() {
            return Ok(static_fields);
        }
        let objects = sources
            .iter()
            .map(|source| parse_config_object(*source))
            .collect::<Vec<_>>();
        let mut trigger_values = BTreeMap::new();
        for key in &trigger_keys {
            if let Some(value) = objects
                .iter()
                .find_map(|object| object.get(key).and_then(config_value_as_trimmed_string))
            {
                trigger_values.insert(key.clone(), value);
            } else if let Some(default_value) = static_default(&static_fields, key) {
                trigger_values.insert(key.clone(), default_value);
            }
        }

        match self
            .resolve_dynamic_config_fields(
                &normalized,
                &static_fields,
                &trigger_keys,
                trigger_values,
            )
            .await
        {
            Ok(dynamic) => Ok(merge_dynamic_config_fields(static_fields, &dynamic)),
            Err(error) => {
                tracing::warn!(
                    provider_type = normalized.as_str(),
                    error = %error,
                    "plugin config fields unavailable; unknown config keys are treated as secrets"
                );
                Ok(with_unknown_keys_as_secrets(static_fields, &objects))
            }
        }
    }

    /// Effective fields for a stored indexer config, for payloads that redact
    /// its secrets. Unknown providers yield no fields, as before.
    pub async fn indexer_config_fields_for_config(
        &self,
        config: &IndexerConfig,
    ) -> Vec<scryer_domain::ConfigFieldDef> {
        self.effective_indexer_config_fields(
            &config.provider_type,
            &[config.config_json.as_deref()],
        )
        .await
        .unwrap_or_default()
    }

    async fn resolve_dynamic_config_fields(
        &self,
        provider_type: &str,
        static_fields: &[scryer_domain::ConfigFieldDef],
        trigger_keys: &[String],
        trigger_values: BTreeMap<String, String>,
    ) -> AppResult<PluginDynamicConfigFields> {
        if self
            .plugin_component_blocker_for_provider(provider_type)
            .await
            .is_some()
        {
            return Ok(PluginDynamicConfigFields::default());
        }
        let provider = self
            .services
            .integrations
            .plugin_provider
            .available()
            .ok_or_else(|| AppError::Validation("indexer provider is unavailable".into()))?;
        let cache_key = CacheKey {
            provider_type: provider_type.to_string(),
            plugin_version: provider.plugin_version_for_provider(provider_type),
            plugin_sdk_version: provider.plugin_sdk_version_for_provider(provider_type),
            values: trigger_values,
        };
        let cache = &self.runtime.plugins.dynamic_config_fields_cache;
        if let Some(cached) = cache.get(&cache_key) {
            return Ok(cached);
        }

        // The plugin sees only its trigger values: no secrets, no connection
        // URL, and no network grants derived from a half-typed form. The id is
        // the probe's synthetic one, so nothing can persist against it.
        let now = chrono::Utc::now();
        let config_json = serde_json::to_string(&cache_key.values)
            .map_err(|error| AppError::Repository(error.to_string()))?;
        let temp_config = IndexerConfig {
            id: crate::CONNECTION_TEST_INDEXER_ID.to_string(),
            name: "Config fields".to_string(),
            provider_type: provider_type.to_string(),
            base_url: String::new(),
            api_key_encrypted: None,
            rate_limit_seconds: None,
            rate_limit_burst: None,
            max_queries_per_minute: None,
            is_enabled: true,
            enable_interactive_search: true,
            enable_auto_search: true,
            disabled_until: None,
            proxy_config_id: None,
            download_client_id: None,
            seeding_profile_id: None,
            managed_parent_config_id: None,
            managed_child_key: None,
            managed_metadata_json: None,
            caps_snapshot_json: None,
            last_health_status: None,
            last_error_message: None,
            last_error_at: None,
            config_json: Some(config_json),
            created_at: now,
            updated_at: now,
        };
        let client = provider
            .client_for_provider_with_accounting(&temp_config, None, None)
            .ok_or_else(|| {
                AppError::Repository(format!(
                    "indexer plugin '{provider_type}' could not be started"
                ))
            })?;
        let answer = client
            .dynamic_config_fields(&cache_key.values)
            .await?
            .unwrap_or_default();
        let validated = validate_dynamic_config_fields(static_fields, trigger_keys, answer)?;
        cache.insert(cache_key, validated.clone());
        Ok(validated)
    }
}

fn parse_config_object(raw: Option<&str>) -> serde_json::Map<String, serde_json::Value> {
    raw.map(str::trim)
        .filter(|raw| !raw.is_empty())
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .and_then(|value| match value {
            serde_json::Value::Object(object) => Some(object),
            _ => None,
        })
        .unwrap_or_default()
}

fn config_value_as_trimmed_string(value: &serde_json::Value) -> Option<String> {
    let value = match value {
        serde_json::Value::String(value) => value.trim().to_string(),
        serde_json::Value::Bool(value) => value.to_string(),
        serde_json::Value::Number(value) => value.to_string(),
        _ => return None,
    };
    (!value.is_empty()).then_some(value)
}

fn static_default(fields: &[scryer_domain::ConfigFieldDef], key: &str) -> Option<String> {
    fields
        .iter()
        .find(|field| field.key == key)
        .and_then(|field| field.default_value.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn trigger_values_from_map(
    trigger_keys: &[String],
    static_fields: &[scryer_domain::ConfigFieldDef],
    values: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    trigger_keys
        .iter()
        .filter_map(|key| {
            values
                .get(key)
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .or_else(|| static_default(static_fields, key))
                .map(|value| (key.clone(), value))
        })
        .collect()
}

fn invalid(message: impl std::fmt::Display) -> AppError {
    AppError::Validation(format!("plugin returned invalid config fields: {message}"))
}

/// Hold a plugin's answer to the rules static fields obey, plus the ones that
/// keep it from reshaping the form underneath the operator. A broken answer is
/// refused whole rather than partially applied.
pub(crate) fn validate_dynamic_config_fields(
    static_fields: &[scryer_domain::ConfigFieldDef],
    trigger_keys: &[String],
    answer: PluginDynamicConfigFields,
) -> AppResult<PluginDynamicConfigFields> {
    use scryer_domain::{ConfigFieldType, ConfigFieldValueSource};

    if answer.fields.len() > MAX_DYNAMIC_FIELDS {
        return Err(invalid(format!(
            "{} fields exceeds the limit of {MAX_DYNAMIC_FIELDS}",
            answer.fields.len()
        )));
    }
    let static_keys = static_fields
        .iter()
        .map(|field| field.key.as_str())
        .collect::<HashSet<_>>();
    let mut fields = Vec::with_capacity(answer.fields.len());
    let mut dynamic_keys = HashSet::new();
    for mut field in answer.fields {
        field.key = field.key.trim().to_string();
        let key = field.key.clone();
        if key.is_empty() {
            return Err(invalid("a field has an empty key"));
        }
        if key.chars().count() > MAX_DYNAMIC_FIELD_KEY_CHARS {
            return Err(invalid(format!("field key '{key}' is too long")));
        }
        if static_keys.contains(key.as_str()) {
            return Err(invalid(format!("field '{key}' reuses a static field key")));
        }
        if !dynamic_keys.insert(key.clone()) {
            return Err(invalid(format!("field '{key}' is declared twice")));
        }
        if field.label.trim().is_empty() {
            return Err(invalid(format!("field '{key}' has no label")));
        }
        if field.value_source != ConfigFieldValueSource::User || field.host_binding.is_some() {
            return Err(invalid(format!("field '{key}' asks for a host binding")));
        }
        if field.role.is_some() {
            return Err(invalid(format!("field '{key}' claims a field role")));
        }
        let is_select = matches!(
            field.field_type,
            ConfigFieldType::Select | ConfigFieldType::FilteredSelect
        );
        if is_select && field.options.is_empty() {
            return Err(invalid(format!("select field '{key}' has no options")));
        }
        // Guidance text is not a setting: nothing may make it take a value.
        if field.field_type == ConfigFieldType::Info
            && (field.required
                || field.required_when.is_some()
                || !field.options.is_empty()
                || field
                    .default_value
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty()))
        {
            return Err(invalid(format!("info field '{key}' asks for a value")));
        }
        if let Some(default_value) = field.default_value.as_deref().map(str::trim)
            && !default_value.is_empty()
        {
            let valid = match field.field_type {
                ConfigFieldType::Bool => matches!(default_value, "true" | "false"),
                ConfigFieldType::Number => default_value
                    .parse::<f64>()
                    .is_ok_and(|value| value.is_finite()),
                ConfigFieldType::Select | ConfigFieldType::FilteredSelect => field
                    .options
                    .iter()
                    .any(|option| option.value == default_value),
                _ => true,
            };
            if !valid {
                return Err(invalid(format!(
                    "field '{key}' has an invalid default value"
                )));
            }
        }
        fields.push(field);
    }

    let known = |key: &str| static_keys.contains(key) || dynamic_keys.contains(key);
    for field in &fields {
        for condition in [field.visible_when.as_ref(), field.required_when.as_ref()]
            .into_iter()
            .flatten()
        {
            if !known(condition.key.as_str()) {
                return Err(invalid(format!(
                    "field '{}' depends on unknown field '{}'",
                    field.key, condition.key
                )));
            }
        }
        for option in &field.options {
            for override_key in option.config_overrides.keys() {
                if !known(override_key) || trigger_keys.contains(override_key) {
                    return Err(invalid(format!(
                        "field '{}' option '{}' overrides '{override_key}'",
                        field.key, option.value
                    )));
                }
            }
        }
    }

    let mut hidden_field_keys = Vec::new();
    for key in answer.hidden_field_keys {
        let key = key.trim().to_string();
        if !static_keys.contains(key.as_str()) {
            return Err(invalid(format!(
                "hidden field '{key}' is not a static field"
            )));
        }
        if trigger_keys.contains(&key) {
            return Err(invalid(format!("trigger field '{key}' cannot be hidden")));
        }
        if !hidden_field_keys.contains(&key) {
            hidden_field_keys.push(key);
        }
    }

    Ok(PluginDynamicConfigFields {
        fields,
        hidden_field_keys,
    })
}

/// Static fields first, hidden ones made optional, then the dynamic fields.
pub(crate) fn merge_dynamic_config_fields(
    static_fields: Vec<scryer_domain::ConfigFieldDef>,
    dynamic: &PluginDynamicConfigFields,
) -> Vec<scryer_domain::ConfigFieldDef> {
    let mut merged = static_fields
        .into_iter()
        .map(|mut field| {
            if dynamic.hidden_field_keys.contains(&field.key) {
                field.required = false;
                field.required_when = None;
            }
            field
        })
        .collect::<Vec<_>>();
    merged.extend(dynamic.fields.iter().cloned());
    merged
}

fn with_unknown_keys_as_secrets(
    mut static_fields: Vec<scryer_domain::ConfigFieldDef>,
    objects: &[serde_json::Map<String, serde_json::Value>],
) -> Vec<scryer_domain::ConfigFieldDef> {
    let mut unknown = objects
        .iter()
        .flat_map(|object| object.keys())
        .filter(|key| !static_fields.iter().any(|field| &field.key == *key))
        .cloned()
        .collect::<Vec<_>>();
    unknown.sort();
    unknown.dedup();
    static_fields.extend(
        unknown
            .into_iter()
            .map(|key| scryer_domain::ConfigFieldDef {
                label: key.clone(),
                key,
                field_type: scryer_domain::ConfigFieldType::Password,
                ..Default::default()
            }),
    );
    static_fields
}

#[cfg(test)]
mod tests {
    use super::*;
    use scryer_domain::{
        ConditionOp, ConfigFieldDef, ConfigFieldOption, ConfigFieldType, FieldCondition,
    };

    fn field(key: &str, field_type: ConfigFieldType) -> ConfigFieldDef {
        ConfigFieldDef {
            key: key.into(),
            label: key.into(),
            field_type,
            ..Default::default()
        }
    }

    fn statics() -> Vec<ConfigFieldDef> {
        vec![
            ConfigFieldDef {
                required: true,
                ..field("definition", ConfigFieldType::String)
            },
            ConfigFieldDef {
                required: true,
                required_when: Some(FieldCondition {
                    key: "definition".into(),
                    op: ConditionOp::NonEmpty,
                    values: vec![],
                }),
                ..field("username", ConfigFieldType::String)
            },
        ]
    }

    fn triggers() -> Vec<String> {
        vec!["definition".into()]
    }

    fn validate(
        fields: Vec<ConfigFieldDef>,
        hidden: &[&str],
    ) -> AppResult<PluginDynamicConfigFields> {
        validate_dynamic_config_fields(
            &statics(),
            &triggers(),
            PluginDynamicConfigFields {
                fields,
                hidden_field_keys: hidden.iter().map(|key| key.to_string()).collect(),
            },
        )
    }

    fn select(key: &str, options: &[&str]) -> ConfigFieldDef {
        ConfigFieldDef {
            options: options
                .iter()
                .map(|value| ConfigFieldOption {
                    value: value.to_string(),
                    label: value.to_string(),
                    config_overrides: Default::default(),
                })
                .collect(),
            ..field(key, ConfigFieldType::Select)
        }
    }

    fn rejection(result: AppResult<PluginDynamicConfigFields>) -> String {
        result.expect_err("answer should be refused").to_string()
    }

    #[test]
    fn valid_answer_is_trimmed_and_hidden_keys_deduped() {
        let answer = validate(
            vec![
                ConfigFieldDef {
                    key: " cookie ".into(),
                    ..field("cookie", ConfigFieldType::Password)
                },
                ConfigFieldDef {
                    default_value: Some("new".into()),
                    visible_when: Some(FieldCondition {
                        key: "cookie".into(),
                        op: ConditionOp::NonEmpty,
                        values: vec![],
                    }),
                    ..select("sort", &["new", "old"])
                },
            ],
            &["username", " username"],
        )
        .unwrap();

        assert_eq!(answer.fields[0].key, "cookie");
        assert_eq!(answer.hidden_field_keys, vec!["username".to_string()]);
    }

    #[test]
    fn info_fields_are_accepted_only_without_a_value() {
        let guidance = ConfigFieldDef {
            help_text: Some("Read this first.".into()),
            ..field("info_activity", ConfigFieldType::Info)
        };
        let answer = validate(vec![guidance.clone()], &[]).unwrap();
        assert_eq!(answer.fields[0].field_type, ConfigFieldType::Info);

        for valued in [
            ConfigFieldDef {
                required: true,
                ..guidance.clone()
            },
            ConfigFieldDef {
                default_value: Some("text".into()),
                ..guidance.clone()
            },
            ConfigFieldDef {
                required_when: Some(FieldCondition {
                    key: "definition".into(),
                    op: ConditionOp::NonEmpty,
                    values: vec![],
                }),
                ..guidance.clone()
            },
            ConfigFieldDef {
                options: select("x", &["a"]).options,
                ..guidance.clone()
            },
        ] {
            assert!(rejection(validate(vec![valued], &[])).contains("asks for a value"));
        }
    }

    #[test]
    fn answers_that_would_reshape_the_form_unsafely_are_refused() {
        assert!(
            rejection(validate(
                vec![field("username", ConfigFieldType::String)],
                &[]
            ))
            .contains("reuses a static field key")
        );
        assert!(
            rejection(validate(
                vec![
                    field("cookie", ConfigFieldType::String),
                    field("cookie", ConfigFieldType::String)
                ],
                &[]
            ))
            .contains("declared twice")
        );
        assert!(
            rejection(validate(vec![field(" ", ConfigFieldType::String)], &[]))
                .contains("empty key")
        );
        assert!(
            rejection(validate(
                vec![ConfigFieldDef {
                    label: " ".into(),
                    ..field("cookie", ConfigFieldType::String)
                }],
                &[]
            ))
            .contains("no label")
        );
        assert!(
            rejection(validate(
                vec![ConfigFieldDef {
                    host_binding: Some(scryer_domain::PluginHostBindingId::SmgOpenSubtitlesApiKey),
                    ..field("cookie", ConfigFieldType::String)
                }],
                &[]
            ))
            .contains("host binding")
        );
        assert!(
            rejection(validate(
                vec![ConfigFieldDef {
                    role: Some(scryer_domain::ConfigFieldRole::ConnectionUrl),
                    ..field("cookie", ConfigFieldType::String)
                }],
                &[]
            ))
            .contains("field role")
        );
        assert!(rejection(validate(vec![select("sort", &[])], &[])).contains("no options"));
        assert!(
            rejection(validate(
                vec![ConfigFieldDef {
                    default_value: Some("sideways".into()),
                    ..select("sort", &["new"])
                }],
                &[]
            ))
            .contains("invalid default")
        );
        assert!(
            rejection(validate(
                vec![ConfigFieldDef {
                    default_value: Some("yes".into()),
                    ..field("flag", ConfigFieldType::Bool)
                }],
                &[]
            ))
            .contains("invalid default")
        );
        assert!(
            rejection(validate(
                vec![ConfigFieldDef {
                    default_value: Some("NaN".into()),
                    ..field("limit", ConfigFieldType::Number)
                }],
                &[]
            ))
            .contains("invalid default")
        );
        assert!(
            rejection(validate(
                vec![ConfigFieldDef {
                    required_when: Some(FieldCondition {
                        key: "nowhere".into(),
                        op: ConditionOp::NonEmpty,
                        values: vec![],
                    }),
                    ..field("cookie", ConfigFieldType::String)
                }],
                &[]
            ))
            .contains("unknown field 'nowhere'")
        );
        let mut overriding = select("mirror", &["a"]);
        overriding.options[0]
            .config_overrides
            .insert("definition".into(), "other".into());
        assert!(rejection(validate(vec![overriding], &[])).contains("overrides 'definition'"));
        assert!(rejection(validate(vec![], &["cookie"])).contains("not a static field"));
        assert!(rejection(validate(vec![], &["definition"])).contains("cannot be hidden"));
        let too_many = (0..=MAX_DYNAMIC_FIELDS)
            .map(|index| field(&format!("f{index}"), ConfigFieldType::String))
            .collect();
        assert!(rejection(validate(too_many, &[])).contains("exceeds the limit"));
    }

    #[test]
    fn merge_appends_dynamic_fields_and_makes_hidden_statics_optional() {
        let merged = merge_dynamic_config_fields(
            statics(),
            &PluginDynamicConfigFields {
                fields: vec![field("cookie", ConfigFieldType::Password)],
                hidden_field_keys: vec!["username".into()],
            },
        );

        let keys = merged
            .iter()
            .map(|field| field.key.as_str())
            .collect::<Vec<_>>();
        assert_eq!(keys, ["definition", "username", "cookie"]);
        assert!(merged[0].required);
        assert!(!merged[1].required);
        assert!(merged[1].required_when.is_none());
    }

    #[test]
    fn fallback_marks_only_unknown_keys_as_optional_secrets() {
        let objects = vec![parse_config_object(Some(
            r#"{"definition":"alpha","cookie":"c1","flag":true}"#,
        ))];

        let fields = with_unknown_keys_as_secrets(statics(), &objects);

        let keys = fields
            .iter()
            .map(|field| field.key.as_str())
            .collect::<Vec<_>>();
        assert_eq!(keys, ["definition", "username", "cookie", "flag"]);
        assert!(
            fields[2..]
                .iter()
                .all(|field| { field.field_type == ConfigFieldType::Password && !field.required })
        );
    }

    #[test]
    fn trigger_values_fall_back_to_static_defaults() {
        let mut fields = statics();
        fields[0].default_value = Some("fallback".into());
        let values = BTreeMap::from([("definition".to_string(), "  ".to_string())]);

        assert_eq!(
            trigger_values_from_map(&triggers(), &fields, &values),
            BTreeMap::from([("definition".to_string(), "fallback".to_string())])
        );
    }
}

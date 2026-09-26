use std::collections::BTreeMap;

use scryer_plugin_sdk::{ConfigFieldDef, ConfigFieldType, ConfigFieldValueSource};

use super::*;
use crate::lists::test_support::{PROVIDER, ScriptedLists, ScriptedProvider};

pub(crate) fn field(key: &str, field_type: ConfigFieldType) -> ConfigFieldDef {
    serde_json::from_value(serde_json::json!({
        "key": key,
        "label": key,
        "field_type": field_type,
    }))
    .expect("config field")
}

pub(crate) fn fixture_fields() -> Vec<ConfigFieldDef> {
    let mut bound = field("gateway_key", ConfigFieldType::Password);
    bound.value_source = ConfigFieldValueSource::HostBinding;
    vec![
        field("instance_key", ConfigFieldType::Password),
        field("region", ConfigFieldType::String),
        bound,
    ]
}

fn descriptor() -> PluginDescriptor {
    ScriptedProvider(ScriptedLists::with_config_fields(fixture_fields()))
        .descriptors()
        .remove(0)
}

fn changes(pairs: &[(&str, Option<&str>)]) -> BTreeMap<String, Option<String>> {
    pairs
        .iter()
        .map(|(key, value)| (key.to_string(), value.map(str::to_string)))
        .collect()
}

#[test]
fn a_merge_keeps_omitted_keys_and_clears_blank_ones() {
    let stored = BTreeMap::from([
        ("instance_key".to_string(), "synthetic-key-one".to_string()),
        ("region".to_string(), "north".to_string()),
    ]);

    let merged = merge_provider_settings(
        &descriptor(),
        &stored,
        &changes(&[("region", Some("  ")), ("instance_key", None)]),
    )
    .expect("merge");
    assert!(merged.is_empty());

    let merged = merge_provider_settings(
        &descriptor(),
        &stored,
        &changes(&[("region", Some(" south "))]),
    )
    .expect("merge");
    assert_eq!(
        merged,
        BTreeMap::from([
            ("instance_key".to_string(), "synthetic-key-one".to_string()),
            ("region".to_string(), "south".to_string()),
        ])
    );
}

#[test]
fn a_merge_refuses_undeclared_and_host_bound_keys() {
    for key in ["account_token", "gateway_key"] {
        let error = merge_provider_settings(
            &descriptor(),
            &BTreeMap::new(),
            &changes(&[(key, Some("synthetic"))]),
        )
        .expect_err("refused");
        assert!(matches!(error, AppError::Validation(_)), "{key}: {error:?}");
    }
}

#[test]
fn a_merge_drops_stored_keys_the_provider_no_longer_declares() {
    let stored = BTreeMap::from([("retired".to_string(), "old".to_string())]);
    let merged = merge_provider_settings(&descriptor(), &stored, &BTreeMap::new()).expect("merge");
    assert!(merged.is_empty());
}

#[test]
fn a_view_never_returns_a_secret_value() {
    let stored = BTreeMap::from([
        ("instance_key".to_string(), "synthetic-key-one".to_string()),
        ("region".to_string(), "north".to_string()),
    ]);
    let view = settings_view(PROVIDER.to_string(), &descriptor(), &stored);
    let keys = view
        .fields
        .iter()
        .map(|field| field.key.as_str())
        .collect::<Vec<_>>();
    assert_eq!(keys, vec!["instance_key", "region"]);
    let secret = &view.fields[0];
    assert!(secret.secret && secret.is_set);
    assert_eq!(secret.value, None);
    assert_eq!(view.fields[1].value.as_deref(), Some("north"));
}

#[test]
fn configs_are_found_by_any_casing_of_the_provider_type() {
    let provider = ScriptedProvider(ScriptedLists::with_config_fields(fixture_fields()));
    let mut configs = ListProviderConfigs::default();
    let values = BTreeMap::from([("region".to_string(), "north".to_string())]);
    configs.insert(PROVIDER, values.clone());

    assert_eq!(
        configs.for_provider(&provider, &PROVIDER.to_ascii_uppercase()),
        values
    );
    assert!(configs.for_provider(&provider, "unknown").is_empty());
}

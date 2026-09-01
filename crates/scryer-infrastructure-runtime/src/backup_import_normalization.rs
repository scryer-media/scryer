use std::collections::{BTreeMap, BTreeSet};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use chrono::{DateTime, Utc};
use scryer_application::{AppError, AppResult};
use scryer_infrastructure_library::media::libraries::state_store::encode_release_decision_explanation;
use scryer_infrastructure_sql::domain_event_payload::{
    derive_domain_event_projections, encode_domain_event_payload,
};
use serde_json::{Map as JsonMap, Value as JsonValue};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportColumnKind {
    Generic,
    TimestampLike,
}

#[derive(Clone, Debug)]
pub struct ImportColumnRule {
    pub name: String,
    pub nullable: bool,
    pub has_default: bool,
    pub nullable_foreign_key: bool,
    pub kind: ImportColumnKind,
}

pub fn strip_nonportable_backup_fields(table: &str, object: &mut JsonMap<String, JsonValue>) {
    if table == "plugin_installations" {
        object.remove("wasm_bytes");
    }
}

pub fn validate_restore_manifest_table_set(
    row_counts: &BTreeMap<String, u64>,
    export_tables: &[String],
) -> AppResult<()> {
    let expected_tables = export_tables.iter().cloned().collect::<BTreeSet<_>>();
    let manifest_tables = row_counts.keys().cloned().collect::<BTreeSet<_>>();
    if manifest_tables == expected_tables {
        return Ok(());
    }

    let missing = expected_tables
        .difference(&manifest_tables)
        .cloned()
        .collect::<Vec<_>>();
    let unexpected = manifest_tables
        .difference(&expected_tables)
        .cloned()
        .collect::<Vec<_>>();
    Err(AppError::Validation(format!(
        "backup bundle table set does not match the current restore catalog: missing [{}], unexpected [{}]",
        missing.join(", "),
        unexpected.join(", ")
    )))
}

pub fn normalize_import_object_for_target(
    table: &str,
    object: &mut JsonMap<String, JsonValue>,
    now: DateTime<Utc>,
    columns: &[ImportColumnRule],
    line_number: usize,
) -> AppResult<()> {
    strip_nonportable_backup_fields(table, object);
    normalize_import_object(table, object, now)?;

    for column in columns {
        normalize_column_value(table, object, column, line_number)?;
    }

    for column in columns {
        if !column.nullable && !column.has_default && !object.contains_key(&column.name) {
            return Err(AppError::Validation(format!(
                "backup row for {table}:{line_number} is missing required column `{}` for the current schema",
                column.name
            )));
        }
    }

    Ok(())
}

fn normalize_import_object(
    table: &str,
    object: &mut JsonMap<String, JsonValue>,
    now: DateTime<Utc>,
) -> AppResult<()> {
    match table {
        "settings_definitions" => normalize_settings_definition_import_object(object, now),
        "settings_values" => normalize_settings_value_import_object(object, now),
        "titles" => normalize_title_import_object(object, now),
        "domain_events" => normalize_domain_event_import_object(object)?,
        "release_decisions" => normalize_release_decision_import_object(object)?,
        _ => {}
    }
    Ok(())
}

fn normalize_domain_event_import_object(object: &mut JsonMap<String, JsonValue>) -> AppResult<()> {
    let Some(payload) = object.get("payload_json").cloned() else {
        return Ok(());
    };
    if is_blob_marker(&payload) {
        return Ok(());
    }
    let payload = legacy_json_value(payload, "domain_events.payload_json")?;
    let encoded = encode_domain_event_payload(&payload).map_err(|error| {
        AppError::Validation(format!(
            "failed to encode restored domain event payload: {error}"
        ))
    })?;
    let event_type = object
        .get("event_type")
        .and_then(JsonValue::as_str)
        .unwrap_or_default()
        .to_string();
    let projections = derive_domain_event_projections(&event_type, &payload);
    object.insert(
        "import_status".to_string(),
        projections
            .import_status
            .map_or(JsonValue::Null, JsonValue::String),
    );
    object.insert(
        "media_file_delete_reason".to_string(),
        projections
            .media_file_delete_reason
            .map_or(JsonValue::Null, JsonValue::String),
    );
    object.insert(
        "download_id".to_string(),
        projections
            .download_id
            .map_or(JsonValue::Null, JsonValue::String),
    );
    object.insert("payload_json".to_string(), blob_value(&encoded));
    Ok(())
}

fn normalize_release_decision_import_object(
    object: &mut JsonMap<String, JsonValue>,
) -> AppResult<()> {
    let Some(explanation) = object.get("explanation_json").cloned() else {
        return Ok(());
    };
    if explanation.is_null() || is_blob_marker(&explanation) {
        return Ok(());
    }
    let decision_id = object
        .get("id")
        .and_then(JsonValue::as_str)
        .unwrap_or("<unknown>");
    let encoded = (|| -> AppResult<Vec<u8>> {
        let explanation = legacy_json_value(explanation, "release_decisions.explanation_json")?;
        let compact = serde_json::to_string(&explanation).map_err(|error| {
            AppError::Validation(format!(
                "failed to compact restored release explanation: {error}"
            ))
        })?;
        encode_release_decision_explanation(Some(&compact))
            .map_err(|error| {
                AppError::Validation(format!(
                    "failed to encode restored release explanation: {error}"
                ))
            })?
            .ok_or_else(|| {
                AppError::Validation(
                    "present restored release explanation encoded as null".to_string(),
                )
            })
    })();
    match encoded {
        Ok(encoded) => {
            object.insert("explanation_json".to_string(), blob_value(&encoded));
        }
        Err(error) => {
            tracing::warn!(
                decision_id,
                error = %error,
                "discarding invalid release-decision explanation during backup restore"
            );
            object.insert("explanation_json".to_string(), JsonValue::Null);
        }
    }
    Ok(())
}

fn legacy_json_value(value: JsonValue, field: &str) -> AppResult<JsonValue> {
    match value {
        JsonValue::String(value) => serde_json::from_str(&value).map_err(|error| {
            AppError::Validation(format!("restored {field} is invalid JSON: {error}"))
        }),
        JsonValue::Object(_) | JsonValue::Array(_) | JsonValue::Bool(_) | JsonValue::Number(_) => {
            Ok(value)
        }
        JsonValue::Null => Err(AppError::Validation(format!(
            "restored {field} is unexpectedly null"
        ))),
    }
}

fn is_blob_marker(value: &JsonValue) -> bool {
    value
        .as_object()
        .and_then(|object| object.get("__scryer_type"))
        .and_then(JsonValue::as_str)
        == Some("blob")
}

fn blob_value(bytes: &[u8]) -> JsonValue {
    JsonValue::Object(JsonMap::from_iter([
        (
            "__scryer_type".to_string(),
            JsonValue::String("blob".to_string()),
        ),
        (
            "base64".to_string(),
            JsonValue::String(BASE64.encode(bytes)),
        ),
    ]))
}

fn normalize_column_value(
    table: &str,
    object: &mut JsonMap<String, JsonValue>,
    column: &ImportColumnRule,
    line_number: usize,
) -> AppResult<()> {
    let Some(value) = object.get(&column.name).cloned() else {
        return Ok(());
    };

    if let JsonValue::String(text) = &value {
        if text.trim().is_empty() && column.nullable_foreign_key {
            object.insert(column.name.clone(), JsonValue::Null);
            return Ok(());
        }

        if text.trim().is_empty() && matches!(column.kind, ImportColumnKind::TimestampLike) {
            if column.nullable {
                object.insert(column.name.clone(), JsonValue::Null);
                return Ok(());
            }
            if column.has_default {
                object.remove(&column.name);
                return Ok(());
            }
            return Err(AppError::Validation(format!(
                "backup row for {table}:{line_number} contains a blank required timestamp column `{}`",
                column.name
            )));
        }
    }

    if matches!(value, JsonValue::Null) {
        if !column.nullable && column.has_default {
            object.remove(&column.name);
        } else if !column.nullable {
            return Err(AppError::Validation(format!(
                "backup row for {table}:{line_number} contains null for required column `{}`",
                column.name
            )));
        }
    }

    Ok(())
}

fn normalize_settings_value_import_object(
    object: &mut JsonMap<String, JsonValue>,
    now: DateTime<Utc>,
) {
    if object
        .get("value_json")
        .is_none_or(|value| matches!(value, JsonValue::Null))
    {
        object.insert("value_json".to_string(), JsonValue::Object(JsonMap::new()));
    }

    if missing_or_blank(object.get("source")) {
        object.insert(
            "source".to_string(),
            JsonValue::String("system".to_string()),
        );
    }

    let now_rfc3339 = now.to_rfc3339();
    for field in ["created_at", "updated_at"] {
        if missing_or_blank(object.get(field)) {
            object.insert(field.to_string(), JsonValue::String(now_rfc3339.clone()));
        }
    }
}

fn normalize_settings_definition_import_object(
    object: &mut JsonMap<String, JsonValue>,
    now: DateTime<Utc>,
) {
    if object
        .get("default_value_json")
        .is_none_or(|value| missing_or_blank(Some(value)))
    {
        object.insert(
            "default_value_json".to_string(),
            JsonValue::String("null".to_string()),
        );
    }

    if object
        .get("validation_json")
        .is_some_and(|value| missing_or_blank(Some(value)))
    {
        object.insert("validation_json".to_string(), JsonValue::Null);
    }

    let now_rfc3339 = now.to_rfc3339();
    for field in ["created_at", "updated_at"] {
        if missing_or_blank(object.get(field)) {
            object.insert(field.to_string(), JsonValue::String(now_rfc3339.clone()));
        }
    }
}

fn normalize_title_import_object(object: &mut JsonMap<String, JsonValue>, now: DateTime<Utc>) {
    let record = object
        .get("record_json")
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default();

    for field in [
        "id",
        "name",
        "created_by",
        "created_at",
        "year",
        "overview",
        "poster_url",
        "background_url",
        "sort_title",
        "slug",
        "imdb_id",
        "runtime_minutes",
        "content_status",
        "language",
        "first_aired",
        "network",
        "studio",
        "country",
        "metadata_language",
        "metadata_fetched_at",
        "min_availability",
        "digital_release_date",
        "folder_path",
    ] {
        copy_title_record_field(object, &record, field, field);
    }

    copy_title_record_field(object, &record, "library_id", "library_id");
    copy_title_record_field(object, &record, "facet", "facet");

    object
        .entry("library_id".to_string())
        .or_insert_with(|| JsonValue::String(String::new()));
    object
        .entry("facet".to_string())
        .or_insert_with(|| JsonValue::String("movie".to_string()));
    let monitored = sqlite_bool_value(object.get("monitored"))
        .or_else(|| {
            record
                .get("monitored")
                .and_then(|value| sqlite_bool_value(Some(value)))
        })
        .unwrap_or(JsonValue::Bool(true));
    object.insert("monitored".to_string(), monitored);

    for (record_field, source_field) in [
        ("tags", "tags"),
        ("external_ids", "external_ids"),
        ("aliases", "aliases"),
        ("tagged_aliases", "tagged_aliases_json"),
    ] {
        if object.contains_key(source_field) {
            continue;
        }
        let value = record
            .get(record_field)
            .and_then(logical_json_value)
            .unwrap_or_else(|| JsonValue::Array(Vec::new()));
        object.insert(source_field.to_string(), value);
    }

    for generated_field in [
        "poster_local_path",
        "background_local_path",
        "banner_local_path",
    ] {
        object.remove(generated_field);
    }

    let mut requires_metadata_rehydration = false;
    for source_field in ["poster_url", "background_url"] {
        if object
            .get(source_field)
            .and_then(JsonValue::as_str)
            .is_some_and(is_local_title_image_route)
        {
            object.insert(source_field.to_string(), JsonValue::Null);
            requires_metadata_rehydration = true;
        }
    }
    if requires_metadata_rehydration {
        object.insert("metadata_fetched_at".to_string(), JsonValue::Null);
        object.insert(
            "metadata_hydration_next_attempt_at".to_string(),
            JsonValue::String(now.to_rfc3339()),
        );
        object.insert(
            "metadata_hydration_attempt_count".to_string(),
            JsonValue::Number(0.into()),
        );
    }
}

fn is_local_title_image_route(value: &str) -> bool {
    let value = value.trim();
    value.starts_with('/') && value.contains("/images/titles/")
}

fn copy_title_record_field(
    object: &mut JsonMap<String, JsonValue>,
    record: &JsonMap<String, JsonValue>,
    record_field: &str,
    column: &str,
) {
    if object.contains_key(column) {
        return;
    }
    if let Some(value) = record.get(record_field).filter(|value| !value.is_null()) {
        object.insert(column.to_string(), value.clone());
    }
}

fn sqlite_bool_value(value: Option<&JsonValue>) -> Option<JsonValue> {
    match value {
        Some(JsonValue::Bool(value)) => Some(JsonValue::Bool(*value)),
        Some(JsonValue::Number(value)) => value.as_i64().map(|value| JsonValue::Bool(value != 0)),
        Some(JsonValue::String(value)) => match value.as_str() {
            "1" | "true" | "TRUE" => Some(JsonValue::Bool(true)),
            "0" | "false" | "FALSE" => Some(JsonValue::Bool(false)),
            _ => None,
        },
        _ => None,
    }
}

fn logical_json_value(value: &JsonValue) -> Option<JsonValue> {
    match value {
        JsonValue::Null => None,
        JsonValue::String(value) => {
            Some(serde_json::from_str(value).unwrap_or_else(|_| JsonValue::String(value.clone())))
        }
        value => Some(value.clone()),
    }
}

fn missing_or_blank(value: Option<&JsonValue>) -> bool {
    match value {
        None | Some(JsonValue::Null) => true,
        Some(JsonValue::String(value)) => value.trim().is_empty(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
    use chrono::TimeZone;
    use serde_json::{Map as JsonMap, Value as JsonValue, json};

    use super::{
        ImportColumnKind, ImportColumnRule, normalize_domain_event_import_object,
        normalize_import_object_for_target, normalize_release_decision_import_object,
        normalize_title_import_object, strip_nonportable_backup_fields,
        validate_restore_manifest_table_set,
    };

    #[test]
    fn plugin_installation_backup_rows_drop_wasm_bytes_but_keep_metadata() {
        let mut object = JsonMap::from_iter([
            (
                "plugin_id".to_string(),
                JsonValue::String("demo".to_string()),
            ),
            (
                "descriptor_json".to_string(),
                JsonValue::String("{\"name\":\"demo\"}".to_string()),
            ),
            ("is_enabled".to_string(), JsonValue::Bool(true)),
            (
                "wasm_bytes".to_string(),
                json!({
                    "__scryer_type": "blob",
                    "base64": "AQIDBA==",
                }),
            ),
        ]);

        strip_nonportable_backup_fields("plugin_installations", &mut object);

        assert!(!object.contains_key("wasm_bytes"));
        assert_eq!(
            object.get("plugin_id"),
            Some(&JsonValue::String("demo".to_string()))
        );
        assert_eq!(
            object.get("descriptor_json"),
            Some(&JsonValue::String("{\"name\":\"demo\"}".to_string()))
        );
        assert_eq!(object.get("is_enabled"), Some(&JsonValue::Bool(true)));
    }

    #[test]
    fn plugin_installation_import_normalization_ignores_wasm_bytes() {
        let now = chrono::Utc
            .with_ymd_and_hms(2026, 5, 15, 9, 30, 0)
            .single()
            .expect("fixed timestamp");
        let mut object = JsonMap::from_iter([
            (
                "plugin_id".to_string(),
                JsonValue::String("demo".to_string()),
            ),
            ("name".to_string(), JsonValue::String("Demo".to_string())),
            (
                "wasm_bytes".to_string(),
                json!({
                    "__scryer_type": "blob",
                    "base64": "AQIDBA==",
                }),
            ),
        ]);

        normalize_import_object_for_target(
            "plugin_installations",
            &mut object,
            now,
            &[
                ImportColumnRule {
                    name: "plugin_id".to_string(),
                    nullable: false,
                    has_default: false,
                    nullable_foreign_key: false,
                    kind: ImportColumnKind::Generic,
                },
                ImportColumnRule {
                    name: "name".to_string(),
                    nullable: false,
                    has_default: false,
                    nullable_foreign_key: false,
                    kind: ImportColumnKind::Generic,
                },
            ],
            3,
        )
        .expect("plugin installation row should normalize");

        assert!(!object.contains_key("wasm_bytes"));
    }

    #[test]
    fn restore_manifest_validation_rejects_non_export_catalog_tables() {
        let row_counts = BTreeMap::from_iter([
            ("settings_definitions".to_string(), 1),
            ("settings_values".to_string(), 1),
            ("title_image_blobs".to_string(), 21),
            ("title_image_variants".to_string(), 42),
        ]);
        let export_tables = vec![
            "settings_definitions".to_string(),
            "settings_values".to_string(),
        ];

        let error = validate_restore_manifest_table_set(&row_counts, &export_tables)
            .expect_err("non-export catalog tables should invalidate the bundle");
        assert!(error.to_string().contains("title_image_blobs"));
        assert!(error.to_string().contains("title_image_variants"));
    }

    #[test]
    fn restore_manifest_validation_rejects_unknown_tables() {
        let row_counts = BTreeMap::from_iter([
            ("settings_definitions".to_string(), 1),
            ("settings_values".to_string(), 1),
            ("mystery_cache".to_string(), 42),
        ]);
        let export_tables = vec![
            "settings_definitions".to_string(),
            "settings_values".to_string(),
        ];

        let error = validate_restore_manifest_table_set(&row_counts, &export_tables)
            .expect_err("unknown tables should stay invalid");
        assert!(error.to_string().contains("mystery_cache"));
    }

    #[test]
    fn title_import_normalization_drops_generated_local_image_paths() {
        let now = chrono::Utc
            .with_ymd_and_hms(2026, 5, 15, 9, 30, 0)
            .single()
            .expect("fixed timestamp");
        let mut object = JsonMap::from_iter([
            ("id".to_string(), JsonValue::String("title-1".to_string())),
            (
                "library_id".to_string(),
                JsonValue::String("library-1".to_string()),
            ),
            ("name".to_string(), JsonValue::String("Title".to_string())),
            ("facet".to_string(), JsonValue::String("movie".to_string())),
            (
                "poster_url".to_string(),
                JsonValue::String("https://example.invalid/poster.jpg".to_string()),
            ),
            (
                "poster_local_path".to_string(),
                JsonValue::String("/images/titles/title-1/poster/w250/hash".to_string()),
            ),
            (
                "background_local_path".to_string(),
                JsonValue::String("/images/titles/title-1/fanart/w1280/hash".to_string()),
            ),
            (
                "banner_local_path".to_string(),
                JsonValue::String("/images/titles/title-1/banner/w500/hash".to_string()),
            ),
        ]);

        normalize_import_object_for_target(
            "titles",
            &mut object,
            now,
            &[
                ImportColumnRule {
                    name: "id".to_string(),
                    nullable: false,
                    has_default: false,
                    nullable_foreign_key: false,
                    kind: ImportColumnKind::Generic,
                },
                ImportColumnRule {
                    name: "library_id".to_string(),
                    nullable: false,
                    has_default: false,
                    nullable_foreign_key: false,
                    kind: ImportColumnKind::Generic,
                },
                ImportColumnRule {
                    name: "name".to_string(),
                    nullable: false,
                    has_default: false,
                    nullable_foreign_key: false,
                    kind: ImportColumnKind::Generic,
                },
                ImportColumnRule {
                    name: "facet".to_string(),
                    nullable: false,
                    has_default: false,
                    nullable_foreign_key: false,
                    kind: ImportColumnKind::Generic,
                },
                ImportColumnRule {
                    name: "poster_url".to_string(),
                    nullable: true,
                    has_default: false,
                    nullable_foreign_key: false,
                    kind: ImportColumnKind::Generic,
                },
            ],
            7,
        )
        .expect("title row should normalize");

        assert_eq!(
            object.get("poster_url"),
            Some(&JsonValue::String(
                "https://example.invalid/poster.jpg".to_string()
            ))
        );
        assert!(!object.contains_key("poster_local_path"));
        assert!(!object.contains_key("background_local_path"));
        assert!(!object.contains_key("banner_local_path"));
    }

    #[test]
    fn title_import_normalization_rehydrates_local_only_artwork_sources() {
        let now = chrono::Utc
            .with_ymd_and_hms(2026, 5, 15, 9, 30, 0)
            .single()
            .expect("fixed timestamp");
        let mut object = JsonMap::from_iter([
            (
                "poster_url".to_string(),
                JsonValue::String("/scryer/images/titles/title-1/poster/w250/hash".to_string()),
            ),
            (
                "background_url".to_string(),
                JsonValue::String("https://example.invalid/fanart.jpg".to_string()),
            ),
            (
                "metadata_fetched_at".to_string(),
                JsonValue::String("2026-05-14T00:00:00Z".to_string()),
            ),
        ]);

        normalize_title_import_object(&mut object, now);

        assert_eq!(object.get("poster_url"), Some(&JsonValue::Null));
        assert_eq!(
            object.get("background_url"),
            Some(&JsonValue::String(
                "https://example.invalid/fanart.jpg".to_string()
            ))
        );
        assert_eq!(object.get("metadata_fetched_at"), Some(&JsonValue::Null));
        assert_eq!(
            object.get("metadata_hydration_next_attempt_at"),
            Some(&JsonValue::String(now.to_rfc3339()))
        );
        assert_eq!(
            object.get("metadata_hydration_attempt_count"),
            Some(&JsonValue::Number(0.into()))
        );
    }

    #[test]
    fn settings_values_normalization_fills_required_fields() {
        let now = chrono::Utc
            .with_ymd_and_hms(2026, 5, 15, 9, 30, 0)
            .single()
            .expect("fixed timestamp");
        let mut object = JsonMap::from_iter([
            ("id".to_string(), JsonValue::String("setting-1".to_string())),
            (
                "setting_definition_id".to_string(),
                JsonValue::String("definition-1".to_string()),
            ),
            (
                "scope".to_string(),
                JsonValue::String("backup_matrix".to_string()),
            ),
            ("scope_id".to_string(), JsonValue::Null),
            ("value_json".to_string(), JsonValue::Null),
            ("source".to_string(), JsonValue::String(String::new())),
            ("created_at".to_string(), JsonValue::Null),
        ]);

        normalize_import_object_for_target(
            "settings_values",
            &mut object,
            now,
            &[ImportColumnRule {
                name: "value_json".to_string(),
                nullable: false,
                has_default: false,
                nullable_foreign_key: false,
                kind: ImportColumnKind::Generic,
            }],
            8,
        )
        .expect("normalization");

        assert_eq!(object.get("value_json"), Some(&json!({})));
        assert_eq!(
            object.get("source"),
            Some(&JsonValue::String("system".to_string()))
        );
        assert_eq!(
            object.get("created_at"),
            Some(&JsonValue::String(now.to_rfc3339()))
        );
        assert_eq!(
            object.get("updated_at"),
            Some(&JsonValue::String(now.to_rfc3339()))
        );
    }

    #[test]
    fn settings_definitions_normalization_fills_default_and_timestamps() {
        let now = chrono::Utc
            .with_ymd_and_hms(2026, 5, 15, 9, 30, 0)
            .single()
            .expect("fixed timestamp");
        let mut object = JsonMap::from_iter([
            (
                "id".to_string(),
                JsonValue::String("backup_matrix:backup_matrix:json_payload".to_string()),
            ),
            (
                "category".to_string(),
                JsonValue::String("backup_matrix".to_string()),
            ),
            (
                "scope".to_string(),
                JsonValue::String("backup_matrix".to_string()),
            ),
            (
                "key_name".to_string(),
                JsonValue::String("json_payload".to_string()),
            ),
            (
                "data_type".to_string(),
                JsonValue::String("json".to_string()),
            ),
            ("default_value_json".to_string(), JsonValue::Null),
            (
                "validation_json".to_string(),
                JsonValue::String("   ".to_string()),
            ),
            ("is_sensitive".to_string(), JsonValue::Bool(false)),
        ]);

        normalize_import_object_for_target(
            "settings_definitions",
            &mut object,
            now,
            &[
                ImportColumnRule {
                    name: "default_value_json".to_string(),
                    nullable: false,
                    has_default: false,
                    nullable_foreign_key: false,
                    kind: ImportColumnKind::Generic,
                },
                ImportColumnRule {
                    name: "created_at".to_string(),
                    nullable: false,
                    has_default: false,
                    nullable_foreign_key: false,
                    kind: ImportColumnKind::TimestampLike,
                },
                ImportColumnRule {
                    name: "updated_at".to_string(),
                    nullable: false,
                    has_default: false,
                    nullable_foreign_key: false,
                    kind: ImportColumnKind::TimestampLike,
                },
            ],
            11,
        )
        .expect("settings definitions row should normalize");

        assert_eq!(
            object.get("default_value_json"),
            Some(&JsonValue::String("null".to_string()))
        );
        assert_eq!(object.get("validation_json"), Some(&JsonValue::Null));
        assert_eq!(
            object.get("created_at"),
            Some(&JsonValue::String(now.to_rfc3339()))
        );
        assert_eq!(
            object.get("updated_at"),
            Some(&JsonValue::String(now.to_rfc3339()))
        );
    }

    #[test]
    fn defaulted_required_nulls_are_omitted_for_target_defaults() {
        let now = chrono::Utc
            .with_ymd_and_hms(2026, 5, 15, 9, 30, 0)
            .single()
            .expect("fixed timestamp");
        let mut object = JsonMap::from_iter([("created_at".to_string(), JsonValue::Null)]);

        normalize_import_object_for_target(
            "workflow_operations",
            &mut object,
            now,
            &[ImportColumnRule {
                name: "created_at".to_string(),
                nullable: false,
                has_default: true,
                nullable_foreign_key: false,
                kind: ImportColumnKind::TimestampLike,
            }],
            3,
        )
        .expect("default should be used");

        assert!(!object.contains_key("created_at"));
    }

    #[test]
    fn missing_required_non_default_columns_fail_early() {
        let now = chrono::Utc
            .with_ymd_and_hms(2026, 5, 15, 9, 30, 0)
            .single()
            .expect("fixed timestamp");
        let mut object = JsonMap::new();

        let error = normalize_import_object_for_target(
            "custom_table",
            &mut object,
            now,
            &[ImportColumnRule {
                name: "value_json".to_string(),
                nullable: false,
                has_default: false,
                nullable_foreign_key: false,
                kind: ImportColumnKind::Generic,
            }],
            11,
        )
        .expect_err("missing required column should fail");

        assert!(
            error
                .to_string()
                .contains("missing required column `value_json`")
        );
    }

    #[test]
    fn legacy_domain_event_payloads_restore_as_compressed_blobs_with_projections() {
        let payload = serde_json::json!({
            "type": "import_rejected",
            "data": { "status": "failed", "reason": "synthetic" }
        });
        let mut object = JsonMap::from_iter([
            (
                "event_type".to_string(),
                JsonValue::String("import_rejected".to_string()),
            ),
            (
                "payload_json".to_string(),
                JsonValue::String(payload.to_string()),
            ),
        ]);

        normalize_domain_event_import_object(&mut object).unwrap();

        assert_eq!(
            object.get("import_status"),
            Some(&JsonValue::String("failed".to_string()))
        );
        let bytes = blob_marker_bytes(object.get("payload_json").unwrap());
        assert_eq!(
            scryer_infrastructure_sql::domain_event_payload::decode_domain_event_payload(&bytes)
                .unwrap(),
            payload
        );
    }

    #[test]
    fn legacy_release_explanations_restore_as_compressed_blobs() {
        let explanation = serde_json::json!({
            "quality_profile_decision": { "scoring_log": [{ "code": "quality_tier", "delta": 10 }] }
        });
        let mut object =
            JsonMap::from_iter([("explanation_json".to_string(), explanation.clone())]);

        normalize_release_decision_import_object(&mut object).unwrap();

        let bytes = blob_marker_bytes(object.get("explanation_json").unwrap());
        let decoded = scryer_infrastructure_library::media::libraries::state_store::decode_release_decision_explanation(Some(&bytes))
            .unwrap()
            .unwrap();
        assert_eq!(
            serde_json::from_str::<JsonValue>(&decoded).unwrap(),
            explanation
        );
    }

    #[test]
    fn invalid_and_oversized_release_explanations_restore_as_null() {
        for explanation in [
            JsonValue::String("not-json".to_string()),
            JsonValue::String(
                serde_json::to_string(&"x".repeat(70 * 1024))
                    .expect("oversized explanation should serialize"),
            ),
        ] {
            let mut object = JsonMap::from_iter([
                (
                    "id".to_string(),
                    JsonValue::String("decision-1".to_string()),
                ),
                ("explanation_json".to_string(), explanation),
            ]);

            normalize_release_decision_import_object(&mut object)
                .expect("invalid diagnostics must not abort restore");

            assert_eq!(object.get("explanation_json"), Some(&JsonValue::Null));
        }
    }

    #[test]
    fn binary_and_null_release_explanations_remain_unchanged() {
        for explanation in [
            JsonValue::Null,
            json!({"__scryer_type": "blob", "base64": "AQIDBA=="}),
        ] {
            let mut object =
                JsonMap::from_iter([("explanation_json".to_string(), explanation.clone())]);
            normalize_release_decision_import_object(&mut object).unwrap();
            assert_eq!(object.get("explanation_json"), Some(&explanation));
        }
    }

    #[test]
    fn invalid_domain_event_payloads_still_abort_restore() {
        let mut object = JsonMap::from_iter([
            (
                "event_type".to_string(),
                JsonValue::String("title_updated".to_string()),
            ),
            (
                "payload_json".to_string(),
                JsonValue::String("not-json".to_string()),
            ),
        ]);

        assert!(normalize_domain_event_import_object(&mut object).is_err());
    }

    fn blob_marker_bytes(value: &JsonValue) -> Vec<u8> {
        let encoded = value
            .as_object()
            .and_then(|object| object.get("base64"))
            .and_then(JsonValue::as_str)
            .expect("blob marker should contain base64");
        BASE64.decode(encoded).expect("blob marker should decode")
    }
}

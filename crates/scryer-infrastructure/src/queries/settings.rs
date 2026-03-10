use std::collections::HashMap;

use chrono::Utc;
use scryer_application::{AppError, AppResult};
use scryer_domain::Id;
use sqlx::{Row, SqlitePool};

use crate::encryption::EncryptionKey;
use crate::types::SettingDefinitionSeed;
use crate::{SettingsDefinitionRecord, SettingsValueRecord};

#[allow(clippy::too_many_arguments)]
pub(crate) async fn ensure_setting_definition_query(
    pool: &SqlitePool,
    category: &str,
    scope: &str,
    key_name: &str,
    data_type: &str,
    default_value_json: &str,
    is_sensitive: bool,
    validation_json: Option<String>,
) -> AppResult<()> {
    let id = format!("{category}:{scope}:{key_name}");
    let now = Utc::now().to_rfc3339();
    let normalized_default = if default_value_json.trim().is_empty() {
        "null".to_string()
    } else {
        default_value_json.to_string()
    };

    sqlx::query(
        "INSERT INTO settings_definitions
            (id, category, scope, key_name, data_type, default_value_json, is_sensitive, validation_json, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(category, scope, key_name) DO UPDATE SET
            category = excluded.category,
            scope = excluded.scope,
            key_name = excluded.key_name,
            data_type = excluded.data_type,
            default_value_json = excluded.default_value_json,
            is_sensitive = excluded.is_sensitive,
            validation_json = excluded.validation_json,
            updated_at = excluded.updated_at",
    )
    .bind(&id)
    .bind(category.trim())
    .bind(scope.trim())
    .bind(key_name.trim())
    .bind(data_type.trim())
    .bind(&normalized_default)
    .bind(if is_sensitive { 1_i64 } else { 0_i64 })
    .bind(&validation_json)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(())
}

pub(crate) async fn list_setting_definitions_query(
    pool: &SqlitePool,
    scope: Option<String>,
) -> AppResult<Vec<SettingsDefinitionRecord>> {
    let mut query = String::from(
        "SELECT id, category, scope, key_name, data_type, default_value_json,
            is_sensitive, validation_json, created_at, updated_at
         FROM settings_definitions",
    );
    if scope.is_some() {
        query.push_str(" WHERE scope = ?");
    }
    query.push_str(" ORDER BY category, scope, key_name");

    let mut statement = sqlx::query(&query);
    if let Some(scope) = scope.as_ref() {
        statement = statement.bind(scope);
    }

    let rows = statement
        .fetch_all(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let id: String = row
            .try_get("id")
            .map_err(|err| AppError::Repository(err.to_string()))?;
        let category: String = row
            .try_get("category")
            .map_err(|err| AppError::Repository(err.to_string()))?;
        let definition_scope: String = row
            .try_get("scope")
            .map_err(|err| AppError::Repository(err.to_string()))?;
        let key_name: String = row
            .try_get("key_name")
            .map_err(|err| AppError::Repository(err.to_string()))?;
        let data_type: String = row
            .try_get("data_type")
            .map_err(|err| AppError::Repository(err.to_string()))?;
        let default_value_json: String = row
            .try_get("default_value_json")
            .map_err(|err| AppError::Repository(err.to_string()))?;
        let is_sensitive: i64 = row
            .try_get("is_sensitive")
            .map_err(|err| AppError::Repository(err.to_string()))?;
        let validation_json: Option<String> = row
            .try_get("validation_json")
            .map_err(|err| AppError::Repository(err.to_string()))?;
        let created_at: String = row
            .try_get("created_at")
            .map_err(|err| AppError::Repository(err.to_string()))?;
        let updated_at: String = row
            .try_get("updated_at")
            .map_err(|err| AppError::Repository(err.to_string()))?;

        out.push(SettingsDefinitionRecord {
            id,
            category,
            scope: definition_scope,
            key_name,
            data_type,
            default_value_json,
            is_sensitive: is_sensitive != 0,
            validation_json,
            created_at,
            updated_at,
        });
    }

    Ok(out)
}

pub(crate) async fn list_settings_with_defaults_query(
    pool: &SqlitePool,
    scope: &str,
    scope_id: Option<String>,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<Vec<SettingsValueRecord>> {
    let statement = if scope_id.is_some() {
        sqlx::query(
            "SELECT
                d.id AS definition_id,
                d.category,
                d.scope,
                d.key_name,
                d.data_type,
                d.default_value_json,
                d.is_sensitive,
                d.validation_json,
                COALESCE(sv.value_json, d.default_value_json) AS effective_value_json,
                sv.value_json,
                sv.source,
                sv.scope_id,
                sv.updated_by_user_id,
                sv.created_at,
                sv.updated_at
             FROM settings_definitions d
             LEFT JOIN settings_values sv
               ON sv.setting_definition_id = d.id
              AND sv.scope = d.scope
              AND sv.scope = ?
              AND sv.scope_id = ?
             WHERE d.scope = ?
             ORDER BY d.category, d.key_name",
        )
    } else {
        sqlx::query(
            "SELECT
                d.id AS definition_id,
                d.category,
                d.scope,
                d.key_name,
                d.data_type,
                d.default_value_json,
                d.is_sensitive,
                d.validation_json,
                COALESCE(sv.value_json, d.default_value_json) AS effective_value_json,
                sv.value_json,
                sv.source,
                sv.scope_id,
                sv.updated_by_user_id,
                sv.created_at,
                sv.updated_at
             FROM settings_definitions d
             LEFT JOIN settings_values sv
               ON sv.setting_definition_id = d.id
              AND sv.scope = d.scope
              AND sv.scope = ?
              AND sv.scope_id IS NULL
             WHERE d.scope = ?
             ORDER BY d.category, d.key_name",
        )
    };

    let mut query = statement;
    if let Some(scope_id) = scope_id {
        query = query.bind(scope).bind(scope_id).bind(scope);
    } else {
        query = query.bind(scope).bind(scope);
    }

    let rows = query
        .fetch_all(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let definition_id: String = row
            .try_get("definition_id")
            .map_err(|err| AppError::Repository(err.to_string()))?;
        let category: String = row
            .try_get("category")
            .map_err(|err| AppError::Repository(err.to_string()))?;
        let definition_scope: String = row
            .try_get("scope")
            .map_err(|err| AppError::Repository(err.to_string()))?;
        let key_name: String = row
            .try_get("key_name")
            .map_err(|err| AppError::Repository(err.to_string()))?;
        let data_type: String = row
            .try_get("data_type")
            .map_err(|err| AppError::Repository(err.to_string()))?;
        let default_value_json: String = row
            .try_get("default_value_json")
            .map_err(|err| AppError::Repository(err.to_string()))?;
        let is_sensitive: i64 = row
            .try_get("is_sensitive")
            .map_err(|err| AppError::Repository(err.to_string()))?;
        let validation_json: Option<String> = row
            .try_get("validation_json")
            .map_err(|err| AppError::Repository(err.to_string()))?;
        let effective_value_json: String = row
            .try_get("effective_value_json")
            .map_err(|err| AppError::Repository(err.to_string()))?;
        let value_json: Option<String> = row
            .try_get("value_json")
            .map_err(|err| AppError::Repository(err.to_string()))?;
        let source: Option<String> = row
            .try_get("source")
            .map_err(|err| AppError::Repository(err.to_string()))?;
        let value_scope_id: Option<String> = row
            .try_get("scope_id")
            .map_err(|err| AppError::Repository(err.to_string()))?;
        let updated_by_user_id: Option<String> = row
            .try_get("updated_by_user_id")
            .map_err(|err| AppError::Repository(err.to_string()))?;
        let created_at: Option<String> = row
            .try_get("created_at")
            .map_err(|err| AppError::Repository(err.to_string()))?;
        let updated_at: Option<String> = row
            .try_get("updated_at")
            .map_err(|err| AppError::Repository(err.to_string()))?;

        // Transparently decrypt encrypted values
        let effective_value_json = maybe_decrypt(encryption_key, effective_value_json)?;
        let value_json = match value_json {
            Some(v) => Some(maybe_decrypt(encryption_key, v)?),
            None => None,
        };

        out.push(SettingsValueRecord {
            definition_id,
            category,
            scope: definition_scope,
            key_name,
            data_type,
            default_value_json,
            is_sensitive: is_sensitive != 0,
            validation_json,
            effective_value_json,
            value_json,
            source,
            scope_id: value_scope_id,
            updated_by_user_id,
            created_at,
            updated_at,
        });
    }

    Ok(out)
}

pub(crate) async fn get_setting_with_defaults_query(
    pool: &SqlitePool,
    scope: &str,
    key_name: &str,
    scope_id: Option<String>,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<Option<SettingsValueRecord>> {
    let scope = scope.trim().to_string();
    let key_name = key_name.trim().to_string();

    if scope.is_empty() || key_name.is_empty() {
        return Err(AppError::Validation(
            "scope and key_name are required to read a setting".to_string(),
        ));
    }

    let rows = list_settings_with_defaults_query(pool, &scope, scope_id.clone(), encryption_key).await?;
    let result = rows.into_iter().find(|row| row.key_name == key_name);
    Ok(result)
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn upsert_setting_value_query(
    pool: &SqlitePool,
    scope: &str,
    key_name: &str,
    scope_id: Option<String>,
    value_json: &str,
    source: &str,
    updated_by_user_id: Option<String>,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<SettingsValueRecord> {
    let scope = scope.trim().to_string();
    let key_name = key_name.trim().to_string();
    if scope.is_empty() || key_name.is_empty() {
        return Err(AppError::Validation(
            "scope and key_name are required to update a setting".to_string(),
        ));
    }

    let (definition_id, is_sensitive) = get_setting_definition_meta_query(pool, &scope, &key_name)
        .await?
        .ok_or_else(|| {
            AppError::Validation(format!("unknown setting key: {}.{}", scope, key_name))
        })?;

    let value_json = value_json.trim().to_string();
    if value_json.is_empty() {
        return Err(AppError::Validation(
            "setting value cannot be empty".to_string(),
        ));
    }

    // Encrypt sensitive values before storing
    let stored_value = if is_sensitive {
        if let Some(key) = encryption_key {
            crate::encryption::encrypt_value(key, &value_json)
                .map_err(|e| AppError::Repository(format!("failed to encrypt setting value: {e}")))?
        } else {
            value_json.clone()
        }
    } else {
        value_json.clone()
    };

    let now = Utc::now().to_rfc3339();
    let normalized_scope_id = scope_id
        .and_then(|value| {
            let value = value.trim().to_string();
            if value.is_empty() {
                None
            } else {
                Some(value)
            }
        });

    sqlx::query(
        "INSERT INTO settings_values
            (id, setting_definition_id, scope, scope_id, value_json, source,
             updated_by_user_id, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(setting_definition_id, scope, COALESCE(scope_id, ''))
         DO UPDATE SET
            value_json = excluded.value_json,
            source = excluded.source,
            updated_by_user_id = excluded.updated_by_user_id,
            updated_at = excluded.updated_at",
    )
    .bind(Id::new().0)
    .bind(&definition_id)
    .bind(&scope)
    .bind(&normalized_scope_id)
    .bind(&stored_value)
    .bind(source.trim())
    .bind(updated_by_user_id)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    let scope_id = normalized_scope_id;
    get_setting_with_defaults_query(pool, &scope, &key_name, scope_id, encryption_key)
        .await?
        .ok_or_else(|| AppError::Repository("setting write did not persist".to_string()))
}

/// Returns (definition_id, is_sensitive) for a setting definition.
pub(crate) async fn get_setting_definition_meta_query(
    pool: &SqlitePool,
    scope: &str,
    key_name: &str,
) -> AppResult<Option<(String, bool)>> {
    let rows = sqlx::query_as::<_, (String, String, i64)>(
        "SELECT id, category, is_sensitive FROM settings_definitions WHERE scope = ? AND key_name = ?",
    )
    .bind(scope)
    .bind(key_name)
    .fetch_all(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut rows = rows;
    if rows.is_empty() {
        return Ok(None);
    }

    if rows.len() > 1 {
        let mut categories: Vec<String> = rows.drain(..).map(|(_id, category, _)| category).collect();
        categories.sort();
        categories.dedup();
        return Err(AppError::Validation(format!(
            "ambiguous setting key {}.{} found in categories: {}",
            scope,
            key_name,
            categories.join(", ")
        )));
    }

    let (id, _, is_sensitive) = rows.into_iter().next().unwrap();
    Ok(Some((id, is_sensitive != 0)))
}

fn maybe_decrypt(key: Option<&EncryptionKey>, value: String) -> AppResult<String> {
    if !crate::encryption::is_encrypted(&value) {
        return Ok(value);
    }
    let Some(key) = key else {
        return Ok(value);
    };
    crate::encryption::decrypt_value(key, &value)
        .map_err(|e| AppError::Repository(format!("failed to decrypt setting value: {e}")))
}

// ---------------------------------------------------------------------------
// Batch operations for startup bootstrap
// ---------------------------------------------------------------------------

pub(crate) async fn batch_ensure_setting_definitions_query(
    pool: &SqlitePool,
    definitions: &[SettingDefinitionSeed],
) -> AppResult<()> {
    let now = Utc::now().to_rfc3339();
    let mut tx = pool
        .begin()
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    for def in definitions {
        let id = format!("{}:{}:{}", def.category, def.scope, def.key_name);
        let normalized_default = if def.default_value_json.trim().is_empty() {
            "null".to_string()
        } else {
            def.default_value_json.clone()
        };

        sqlx::query(
            "INSERT INTO settings_definitions
                (id, category, scope, key_name, data_type, default_value_json, is_sensitive, validation_json, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(category, scope, key_name) DO UPDATE SET
                category = excluded.category,
                scope = excluded.scope,
                key_name = excluded.key_name,
                data_type = excluded.data_type,
                default_value_json = excluded.default_value_json,
                is_sensitive = excluded.is_sensitive,
                validation_json = excluded.validation_json,
                updated_at = excluded.updated_at",
        )
        .bind(&id)
        .bind(def.category.trim())
        .bind(def.scope.trim())
        .bind(def.key_name.trim())
        .bind(def.data_type.trim())
        .bind(&normalized_default)
        .bind(if def.is_sensitive { 1_i64 } else { 0_i64 })
        .bind(&def.validation_json)
        .bind(&now)
        .bind(&now)
        .execute(&mut *tx)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;
    }

    tx.commit()
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(())
}

pub(crate) async fn batch_get_settings_with_defaults_query(
    pool: &SqlitePool,
    keys: &[(String, String, Option<String>)],
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<Vec<Option<SettingsValueRecord>>> {
    // Group requests by (scope, scope_id) to minimize queries
    let mut scope_groups: HashMap<(String, Option<String>), Vec<usize>> = HashMap::new();
    for (idx, (scope, _key_name, scope_id)) in keys.iter().enumerate() {
        scope_groups
            .entry((scope.clone(), scope_id.clone()))
            .or_default()
            .push(idx);
    }

    let mut results: Vec<Option<SettingsValueRecord>> = vec![None; keys.len()];

    for ((scope, scope_id), indices) in &scope_groups {
        let all_for_scope =
            list_settings_with_defaults_query(pool, scope, scope_id.clone(), encryption_key)
                .await?;

        for &idx in indices {
            let key_name = &keys[idx].1;
            let found = all_for_scope
                .iter()
                .find(|row| row.key_name == *key_name)
                .cloned();
            results[idx] = found;
        }
    }

    Ok(results)
}

pub(crate) async fn batch_upsert_settings_if_not_overridden_query(
    pool: &SqlitePool,
    entries: &[(String, String, String, String)],
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<()> {
    if entries.is_empty() {
        return Ok(());
    }

    // Fetch existing settings per scope to check for overrides
    let mut scope_cache: HashMap<String, Vec<SettingsValueRecord>> = HashMap::new();
    for (scope, _, _, _) in entries {
        if !scope_cache.contains_key(scope) {
            let existing =
                list_settings_with_defaults_query(pool, scope, None, encryption_key).await?;
            scope_cache.insert(scope.clone(), existing);
        }
    }

    // Filter to entries that don't already have an override
    let to_write: Vec<&(String, String, String, String)> = entries
        .iter()
        .filter(|(scope, key_name, _, _)| {
            let has_override = scope_cache
                .get(scope)
                .and_then(|settings| settings.iter().find(|s| s.key_name == *key_name))
                .is_some_and(|record| record.has_override());
            !has_override
        })
        .collect();

    if to_write.is_empty() {
        return Ok(());
    }

    let now = Utc::now().to_rfc3339();
    let mut tx = pool
        .begin()
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    for (scope, key_name, value_json, source) in &to_write {
        let (definition_id, is_sensitive) = {
            let row = sqlx::query_as::<_, (String, String, i64)>(
                "SELECT id, category, is_sensitive FROM settings_definitions WHERE scope = ? AND key_name = ?",
            )
            .bind(scope.as_str())
            .bind(key_name.as_str())
            .fetch_optional(&mut *tx)
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
            row.map(|(id, _cat, sensitive)| (id, sensitive != 0))
        }
            .ok_or_else(|| {
                AppError::Validation(format!("unknown setting key: {scope}.{key_name}"))
            })?;

        let stored_value = if is_sensitive {
            if let Some(key) = encryption_key {
                crate::encryption::encrypt_value(key, value_json).map_err(|e| {
                    AppError::Repository(format!("failed to encrypt setting value: {e}"))
                })?
            } else {
                value_json.clone()
            }
        } else {
            value_json.clone()
        };

        sqlx::query(
            "INSERT INTO settings_values
                (id, setting_definition_id, scope, scope_id, value_json, source,
                 updated_by_user_id, created_at, updated_at)
             VALUES (?, ?, ?, NULL, ?, ?, NULL, ?, ?)
             ON CONFLICT(setting_definition_id, scope, COALESCE(scope_id, ''))
             DO UPDATE SET
                value_json = excluded.value_json,
                source = excluded.source,
                updated_at = excluded.updated_at",
        )
        .bind(Id::new().0)
        .bind(&definition_id)
        .bind(scope.as_str())
        .bind(&stored_value)
        .bind(source.as_str())
        .bind(&now)
        .bind(&now)
        .execute(&mut *tx)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;
    }

    tx.commit()
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(())
}

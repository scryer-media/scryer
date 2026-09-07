use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use chrono::Utc;
use scryer_application::{AppError, AppResult, SettingsRepository, SystemInfoProvider};
use scryer_domain::Id;
use serde_json::Value as JsonValue;
use sqlx::{Row, types::Json};

use crate::config_store::current_encryption_key;
use crate::encryption::{EncryptionKey, decrypt_value, encrypt_value, is_encrypted};
use crate::queries::sql_runtime::{
    SqlArg, SqlExec, SqlRow, SqlRuntime, SqlTarget, SqlTx, StoreDatastore, repo_err,
};
use crate::types::{SettingDefinitionSeed, SettingsValueRecord};

#[derive(Clone)]
pub struct SettingsStore {
    datastore: StoreDatastore,
    encryption_key: Arc<RwLock<Option<EncryptionKey>>>,
}

impl SettingsStore {
    pub fn new(
        datastore: StoreDatastore,
        encryption_key: Arc<RwLock<Option<EncryptionKey>>>,
    ) -> Self {
        Self {
            datastore,
            encryption_key,
        }
    }

    fn encryption_key(&self) -> AppResult<Option<EncryptionKey>> {
        current_encryption_key(&self.encryption_key)
    }

    fn engine_name(&self) -> &'static str {
        match &self.datastore {
            StoreDatastore::Sqlite { .. } => "sqlite",
            StoreDatastore::Postgres { .. } => "postgres",
        }
    }

    pub async fn batch_ensure_setting_definitions(
        &self,
        definitions: Vec<SettingDefinitionSeed>,
    ) -> AppResult<()> {
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "batch_ensure_setting_definitions",
            move |tx| {
                let definitions = definitions.clone();
                Box::pin(async move {
                    let now = Utc::now();
                    for definition in definitions {
                        upsert_setting_definition_tx(tx, definition, now).await?;
                    }
                    Ok(())
                })
            },
        )
        .await
    }

    pub async fn batch_get_settings_with_defaults(
        &self,
        keys: Vec<(String, String, Option<String>)>,
    ) -> AppResult<Vec<Option<SettingsValueRecord>>> {
        let encryption_key = self.encryption_key()?;
        let mut scope_groups: HashMap<(String, Option<String>), Vec<usize>> = HashMap::new();
        for (idx, (scope, _key_name, scope_id)) in keys.iter().enumerate() {
            scope_groups
                .entry((scope.clone(), normalize_scope_id(scope_id.clone())))
                .or_default()
                .push(idx);
        }

        let mut results = vec![None; keys.len()];
        for ((scope, scope_id), indices) in scope_groups {
            let all_for_scope = list_settings_with_defaults_exec(
                self.datastore.read_exec(),
                &scope,
                scope_id.clone(),
                encryption_key.as_ref(),
            )
            .await?;

            for idx in indices {
                let key_name = keys[idx].1.trim();
                results[idx] = all_for_scope
                    .iter()
                    .find(|row| row.key_name == key_name)
                    .cloned();
            }
        }

        Ok(results)
    }

    pub async fn batch_upsert_settings_if_not_overridden(
        &self,
        entries: Vec<(String, String, String, String)>,
    ) -> AppResult<()> {
        if entries.is_empty() {
            return Ok(());
        }

        let encryption_key = self.encryption_key()?;
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "batch_upsert_settings_if_not_overridden",
            move |tx| {
                let entries = entries.clone();
                let encryption_key = encryption_key.clone();
                Box::pin(async move {
                    let mut scope_cache: HashMap<String, Vec<SettingsValueRecord>> = HashMap::new();

                    for (scope, _key_name, _value_json, _source) in &entries {
                        if !scope_cache.contains_key(scope) {
                            let rows = list_settings_with_defaults_exec(
                                SqlExec::Tx(tx),
                                scope,
                                None,
                                encryption_key.as_ref(),
                            )
                            .await?;
                            scope_cache.insert(scope.clone(), rows);
                        }
                    }

                    for (scope, key_name, value_json, source) in entries {
                        let has_override = scope_cache
                            .get(&scope)
                            .and_then(|settings| {
                                settings.iter().find(|row| row.key_name == key_name)
                            })
                            .is_some_and(SettingsValueRecord::has_override);
                        if has_override {
                            continue;
                        }

                        upsert_setting_value_tx(
                            tx,
                            &scope,
                            &key_name,
                            None,
                            &value_json,
                            &source,
                            None,
                            encryption_key.as_ref(),
                        )
                        .await?;
                    }

                    Ok(())
                })
            },
        )
        .await
    }

    pub async fn list_settings_with_defaults(
        &self,
        scope: impl Into<String>,
        scope_id: Option<String>,
    ) -> AppResult<Vec<SettingsValueRecord>> {
        let encryption_key = self.encryption_key()?;
        list_settings_with_defaults_exec(
            self.datastore.read_exec(),
            &scope.into(),
            scope_id,
            encryption_key.as_ref(),
        )
        .await
    }

    pub async fn get_setting_with_defaults(
        &self,
        scope: impl Into<String>,
        key_name: impl Into<String>,
        scope_id: Option<String>,
    ) -> AppResult<Option<SettingsValueRecord>> {
        let encryption_key = self.encryption_key()?;
        get_setting_with_defaults_exec(
            self.datastore.read_exec(),
            &scope.into(),
            &key_name.into(),
            scope_id,
            encryption_key.as_ref(),
        )
        .await
    }

    pub async fn upsert_setting_value(
        &self,
        scope: impl Into<String>,
        key_name: impl Into<String>,
        scope_id: Option<String>,
        value_json: impl Into<String>,
        source: impl Into<String>,
        updated_by_user_id: Option<String>,
    ) -> AppResult<SettingsValueRecord> {
        let scope = scope.into();
        let key_name = key_name.into();
        let value_json = value_json.into();
        let source = source.into();
        let encryption_key = self.encryption_key()?;
        SqlRuntime::run_in_transaction(&self.datastore, "upsert_setting_value", move |tx| {
            let scope = scope.clone();
            let key_name = key_name.clone();
            let scope_id = scope_id.clone();
            let value_json = value_json.clone();
            let source = source.clone();
            let updated_by_user_id = updated_by_user_id.clone();
            let encryption_key = encryption_key.clone();
            Box::pin(async move {
                upsert_setting_value_tx(
                    tx,
                    &scope,
                    &key_name,
                    scope_id,
                    &value_json,
                    &source,
                    updated_by_user_id,
                    encryption_key.as_ref(),
                )
                .await
            })
        })
        .await
    }

    pub async fn delete_setting_value(
        &self,
        scope: impl Into<String>,
        key_name: impl Into<String>,
        scope_id: Option<String>,
    ) -> AppResult<()> {
        let scope = scope.into();
        let key_name = key_name.into();
        SqlRuntime::run_in_transaction(&self.datastore, "delete_setting_value", move |tx| {
            let scope = scope.clone();
            let key_name = key_name.clone();
            let scope_id = scope_id.clone();
            Box::pin(async move { delete_setting_value_tx(tx, &scope, &key_name, scope_id).await })
        })
        .await
    }

    pub async fn delete_values_for_scope_id(&self, scope_id: &str) -> AppResult<u32> {
        let scope_id = scope_id.to_string();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "delete_settings_values_for_scope_id",
            move |tx| {
                let scope_id = scope_id.clone();
                Box::pin(async move {
                    let rows = SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "DELETE FROM settings_values WHERE scope_id = {}",
                        &[SqlArg::Text(scope_id)],
                    )
                    .await?;
                    Ok(rows.min(u32::MAX as u64) as u32)
                })
            },
        )
        .await
    }
}

#[async_trait]
impl SettingsRepository for SettingsStore {
    async fn get_setting_json(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<String>,
    ) -> AppResult<Option<String>> {
        Ok(self
            .get_setting_with_defaults(scope, key_name, scope_id)
            .await?
            .map(|record| record.effective_value_json))
    }

    async fn get_setting_json_explicit(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<String>,
    ) -> AppResult<Option<String>> {
        let encryption_key = self.encryption_key()?;
        Ok(get_setting_explicit_exec(
            self.datastore.read_exec(),
            scope,
            key_name,
            scope_id,
            encryption_key.as_ref(),
        )
        .await?
        .map(|record| record.effective_value_json))
    }

    async fn list_setting_json_explicit_for_scope_ids(
        &self,
        scope: &str,
        key_name: &str,
        scope_ids: &[String],
    ) -> AppResult<Vec<(String, String)>> {
        if scope_ids.is_empty() {
            return Ok(Vec::new());
        }
        let scope = scope.trim();
        let key_name = key_name.trim();
        if scope.is_empty() || key_name.is_empty() {
            return Err(AppError::Validation(
                "scope and key_name are required to read a setting".to_string(),
            ));
        }

        let encryption_key = self.encryption_key()?;
        let exec = self.datastore.read_exec();
        let postgres_timestamps = exec_is_postgres(&exec);
        let created_at = if postgres_timestamps {
            "sv.created_at::TEXT"
        } else {
            "sv.created_at"
        };
        let updated_at = if postgres_timestamps {
            "sv.updated_at::TEXT"
        } else {
            "sv.updated_at"
        };
        let placeholders = std::iter::repeat_n("{}", scope_ids.len())
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT
                d.id AS definition_id,
                d.category,
                d.scope,
                d.key_name,
                d.data_type,
                d.default_value_json,
                d.is_sensitive,
                d.validation_json,
                sv.value_json AS effective_value_json,
                sv.value_json,
                sv.source,
                sv.scope_id,
                sv.updated_by_user_id,
                {created_at} AS created_at,
                {updated_at} AS updated_at
             FROM settings_definitions d
             JOIN settings_values sv
               ON sv.setting_definition_id = d.id
              AND sv.scope = d.scope
             WHERE d.scope = {{}}
               AND d.key_name = {{}}
               AND sv.scope = {{}}
               AND sv.scope_id IN ({placeholders})"
        );
        let mut args = vec![
            SqlArg::Text(scope.to_string()),
            SqlArg::Text(key_name.to_string()),
            SqlArg::Text(scope.to_string()),
        ];
        args.extend(scope_ids.iter().cloned().map(SqlArg::Text));

        let rows = SqlRuntime::fetch_all(exec, &sql, &args).await?;
        let mut values = Vec::with_capacity(rows.len());
        for row in rows.iter() {
            let record = decode_settings_row(row, encryption_key.as_ref())?;
            if let Some(scope_id) = record.scope_id {
                values.push((scope_id, record.effective_value_json));
            }
        }
        Ok(values)
    }

    async fn upsert_setting_json(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<String>,
        value_json: String,
        source: &str,
        updated_by_user_id: Option<String>,
    ) -> AppResult<()> {
        self.upsert_setting_value(
            scope,
            key_name,
            scope_id,
            value_json,
            source,
            updated_by_user_id,
        )
        .await?;
        Ok(())
    }

    async fn delete_setting_value(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<String>,
    ) -> AppResult<()> {
        SettingsStore::delete_setting_value(self, scope, key_name, scope_id).await
    }

    async fn delete_values_for_scope_id(&self, scope_id: &str) -> AppResult<u32> {
        SettingsStore::delete_values_for_scope_id(self, scope_id).await
    }
}

#[async_trait]
impl SystemInfoProvider for SettingsStore {
    async fn datastore_info(&self) -> AppResult<scryer_application::DatastoreInfo> {
        Ok(scryer_application::DatastoreInfo {
            engine: self.engine_name().to_string(),
            current_migration_key: self.current_migration_version().await?,
        })
    }

    async fn current_migration_version(&self) -> AppResult<Option<String>> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT version, description
               FROM _sqlx_migrations
              WHERE success = {}
              ORDER BY version DESC, description DESC
              LIMIT 1",
            &[SqlArg::Bool(true)],
        )
        .await?;

        row.map(|row| {
            let version = row.i64("version")?;
            let description = row.text("description")?;
            Ok(
                scryer_infrastructure_sql::migration::migration_key_from_version_and_desc(
                    version,
                    &description,
                ),
            )
        })
        .transpose()
    }

    async fn current_encryption_key_base64(&self) -> AppResult<Option<String>> {
        Ok(self.encryption_key()?.map(|key| key.to_base64()))
    }
}

/// Builds the definitions-with-values projection for one scope. With
/// `single_key` the statement is narrowed to one `key_name` and drops the
/// ordering so a point read never materialises the whole scope.
fn settings_with_defaults_sql(
    has_scope_id: bool,
    postgres_timestamps: bool,
    single_key: bool,
) -> String {
    let created_at = if postgres_timestamps {
        "sv.created_at::TEXT"
    } else {
        "sv.created_at"
    };
    let updated_at = if postgres_timestamps {
        "sv.updated_at::TEXT"
    } else {
        "sv.updated_at"
    };
    let scope_join = if has_scope_id {
        "AND sv.scope = {}
         AND sv.scope_id = {}"
    } else {
        "AND sv.scope = {}
         AND sv.scope_id IS NULL"
    };
    let tail = if single_key {
        "AND d.key_name = {}
         LIMIT 1"
    } else {
        "ORDER BY d.category, d.key_name"
    };

    format!(
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
            {created_at} AS created_at,
            {updated_at} AS updated_at
         FROM settings_definitions d
         LEFT JOIN settings_values sv
           ON sv.setting_definition_id = d.id
          AND sv.scope = d.scope
          {scope_join}
         WHERE d.scope = {{}}
         {tail}"
    )
}

fn explicit_setting_sql(has_scope_id: bool, postgres_timestamps: bool) -> String {
    let created_at = if postgres_timestamps {
        "sv.created_at::TEXT"
    } else {
        "sv.created_at"
    };
    let updated_at = if postgres_timestamps {
        "sv.updated_at::TEXT"
    } else {
        "sv.updated_at"
    };
    let scope_filter = if has_scope_id {
        "AND sv.scope_id = {}"
    } else {
        "AND sv.scope_id IS NULL"
    };

    format!(
        "SELECT
            d.id AS definition_id,
            d.category,
            d.scope,
            d.key_name,
            d.data_type,
            d.default_value_json,
            d.is_sensitive,
            d.validation_json,
            sv.value_json AS effective_value_json,
            sv.value_json,
            sv.source,
            sv.scope_id,
            sv.updated_by_user_id,
            {created_at} AS created_at,
            {updated_at} AS updated_at
         FROM settings_definitions d
         JOIN settings_values sv
           ON sv.setting_definition_id = d.id
          AND sv.scope = d.scope
         WHERE d.scope = {{}}
           AND d.key_name = {{}}
           AND sv.scope = {{}}
           {scope_filter}
         LIMIT 1"
    )
}

async fn list_settings_with_defaults_exec(
    exec: SqlExec<'_, '_>,
    scope: &str,
    scope_id: Option<String>,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<Vec<SettingsValueRecord>> {
    let has_scope_id = scope_id
        .as_ref()
        .is_some_and(|value| !value.trim().is_empty());
    let postgres_timestamps = exec_is_postgres(&exec);
    let sql = settings_with_defaults_sql(has_scope_id, postgres_timestamps, false);
    let normalized_scope_id = normalize_scope_id(scope_id);

    let args = if let Some(scope_id) = normalized_scope_id {
        vec![
            SqlArg::Text(scope.to_string()),
            SqlArg::Text(scope_id),
            SqlArg::Text(scope.to_string()),
        ]
    } else {
        vec![
            SqlArg::Text(scope.to_string()),
            SqlArg::Text(scope.to_string()),
        ]
    };

    let rows = SqlRuntime::fetch_all(exec, &sql, &args).await?;
    rows.iter()
        .map(|row| decode_settings_row(row, encryption_key))
        .collect()
}

async fn get_setting_with_defaults_exec(
    exec: SqlExec<'_, '_>,
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

    let has_scope_id = scope_id
        .as_ref()
        .is_some_and(|value| !value.trim().is_empty());
    let postgres_timestamps = exec_is_postgres(&exec);
    let sql = settings_with_defaults_sql(has_scope_id, postgres_timestamps, true);
    let mut args = match normalize_scope_id(scope_id) {
        Some(scope_id) => vec![SqlArg::Text(scope.clone()), SqlArg::Text(scope_id)],
        None => vec![SqlArg::Text(scope.clone())],
    };
    args.push(SqlArg::Text(scope));
    args.push(SqlArg::Text(key_name));

    SqlRuntime::fetch_optional(exec, &sql, &args)
        .await?
        .map(|row| decode_settings_row(&row, encryption_key))
        .transpose()
}

async fn get_setting_explicit_exec(
    exec: SqlExec<'_, '_>,
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

    let normalized_scope_id = normalize_scope_id(scope_id);
    let has_scope_id = normalized_scope_id.is_some();
    let postgres_timestamps = exec_is_postgres(&exec);
    let sql = explicit_setting_sql(has_scope_id, postgres_timestamps);

    let mut args = vec![
        SqlArg::Text(scope.clone()),
        SqlArg::Text(key_name),
        SqlArg::Text(scope),
    ];
    if let Some(scope_id) = normalized_scope_id {
        args.push(SqlArg::Text(scope_id));
    }

    SqlRuntime::fetch_optional(exec, &sql, &args)
        .await?
        .as_ref()
        .map(|row| decode_settings_row(row, encryption_key))
        .transpose()
}

async fn upsert_setting_definition_tx(
    tx: &mut SqlTx<'_>,
    definition: SettingDefinitionSeed,
    now: chrono::DateTime<Utc>,
) -> AppResult<()> {
    let id = format!(
        "{}:{}:{}",
        definition.category.trim(),
        definition.scope.trim(),
        definition.key_name.trim()
    );
    let default_value = parse_setting_json_or_string(&definition.default_value_json, true)?;
    let validation = definition
        .validation_json
        .as_deref()
        .map(|value| parse_setting_json_or_string(value, false))
        .transpose()?;

    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "INSERT INTO settings_definitions
            (id, category, scope, key_name, data_type, default_value_json, is_sensitive,
             validation_json, created_at, updated_at)
         VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {})
         ON CONFLICT(category, scope, key_name) DO UPDATE SET
            category = excluded.category,
            scope = excluded.scope,
            key_name = excluded.key_name,
            data_type = excluded.data_type,
            default_value_json = excluded.default_value_json,
            is_sensitive = excluded.is_sensitive,
            validation_json = excluded.validation_json,
            updated_at = excluded.updated_at",
        &[
            SqlArg::Text(id),
            SqlArg::Text(definition.category.trim().to_string()),
            SqlArg::Text(definition.scope.trim().to_string()),
            SqlArg::Text(definition.key_name.trim().to_string()),
            SqlArg::Text(definition.data_type.trim().to_string()),
            SqlArg::Json(default_value),
            SqlArg::Bool(definition.is_sensitive),
            SqlArg::OptJson(validation),
            SqlArg::Timestamp(now),
            SqlArg::Timestamp(now),
        ],
    )
    .await?;
    Ok(())
}

#[expect(clippy::too_many_arguments)]
async fn upsert_setting_value_tx(
    tx: &mut SqlTx<'_>,
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

    let (definition_id, is_sensitive) = setting_definition_meta_tx(tx, &scope, &key_name)
        .await?
        .ok_or_else(|| {
            AppError::Validation(format!("unknown setting key: {}.{}", scope, key_name))
        })?;

    let value_json = value_json.trim();
    if value_json.is_empty() {
        return Err(AppError::Validation(
            "setting value cannot be empty".to_string(),
        ));
    }

    let stored_value = stored_setting_value(value_json, is_sensitive, encryption_key)?;
    let now = Utc::now();
    let normalized_scope_id = normalize_scope_id(scope_id);
    let existing_id = SqlRuntime::fetch_optional(
        SqlExec::Tx(tx),
        "SELECT id
           FROM settings_values
          WHERE setting_definition_id = {}
            AND scope = {}
            AND (({} IS NULL AND scope_id IS NULL) OR scope_id = {})",
        &[
            SqlArg::Text(definition_id.clone()),
            SqlArg::Text(scope.clone()),
            SqlArg::OptText(normalized_scope_id.clone()),
            SqlArg::OptText(normalized_scope_id.clone()),
        ],
    )
    .await?
    .map(|row| row.text("id"))
    .transpose()?;

    if let Some(existing_id) = existing_id {
        SqlRuntime::execute(
            SqlExec::Tx(tx),
            "UPDATE settings_values
                SET value_json = {},
                    source = {},
                    updated_by_user_id = {},
                    updated_at = {}
              WHERE id = {}",
            &[
                SqlArg::Json(stored_value),
                SqlArg::Text(source.trim().to_string()),
                SqlArg::OptText(updated_by_user_id),
                SqlArg::Timestamp(now),
                SqlArg::Text(existing_id),
            ],
        )
        .await?;
    } else {
        SqlRuntime::execute(
            SqlExec::Tx(tx),
            "INSERT INTO settings_values
                (id, setting_definition_id, scope, scope_id, value_json, source,
                 updated_by_user_id, created_at, updated_at)
             VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {})",
            &[
                SqlArg::Text(Id::new().0),
                SqlArg::Text(definition_id),
                SqlArg::Text(scope.clone()),
                SqlArg::OptText(normalized_scope_id.clone()),
                SqlArg::Json(stored_value),
                SqlArg::Text(source.trim().to_string()),
                SqlArg::OptText(updated_by_user_id),
                SqlArg::Timestamp(now),
                SqlArg::Timestamp(now),
            ],
        )
        .await?;
    }

    get_setting_with_defaults_exec(
        SqlExec::Tx(tx),
        &scope,
        &key_name,
        normalized_scope_id,
        encryption_key,
    )
    .await?
    .ok_or_else(|| AppError::Repository("setting write did not persist".to_string()))
}

async fn delete_setting_value_tx(
    tx: &mut SqlTx<'_>,
    scope: &str,
    key_name: &str,
    scope_id: Option<String>,
) -> AppResult<()> {
    let scope = scope.trim().to_string();
    let key_name = key_name.trim().to_string();
    if scope.is_empty() || key_name.is_empty() {
        return Err(AppError::Validation(
            "scope and key_name are required to delete a setting override".to_string(),
        ));
    }

    let (definition_id, _is_sensitive) = setting_definition_meta_tx(tx, &scope, &key_name)
        .await?
        .ok_or_else(|| {
        AppError::Validation(format!("unknown setting key: {}.{}", scope, key_name))
    })?;
    let normalized_scope_id = normalize_scope_id(scope_id);

    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "DELETE FROM settings_values
          WHERE setting_definition_id = {}
            AND scope = {}
            AND (({} IS NULL AND scope_id IS NULL) OR scope_id = {})",
        &[
            SqlArg::Text(definition_id),
            SqlArg::Text(scope),
            SqlArg::OptText(normalized_scope_id.clone()),
            SqlArg::OptText(normalized_scope_id),
        ],
    )
    .await?;
    Ok(())
}

async fn setting_definition_meta_tx(
    tx: &mut SqlTx<'_>,
    scope: &str,
    key_name: &str,
) -> AppResult<Option<(String, bool)>> {
    let rows = SqlRuntime::fetch_all(
        SqlExec::Tx(tx),
        "SELECT id, category, is_sensitive
           FROM settings_definitions
          WHERE scope = {} AND key_name = {}",
        &[
            SqlArg::Text(scope.to_string()),
            SqlArg::Text(key_name.to_string()),
        ],
    )
    .await?;

    if rows.is_empty() {
        return Ok(None);
    }
    if rows.len() > 1 {
        let mut categories = rows
            .iter()
            .map(|row| row.text("category"))
            .collect::<AppResult<Vec<_>>>()?;
        categories.sort();
        categories.dedup();
        return Err(AppError::Validation(format!(
            "ambiguous setting key {}.{} found in categories: {}",
            scope,
            key_name,
            categories.join(", ")
        )));
    }

    let row = rows.into_iter().next().expect("checked non-empty");
    Ok(Some((row.text("id")?, row.bool("is_sensitive")?)))
}

fn decode_settings_row(
    row: &SqlRow,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<SettingsValueRecord> {
    let default_value_json =
        json_to_logical_string(setting_json(row, "default_value_json")?, None)?;
    let validation_json = optional_setting_json(row, "validation_json")?
        .map(|value| json_to_logical_string(value, None))
        .transpose()?;
    let effective_value_json =
        json_to_logical_string(setting_json(row, "effective_value_json")?, encryption_key)?;
    let value_json = optional_setting_json(row, "value_json")?
        .map(|value| json_to_logical_string(value, encryption_key))
        .transpose()?;

    Ok(SettingsValueRecord {
        definition_id: row.text("definition_id")?,
        category: row.text("category")?,
        scope: row.text("scope")?,
        key_name: row.text("key_name")?,
        data_type: row.text("data_type")?,
        default_value_json,
        is_sensitive: row.bool("is_sensitive")?,
        validation_json,
        effective_value_json,
        value_json,
        source: row.opt_text("source")?,
        scope_id: row.opt_text("scope_id")?,
        updated_by_user_id: row.opt_text("updated_by_user_id")?,
        created_at: row.opt_text("created_at")?,
        updated_at: row.opt_text("updated_at")?,
    })
}

fn setting_json(row: &SqlRow, column: &str) -> AppResult<JsonValue> {
    Ok(optional_setting_json(row, column)?.unwrap_or(JsonValue::Null))
}

fn optional_setting_json(row: &SqlRow, column: &str) -> AppResult<Option<JsonValue>> {
    match row {
        SqlRow::Sqlite(row) => {
            let raw: Option<String> = row.try_get(column).map_err(repo_err)?;
            raw.map(|raw| parse_sqlite_json_storage(&raw)).transpose()
        }
        SqlRow::Postgres(row) => {
            let raw: Option<Json<JsonValue>> = row.try_get(column).map_err(repo_err)?;
            Ok(raw.map(|value| value.0))
        }
    }
}

fn parse_sqlite_json_storage(raw: &str) -> AppResult<JsonValue> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(JsonValue::Null);
    }

    match serde_json::from_str(trimmed) {
        Ok(value) => Ok(value),
        Err(_) => Ok(JsonValue::String(trimmed.to_string())),
    }
}

fn parse_setting_json_or_string(raw: &str, empty_as_null: bool) -> AppResult<JsonValue> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return if empty_as_null {
            Ok(JsonValue::Null)
        } else {
            Ok(JsonValue::String(String::new()))
        };
    }

    match serde_json::from_str(trimmed) {
        Ok(value) => Ok(value),
        Err(_) => Ok(JsonValue::String(trimmed.to_string())),
    }
}

fn stored_setting_value(
    value_json: &str,
    is_sensitive: bool,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<JsonValue> {
    let logical_value = parse_setting_json_or_string(value_json, false)?;
    if !is_sensitive {
        return Ok(logical_value);
    }

    let Some(key) = encryption_key else {
        return Ok(logical_value);
    };

    let logical_json = serde_json::to_string(&logical_value)
        .map_err(|error| AppError::Repository(error.to_string()))?;
    encrypt_value(key, &logical_json)
        .map(JsonValue::String)
        .map_err(|error| AppError::Repository(format!("failed to encrypt setting value: {error}")))
}

fn json_to_logical_string(
    value: JsonValue,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<String> {
    match value {
        JsonValue::String(stored) if is_encrypted(&stored) => {
            if let Some(key) = encryption_key {
                decrypt_value(key, &stored).map_err(|error| {
                    AppError::Repository(format!("failed to decrypt setting value: {error}"))
                })
            } else {
                Ok(stored)
            }
        }
        value => {
            serde_json::to_string(&value).map_err(|error| AppError::Repository(error.to_string()))
        }
    }
}

fn normalize_scope_id(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let value = value.trim().to_string();
        if value.is_empty() { None } else { Some(value) }
    })
}

fn exec_is_postgres(exec: &SqlExec<'_, '_>) -> bool {
    matches!(
        exec,
        SqlExec::Target(SqlTarget::Postgres(_)) | SqlExec::Tx(SqlTx::Postgres(_))
    )
}

#[cfg(test)]
mod tests {
    use super::settings_with_defaults_sql;

    #[test]
    fn single_key_reads_filter_on_key_name_and_skip_the_scope_ordering() {
        for has_scope_id in [false, true] {
            for postgres in [false, true] {
                let listing = settings_with_defaults_sql(has_scope_id, postgres, false);
                assert!(listing.contains("ORDER BY d.category, d.key_name"));
                assert!(!listing.contains("d.key_name = {}"));
                // scope join (1 or 2 binds) + the definition scope filter.
                assert_eq!(listing.matches("{}").count(), 2 + usize::from(has_scope_id));

                let point = settings_with_defaults_sql(has_scope_id, postgres, true);
                assert!(point.contains("AND d.key_name = {}"));
                assert!(point.contains("LIMIT 1"));
                assert!(!point.contains("ORDER BY"));
                // ... plus the key_name bind.
                assert_eq!(point.matches("{}").count(), 3 + usize::from(has_scope_id));
            }
        }
    }
}

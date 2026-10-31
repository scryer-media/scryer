use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use chrono::Utc;
use scryer_application::{
    AppError, AppResult, SubtitleProviderConfigRepository, SubtitleProviderConfigUpdate,
};
use scryer_domain::SubtitleProviderConfig;

use crate::config_store::{
    current_encryption_key, decrypt_value, enabled_facets_from_json, maybe_encrypt_value,
};
use crate::encryption::EncryptionKey;
use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRow, SqlRuntime, StoreDatastore, repo_err};

const SUBTITLE_PROVIDER_COLUMNS: &str = "id, name, provider_type, config_json, is_enabled,
    enabled_facets, last_health_status, last_error, last_error_at, disabled_until,
    created_at, updated_at";

const SUBTITLE_PROVIDER_INSERT_SQL: &str = "INSERT INTO subtitle_provider_configs (
    id, name, provider_type, config_json, is_enabled, enabled_facets,
    last_health_status, last_error, last_error_at, disabled_until, created_at, updated_at
) VALUES (
    {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}
)";

#[derive(Clone)]
pub struct SubtitleProviderConfigStore {
    datastore: StoreDatastore,
    encryption_key: Arc<RwLock<Option<EncryptionKey>>>,
}

impl SubtitleProviderConfigStore {
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
}

#[async_trait]
impl SubtitleProviderConfigRepository for SubtitleProviderConfigStore {
    async fn list(&self, provider_type: Option<String>) -> AppResult<Vec<SubtitleProviderConfig>> {
        let encryption_key = self.encryption_key()?;
        let (sql, args) = match provider_type {
            Some(provider_type) => (
                format!(
                    "SELECT {SUBTITLE_PROVIDER_COLUMNS} FROM subtitle_provider_configs WHERE provider_type = {{}} ORDER BY created_at DESC"
                ),
                vec![SqlArg::Text(provider_type)],
            ),
            None => (
                format!(
                    "SELECT {SUBTITLE_PROVIDER_COLUMNS} FROM subtitle_provider_configs ORDER BY created_at DESC"
                ),
                Vec::new(),
            ),
        };
        fetch_subtitle_providers(
            self.datastore.read_exec(),
            &sql,
            &args,
            encryption_key.as_ref(),
        )
        .await
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<SubtitleProviderConfig>> {
        let encryption_key = self.encryption_key()?;
        fetch_optional_subtitle_provider(
            self.datastore.read_exec(),
            &format!(
                "SELECT {SUBTITLE_PROVIDER_COLUMNS} FROM subtitle_provider_configs WHERE id = {{}}"
            ),
            &[SqlArg::Text(id.to_string())],
            encryption_key.as_ref(),
        )
        .await
    }

    async fn create(&self, config: SubtitleProviderConfig) -> AppResult<SubtitleProviderConfig> {
        let encryption_key = self.encryption_key()?;
        let args = subtitle_provider_insert_args(&config, encryption_key.as_ref())?;
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "create_subtitle_provider_config",
            move |tx| {
                let config = config.clone();
                let args = args.clone();
                Box::pin(async move {
                    SqlRuntime::execute(SqlExec::Tx(tx), SUBTITLE_PROVIDER_INSERT_SQL, &args)
                        .await?;
                    Ok(config)
                })
            },
        )
        .await
    }

    async fn update(
        &self,
        update: SubtitleProviderConfigUpdate,
    ) -> AppResult<SubtitleProviderConfig> {
        let encryption_key = self.encryption_key()?;
        let mut assignments = vec!["updated_at = {}".to_string()];
        let mut args = vec![SqlArg::Timestamp(Utc::now())];

        if let Some(name) = update.name.as_ref() {
            assignments.push("name = {}".to_string());
            args.push(SqlArg::Text(name.clone()));
        }
        if let Some(provider_type) = update.provider_type.as_ref() {
            assignments.push("provider_type = {}".to_string());
            args.push(SqlArg::Text(provider_type.clone()));
        }
        if let Some(config_json) = update.config_json.as_ref() {
            assignments.push("config_json = {}".to_string());
            args.push(SqlArg::Text(maybe_encrypt_value(
                encryption_key.as_ref(),
                config_json,
            )?));
        }
        if let Some(enabled_facets) = update.enabled_facets.as_ref() {
            assignments.push("enabled_facets = {}".to_string());
            args.push(SqlArg::Json(
                serde_json::to_value(enabled_facets).map_err(repo_err)?,
            ));
        }
        if let Some(is_enabled) = update.is_enabled {
            assignments.push("is_enabled = {}".to_string());
            args.push(SqlArg::Bool(is_enabled));
        }
        if let Some(last_health_status) = update.last_health_status.as_ref() {
            assignments.push("last_health_status = {}".to_string());
            args.push(SqlArg::Text(last_health_status.clone()));
        }
        if let Some(last_error) = update.last_error.as_ref() {
            assignments.push("last_error = {}".to_string());
            args.push(SqlArg::OptText(last_error.clone()));
        }
        if let Some(last_error_at) = update.last_error_at.as_ref() {
            assignments.push("last_error_at = {}".to_string());
            args.push(SqlArg::OptTimestamp(*last_error_at));
        }
        if let Some(disabled_until) = update.disabled_until.as_ref() {
            assignments.push("disabled_until = {}".to_string());
            args.push(SqlArg::OptTimestamp(*disabled_until));
        }

        if assignments.len() == 1 {
            return Err(AppError::validation_refused(
                crate::error_reasons::CONFIGURATION_SUBTITLE_PROVIDER_UPDATE_EMPTY,
                "at least one subtitle provider config field must be provided",
            ));
        }

        let id = update.id.clone();
        args.push(SqlArg::Text(id.clone()));
        let sql = format!(
            "UPDATE subtitle_provider_configs SET {} WHERE id = {{}}",
            assignments.join(", ")
        );
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "update_subtitle_provider_config",
            move |tx| {
                let sql = sql.clone();
                let args = args.clone();
                let id = id.clone();
                let encryption_key = encryption_key.clone();
                Box::pin(async move {
                    let rows = SqlRuntime::execute(SqlExec::Tx(tx), &sql, &args).await?;
                    if rows == 0 {
                        return Err(AppError::not_found_reason(crate::error_reasons::CONFIGURATION_SUBTITLE_PROVIDER_CONFIG_NOT_FOUND, format!("subtitle provider config {id}")));
                    }
                    fetch_optional_subtitle_provider(
                        SqlExec::Tx(tx),
                        &format!(
                            "SELECT {SUBTITLE_PROVIDER_COLUMNS} FROM subtitle_provider_configs WHERE id = {{}}"
                        ),
                        &[SqlArg::Text(id.clone())],
                        encryption_key.as_ref(),
                    )
                    .await?
                    .ok_or_else(|| AppError::not_found_reason(crate::error_reasons::CONFIGURATION_SUBTITLE_PROVIDER_CONFIG_NOT_FOUND, format!("subtitle provider config {id}")))
                })
            },
        )
        .await
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        let id = id.to_string();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "delete_subtitle_provider_config",
            move |tx| {
                let id = id.clone();
                Box::pin(async move {
                    let rows = SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "DELETE FROM subtitle_provider_configs WHERE id = {}",
                        &[SqlArg::Text(id.clone())],
                    )
                    .await?;
                    if rows == 0 {
                        return Err(AppError::not_found_reason(
                            crate::error_reasons::CONFIGURATION_SUBTITLE_PROVIDER_CONFIG_NOT_FOUND,
                            format!("subtitle provider config {id}"),
                        ));
                    }
                    Ok(())
                })
            },
        )
        .await
    }
}

fn subtitle_provider_insert_args(
    config: &SubtitleProviderConfig,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<Vec<SqlArg>> {
    Ok(vec![
        SqlArg::Text(config.id.clone()),
        SqlArg::Text(config.name.clone()),
        SqlArg::Text(config.provider_type.clone()),
        SqlArg::Text(maybe_encrypt_value(encryption_key, &config.config_json)?),
        SqlArg::Bool(config.is_enabled),
        SqlArg::Json(serde_json::to_value(&config.enabled_facets).map_err(repo_err)?),
        SqlArg::OptText(config.last_health_status.clone()),
        SqlArg::OptText(config.last_error.clone()),
        SqlArg::OptTimestamp(config.last_error_at),
        SqlArg::OptTimestamp(config.disabled_until),
        SqlArg::Timestamp(config.created_at),
        SqlArg::Timestamp(config.updated_at),
    ])
}

async fn fetch_subtitle_providers(
    exec: SqlExec<'_, '_>,
    sql: &str,
    args: &[SqlArg],
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<Vec<SubtitleProviderConfig>> {
    SqlRuntime::fetch_all(exec, sql, args)
        .await?
        .into_iter()
        .map(|row| row_to_subtitle_provider_config(&row, encryption_key))
        .collect()
}

async fn fetch_optional_subtitle_provider(
    exec: SqlExec<'_, '_>,
    sql: &str,
    args: &[SqlArg],
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<Option<SubtitleProviderConfig>> {
    SqlRuntime::fetch_optional(exec, sql, args)
        .await?
        .map(|row| row_to_subtitle_provider_config(&row, encryption_key))
        .transpose()
}

fn row_to_subtitle_provider_config(
    row: &SqlRow,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<SubtitleProviderConfig> {
    let enabled_facets = row
        .opt_json("enabled_facets")?
        .ok_or_else(|| AppError::Repository("enabled_facets must be a JSON array".to_string()))
        .and_then(enabled_facets_from_json)?;

    Ok(SubtitleProviderConfig {
        id: row.text("id")?,
        name: row.text("name")?,
        provider_type: row.text("provider_type")?,
        config_json: decrypt_value(
            encryption_key,
            row.text("config_json")?,
            "config_json",
            true,
        )?,
        enabled_facets,
        is_enabled: row.bool("is_enabled")?,
        last_health_status: row.opt_text("last_health_status")?,
        last_error: row.opt_text("last_error")?,
        last_error_at: row.opt_timestamp("last_error_at")?,
        disabled_until: row.opt_timestamp("disabled_until")?,
        created_at: row.timestamp("created_at")?,
        updated_at: row.timestamp("updated_at")?,
    })
}

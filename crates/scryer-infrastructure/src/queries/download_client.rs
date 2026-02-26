use chrono::{DateTime, Utc};
use scryer_application::{AppError, AppResult};
use scryer_domain::DownloadClientConfig;
use sqlx::{Row, SqlitePool};

use crate::encryption::EncryptionKey;

use super::common::{parse_optional_utc_datetime, parse_utc_datetime};

fn row_to_download_client_config(
    row: &sqlx::sqlite::SqliteRow,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<DownloadClientConfig> {
    let id: String = row
        .try_get("id")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let name: String = row
        .try_get("name")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let client_type: String = row
        .try_get("client_type")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let base_url: Option<String> = row
        .try_get("base_url")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let config_json_raw: String = row
        .try_get("config_json")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let client_priority: i64 = row
        .try_get("client_priority")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let is_enabled: i64 = row
        .try_get("is_enabled")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let status: String = row
        .try_get("status")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let last_error: Option<String> = row
        .try_get("last_error")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let last_seen_at_raw: Option<String> = row
        .try_get("last_seen_at")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let created_at_raw: String = row
        .try_get("created_at")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let updated_at_raw: String = row
        .try_get("updated_at")
        .map_err(|err| AppError::Repository(err.to_string()))?;

    // Decrypt config_json if encrypted
    let config_json = if crate::encryption::is_encrypted(&config_json_raw) {
        if let Some(key) = encryption_key {
            crate::encryption::decrypt_value(key, &config_json_raw)
                .map_err(|e| AppError::Repository(format!("failed to decrypt config_json: {e}")))?
        } else {
            config_json_raw
        }
    } else {
        config_json_raw
    };

    Ok(DownloadClientConfig {
        id,
        name,
        client_type,
        base_url,
        config_json,
        client_priority,
        is_enabled: is_enabled != 0,
        status,
        last_error,
        last_seen_at: parse_optional_utc_datetime(last_seen_at_raw)?,
        created_at: parse_utc_datetime(&created_at_raw)?,
        updated_at: parse_utc_datetime(&updated_at_raw)?,
    })
}

pub(crate) async fn list_download_client_configs_query(
    pool: &SqlitePool,
    client_type: Option<String>,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<Vec<DownloadClientConfig>> {
    let mut sql = String::from(
        "SELECT id, name, client_type, base_url, config_json, is_enabled, status,
                client_priority, last_error, last_seen_at, created_at, updated_at
           FROM download_clients",
    );

    if client_type.is_some() {
        sql.push_str(" WHERE client_type = ?");
    }

    sql.push_str(" ORDER BY client_priority ASC");

    let mut statement = sqlx::query(&sql);
    if let Some(client_type) = client_type {
        statement = statement.bind(client_type);
    }

    let rows = statement
        .fetch_all(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(row_to_download_client_config(&row, encryption_key)?);
    }

    Ok(out)
}

pub(crate) async fn get_download_client_config_query(
    pool: &SqlitePool,
    id: &str,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<Option<DownloadClientConfig>> {
    let row = sqlx::query(
        "SELECT id, name, client_type, base_url, config_json, is_enabled, status,
                client_priority, last_error, last_seen_at, created_at, updated_at
           FROM download_clients
          WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    row.map(|row| row_to_download_client_config(&row, encryption_key))
        .transpose()
}

fn maybe_encrypt_config_json(
    key: Option<&EncryptionKey>,
    config_json: &str,
) -> AppResult<String> {
    let Some(key) = key else {
        return Ok(config_json.to_string());
    };
    crate::encryption::encrypt_value(key, config_json)
        .map_err(|e| AppError::Repository(format!("failed to encrypt config_json: {e}")))
}

pub(crate) async fn create_download_client_config_query(
    pool: &SqlitePool,
    config: &DownloadClientConfig,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<DownloadClientConfig> {
    let stored_config_json = maybe_encrypt_config_json(encryption_key, &config.config_json)?;

    sqlx::query(
        "INSERT INTO download_clients
            (id, name, client_type, base_url, config_json, is_enabled, status,
             client_priority, last_error, last_seen_at, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&config.id)
    .bind(&config.name)
    .bind(&config.client_type)
    .bind(&config.base_url)
    .bind(&stored_config_json)
    .bind(if config.is_enabled { 1_i64 } else { 0_i64 })
    .bind(&config.status)
    .bind(config.client_priority)
    .bind(&config.last_error)
    .bind(
        config
            .last_seen_at
            .as_ref()
            .map(DateTime::<Utc>::to_rfc3339),
    )
    .bind(config.created_at.to_rfc3339())
    .bind(config.updated_at.to_rfc3339())
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(config.clone())
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn update_download_client_config_query(
    pool: &SqlitePool,
    id: &str,
    name: Option<String>,
    client_type: Option<String>,
    base_url: Option<String>,
    config_json: Option<String>,
    is_enabled: Option<bool>,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<DownloadClientConfig> {
    let mut assignments = vec!["updated_at = ?".to_string()];

    if name.is_some() {
        assignments.push("name = ?".to_string());
    }
    if client_type.is_some() {
        assignments.push("client_type = ?".to_string());
    }
    if base_url.is_some() {
        assignments.push("base_url = ?".to_string());
    }
    if config_json.is_some() {
        assignments.push("config_json = ?".to_string());
    }
    if is_enabled.is_some() {
        assignments.push("is_enabled = ?".to_string());
    }

    if assignments.len() == 1 {
        return Err(AppError::Validation(
            "at least one download client config field must be provided".into(),
        ));
    }

    let mut sql = String::from("UPDATE download_clients SET ");
    sql.push_str(&assignments.join(", "));
    sql.push_str(" WHERE id = ?");

    let mut statement = sqlx::query(&sql);
    statement = statement.bind(Utc::now().to_rfc3339());

    if let Some(name) = name {
        statement = statement.bind(name);
    }
    if let Some(client_type) = client_type {
        statement = statement.bind(client_type);
    }
    if let Some(base_url) = base_url {
        statement = statement.bind(base_url);
    }
    if let Some(config_json) = config_json {
        let stored = maybe_encrypt_config_json(encryption_key, &config_json)?;
        statement = statement.bind(stored);
    }
    if let Some(is_enabled) = is_enabled {
        statement = statement.bind(if is_enabled { 1_i64 } else { 0_i64 });
    }

    statement = statement.bind(id);

    let result = statement
        .execute(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("download client config {}", id)));
    }

    get_download_client_config_query(pool, id, encryption_key)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("download client config {}", id)))
}

pub(crate) async fn delete_download_client_config_query(
    pool: &SqlitePool,
    id: &str,
) -> AppResult<()> {
    let result = sqlx::query("DELETE FROM download_clients WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("download client config {}", id)));
    }

    Ok(())
}

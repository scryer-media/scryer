use chrono::Utc;
use scryer_application::{AppError, AppResult};
use scryer_domain::NotificationChannelConfig;
use sqlx::{Row, SqlitePool};

use crate::encryption::EncryptionKey;

use super::common::parse_utc_datetime;

fn row_to_channel(
    row: &sqlx::sqlite::SqliteRow,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<NotificationChannelConfig> {
    let id: String = row.try_get("id").map_err(|e| AppError::Repository(e.to_string()))?;
    let name: String = row.try_get("name").map_err(|e| AppError::Repository(e.to_string()))?;
    let channel_type: String = row.try_get("channel_type").map_err(|e| AppError::Repository(e.to_string()))?;
    let config_json_raw: String = row.try_get("config_json").map_err(|e| AppError::Repository(e.to_string()))?;
    let is_enabled: i64 = row.try_get("is_enabled").map_err(|e| AppError::Repository(e.to_string()))?;
    let created_at_raw: String = row.try_get("created_at").map_err(|e| AppError::Repository(e.to_string()))?;
    let updated_at_raw: String = row.try_get("updated_at").map_err(|e| AppError::Repository(e.to_string()))?;

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

    Ok(NotificationChannelConfig {
        id,
        name,
        channel_type,
        config_json,
        is_enabled: is_enabled != 0,
        created_at: parse_utc_datetime(&created_at_raw)?,
        updated_at: parse_utc_datetime(&updated_at_raw)?,
    })
}

fn maybe_encrypt(key: Option<&EncryptionKey>, value: &str) -> AppResult<String> {
    match key {
        Some(k) => crate::encryption::encrypt_value(k, value)
            .map_err(|e| AppError::Repository(format!("failed to encrypt config_json: {e}"))),
        None => Ok(value.to_string()),
    }
}

pub(crate) async fn list_notification_channels_query(
    pool: &SqlitePool,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<Vec<NotificationChannelConfig>> {
    let rows = sqlx::query(
        "SELECT id, name, channel_type, config_json, is_enabled, created_at, updated_at
         FROM notification_channels ORDER BY created_at DESC",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::Repository(e.to_string()))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(row_to_channel(&row, encryption_key)?);
    }
    Ok(out)
}

pub(crate) async fn get_notification_channel_query(
    pool: &SqlitePool,
    id: &str,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<Option<NotificationChannelConfig>> {
    let row = sqlx::query(
        "SELECT id, name, channel_type, config_json, is_enabled, created_at, updated_at
         FROM notification_channels WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::Repository(e.to_string()))?;

    row.map(|r| row_to_channel(&r, encryption_key)).transpose()
}

pub(crate) async fn create_notification_channel_query(
    pool: &SqlitePool,
    config: &NotificationChannelConfig,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<NotificationChannelConfig> {
    let stored_config = maybe_encrypt(encryption_key, &config.config_json)?;

    sqlx::query(
        "INSERT INTO notification_channels (id, name, channel_type, config_json, is_enabled, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&config.id)
    .bind(&config.name)
    .bind(&config.channel_type)
    .bind(&stored_config)
    .bind(if config.is_enabled { 1_i64 } else { 0_i64 })
    .bind(config.created_at.to_rfc3339())
    .bind(config.updated_at.to_rfc3339())
    .execute(pool)
    .await
    .map_err(|e| AppError::Repository(e.to_string()))?;

    Ok(config.clone())
}

pub(crate) async fn update_notification_channel_query(
    pool: &SqlitePool,
    config: &NotificationChannelConfig,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<NotificationChannelConfig> {
    let stored_config = maybe_encrypt(encryption_key, &config.config_json)?;

    let result = sqlx::query(
        "UPDATE notification_channels SET name = ?, config_json = ?, is_enabled = ?, updated_at = ? WHERE id = ?",
    )
    .bind(&config.name)
    .bind(&stored_config)
    .bind(if config.is_enabled { 1_i64 } else { 0_i64 })
    .bind(Utc::now().to_rfc3339())
    .bind(&config.id)
    .execute(pool)
    .await
    .map_err(|e| AppError::Repository(e.to_string()))?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("notification channel {}", config.id)));
    }

    get_notification_channel_query(pool, &config.id, encryption_key)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("notification channel {}", config.id)))
}

pub(crate) async fn delete_notification_channel_query(
    pool: &SqlitePool,
    id: &str,
) -> AppResult<()> {
    let result = sqlx::query("DELETE FROM notification_channels WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| AppError::Repository(e.to_string()))?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("notification channel {id}")));
    }
    Ok(())
}

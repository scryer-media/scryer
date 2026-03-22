use chrono::Utc;
use scryer_application::{AppError, AppResult};
use scryer_domain::NotificationSubscription;
use sqlx::{Row, SqlitePool};

use super::common::parse_utc_datetime;

fn row_to_subscription(row: &sqlx::sqlite::SqliteRow) -> AppResult<NotificationSubscription> {
    let id: String = row
        .try_get("id")
        .map_err(|e| AppError::Repository(e.to_string()))?;
    let channel_id: String = row
        .try_get("channel_id")
        .map_err(|e| AppError::Repository(e.to_string()))?;
    let event_type_str: String = row
        .try_get("event_type")
        .map_err(|e| AppError::Repository(e.to_string()))?;
    let event_type = scryer_domain::NotificationEventType::parse(&event_type_str)
        .ok_or_else(|| AppError::Repository(format!("unknown event_type: {event_type_str}")))?;
    let scope: String = row
        .try_get("scope")
        .map_err(|e| AppError::Repository(e.to_string()))?;
    let scope_id: Option<String> = row
        .try_get("scope_id")
        .map_err(|e| AppError::Repository(e.to_string()))?;
    let is_enabled: i64 = row
        .try_get("is_enabled")
        .map_err(|e| AppError::Repository(e.to_string()))?;
    let created_at_raw: String = row
        .try_get("created_at")
        .map_err(|e| AppError::Repository(e.to_string()))?;
    let updated_at_raw: String = row
        .try_get("updated_at")
        .map_err(|e| AppError::Repository(e.to_string()))?;

    Ok(NotificationSubscription {
        id,
        channel_id,
        event_type,
        scope,
        scope_id,
        is_enabled: is_enabled != 0,
        created_at: parse_utc_datetime(&created_at_raw)?,
        updated_at: parse_utc_datetime(&updated_at_raw)?,
    })
}

pub(crate) async fn list_notification_subscriptions_query(
    pool: &SqlitePool,
) -> AppResult<Vec<NotificationSubscription>> {
    let rows = sqlx::query(
        "SELECT id, channel_id, event_type, scope, scope_id, is_enabled, created_at, updated_at
         FROM notification_subscriptions ORDER BY created_at DESC",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::Repository(e.to_string()))?;

    rows.iter().map(row_to_subscription).collect()
}

pub(crate) async fn list_notification_subscriptions_for_channel_query(
    pool: &SqlitePool,
    channel_id: &str,
) -> AppResult<Vec<NotificationSubscription>> {
    let rows = sqlx::query(
        "SELECT id, channel_id, event_type, scope, scope_id, is_enabled, created_at, updated_at
         FROM notification_subscriptions WHERE channel_id = ? ORDER BY created_at DESC",
    )
    .bind(channel_id)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::Repository(e.to_string()))?;

    rows.iter().map(row_to_subscription).collect()
}

pub(crate) async fn list_notification_subscriptions_for_event_query(
    pool: &SqlitePool,
    event_type: &str,
) -> AppResult<Vec<NotificationSubscription>> {
    let rows = sqlx::query(
        "SELECT id, channel_id, event_type, scope, scope_id, is_enabled, created_at, updated_at
         FROM notification_subscriptions WHERE event_type = ? ORDER BY created_at DESC",
    )
    .bind(event_type)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::Repository(e.to_string()))?;

    rows.iter().map(row_to_subscription).collect()
}

pub(crate) async fn create_notification_subscription_query(
    pool: &SqlitePool,
    sub: &NotificationSubscription,
) -> AppResult<NotificationSubscription> {
    sqlx::query(
        "INSERT INTO notification_subscriptions (id, channel_id, event_type, scope, scope_id, is_enabled, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&sub.id)
    .bind(&sub.channel_id)
    .bind(sub.event_type.as_str())
    .bind(&sub.scope)
    .bind(&sub.scope_id)
    .bind(if sub.is_enabled { 1_i64 } else { 0_i64 })
    .bind(sub.created_at.to_rfc3339())
    .bind(sub.updated_at.to_rfc3339())
    .execute(pool)
    .await
    .map_err(|e| AppError::Repository(e.to_string()))?;

    Ok(sub.clone())
}

pub(crate) async fn update_notification_subscription_query(
    pool: &SqlitePool,
    sub: &NotificationSubscription,
) -> AppResult<NotificationSubscription> {
    let result = sqlx::query(
        "UPDATE notification_subscriptions
         SET event_type = ?, scope = ?, scope_id = ?, is_enabled = ?, updated_at = ?
         WHERE id = ?",
    )
    .bind(sub.event_type.as_str())
    .bind(&sub.scope)
    .bind(&sub.scope_id)
    .bind(if sub.is_enabled { 1_i64 } else { 0_i64 })
    .bind(Utc::now().to_rfc3339())
    .bind(&sub.id)
    .execute(pool)
    .await
    .map_err(|e| AppError::Repository(e.to_string()))?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!(
            "notification subscription {}",
            sub.id
        )));
    }

    Ok(sub.clone())
}

pub(crate) async fn delete_notification_subscription_query(
    pool: &SqlitePool,
    id: &str,
) -> AppResult<()> {
    let result = sqlx::query("DELETE FROM notification_subscriptions WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| AppError::Repository(e.to_string()))?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!(
            "notification subscription {id}"
        )));
    }
    Ok(())
}

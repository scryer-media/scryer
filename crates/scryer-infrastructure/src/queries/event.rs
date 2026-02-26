use scryer_application::{AppError, AppResult};
use scryer_domain::{EventType, HistoryEvent};
use sqlx::{Row, SqlitePool};

use super::common::parse_utc_datetime;

fn parse_event_type(raw: &str) -> EventType {
    match raw.to_lowercase().as_str() {
        "titleadded" => EventType::TitleAdded,
        "titleupdated" => EventType::TitleUpdated,
        "policyevaluated" => EventType::PolicyEvaluated,
        "actiontriggered" => EventType::ActionTriggered,
        "actioncompleted" => EventType::ActionCompleted,
        _ => EventType::Error,
    }
}
pub(crate) async fn list_events_query(
    pool: &SqlitePool,
    title_id: Option<String>,
    limit: i64,
    offset: i64,
) -> AppResult<Vec<HistoryEvent>> {
    let mut sql = String::from(
        "SELECT id, event_type, actor_user_id, title_id, message, occurred_at FROM history_events",
    );

    if title_id.is_some() {
        sql.push_str(" WHERE title_id = ?");
    }

    sql.push_str(" ORDER BY occurred_at DESC LIMIT ? OFFSET ?");

    let mut statement = sqlx::query(&sql);
    if let Some(id) = title_id {
        statement = statement.bind(id);
    }

    let clamped_limit = if limit <= 0 { 50 } else { limit };
    let clamped_offset = if offset <= 0 { 0 } else { offset };

    let rows = statement
        .bind(clamped_limit)
        .bind(clamped_offset)
        .fetch_all(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let id: String = row
            .try_get("id")
            .map_err(|err| AppError::Repository(err.to_string()))?;
        let raw_event_type: String = row
            .try_get("event_type")
            .map_err(|err| AppError::Repository(err.to_string()))?;
        let actor_user_id: Option<String> = row
            .try_get("actor_user_id")
            .map_err(|err| AppError::Repository(err.to_string()))?;
        let event_title_id: Option<String> = row
            .try_get("title_id")
            .map_err(|err| AppError::Repository(err.to_string()))?;
        let message: String = row
            .try_get("message")
            .map_err(|err| AppError::Repository(err.to_string()))?;
        let occurred_at_raw: String = row
            .try_get("occurred_at")
            .map_err(|err| AppError::Repository(err.to_string()))?;

        let occurred_at = parse_utc_datetime(&occurred_at_raw)?;

        out.push(HistoryEvent {
            id,
            event_type: parse_event_type(&raw_event_type),
            actor_user_id,
            title_id: event_title_id,
            message,
            occurred_at,
        });
    }

    Ok(out)
}

pub(crate) async fn append_event_query(pool: &SqlitePool, event: &HistoryEvent) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO history_events (id, event_type, actor_user_id, title_id, message, occurred_at, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&event.id)
    .bind(format!("{:?}", event.event_type).to_lowercase())
    .bind(&event.actor_user_id)
    .bind(&event.title_id)
    .bind(&event.message)
    .bind(event.occurred_at.to_rfc3339())
    .bind(event.occurred_at.to_rfc3339())
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(())
}

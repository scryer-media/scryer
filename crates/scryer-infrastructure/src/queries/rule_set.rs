use scryer_application::{AppError, AppResult};
use scryer_domain::RuleSet;
use sqlx::{Row, SqlitePool};

pub(crate) async fn list_rule_sets_query(pool: &SqlitePool) -> AppResult<Vec<RuleSet>> {
    let rows = sqlx::query(
        "SELECT id, name, description, rego_source, enabled, priority, applied_facets,
                created_at, updated_at, is_managed, managed_key
           FROM rule_sets
          ORDER BY priority DESC, name",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::Repository(e.to_string()))?;

    rows.into_iter().map(|row| row_to_rule_set(&row)).collect()
}

pub(crate) async fn list_enabled_rule_sets_query(pool: &SqlitePool) -> AppResult<Vec<RuleSet>> {
    let rows = sqlx::query(
        "SELECT id, name, description, rego_source, enabled, priority, applied_facets,
                created_at, updated_at, is_managed, managed_key
           FROM rule_sets
          WHERE enabled = 1
          ORDER BY priority DESC, name",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::Repository(e.to_string()))?;

    rows.into_iter().map(|row| row_to_rule_set(&row)).collect()
}

pub(crate) async fn get_rule_set_by_id_query(
    pool: &SqlitePool,
    id: &str,
) -> AppResult<Option<RuleSet>> {
    let row = sqlx::query(
        "SELECT id, name, description, rego_source, enabled, priority, applied_facets,
                created_at, updated_at, is_managed, managed_key
           FROM rule_sets
          WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::Repository(e.to_string()))?;

    match row {
        Some(row) => Ok(Some(row_to_rule_set(&row)?)),
        None => Ok(None),
    }
}

pub(crate) async fn insert_rule_set_query(pool: &SqlitePool, rule_set: &RuleSet) -> AppResult<()> {
    let facets_json = serde_json::to_string(&rule_set.applied_facets)
        .map_err(|e| AppError::Repository(e.to_string()))?;

    sqlx::query(
        "INSERT INTO rule_sets (id, name, description, rego_source, enabled, priority,
                                applied_facets, created_at, updated_at, is_managed, managed_key)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&rule_set.id)
    .bind(&rule_set.name)
    .bind(&rule_set.description)
    .bind(&rule_set.rego_source)
    .bind(rule_set.enabled)
    .bind(rule_set.priority)
    .bind(&facets_json)
    .bind(rule_set.created_at.to_rfc3339())
    .bind(rule_set.updated_at.to_rfc3339())
    .bind(rule_set.is_managed as i32)
    .bind(&rule_set.managed_key)
    .execute(pool)
    .await
    .map_err(|e| AppError::Repository(e.to_string()))?;

    Ok(())
}

pub(crate) async fn update_rule_set_query(pool: &SqlitePool, rule_set: &RuleSet) -> AppResult<()> {
    let facets_json = serde_json::to_string(&rule_set.applied_facets)
        .map_err(|e| AppError::Repository(e.to_string()))?;

    sqlx::query(
        "UPDATE rule_sets
            SET name = ?, description = ?, rego_source = ?, enabled = ?, priority = ?,
                applied_facets = ?, updated_at = ?, is_managed = ?, managed_key = ?
          WHERE id = ?",
    )
    .bind(&rule_set.name)
    .bind(&rule_set.description)
    .bind(&rule_set.rego_source)
    .bind(rule_set.enabled)
    .bind(rule_set.priority)
    .bind(&facets_json)
    .bind(rule_set.updated_at.to_rfc3339())
    .bind(rule_set.is_managed as i32)
    .bind(&rule_set.managed_key)
    .bind(&rule_set.id)
    .execute(pool)
    .await
    .map_err(|e| AppError::Repository(e.to_string()))?;

    Ok(())
}

pub(crate) async fn delete_rule_set_query(pool: &SqlitePool, id: &str) -> AppResult<()> {
    sqlx::query("DELETE FROM rule_sets WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| AppError::Repository(e.to_string()))?;
    Ok(())
}

pub(crate) async fn get_rule_set_by_managed_key_query(
    pool: &SqlitePool,
    key: &str,
) -> AppResult<Option<RuleSet>> {
    let row = sqlx::query(
        "SELECT id, name, description, rego_source, enabled, priority, applied_facets,
                created_at, updated_at, is_managed, managed_key
           FROM rule_sets
          WHERE managed_key = ?",
    )
    .bind(key)
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::Repository(e.to_string()))?;

    match row {
        Some(row) => Ok(Some(row_to_rule_set(&row)?)),
        None => Ok(None),
    }
}

pub(crate) async fn delete_rule_set_by_managed_key_query(
    pool: &SqlitePool,
    key: &str,
) -> AppResult<()> {
    sqlx::query("DELETE FROM rule_sets WHERE managed_key = ?")
        .bind(key)
        .execute(pool)
        .await
        .map_err(|e| AppError::Repository(e.to_string()))?;
    Ok(())
}

pub(crate) async fn list_rule_sets_by_managed_key_prefix_query(
    pool: &SqlitePool,
    prefix: &str,
) -> AppResult<Vec<RuleSet>> {
    let pattern = format!("{}%", prefix);
    let rows = sqlx::query(
        "SELECT id, name, description, rego_source, enabled, priority, applied_facets,
                created_at, updated_at, is_managed, managed_key
           FROM rule_sets
          WHERE managed_key LIKE ?
          ORDER BY managed_key",
    )
    .bind(&pattern)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::Repository(e.to_string()))?;

    rows.into_iter().map(|row| row_to_rule_set(&row)).collect()
}

pub(crate) async fn insert_rule_set_history_query(
    pool: &SqlitePool,
    id: &str,
    rule_set_id: &str,
    action: &str,
    rego_source: Option<&str>,
    actor_id: Option<&str>,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO rule_set_history (id, rule_set_id, action, rego_source, actor_id)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(rule_set_id)
    .bind(action)
    .bind(rego_source)
    .bind(actor_id)
    .execute(pool)
    .await
    .map_err(|e| AppError::Repository(e.to_string()))?;
    Ok(())
}

fn row_to_rule_set(row: &sqlx::sqlite::SqliteRow) -> AppResult<RuleSet> {
    use chrono::{DateTime, Utc};
    use scryer_domain::MediaFacet;

    let facets_json: String = row
        .try_get("applied_facets")
        .map_err(|e| AppError::Repository(e.to_string()))?;
    let applied_facets: Vec<MediaFacet> = serde_json::from_str(&facets_json).unwrap_or_default();

    let created_str: String = row
        .try_get("created_at")
        .map_err(|e| AppError::Repository(e.to_string()))?;
    let updated_str: String = row
        .try_get("updated_at")
        .map_err(|e| AppError::Repository(e.to_string()))?;

    let created_at = DateTime::parse_from_rfc3339(&created_str)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());
    let updated_at = DateTime::parse_from_rfc3339(&updated_str)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());

    let enabled_int: i32 = row
        .try_get("enabled")
        .map_err(|e| AppError::Repository(e.to_string()))?;

    let is_managed_int: i32 = row
        .try_get("is_managed")
        .map_err(|e| AppError::Repository(e.to_string()))?;
    let managed_key: Option<String> = row
        .try_get("managed_key")
        .map_err(|e| AppError::Repository(e.to_string()))?;

    Ok(RuleSet {
        id: row
            .try_get("id")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        name: row
            .try_get("name")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        description: row
            .try_get("description")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        rego_source: row
            .try_get("rego_source")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        enabled: enabled_int != 0,
        priority: row
            .try_get("priority")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        applied_facets,
        created_at,
        updated_at,
        is_managed: is_managed_int != 0,
        managed_key,
    })
}

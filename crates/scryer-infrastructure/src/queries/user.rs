use scryer_application::{AppError, AppResult};
use scryer_domain::{Entitlement, User};
use serde_json;
use sqlx::SqlitePool;

pub(crate) async fn create_user_query(pool: &SqlitePool, user: &User) -> AppResult<User> {
    let entitlements_json = serde_json::to_string(&user.entitlements)
        .map_err(|err| AppError::Repository(err.to_string()))?;

    sqlx::query(
        "INSERT INTO users (id, username, entitlements, password_hash) VALUES (?, ?, ?, ?)",
    )
    .bind(&user.id)
    .bind(&user.username)
    .bind(&entitlements_json)
    .bind(&user.password_hash)
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(user.clone())
}

pub(crate) async fn get_user_by_id_query(pool: &SqlitePool, id: &str) -> AppResult<Option<User>> {
    let row = sqlx::query_as::<_, (String, String, String, Option<String>)>(
        "SELECT id, username, entitlements, password_hash FROM users WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    match row {
        Some((id, username, entitlements_raw, password_hash)) => {
            let entitlements: Vec<Entitlement> = serde_json::from_str(&entitlements_raw)
                .map_err(|err| AppError::Repository(err.to_string()))?;
            Ok(Some(User {
                id,
                username,
                password_hash,
                entitlements,
            }))
        }
        None => Ok(None),
    }
}

pub(crate) async fn get_user_by_username_query(
    pool: &SqlitePool,
    username: &str,
) -> AppResult<Option<User>> {
    let row = sqlx::query_as::<_, (String, String, String, Option<String>)>(
        "SELECT id, username, entitlements, password_hash FROM users WHERE username = ?",
    )
    .bind(username)
    .fetch_optional(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    match row {
        Some((id, username, entitlements_raw, password_hash)) => {
            let entitlements: Vec<Entitlement> = serde_json::from_str(&entitlements_raw)
                .map_err(|err| AppError::Repository(err.to_string()))?;
            Ok(Some(User {
                id,
                username,
                password_hash,
                entitlements,
            }))
        }
        None => Ok(None),
    }
}

pub(crate) async fn list_users_query(pool: &SqlitePool) -> AppResult<Vec<User>> {
    let rows = sqlx::query_as::<_, (String, String, String, Option<String>)>(
        "SELECT id, username, entitlements, password_hash FROM users",
    )
    .fetch_all(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    rows.into_iter()
        .map(|(id, username, entitlements_json, password_hash)| {
            let entitlements: Vec<Entitlement> = serde_json::from_str(&entitlements_json)
                .map_err(|err| AppError::Repository(err.to_string()))?;
            Ok(User {
                id,
                username,
                password_hash,
                entitlements,
            })
        })
        .collect()
}

pub(crate) async fn update_user_entitlements_query(
    pool: &SqlitePool,
    id: &str,
    entitlements_json: &str,
) -> AppResult<User> {
    let result = sqlx::query("UPDATE users SET entitlements = ? WHERE id = ?")
        .bind(entitlements_json)
        .bind(id)
        .execute(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("user {}", id)));
    }

    get_user_by_id_query(pool, id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("user {}", id)))
}

pub(crate) async fn update_user_password_query(
    pool: &SqlitePool,
    id: &str,
    password_hash: &str,
) -> AppResult<User> {
    let result = sqlx::query("UPDATE users SET password_hash = ? WHERE id = ?")
        .bind(password_hash)
        .bind(id)
        .execute(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("user {}", id)));
    }

    get_user_by_id_query(pool, id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("user {}", id)))
}

pub(crate) async fn delete_user_query(pool: &SqlitePool, id: &str) -> AppResult<()> {
    let result = sqlx::query("DELETE FROM users WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("user {}", id)));
    }

    Ok(())
}

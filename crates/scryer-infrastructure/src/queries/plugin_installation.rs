use scryer_application::AppResult;
use scryer_domain::PluginInstallation;
use sqlx::SqlitePool;

fn row_to_plugin_installation(row: &sqlx::sqlite::SqliteRow) -> PluginInstallation {
    use chrono::{DateTime, Utc};
    use sqlx::Row;

    let installed_str: String = row.get("installed_at");
    let updated_str: String = row.get("updated_at");

    let installed_at = DateTime::parse_from_rfc3339(&installed_str)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());
    let updated_at = DateTime::parse_from_rfc3339(&updated_str)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());

    PluginInstallation {
        id: row.get("id"),
        plugin_id: row.get("plugin_id"),
        name: row.get("name"),
        description: row.get("description"),
        version: row.get("version"),
        plugin_type: row.get("plugin_type"),
        provider_type: row.get("provider_type"),
        is_enabled: row.get::<i32, _>("is_enabled") != 0,
        is_builtin: row.get::<i32, _>("is_builtin") != 0,
        wasm_sha256: row.get("wasm_sha256"),
        source_url: row.get("source_url"),
        installed_at,
        updated_at,
    }
}

pub(crate) async fn list_plugin_installations_query(
    pool: &SqlitePool,
) -> AppResult<Vec<PluginInstallation>> {
    let rows = sqlx::query(
        "SELECT id, plugin_id, name, description, version, plugin_type, provider_type,
                is_enabled, is_builtin, wasm_sha256, source_url, installed_at, updated_at
         FROM plugin_installations
         WHERE plugin_type != '__cache'
         ORDER BY is_builtin DESC, name ASC",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| scryer_application::AppError::Repository(e.to_string()))?;

    Ok(rows.iter().map(row_to_plugin_installation).collect())
}

pub(crate) async fn get_plugin_installation_query(
    pool: &SqlitePool,
    plugin_id: &str,
) -> AppResult<Option<PluginInstallation>> {
    let row = sqlx::query(
        "SELECT id, plugin_id, name, description, version, plugin_type, provider_type,
                is_enabled, is_builtin, wasm_sha256, source_url, installed_at, updated_at
         FROM plugin_installations
         WHERE plugin_id = ?",
    )
    .bind(plugin_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| scryer_application::AppError::Repository(e.to_string()))?;

    Ok(row.as_ref().map(row_to_plugin_installation))
}

pub(crate) async fn create_plugin_installation_query(
    pool: &SqlitePool,
    installation: &PluginInstallation,
    wasm_bytes: Option<&[u8]>,
) -> AppResult<PluginInstallation> {
    sqlx::query(
        "INSERT INTO plugin_installations
            (id, plugin_id, name, description, version, plugin_type, provider_type,
             is_enabled, is_builtin, wasm_bytes, wasm_sha256, source_url, installed_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&installation.id)
    .bind(&installation.plugin_id)
    .bind(&installation.name)
    .bind(&installation.description)
    .bind(&installation.version)
    .bind(&installation.plugin_type)
    .bind(&installation.provider_type)
    .bind(installation.is_enabled as i32)
    .bind(installation.is_builtin as i32)
    .bind(wasm_bytes)
    .bind(&installation.wasm_sha256)
    .bind(&installation.source_url)
    .bind(installation.installed_at.to_rfc3339())
    .bind(installation.updated_at.to_rfc3339())
    .execute(pool)
    .await
    .map_err(|e| scryer_application::AppError::Repository(e.to_string()))?;

    get_plugin_installation_query(pool, &installation.plugin_id)
        .await?
        .ok_or_else(|| {
            scryer_application::AppError::Repository(
                "failed to read back created plugin installation".to_string(),
            )
        })
}

pub(crate) async fn update_plugin_installation_query(
    pool: &SqlitePool,
    installation: &PluginInstallation,
    wasm_bytes: Option<&[u8]>,
) -> AppResult<PluginInstallation> {
    sqlx::query(
        "UPDATE plugin_installations
         SET name = ?, description = ?, version = ?, is_enabled = ?,
             wasm_bytes = COALESCE(?, wasm_bytes),
             wasm_sha256 = COALESCE(?, wasm_sha256),
             updated_at = ?
         WHERE plugin_id = ?",
    )
    .bind(&installation.name)
    .bind(&installation.description)
    .bind(&installation.version)
    .bind(installation.is_enabled as i32)
    .bind(wasm_bytes)
    .bind(&installation.wasm_sha256)
    .bind(installation.updated_at.to_rfc3339())
    .bind(&installation.plugin_id)
    .execute(pool)
    .await
    .map_err(|e| scryer_application::AppError::Repository(e.to_string()))?;

    get_plugin_installation_query(pool, &installation.plugin_id)
        .await?
        .ok_or_else(|| {
            scryer_application::AppError::Repository(
                "failed to read back updated plugin installation".to_string(),
            )
        })
}

pub(crate) async fn delete_plugin_installation_query(
    pool: &SqlitePool,
    plugin_id: &str,
) -> AppResult<()> {
    sqlx::query("DELETE FROM plugin_installations WHERE plugin_id = ?")
        .bind(plugin_id)
        .execute(pool)
        .await
        .map_err(|e| scryer_application::AppError::Repository(e.to_string()))?;
    Ok(())
}

pub(crate) async fn get_enabled_plugin_wasm_bytes_query(
    pool: &SqlitePool,
) -> AppResult<Vec<(PluginInstallation, Option<Vec<u8>>)>> {
    use sqlx::Row;
    let rows = sqlx::query(
        "SELECT id, plugin_id, name, description, version, plugin_type, provider_type,
                is_enabled, is_builtin, wasm_bytes, wasm_sha256, source_url, installed_at, updated_at
         FROM plugin_installations
         WHERE is_enabled = 1 AND plugin_type != '__cache'",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| scryer_application::AppError::Repository(e.to_string()))?;

    Ok(rows
        .iter()
        .map(|row| {
            let installation = row_to_plugin_installation(row);
            let wasm_bytes: Option<Vec<u8>> = row.get("wasm_bytes");
            (installation, wasm_bytes)
        })
        .collect())
}

pub(crate) async fn seed_builtin_query(
    pool: &SqlitePool,
    plugin_id: &str,
    name: &str,
    description: &str,
    version: &str,
    provider_type: &str,
) -> AppResult<()> {
    let now = chrono::Utc::now().to_rfc3339();
    let id = scryer_domain::Id::new().0;
    sqlx::query(
        "INSERT OR IGNORE INTO plugin_installations
            (id, plugin_id, name, description, version, plugin_type, provider_type,
             is_enabled, is_builtin, installed_at, updated_at)
         VALUES (?, ?, ?, ?, ?, 'indexer', ?, 1, 1, ?, ?)",
    )
    .bind(&id)
    .bind(plugin_id)
    .bind(name)
    .bind(description)
    .bind(version)
    .bind(provider_type)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|e| scryer_application::AppError::Repository(e.to_string()))?;
    Ok(())
}

pub(crate) async fn store_registry_cache_query(pool: &SqlitePool, json: &str) -> AppResult<()> {
    // Use a special plugin_id "__registry_cache" to store the JSON in the same table
    // This avoids needing a separate table or expanding the settings system.
    let now = chrono::Utc::now().to_rfc3339();
    let id = scryer_domain::Id::new().0;
    sqlx::query(
        "INSERT INTO plugin_installations
            (id, plugin_id, name, description, version, plugin_type, provider_type,
             is_enabled, is_builtin, wasm_sha256, installed_at, updated_at)
         VALUES (?, '__registry_cache', '__registry_cache', ?, '', '__cache', '__cache', 0, 0, NULL, ?, ?)
         ON CONFLICT(plugin_id) DO UPDATE SET description = excluded.description, updated_at = excluded.updated_at",
    )
    .bind(&id)
    .bind(json)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|e| scryer_application::AppError::Repository(e.to_string()))?;
    Ok(())
}

pub(crate) async fn get_registry_cache_query(pool: &SqlitePool) -> AppResult<Option<String>> {
    use sqlx::Row;
    let row = sqlx::query(
        "SELECT description FROM plugin_installations WHERE plugin_id = '__registry_cache'",
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| scryer_application::AppError::Repository(e.to_string()))?;

    Ok(row.map(|r| r.get::<String, _>("description")))
}

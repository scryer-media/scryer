use scryer_application::{AppError, AppResult};
use scryer_domain::title_catalog_sort_key;
use sqlx::Row;

#[derive(Clone, Debug)]
struct TitleSortKeyRow {
    id: String,
    name: String,
    metadata_language: Option<String>,
}

pub async fn migrate_title_catalog_sort_keys_sqlite(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> AppResult<()> {
    let titles = sqlite_titles(tx).await?;
    for title in titles {
        let sort_key = title_catalog_sort_key(&title.name, title.metadata_language.as_deref());
        sqlx::query("UPDATE titles SET catalog_sort_key = ?1 WHERE id = ?2")
            .bind(sort_key)
            .bind(&title.id)
            .execute(&mut **tx)
            .await
            .map_err(repo_err)?;
    }
    Ok(())
}

pub async fn migrate_title_catalog_sort_keys_postgres(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> AppResult<()> {
    let titles = postgres_titles(tx).await?;
    for title in titles {
        let sort_key = title_catalog_sort_key(&title.name, title.metadata_language.as_deref());
        sqlx::query("UPDATE titles SET catalog_sort_key = $1 WHERE id = $2")
            .bind(sort_key)
            .bind(&title.id)
            .execute(&mut **tx)
            .await
            .map_err(repo_err)?;
    }
    Ok(())
}

pub async fn migrate_title_catalog_order_keys_sqlite(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> AppResult<()> {
    // A nullable cursor includes even an empty legacy ID on the first page.
    let mut after: Option<String> = None;
    loop {
        let query = match after.as_deref() {
            Some(after) => sqlx::query(
                "SELECT id, name, metadata_language FROM titles WHERE id > ?1 ORDER BY id LIMIT 1000",
            ).bind(after),
            None => sqlx::query("SELECT id, name, metadata_language FROM titles ORDER BY id LIMIT 1000"),
        };
        let rows = query.fetch_all(&mut **tx).await.map_err(repo_err)?;
        if rows.is_empty() {
            break;
        }
        for row in rows {
            let title = title_row_from_sqlite(row)?;
            let primary = title_catalog_sort_key(&title.name, title.metadata_language.as_deref());
            let secondary = scryer_domain::title_catalog_name_tie_key(&title.name);
            sqlx::query(
                "UPDATE titles SET catalog_sort_key = ?1, catalog_name_tie_key = ?2 WHERE id = ?3",
            )
            .bind(primary)
            .bind(secondary)
            .bind(&title.id)
            .execute(&mut **tx)
            .await
            .map_err(repo_err)?;
            after = Some(title.id);
        }
    }
    Ok(())
}

pub async fn migrate_title_catalog_order_keys_postgres(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> AppResult<()> {
    // A nullable cursor includes even an empty legacy ID on the first page.
    let mut after: Option<String> = None;
    loop {
        let query = match after.as_deref() {
            Some(after) => sqlx::query(
                "SELECT id, name, metadata_language FROM titles WHERE id > $1 ORDER BY id LIMIT 1000",
            ).bind(after),
            None => sqlx::query("SELECT id, name, metadata_language FROM titles ORDER BY id LIMIT 1000"),
        };
        let rows = query.fetch_all(&mut **tx).await.map_err(repo_err)?;
        if rows.is_empty() {
            break;
        }
        for row in rows {
            let title = title_row_from_postgres(row)?;
            let primary = title_catalog_sort_key(&title.name, title.metadata_language.as_deref());
            let secondary = scryer_domain::title_catalog_name_tie_key(&title.name);
            sqlx::query(
                "UPDATE titles SET catalog_sort_key = $1, catalog_name_tie_key = $2 WHERE id = $3",
            )
            .bind(primary)
            .bind(secondary)
            .bind(&title.id)
            .execute(&mut **tx)
            .await
            .map_err(repo_err)?;
            after = Some(title.id);
        }
    }
    Ok(())
}

async fn sqlite_titles(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> AppResult<Vec<TitleSortKeyRow>> {
    let rows = sqlx::query("SELECT id, name, metadata_language FROM titles ORDER BY id")
        .fetch_all(&mut **tx)
        .await
        .map_err(repo_err)?;
    rows.into_iter().map(title_row_from_sqlite).collect()
}

async fn postgres_titles(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> AppResult<Vec<TitleSortKeyRow>> {
    let rows = sqlx::query("SELECT id, name, metadata_language FROM titles ORDER BY id")
        .fetch_all(&mut **tx)
        .await
        .map_err(repo_err)?;
    rows.into_iter().map(title_row_from_postgres).collect()
}

fn title_row_from_sqlite(row: sqlx::sqlite::SqliteRow) -> AppResult<TitleSortKeyRow> {
    Ok(TitleSortKeyRow {
        id: row.try_get("id").map_err(repo_err)?,
        name: row.try_get("name").map_err(repo_err)?,
        metadata_language: row.try_get("metadata_language").map_err(repo_err)?,
    })
}

fn title_row_from_postgres(row: sqlx::postgres::PgRow) -> AppResult<TitleSortKeyRow> {
    Ok(TitleSortKeyRow {
        id: row.try_get("id").map_err(repo_err)?,
        name: row.try_get("name").map_err(repo_err)?,
        metadata_language: row.try_get("metadata_language").map_err(repo_err)?,
    })
}

fn repo_err(error: impl std::fmt::Display) -> AppError {
    AppError::Repository(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn sqlite_catalog_order_migration_backfills_multiple_batches() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        let mut tx = pool.begin().await.unwrap();

        sqlx::raw_sql("CREATE TABLE titles (id TEXT PRIMARY KEY NOT NULL, name TEXT NOT NULL, metadata_language TEXT, year INTEGER, catalog_sort_key TEXT NOT NULL DEFAULT '')")
            .execute(&mut *tx).await.unwrap();
        for i in 0..1005 {
            let id = if i == 0 {
                String::new()
            } else {
                format!("title-{i:04}")
            };
            let name = [
                "The Meridian",
                "Ｍｅｒｉｄｉａｎ",
                "ÉCLAIR",
                "éclair",
                "鋼の錬金術師",
            ][i % 5];
            sqlx::query("INSERT INTO titles (id, name, metadata_language) VALUES (?1, ?2, 'eng')")
                .bind(id)
                .bind(name)
                .execute(&mut *tx)
                .await
                .unwrap();
        }
        sqlx::raw_sql(include_str!(
            "../../../scryer/src/db/migrations/0327_catalog_order_keys.sql"
        ))
        .execute(&mut *tx)
        .await
        .unwrap();
        migrate_title_catalog_order_keys_sqlite(&mut tx)
            .await
            .unwrap();
        sqlx::raw_sql(include_str!(
            "../../../scryer/src/db/migrations/0327_catalog_order_indexes.sql"
        ))
        .execute(&mut *tx)
        .await
        .unwrap();
        let rows = sqlx::query("SELECT name, catalog_sort_key, catalog_name_tie_key FROM titles")
            .fetch_all(&mut *tx)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1005);
        for row in rows {
            let name: String = row.try_get("name").unwrap();
            assert_eq!(
                row.get::<String, _>("catalog_sort_key"),
                title_catalog_sort_key(&name, Some("eng"))
            );
            assert_eq!(
                row.get::<String, _>("catalog_name_tie_key"),
                scryer_domain::title_catalog_name_tie_key(&name)
            );
        }
        tx.rollback().await.unwrap();
    }

    #[tokio::test]
    async fn postgres_catalog_order_migration_backfills_multiple_batches() {
        let Ok(url) = std::env::var("SCRYER_TEST_POSTGRES_URL") else {
            eprintln!("skipped: SCRYER_TEST_POSTGRES_URL must name an isolated test database");
            return;
        };
        let pool = sqlx::PgPool::connect(&url).await.unwrap();
        let mut tx = pool.begin().await.unwrap();
        let schema = format!("catalog_order_{}", uuid::Uuid::new_v4().simple());
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "CREATE SCHEMA {schema}; SET LOCAL search_path = {schema}"
        )))
        .execute(&mut *tx)
        .await
        .unwrap();
        sqlx::raw_sql("CREATE TABLE titles (id TEXT PRIMARY KEY NOT NULL, name TEXT NOT NULL, metadata_language TEXT, year INTEGER, catalog_sort_key TEXT NOT NULL DEFAULT '')")
            .execute(&mut *tx).await.unwrap();
        for i in 0..1005 {
            let id = if i == 0 {
                String::new()
            } else {
                format!("title-{i:04}")
            };
            let name = [
                "The Meridian",
                "Ｍｅｒｉｄｉａｎ",
                "ÉCLAIR",
                "éclair",
                "鋼の錬金術師",
            ][i % 5];
            sqlx::query("INSERT INTO titles (id, name, metadata_language) VALUES ($1, $2, 'eng')")
                .bind(id)
                .bind(name)
                .execute(&mut *tx)
                .await
                .unwrap();
        }
        sqlx::raw_sql(include_str!(
            "../../../scryer/src/db/postgres/migrations/0327_catalog_order_keys.sql"
        ))
        .execute(&mut *tx)
        .await
        .unwrap();
        migrate_title_catalog_order_keys_postgres(&mut tx)
            .await
            .unwrap();
        sqlx::raw_sql(include_str!(
            "../../../scryer/src/db/postgres/migrations/0327_catalog_order_indexes.sql"
        ))
        .execute(&mut *tx)
        .await
        .unwrap();
        let rows = sqlx::query("SELECT name, catalog_sort_key, catalog_name_tie_key FROM titles")
            .fetch_all(&mut *tx)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1005);
        for row in rows {
            let name: String = row.try_get("name").unwrap();
            assert_eq!(
                row.get::<String, _>("catalog_sort_key"),
                title_catalog_sort_key(&name, Some("eng"))
            );
            assert_eq!(
                row.get::<String, _>("catalog_name_tie_key"),
                scryer_domain::title_catalog_name_tie_key(&name)
            );
        }
        tx.rollback().await.unwrap();
    }

    #[tokio::test]
    async fn sqlite_hook_backfills_catalog_sort_keys() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite pool should open");
        sqlx::raw_sql(
            "
            CREATE TABLE titles (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                metadata_language TEXT,
                catalog_sort_key TEXT NOT NULL DEFAULT ''
            );
            INSERT INTO titles (id, name, metadata_language)
            VALUES
                ('title-a', 'The Meridian', 'eng'),
                ('title-b', '鋼の錬金術師', 'jpn');
            ",
        )
        .execute(&pool)
        .await
        .expect("fixture schema should load");

        let mut tx = pool.begin().await.expect("transaction should start");
        migrate_title_catalog_sort_keys_sqlite(&mut tx)
            .await
            .expect("hook should backfill sort keys");
        tx.commit().await.expect("transaction should commit");

        let rows = sqlx::query("SELECT id, catalog_sort_key FROM titles ORDER BY id")
            .fetch_all(&pool)
            .await
            .expect("titles should load");
        let key_a: String = rows[0].try_get("catalog_sort_key").expect("key a");
        let key_b: String = rows[1].try_get("catalog_sort_key").expect("key b");

        assert_eq!(key_a, title_catalog_sort_key("The Meridian", Some("eng")));
        assert_eq!(key_b, title_catalog_sort_key("鋼の錬金術師", Some("jpn")));
        assert!(!key_a.is_empty());
        assert!(!key_b.is_empty());
    }
}

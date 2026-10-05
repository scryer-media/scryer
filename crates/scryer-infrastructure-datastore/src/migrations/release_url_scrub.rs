//! Drop indexer and tracker credentials from release URLs that were persisted
//! before those copies were redacted on write.
//!
//! Only history copies are scrubbed: the release-decision ledger, the
//! download-attempt log and the download-submission record. None of them is
//! ever fetched from, and the write paths that fill the first two already
//! store the redacted form. `pending_releases.release_url` is deliberately not
//! touched: a held release is grabbed from that stored URL, so it must keep
//! its live credential.

use scryer_application::url_redaction::redact_url_credentials;
use scryer_application::{AppError, AppResult};
use sqlx::Row;

const BATCH_SIZE: i64 = 500;

/// `(table, column)` pairs holding display-only release URLs, each keyed by a
/// text `id` primary key. These constants are the only identifiers interpolated
/// into the scrub's SQL; every value is bound.
const SCRUBBED_COLUMNS: &[(&str, &str)] = &[
    ("release_decisions", "release_url"),
    ("release_download_attempts", "source_hint"),
    ("download_submissions", "source_hint"),
];

/// A credential is either a `name=value` query parameter or URL userinfo, so a
/// value with neither character cannot need a change. The coarse filter keeps
/// clean rows out of the scan; the redaction itself decides what is written.
fn candidate_filter(column: &str) -> String {
    format!("({column} LIKE '%=%' OR {column} LIKE '%@%')")
}

/// The redacted value, or `None` when the stored value is already clean or
/// already redacted, so an unchanged row is never rewritten.
fn scrubbed(value: &str) -> Option<String> {
    let redacted = redact_url_credentials(value);
    (redacted != value).then_some(redacted)
}

pub async fn scrub_stored_release_url_credentials_sqlite(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> AppResult<()> {
    for (table, column) in SCRUBBED_COLUMNS {
        let select = format!(
            "SELECT id, {column} AS value FROM {table}
              WHERE id > ?1 AND {filter}
              ORDER BY id
              LIMIT ?2",
            filter = candidate_filter(column)
        );
        let update = format!("UPDATE {table} SET {column} = ?1 WHERE id = ?2");
        let mut last_id = String::new();
        loop {
            let rows = sqlx::query(sqlx::AssertSqlSafe(select.as_str()))
                .bind(&last_id)
                .bind(BATCH_SIZE)
                .fetch_all(&mut **tx)
                .await
                .map_err(repo_error)?;
            if rows.is_empty() {
                break;
            }
            for row in rows {
                let id: String = row.try_get("id").map_err(repo_error)?;
                let value: String = row.try_get("value").map_err(repo_error)?;
                if let Some(redacted) = scrubbed(&value) {
                    sqlx::query(sqlx::AssertSqlSafe(update.as_str()))
                        .bind(redacted)
                        .bind(&id)
                        .execute(&mut **tx)
                        .await
                        .map_err(repo_error)?;
                }
                last_id = id;
            }
        }
    }
    Ok(())
}

/// [`scrub_stored_release_url_credentials_sqlite`] for Postgres.
pub async fn scrub_stored_release_url_credentials_postgres(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> AppResult<()> {
    for (table, column) in SCRUBBED_COLUMNS {
        let select = format!(
            "SELECT id, {column} AS value FROM {table}
              WHERE id > $1 AND {filter}
              ORDER BY id
              LIMIT $2",
            filter = candidate_filter(column)
        );
        let update = format!("UPDATE {table} SET {column} = $1 WHERE id = $2");
        let mut last_id = String::new();
        loop {
            let rows = sqlx::query(sqlx::AssertSqlSafe(select.as_str()))
                .bind(&last_id)
                .bind(BATCH_SIZE)
                .fetch_all(&mut **tx)
                .await
                .map_err(repo_error)?;
            if rows.is_empty() {
                break;
            }
            for row in rows {
                let id: String = row.try_get("id").map_err(repo_error)?;
                let value: String = row.try_get("value").map_err(repo_error)?;
                if let Some(redacted) = scrubbed(&value) {
                    sqlx::query(sqlx::AssertSqlSafe(update.as_str()))
                        .bind(redacted)
                        .bind(&id)
                        .execute(&mut **tx)
                        .await
                        .map_err(repo_error)?;
                }
                last_id = id;
            }
        }
    }
    Ok(())
}

fn repo_error(error: impl std::fmt::Display) -> AppError {
    AppError::Repository(error.to_string())
}

#[cfg(test)]
mod tests {
    use sqlx::sqlite::SqlitePoolOptions;

    use super::*;

    async fn pool_with_tables() -> sqlx::SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::raw_sql(
            "CREATE TABLE release_decisions (id TEXT PRIMARY KEY, release_url TEXT);
             CREATE TABLE release_download_attempts (id TEXT PRIMARY KEY, source_hint TEXT);
             CREATE TABLE download_submissions (id TEXT PRIMARY KEY, source_hint TEXT);
             CREATE TABLE pending_releases (id TEXT PRIMARY KEY, release_url TEXT);",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    async fn insert(pool: &sqlx::SqlitePool, table: &str, column: &str, id: &str, value: &str) {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "INSERT INTO {table} (id, {column}) VALUES (?1, ?2)"
        )))
        .bind(id)
        .bind(value)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn values(pool: &sqlx::SqlitePool, table: &str, column: &str) -> Vec<(String, String)> {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT id, {column} AS value FROM {table} ORDER BY id"
        )))
        .fetch_all(pool)
        .await
        .unwrap()
        .into_iter()
        .map(|row| (row.get("id"), row.get("value")))
        .collect()
    }

    async fn run_scrub(pool: &sqlx::SqlitePool) {
        let mut tx = pool.begin().await.unwrap();
        scrub_stored_release_url_credentials_sqlite(&mut tx)
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }

    #[tokio::test]
    async fn scrubs_every_credential_spelling_from_history_copies_only() {
        let pool = pool_with_tables().await;
        let keyed = [
            (
                "a-apikey",
                "https://indexer.invalid/api?t=get&id=7&apikey=live-one",
                "https://indexer.invalid/api?t=get&id=7&apikey=[redacted]",
            ),
            (
                "b-api-key",
                "https://indexer.invalid/api?API_KEY=live-two&id=8",
                "https://indexer.invalid/api?API_KEY=[redacted]&id=8",
            ),
            (
                "c-jackett",
                "https://jackett.invalid/dl/fixture?jackett_apikey=live-three&path=x",
                "https://jackett.invalid/dl/fixture?jackett_apikey=[redacted]&path=x",
            ),
            (
                "d-tracker",
                "https://tracker.invalid/dl/9.torrent?passkey=a&torrent_pass=b&rss_key=c",
                "https://tracker.invalid/dl/9.torrent?passkey=[redacted]&torrent_pass=[redacted]&rss_key=[redacted]",
            ),
            (
                "e-userinfo",
                "https://feeduser:s3cret@indexer.invalid/get/fixture.nzb",
                "https://[redacted]@indexer.invalid/get/fixture.nzb",
            ),
        ];
        let untouched = [
            (
                "f-redacted",
                "https://indexer.invalid/api?t=get&apikey=[redacted]",
            ),
            ("g-clean", "https://indexer.invalid/get/fixture.nzb"),
            ("h-magnet", "magnet:?xt=urn:btih:abcdef&dn=Fixture+Release"),
            ("i-name", "Fixture Indexer"),
        ];
        let scrubbed_columns = [
            ("release_decisions", "release_url"),
            ("release_download_attempts", "source_hint"),
            ("download_submissions", "source_hint"),
        ];
        for (table, column) in scrubbed_columns {
            for (id, raw, _) in keyed {
                insert(&pool, table, column, id, raw).await;
            }
            for (id, raw) in untouched {
                insert(&pool, table, column, id, raw).await;
            }
        }
        // The live grab URL of a held release must survive the scrub.
        let live = "https://indexer.invalid/api?t=get&id=7&apikey=live-one";
        insert(&pool, "pending_releases", "release_url", "held", live).await;

        let mut expected: Vec<(String, String)> = keyed
            .iter()
            .map(|(id, _, redacted)| (id.to_string(), redacted.to_string()))
            .chain(
                untouched
                    .iter()
                    .map(|(id, raw)| (id.to_string(), raw.to_string())),
            )
            .collect();
        expected.sort();

        for _ in 0..2 {
            run_scrub(&pool).await;
            for (table, column) in scrubbed_columns {
                assert_eq!(values(&pool, table, column).await, expected, "{table}");
            }
            assert_eq!(
                values(&pool, "pending_releases", "release_url").await,
                vec![("held".to_string(), live.to_string())]
            );
        }
    }

    #[tokio::test]
    async fn rows_that_need_no_change_are_never_rewritten() {
        let pool = pool_with_tables().await;
        for (id, value) in [
            (
                "a-redacted",
                "https://indexer.invalid/api?apikey=[redacted]",
            ),
            ("b-clean", "https://indexer.invalid/get/fixture.nzb?id=4"),
            ("c-keyed", "https://indexer.invalid/api?apikey=live"),
        ] {
            insert(&pool, "download_submissions", "source_hint", id, value).await;
        }
        sqlx::raw_sql(
            "CREATE TABLE writes (id TEXT);
             CREATE TRIGGER count_writes AFTER UPDATE ON download_submissions
             BEGIN INSERT INTO writes (id) VALUES (NEW.id); END;",
        )
        .execute(&pool)
        .await
        .unwrap();

        run_scrub(&pool).await;
        run_scrub(&pool).await;

        let written: Vec<String> = sqlx::query_scalar("SELECT id FROM writes ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(written, vec!["c-keyed".to_string()]);
    }

    #[tokio::test]
    async fn scrub_walks_past_a_full_batch() {
        let pool = pool_with_tables().await;
        let rows = BATCH_SIZE as usize * 2 + 3;
        for index in 0..rows {
            insert(
                &pool,
                "release_decisions",
                "release_url",
                &format!("decision-{index:05}"),
                &format!("https://indexer.invalid/api?id={index}&apikey=live"),
            )
            .await;
        }

        run_scrub(&pool).await;

        let remaining: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM release_decisions WHERE release_url NOT LIKE '%apikey=[redacted]'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(remaining, 0);
    }
}

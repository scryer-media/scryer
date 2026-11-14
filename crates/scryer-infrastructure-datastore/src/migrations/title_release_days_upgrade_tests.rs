use sqlx::sqlite::SqlitePoolOptions;

async fn title(
    pool: &sqlx::SqlitePool,
    id: &str,
    first_aired: Option<&str>,
    digital: Option<&str>,
    listed: Option<&str>,
) {
    sqlx::query(
        "INSERT INTO titles (id, name, facet, created_at, root_folder_id, library_id,
                             catalog_sort_key, catalog_name_tie_key,
                             first_aired, digital_release_date, movie_release_dates_json)
         SELECT ?1, ?1, 'movie', '2026-01-01T00:00:00Z', library_roots.id,
                library_roots.library_id, ?1, ?1, ?2, ?3, ?4
           FROM library_roots
          WHERE library_roots.library_id = 'movie_default_library'
          LIMIT 1",
    )
    .bind(id)
    .bind(first_aired)
    .bind(digital)
    .bind(listed)
    .execute(pool)
    .await
    .unwrap();
}

async fn release_days(pool: &sqlx::SqlitePool) -> Vec<(String, String)> {
    sqlx::query_as(
        "SELECT title_id, release_day FROM title_release_days ORDER BY title_id, release_day",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

fn days(rows: &[(&str, &str)]) -> Vec<(String, String)> {
    rows.iter()
        .map(|(title, day)| (title.to_string(), day.to_string()))
        .collect()
}

#[tokio::test]
async fn title_release_days_index_titles_stored_before_the_upgrade() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    super::replay_source_catalog_for_fresh_install(&pool, Some(342), true)
        .await
        .expect("pre-upgrade catalog should apply");
    let present: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE name = 'title_release_days'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        present, 0,
        "title_release_days must be introduced after 342"
    );

    title(
        &pool,
        "stored-dated",
        Some("2031-03-01"),
        Some("2031-06-04T02:00:00Z"),
        Some(r#"{"theatrical_dates":["2031-03-01","2031-03-05"],"physical_dates":["2031-07-01"]}"#),
    )
    .await;
    title(&pool, "stored-undated", None, Some("  "), None).await;
    title(&pool, "stored-unreadable", Some("Spring 2031"), None, None).await;
    title(&pool, "stored-not-json", None, None, Some("{not json")).await;

    super::run_migrations(&pool, crate::MigrationMode::Apply)
        .await
        .expect("upgrade must apply over stored titles");
    let backfilled = days(&[
        ("stored-dated", "2031-03-01"),
        ("stored-dated", "2031-03-05"),
        ("stored-dated", "2031-06-04"),
        ("stored-dated", "2031-07-01"),
        ("stored-not-json", ""),
        ("stored-unreadable", ""),
    ]);
    assert_eq!(release_days(&pool).await, backfilled);

    super::run_migrations(&pool, crate::MigrationMode::Apply)
        .await
        .expect("later startup must be idempotent");
    assert_eq!(release_days(&pool).await, backfilled);

    // From here on the triggers the upgrade installed keep it.
    title(&pool, "written-after", Some("2031-09-09"), None, None).await;
    sqlx::query("UPDATE titles SET first_aired = NULL WHERE id = 'stored-unreadable'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM titles WHERE id = 'stored-not-json'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        release_days(&pool).await,
        days(&[
            ("stored-dated", "2031-03-01"),
            ("stored-dated", "2031-03-05"),
            ("stored-dated", "2031-06-04"),
            ("stored-dated", "2031-07-01"),
            ("written-after", "2031-09-09"),
        ])
    );

    let plan: Vec<(i64, i64, i64, String)> = sqlx::query_as(
        "EXPLAIN QUERY PLAN SELECT title_id FROM title_release_days
          WHERE release_day >= '2031-03-01' AND release_day <= '2031-03-31'",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(
        plan.iter()
            .any(|row| row.3.contains("idx_title_release_days_day")),
        "{plan:?}"
    );
}

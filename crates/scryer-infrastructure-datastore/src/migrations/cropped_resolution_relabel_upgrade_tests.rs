use sqlx::sqlite::SqlitePoolOptions;

const SQLITE: &str =
    include_str!("../../../scryer/src/db/migrations/0280_cropped_resolution_relabel.sql");
const POSTGRES: &str =
    include_str!("../../../scryer/src/db/postgres/migrations/0280_cropped_resolution_relabel.sql");

type Row = (String, Option<String>, Option<String>);

struct Case {
    id: &'static str,
    width: Option<i64>,
    height: Option<i64>,
    resolution: Option<&'static str>,
    quality_id: Option<&'static str>,
    expected_resolution: Option<&'static str>,
    expected_quality_id: Option<&'static str>,
}

const CASES: [Case; 10] = [
    Case {
        id: "cropped-1080-both-low",
        width: Some(1918),
        height: Some(802),
        resolution: Some("720p"),
        quality_id: Some("720p"),
        expected_resolution: Some("1080p"),
        expected_quality_id: Some("1080p"),
    },
    Case {
        id: "cropped-1080-resolution-low",
        width: Some(1918),
        height: Some(802),
        resolution: Some("720p"),
        quality_id: Some("1080p"),
        expected_resolution: Some("1080p"),
        expected_quality_id: Some("1080p"),
    },
    Case {
        id: "cropped-2160-no-quality",
        width: Some(3832),
        height: Some(1600),
        resolution: Some("1440p"),
        quality_id: None,
        expected_resolution: Some("2160p"),
        expected_quality_id: None,
    },
    Case {
        id: "full-1080-empty",
        width: Some(1920),
        height: Some(1080),
        resolution: None,
        quality_id: None,
        expected_resolution: Some("1080p"),
        expected_quality_id: None,
    },
    Case {
        id: "true-720-labelled-high",
        width: Some(1280),
        height: Some(720),
        resolution: Some("1080p"),
        quality_id: Some("1080p"),
        expected_resolution: Some("720p"),
        expected_quality_id: Some("720p"),
    },
    Case {
        id: "non-tier-labels",
        width: Some(1280),
        height: Some(720),
        resolution: Some("1080i"),
        quality_id: Some("bluray-1080p"),
        expected_resolution: Some("1080i"),
        expected_quality_id: Some("bluray-1080p"),
    },
    Case {
        id: "pal-576",
        width: Some(720),
        height: Some(576),
        resolution: Some("480p"),
        quality_id: Some("480p"),
        expected_resolution: Some("576p"),
        expected_quality_id: Some("576p"),
    },
    Case {
        id: "retired-360",
        width: Some(640),
        height: Some(360),
        resolution: Some("360p"),
        quality_id: Some("360p"),
        expected_resolution: Some("480p"),
        expected_quality_id: Some("480p"),
    },
    Case {
        id: "no-dimensions",
        width: None,
        height: None,
        resolution: Some("720p"),
        quality_id: Some("720p"),
        expected_resolution: Some("720p"),
        expected_quality_id: Some("720p"),
    },
    Case {
        id: "already-correct",
        width: Some(1920),
        height: Some(1080),
        resolution: Some("1080p"),
        quality_id: Some("1080p"),
        expected_resolution: Some("1080p"),
        expected_quality_id: Some("1080p"),
    },
];

fn expected_rows() -> Vec<Row> {
    let mut rows: Vec<Row> = CASES
        .iter()
        .map(|case| {
            (
                case.id.to_owned(),
                case.expected_resolution.map(str::to_owned),
                case.expected_quality_id.map(str::to_owned),
            )
        })
        .collect();
    rows.sort();
    rows
}

const SELECT_ROWS: &str = "SELECT id, resolution, quality_id FROM media_files ORDER BY id";

#[tokio::test]
async fn cropped_resolution_relabel_applies_on_fresh_install() {
    assert_eq!(SQLITE, POSTGRES, "both engines run the same relabel");
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    super::run_migrations(&pool, crate::MigrationMode::Apply)
        .await
        .expect("fresh migration catalog should apply");
}

#[tokio::test]
async fn cropped_resolution_relabel_upgrades_stored_tiers_idempotently() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    super::replay_source_catalog_for_fresh_install(&pool, Some(279), true)
        .await
        .expect("pre-upgrade catalog should apply");
    sqlx::query(
        "INSERT INTO titles (id, name, facet, monitored, created_at, library_id, root_folder_id)
         SELECT 'fixture-title', 'Fixture Title', 'movie', 1, '2026-01-01T00:00:00Z',
                l.id, r.id
         FROM libraries l JOIN library_roots r ON r.library_id = l.id
         WHERE l.facet = 'movie' AND l.is_default = 1 AND r.is_default = 1",
    )
    .execute(&pool)
    .await
    .unwrap();
    for case in &CASES {
        sqlx::query(
            "INSERT INTO media_files (id, title_id, file_path, size_bytes, created_at,
                                      video_width, video_height, resolution, quality_id)
             VALUES (?, 'fixture-title', ?, 1, '2026-01-01T00:00:00Z', ?, ?, ?, ?)",
        )
        .bind(case.id)
        .bind(format!("/fixture/{}.mkv", case.id))
        .bind(case.width)
        .bind(case.height)
        .bind(case.resolution)
        .bind(case.quality_id)
        .execute(&pool)
        .await
        .unwrap();
    }

    super::run_migrations(&pool, crate::MigrationMode::Apply)
        .await
        .expect("upgrade must retain the published migration checksums");
    let rows: Vec<Row> = sqlx::query_as(SELECT_ROWS).fetch_all(&pool).await.unwrap();
    assert_eq!(rows, expected_rows());

    super::run_migrations(&pool, crate::MigrationMode::Apply)
        .await
        .expect("later startup must be idempotent");
    // Replaying the statements directly proves they are fixed points too.
    sqlx::raw_sql(SQLITE).execute(&pool).await.unwrap();
    let rerun: Vec<Row> = sqlx::query_as(SELECT_ROWS).fetch_all(&pool).await.unwrap();
    assert_eq!(rerun, rows);
}

#[tokio::test]
async fn cropped_resolution_relabel_postgres_applies_idempotently() {
    let Ok(url) = std::env::var("SCRYER_TEST_POSTGRES_URL") else {
        eprintln!("skipped: SCRYER_TEST_POSTGRES_URL must name an isolated test database");
        return;
    };
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .unwrap();
    let mut tx = pool.begin().await.unwrap();
    sqlx::raw_sql(
        "CREATE TEMP TABLE media_files (
            id TEXT PRIMARY KEY, video_width INTEGER, video_height INTEGER,
            resolution TEXT, quality_id TEXT)",
    )
    .execute(&mut *tx)
    .await
    .unwrap();
    for case in &CASES {
        sqlx::query(
            "INSERT INTO media_files (id, video_width, video_height, resolution, quality_id)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(case.id)
        .bind(case.width.map(|value| value as i32))
        .bind(case.height.map(|value| value as i32))
        .bind(case.resolution)
        .bind(case.quality_id)
        .execute(&mut *tx)
        .await
        .unwrap();
    }
    sqlx::raw_sql(POSTGRES).execute(&mut *tx).await.unwrap();
    let rows: Vec<Row> = sqlx::query_as(SELECT_ROWS)
        .fetch_all(&mut *tx)
        .await
        .unwrap();
    assert_eq!(rows, expected_rows());
    sqlx::raw_sql(POSTGRES).execute(&mut *tx).await.unwrap();
    let rerun: Vec<Row> = sqlx::query_as(SELECT_ROWS)
        .fetch_all(&mut *tx)
        .await
        .unwrap();
    assert_eq!(rerun, rows);
    tx.rollback().await.unwrap();
}

use sqlx::SqlitePool;
use sqlx::sqlite::SqlitePoolOptions;

use scryer_application::title_folder_repair::reasons;

const BEFORE: i64 = 332;
const INDEX_NAME: &str = "titles_folder_path_key_unique";

async fn catalog_before_repair() -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    super::replay_source_catalog_for_fresh_install(&pool, Some(BEFORE), true)
        .await
        .expect("pre-upgrade catalog should apply");
    sqlx::query(
        "INSERT INTO library_roots
             (id, library_id, path, normalized_path, is_default, created_at, updated_at)
         VALUES ('root-fixture', 'movie_default_library', '/srv/fixture-root',
                 '/srv/fixture-root', 0, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    )
    .execute(&pool)
    .await
    .unwrap();
    pool
}

async fn title(pool: &SqlitePool, id: &str, folder: Option<&str>, added: &str) {
    sqlx::query(
        "INSERT INTO titles
             (id, library_id, name, facet, monitored, root_folder_id, folder_path,
              created_at, updated_at)
         VALUES (?1, 'movie_default_library', ?1, 'movie', 1, 'root-fixture', ?2, ?3, ?3)",
    )
    .bind(id)
    .bind(folder)
    .bind(added)
    .execute(pool)
    .await
    .unwrap();
}

async fn file(pool: &SqlitePool, id: &str, title_id: &str, path: &str, role: &str) {
    sqlx::query(
        "INSERT INTO media_files (id, title_id, file_path, size_bytes, created_at, role)
         VALUES (?1, ?2, ?3, 10, '2026-01-01T00:00:00Z', ?4)",
    )
    .bind(id)
    .bind(title_id)
    .bind(path)
    .bind(role)
    .execute(pool)
    .await
    .unwrap();
}

async fn migrate(pool: &SqlitePool) {
    super::run_migrations(pool, crate::MigrationMode::Apply)
        .await
        .expect("migrations should apply");
}

type TitleState = (
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

async fn titles(pool: &SqlitePool) -> Vec<TitleState> {
    sqlx::query_as(
        "SELECT id, folder_path, folder_path_key, folder_conflict_reason, folder_conflict_path
         FROM titles ORDER BY id",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

fn state(
    id: &str,
    folder: Option<&str>,
    key: Option<&str>,
    reason: Option<&str>,
    lost: Option<&str>,
) -> TitleState {
    (
        id.to_string(),
        folder.map(str::to_string),
        key.map(str::to_string),
        reason.map(str::to_string),
        lost.map(str::to_string),
    )
}

fn kept(id: &str, folder: &str) -> TitleState {
    state(id, Some(folder), Some(folder), None, None)
}

fn cleared(id: &str, reason: &str, lost: &str) -> TitleState {
    state(id, None, None, Some(reason), Some(lost))
}

async fn media_files_snapshot(pool: &SqlitePool) -> Vec<(String, String, String, String)> {
    sqlx::query_as("SELECT id, title_id, file_path, role FROM media_files ORDER BY id")
        .fetch_all(pool)
        .await
        .unwrap()
}

async fn index_exists(pool: &SqlitePool) -> bool {
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = ?")
            .bind(INDEX_NAME)
            .fetch_one(pool)
            .await
            .unwrap();
    count == 1
}

#[tokio::test]
async fn duplicate_folder_goes_to_the_title_with_more_files() {
    let pool = catalog_before_repair().await;
    title(
        &pool,
        "emberfall-early",
        Some("/media/Emberfall"),
        "2026-01-01T00:00:00Z",
    )
    .await;
    title(
        &pool,
        "emberfall-late",
        Some("/media/Emberfall/"),
        "2026-02-01T00:00:00Z",
    )
    .await;
    file(
        &pool,
        "f1",
        "emberfall-late",
        "/media/Emberfall/a.mkv",
        "primary",
    )
    .await;
    file(
        &pool,
        "f2",
        "emberfall-late",
        "/media/Emberfall/b.mkv",
        "primary",
    )
    .await;
    file(
        &pool,
        "f3",
        "emberfall-early",
        "/media/Emberfall/c.mkv",
        "primary",
    )
    .await;
    // Extras do not count towards ownership.
    file(
        &pool,
        "f4",
        "emberfall-early",
        "/media/Emberfall/d.mkv",
        "additional",
    )
    .await;
    file(
        &pool,
        "f5",
        "emberfall-early",
        "/media/Emberfall/e.mkv",
        "additional",
    )
    .await;
    migrate(&pool).await;
    assert_eq!(
        titles(&pool).await,
        [
            cleared(
                "emberfall-early",
                reasons::SHARED_FOLDER,
                "/media/Emberfall"
            ),
            state(
                "emberfall-late",
                Some("/media/Emberfall/"),
                Some("/media/Emberfall"),
                None,
                None
            ),
        ]
    );
    assert!(index_exists(&pool).await);
}

#[tokio::test]
async fn duplicate_folder_tie_goes_to_the_earliest_added_title() {
    let pool = catalog_before_repair().await;
    title(
        &pool,
        "cinderwake-a",
        Some("/media/Cinderwake"),
        "2026-03-01T00:00:00Z",
    )
    .await;
    title(
        &pool,
        "cinderwake-b",
        Some("/media/Cinderwake"),
        "2026-01-01T00:00:00Z",
    )
    .await;
    migrate(&pool).await;
    assert_eq!(
        titles(&pool).await,
        [
            cleared("cinderwake-a", reasons::SHARED_FOLDER, "/media/Cinderwake"),
            kept("cinderwake-b", "/media/Cinderwake"),
        ]
    );
}

#[tokio::test]
async fn nested_pair_goes_to_the_title_with_more_files() {
    let pool = catalog_before_repair().await;
    title(
        &pool,
        "emberfall-outer",
        Some("/media/Emberfall"),
        "2026-01-01T00:00:00Z",
    )
    .await;
    title(
        &pool,
        "emberfall-inner",
        Some("/media/Emberfall/Season 1"),
        "2026-01-01T00:00:00Z",
    )
    .await;
    file(
        &pool,
        "f1",
        "emberfall-outer",
        "/media/Emberfall/Season 2/a.mkv",
        "primary",
    )
    .await;
    file(
        &pool,
        "f2",
        "emberfall-outer",
        "/media/Emberfall/Season 2/b.mkv",
        "primary",
    )
    .await;
    file(
        &pool,
        "f3",
        "emberfall-inner",
        "/media/Emberfall/Season 1/c.mkv",
        "primary",
    )
    .await;
    migrate(&pool).await;
    assert_eq!(
        titles(&pool).await,
        [
            cleared(
                "emberfall-inner",
                reasons::INSIDE_KEPT_FOLDER,
                "/media/Emberfall/Season 1"
            ),
            kept("emberfall-outer", "/media/Emberfall"),
        ]
    );
}

#[tokio::test]
async fn nested_pair_tie_keeps_the_inner_folder() {
    let pool = catalog_before_repair().await;
    title(
        &pool,
        "cinderwake-outer",
        Some("/media/Cinderwake"),
        "2026-01-01T00:00:00Z",
    )
    .await;
    title(
        &pool,
        "cinderwake-inner",
        Some("/media/Cinderwake/Extras"),
        "2026-02-01T00:00:00Z",
    )
    .await;
    migrate(&pool).await;
    assert_eq!(
        titles(&pool).await,
        [
            kept("cinderwake-inner", "/media/Cinderwake/Extras"),
            cleared(
                "cinderwake-outer",
                reasons::HOLDS_KEPT_FOLDER,
                "/media/Cinderwake"
            ),
        ]
    );
}

#[tokio::test]
async fn three_title_chain_is_resolved_until_nothing_overlaps() {
    let pool = catalog_before_repair().await;
    title(
        &pool,
        "chain-top",
        Some("/media/Emberfall"),
        "2026-01-01T00:00:00Z",
    )
    .await;
    title(
        &pool,
        "chain-middle",
        Some("/media/Emberfall/Part"),
        "2026-01-01T00:00:00Z",
    )
    .await;
    title(
        &pool,
        "chain-bottom",
        Some("/media/Emberfall/Part/Two"),
        "2026-01-01T00:00:00Z",
    )
    .await;
    for (id, path) in [
        ("f1", "/media/Emberfall/a.mkv"),
        ("f2", "/media/Emberfall/b.mkv"),
        ("f3", "/media/Emberfall/c.mkv"),
    ] {
        file(&pool, id, "chain-top", path, "primary").await;
    }
    file(
        &pool,
        "f4",
        "chain-middle",
        "/media/Emberfall/Part/d.mkv",
        "primary",
    )
    .await;
    migrate(&pool).await;
    assert_eq!(
        titles(&pool).await,
        [
            cleared(
                "chain-bottom",
                reasons::INSIDE_KEPT_FOLDER,
                "/media/Emberfall/Part/Two"
            ),
            cleared(
                "chain-middle",
                reasons::INSIDE_KEPT_FOLDER,
                "/media/Emberfall/Part"
            ),
            kept("chain-top", "/media/Emberfall"),
        ]
    );
    assert!(index_exists(&pool).await);
}

#[tokio::test]
async fn whitespace_only_folder_becomes_no_folder() {
    let pool = catalog_before_repair().await;
    title(
        &pool,
        "emberfall-blank",
        Some("   "),
        "2026-01-01T00:00:00Z",
    )
    .await;
    title(&pool, "cinderwake-blank", Some(""), "2026-01-01T00:00:00Z").await;
    migrate(&pool).await;
    assert_eq!(
        titles(&pool).await,
        [
            state("cinderwake-blank", None, None, None, None),
            state("emberfall-blank", None, None, None, None),
        ]
    );
}

#[tokio::test]
async fn clean_catalog_only_gains_keys_and_a_second_run_changes_nothing() {
    let pool = catalog_before_repair().await;
    title(
        &pool,
        "emberfall",
        Some("/media/Emberfall"),
        "2026-01-01T00:00:00Z",
    )
    .await;
    // POSIX folders that differ only in case are different folders.
    title(
        &pool,
        "emberfall-lower",
        Some("/media/emberfall"),
        "2026-01-01T00:00:00Z",
    )
    .await;
    title(
        &pool,
        "cinderwake",
        Some(r"D:\Media\Cinderwake\"),
        "2026-01-01T00:00:00Z",
    )
    .await;
    title(&pool, "no-folder", None, "2026-01-01T00:00:00Z").await;
    file(
        &pool,
        "f1",
        "emberfall",
        "/media/Emberfall/a.mkv",
        "primary",
    )
    .await;
    let files_before = media_files_snapshot(&pool).await;
    migrate(&pool).await;
    let after_first = titles(&pool).await;
    assert_eq!(
        after_first,
        [
            state(
                "cinderwake",
                Some(r"D:\Media\Cinderwake\"),
                Some(r"D:\MEDIA\CINDERWAKE"),
                None,
                None
            ),
            kept("emberfall", "/media/Emberfall"),
            kept("emberfall-lower", "/media/emberfall"),
            state("no-folder", None, None, None, None),
        ]
    );

    // Running the repair again over the repaired catalog changes nothing.
    let mut tx = pool.begin().await.unwrap();
    super::title_folder_keys::repair_title_folder_keys_sqlite(&mut tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(titles(&pool).await, after_first);
    assert_eq!(media_files_snapshot(&pool).await, files_before);
}

#[tokio::test]
async fn repair_leaves_media_files_untouched_and_is_idempotent() {
    let pool = catalog_before_repair().await;
    title(
        &pool,
        "emberfall-a",
        Some("/media/Emberfall"),
        "2026-01-01T00:00:00Z",
    )
    .await;
    title(
        &pool,
        "emberfall-b",
        Some("/media/Emberfall"),
        "2026-01-02T00:00:00Z",
    )
    .await;
    title(
        &pool,
        "emberfall-c",
        Some("/media/Emberfall/Inner"),
        "2026-01-03T00:00:00Z",
    )
    .await;
    file(
        &pool,
        "f1",
        "emberfall-a",
        "/media/Emberfall/a.mkv",
        "primary",
    )
    .await;
    file(
        &pool,
        "f2",
        "emberfall-b",
        "/media/Emberfall/b.mkv",
        "primary",
    )
    .await;
    file(
        &pool,
        "f3",
        "emberfall-c",
        "/media/Emberfall/Inner/c.mkv",
        "additional",
    )
    .await;
    file(&pool, "f4", "emberfall-c", "/elsewhere/d.mkv", "primary").await;
    let files_before = media_files_snapshot(&pool).await;
    migrate(&pool).await;
    assert_eq!(media_files_snapshot(&pool).await, files_before);
    let after_first = titles(&pool).await;
    assert_eq!(
        after_first,
        [
            kept("emberfall-a", "/media/Emberfall"),
            cleared("emberfall-b", reasons::SHARED_FOLDER, "/media/Emberfall"),
            cleared(
                "emberfall-c",
                reasons::INSIDE_KEPT_FOLDER,
                "/media/Emberfall/Inner"
            ),
        ]
    );

    let mut tx = pool.begin().await.unwrap();
    super::title_folder_keys::repair_title_folder_keys_sqlite(&mut tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(titles(&pool).await, after_first);
    assert_eq!(media_files_snapshot(&pool).await, files_before);
    // A later startup applies nothing new.
    migrate(&pool).await;
    assert_eq!(titles(&pool).await, after_first);
}

#[tokio::test]
async fn unique_index_refuses_a_second_title_on_one_folder_after_the_repair() {
    let pool = catalog_before_repair().await;
    title(
        &pool,
        "emberfall-a",
        Some("/media/Emberfall"),
        "2026-01-01T00:00:00Z",
    )
    .await;
    title(
        &pool,
        "emberfall-b",
        Some("/media/Emberfall"),
        "2026-01-02T00:00:00Z",
    )
    .await;
    migrate(&pool).await;
    assert!(index_exists(&pool).await);
    let duplicate = sqlx::query(
        "UPDATE titles SET folder_path = '/media/Emberfall', folder_path_key = '/media/Emberfall'
         WHERE id = 'emberfall-b'",
    )
    .execute(&pool)
    .await;
    assert!(
        duplicate.is_err(),
        "the unique key index must refuse a shared folder"
    );
}

#[tokio::test]
async fn fresh_install_creates_the_folder_key_index() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    migrate(&pool).await;
    assert!(index_exists(&pool).await);
}

#[tokio::test]
async fn title_folders_equal_to_or_holding_a_library_root_are_cleared() {
    let pool = catalog_before_repair().await;
    title(
        &pool,
        "emberfall-root",
        Some("/srv/fixture-root"),
        "2026-01-01T00:00:00Z",
    )
    .await;
    title(
        &pool,
        "emberfall-root-slash",
        Some("/srv/fixture-root/"),
        "2026-01-02T00:00:00Z",
    )
    .await;
    title(
        &pool,
        "cinderwake-parent",
        Some("/srv"),
        "2026-01-01T00:00:00Z",
    )
    .await;
    title(
        &pool,
        "ashgrove-inside",
        Some("/srv/fixture-root/Ashgrove"),
        "2026-01-01T00:00:00Z",
    )
    .await;
    // Files under the root give the root-sized titles more files than the
    // real title folder; they still lose.
    for (id, title_id, path) in [
        ("f1", "emberfall-root", "/srv/fixture-root/Ashgrove/a.mkv"),
        ("f2", "emberfall-root", "/srv/fixture-root/Other/b.mkv"),
        ("f3", "cinderwake-parent", "/srv/fixture-root/Other/c.mkv"),
    ] {
        file(&pool, id, title_id, path, "primary").await;
    }
    let files_before = media_files_snapshot(&pool).await;
    migrate(&pool).await;
    let after_first = titles(&pool).await;
    assert_eq!(
        after_first,
        [
            kept("ashgrove-inside", "/srv/fixture-root/Ashgrove"),
            cleared("cinderwake-parent", reasons::LIBRARY_ROOT, "/srv"),
            cleared("emberfall-root", reasons::LIBRARY_ROOT, "/srv/fixture-root"),
            cleared(
                "emberfall-root-slash",
                reasons::LIBRARY_ROOT,
                "/srv/fixture-root/"
            ),
        ]
    );
    assert_eq!(media_files_snapshot(&pool).await, files_before);
    assert!(index_exists(&pool).await);

    let mut tx = pool.begin().await.unwrap();
    super::title_folder_keys::repair_title_folder_keys_sqlite(&mut tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(titles(&pool).await, after_first);
    assert_eq!(media_files_snapshot(&pool).await, files_before);
}

#[tokio::test]
async fn a_root_sized_title_does_not_disturb_an_unrelated_nested_pair() {
    let pool = catalog_before_repair().await;
    title(
        &pool,
        "emberfall-root",
        Some("/srv/fixture-root"),
        "2026-01-01T00:00:00Z",
    )
    .await;
    title(
        &pool,
        "cinderwake-outer",
        Some("/media/Cinderwake"),
        "2026-01-01T00:00:00Z",
    )
    .await;
    title(
        &pool,
        "cinderwake-inner",
        Some("/media/Cinderwake/Extras"),
        "2026-01-01T00:00:00Z",
    )
    .await;
    file(
        &pool,
        "f1",
        "cinderwake-outer",
        "/media/Cinderwake/a.mkv",
        "primary",
    )
    .await;
    file(
        &pool,
        "f2",
        "cinderwake-outer",
        "/media/Cinderwake/b.mkv",
        "primary",
    )
    .await;
    let files_before = media_files_snapshot(&pool).await;
    migrate(&pool).await;
    assert_eq!(
        titles(&pool).await,
        [
            cleared(
                "cinderwake-inner",
                reasons::INSIDE_KEPT_FOLDER,
                "/media/Cinderwake/Extras"
            ),
            kept("cinderwake-outer", "/media/Cinderwake"),
            cleared("emberfall-root", reasons::LIBRARY_ROOT, "/srv/fixture-root"),
        ]
    );
    assert_eq!(media_files_snapshot(&pool).await, files_before);
}

#[tokio::test]
async fn windows_folders_differing_only_in_case_are_one_shared_folder() {
    let pool = catalog_before_repair().await;
    title(
        &pool,
        "emberfall-upper",
        Some(r"D:\Media\Emberfall"),
        "2026-01-01T00:00:00Z",
    )
    .await;
    title(
        &pool,
        "emberfall-lower",
        Some(r"d:\media\EMBERFALL"),
        "2026-01-02T00:00:00Z",
    )
    .await;
    migrate(&pool).await;
    assert_eq!(
        titles(&pool).await,
        [
            cleared(
                "emberfall-lower",
                reasons::SHARED_FOLDER,
                r"d:\media\EMBERFALL"
            ),
            state(
                "emberfall-upper",
                Some(r"D:\Media\Emberfall"),
                Some(r"D:\MEDIA\EMBERFALL"),
                None,
                None
            ),
        ]
    );
    assert!(index_exists(&pool).await);
}

use std::collections::{BTreeMap, HashMap};
use std::path::Component;

use scryer_application::stored_paths::{
    folder_path_identity_key, folder_paths_match, path_to_stored_string, stored_path_to_path_buf,
};
use scryer_application::{AppError, AppResult};
use sqlx::Row;
use tracing::info;
use uuid::Uuid;

#[derive(Clone, Debug)]
struct TitleRow {
    id: String,
    library_id: String,
    folder_path: Option<String>,
    root_path: String,
}

#[derive(Clone, Debug)]
struct MediaFileRow {
    id: String,
    title_id: String,
    file_path: String,
}

#[derive(Clone, Debug)]
struct FolderGroup {
    path: String,
    media_file_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TitleFolderPlan {
    title_id: String,
    library_id: String,
    folder_path: Option<String>,
    selected_media_count: usize,
    folder_counts: Vec<(String, usize)>,
    tied_maximum: bool,
    unlink_media_file_ids: Vec<String>,
    all_media_file_ids: Vec<String>,
}

pub async fn migrate_title_folder_ownership_sqlite(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> AppResult<()> {
    let titles = sqlite_titles(tx).await?;
    let media_files = sqlite_media_files(tx).await?;
    let plans = build_plans(titles, media_files, random_tie_index);
    log_plans(&plans);
    apply_sqlite_plans(tx, &plans).await
}

pub async fn migrate_title_folder_ownership_postgres(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> AppResult<()> {
    let titles = postgres_titles(tx).await?;
    let media_files = postgres_media_files(tx).await?;
    let plans = build_plans(titles, media_files, random_tie_index);
    log_plans(&plans);
    apply_postgres_plans(tx, &plans).await
}

fn build_plans(
    titles: Vec<TitleRow>,
    media_files: Vec<MediaFileRow>,
    mut choose_tie: impl FnMut(usize) -> usize,
) -> Vec<TitleFolderPlan> {
    let mut media_by_title = HashMap::<String, Vec<MediaFileRow>>::new();
    for media_file in media_files {
        media_by_title
            .entry(media_file.title_id.clone())
            .or_default()
            .push(media_file);
    }

    let mut plans = titles
        .into_iter()
        .map(|title| {
            let media_files = media_by_title.remove(&title.id).unwrap_or_default();
            build_title_plan(title, media_files, &mut choose_tie)
        })
        .collect::<Vec<_>>();
    resolve_duplicate_folder_owners(&mut plans, &mut choose_tie);
    plans
}

fn build_title_plan(
    title: TitleRow,
    media_files: Vec<MediaFileRow>,
    choose_tie: &mut impl FnMut(usize) -> usize,
) -> TitleFolderPlan {
    let mut all_media_file_ids = media_files
        .iter()
        .map(|media_file| media_file.id.clone())
        .collect::<Vec<_>>();
    all_media_file_ids.sort();
    let mut groups = BTreeMap::<String, FolderGroup>::new();
    let mut invalid_media_file_ids = Vec::new();

    for media_file in media_files {
        let Some(folder_path) = top_level_title_folder(&title.root_path, &media_file.file_path)
        else {
            invalid_media_file_ids.push(media_file.id);
            continue;
        };
        let Some(key) = folder_path_identity_key(&folder_path) else {
            invalid_media_file_ids.push(media_file.id);
            continue;
        };
        let group = groups.entry(key).or_insert_with(|| FolderGroup {
            path: folder_path,
            media_file_ids: Vec::new(),
        });
        group.media_file_ids.push(media_file.id);
    }

    let folder_counts = groups
        .values()
        .map(|group| (group.path.clone(), group.media_file_ids.len()))
        .collect::<Vec<_>>();
    let maximum = groups
        .values()
        .map(|group| group.media_file_ids.len())
        .max();
    let tied = maximum
        .map(|maximum| {
            groups
                .values()
                .filter(|group| group.media_file_ids.len() == maximum)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let tied_maximum = tied.len() > 1;
    let winner = if tied.is_empty() {
        valid_existing_folder(&title.root_path, title.folder_path.as_deref())
    } else {
        let index = choose_tie(tied.len()) % tied.len();
        Some(tied[index].path.clone())
    };
    let winner_key = winner.as_deref().and_then(folder_path_identity_key);
    let selected_media_count = winner_key
        .as_deref()
        .and_then(|key| groups.get(key))
        .map_or(0, |group| group.media_file_ids.len());

    let mut unlink_media_file_ids = invalid_media_file_ids;
    for (key, group) in groups {
        if winner_key.as_deref() != Some(key.as_str()) {
            unlink_media_file_ids.extend(group.media_file_ids);
        }
    }
    unlink_media_file_ids.sort();

    TitleFolderPlan {
        title_id: title.id,
        library_id: title.library_id,
        folder_path: winner,
        selected_media_count,
        folder_counts,
        tied_maximum,
        unlink_media_file_ids,
        all_media_file_ids,
    }
}

fn resolve_duplicate_folder_owners(
    plans: &mut [TitleFolderPlan],
    choose_tie: &mut impl FnMut(usize) -> usize,
) {
    let mut owners = BTreeMap::<(String, String), Vec<usize>>::new();
    for (index, plan) in plans.iter().enumerate() {
        let Some(folder_key) = plan
            .folder_path
            .as_deref()
            .and_then(folder_path_identity_key)
        else {
            continue;
        };
        owners
            .entry((plan.library_id.clone(), folder_key))
            .or_default()
            .push(index);
    }

    for indexes in owners.into_values().filter(|indexes| indexes.len() > 1) {
        let maximum = indexes
            .iter()
            .map(|index| plans[*index].selected_media_count)
            .max()
            .unwrap_or_default();
        let tied = indexes
            .iter()
            .copied()
            .filter(|index| plans[*index].selected_media_count == maximum)
            .collect::<Vec<_>>();
        let winner = tied[choose_tie(tied.len()) % tied.len()];

        for index in indexes.into_iter().filter(|index| *index != winner) {
            let plan = &mut plans[index];
            plan.folder_path = None;
            plan.selected_media_count = 0;
            plan.unlink_media_file_ids
                .clone_from(&plan.all_media_file_ids);
        }
    }
}

fn top_level_title_folder(root_path: &str, file_path: &str) -> Option<String> {
    let root = stored_path_to_path_buf(root_path);
    let file = stored_path_to_path_buf(file_path);
    let parent = file.parent()?;
    let root_components = root.components().collect::<Vec<_>>();
    let parent_components = parent.components().collect::<Vec<_>>();
    if parent_components.len() <= root_components.len()
        || !root_components
            .iter()
            .zip(&parent_components)
            .all(|(root, parent)| path_components_match(*root, *parent))
    {
        return None;
    }
    let Component::Normal(first) = parent_components[root_components.len()] else {
        return None;
    };
    Some(path_to_stored_string(root.join(first)))
}

fn path_components_match(left: Component<'_>, right: Component<'_>) -> bool {
    if cfg!(windows) {
        left.as_os_str().to_string_lossy().to_lowercase()
            == right.as_os_str().to_string_lossy().to_lowercase()
    } else {
        left == right
    }
}

fn valid_existing_folder(root_path: &str, folder_path: Option<&str>) -> Option<String> {
    let folder_path = folder_path.filter(|path| !path.is_empty())?;
    let folder = stored_path_to_path_buf(folder_path);
    let probe = folder.join("__scryer_title_folder_probe__");
    let inferred = top_level_title_folder(root_path, &path_to_stored_string(probe))?;
    folder_paths_match(folder_path, &inferred).then(|| path_to_stored_string(folder))
}

fn random_tie_index(len: usize) -> usize {
    debug_assert!(len > 0);
    let bytes = Uuid::new_v4().into_bytes();
    let value = u64::from_le_bytes(bytes[..8].try_into().expect("UUID prefix is eight bytes"));
    value as usize % len
}

fn log_plans(plans: &[TitleFolderPlan]) {
    for plan in plans {
        info!(
            title_id = %plan.title_id,
            folder_path = ?plan.folder_path,
            folder_counts = ?plan.folder_counts,
            tied_maximum = plan.tied_maximum,
            unlinked_media_files = plan.unlink_media_file_ids.len(),
            "reconciled title folder ownership"
        );
    }
}

async fn sqlite_titles(tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>) -> AppResult<Vec<TitleRow>> {
    let rows = sqlx::query(
        "SELECT titles.id, titles.library_id, titles.folder_path, library_roots.path AS root_path
           FROM titles
           JOIN library_roots ON library_roots.id = titles.root_folder_id
          ORDER BY titles.id",
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(repo_err)?;
    rows.into_iter().map(sqlite_title_row).collect()
}

async fn postgres_titles(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> AppResult<Vec<TitleRow>> {
    let rows = sqlx::query(
        "SELECT titles.id, titles.library_id, titles.folder_path, library_roots.path AS root_path
           FROM titles
           JOIN library_roots ON library_roots.id = titles.root_folder_id
          ORDER BY titles.id",
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(repo_err)?;
    rows.into_iter().map(postgres_title_row).collect()
}

fn sqlite_title_row(row: sqlx::sqlite::SqliteRow) -> AppResult<TitleRow> {
    Ok(TitleRow {
        id: row.try_get("id").map_err(repo_err)?,
        library_id: row.try_get("library_id").map_err(repo_err)?,
        folder_path: row.try_get("folder_path").map_err(repo_err)?,
        root_path: row.try_get("root_path").map_err(repo_err)?,
    })
}

fn postgres_title_row(row: sqlx::postgres::PgRow) -> AppResult<TitleRow> {
    Ok(TitleRow {
        id: row.try_get("id").map_err(repo_err)?,
        library_id: row.try_get("library_id").map_err(repo_err)?,
        folder_path: row.try_get("folder_path").map_err(repo_err)?,
        root_path: row.try_get("root_path").map_err(repo_err)?,
    })
}

async fn sqlite_media_files(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> AppResult<Vec<MediaFileRow>> {
    let rows = sqlx::query("SELECT id, title_id, file_path FROM media_files ORDER BY title_id, id")
        .fetch_all(&mut **tx)
        .await
        .map_err(repo_err)?;
    rows.into_iter().map(sqlite_media_file_row).collect()
}

async fn postgres_media_files(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> AppResult<Vec<MediaFileRow>> {
    let rows = sqlx::query("SELECT id, title_id, file_path FROM media_files ORDER BY title_id, id")
        .fetch_all(&mut **tx)
        .await
        .map_err(repo_err)?;
    rows.into_iter().map(postgres_media_file_row).collect()
}

fn sqlite_media_file_row(row: sqlx::sqlite::SqliteRow) -> AppResult<MediaFileRow> {
    Ok(MediaFileRow {
        id: row.try_get("id").map_err(repo_err)?,
        title_id: row.try_get("title_id").map_err(repo_err)?,
        file_path: row.try_get("file_path").map_err(repo_err)?,
    })
}

fn postgres_media_file_row(row: sqlx::postgres::PgRow) -> AppResult<MediaFileRow> {
    Ok(MediaFileRow {
        id: row.try_get("id").map_err(repo_err)?,
        title_id: row.try_get("title_id").map_err(repo_err)?,
        file_path: row.try_get("file_path").map_err(repo_err)?,
    })
}

async fn apply_sqlite_plans(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    plans: &[TitleFolderPlan],
) -> AppResult<()> {
    for plan in plans {
        sqlx::query("UPDATE titles SET folder_path = ?1 WHERE id = ?2")
            .bind(&plan.folder_path)
            .bind(&plan.title_id)
            .execute(&mut **tx)
            .await
            .map_err(repo_err)?;
        for media_file_id in &plan.unlink_media_file_ids {
            sqlx::query("DELETE FROM media_files WHERE id = ?1")
                .bind(media_file_id)
                .execute(&mut **tx)
                .await
                .map_err(repo_err)?;
        }
    }
    Ok(())
}

async fn apply_postgres_plans(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    plans: &[TitleFolderPlan],
) -> AppResult<()> {
    for plan in plans {
        sqlx::query("UPDATE titles SET folder_path = $1 WHERE id = $2")
            .bind(&plan.folder_path)
            .bind(&plan.title_id)
            .execute(&mut **tx)
            .await
            .map_err(repo_err)?;
        for media_file_id in &plan.unlink_media_file_ids {
            sqlx::query("DELETE FROM media_files WHERE id = $1")
                .bind(media_file_id)
                .execute(&mut **tx)
                .await
                .map_err(repo_err)?;
        }
    }
    Ok(())
}

fn repo_err(error: impl std::fmt::Display) -> AppError {
    AppError::Repository(format!(
        "0157 title folder ownership migration failed: {error}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    fn title(folder_path: Option<&str>) -> TitleRow {
        title_for("title-1", folder_path)
    }

    fn title_for(id: &str, folder_path: Option<&str>) -> TitleRow {
        TitleRow {
            id: id.to_string(),
            library_id: "library-1".to_string(),
            folder_path: folder_path.map(str::to_string),
            root_path: "/data/Anime".to_string(),
        }
    }

    /// A derived folder is the root joined with the platform separator, so a
    /// Unix-shaped expectation is compared component by component.
    fn assert_folder(actual: Option<&str>, expected: &str) {
        assert_eq!(
            actual.map(std::path::Path::new),
            Some(std::path::Path::new(expected)),
            "{actual:?}"
        );
    }

    fn media(id: &str, path: &str) -> MediaFileRow {
        media_for("title-1", id, path)
    }

    fn media_for(title_id: &str, id: &str, path: &str) -> MediaFileRow {
        MediaFileRow {
            id: id.to_string(),
            title_id: title_id.to_string(),
            file_path: path.to_string(),
        }
    }

    #[test]
    fn largest_media_file_group_wins_and_invalid_rows_unlink() {
        let plans = build_plans(
            vec![title(Some("/data/Anime/Fixture Beta"))],
            vec![
                media("alpha-1", "/data/Anime/Fixture Alpha/Season 01/E01.mkv"),
                media("alpha-2", "/data/Anime/Fixture Alpha/Season 01/E02.mkv"),
                media("beta-1", "/data/Anime/Fixture Beta/Season 01/E01.mkv"),
                media("loose", "/data/Anime/E03.mkv"),
            ],
            |_| 0,
        );

        assert_folder(plans[0].folder_path.as_deref(), "/data/Anime/Fixture Alpha");
        assert_eq!(plans[0].unlink_media_file_ids, vec!["beta-1", "loose"]);
    }

    #[test]
    fn injected_tie_choice_selects_only_a_maximum_group() {
        let plans = build_plans(
            vec![title(None)],
            vec![
                media("alpha", "/data/Anime/Fixture Alpha/E01.mkv"),
                media("beta", "/data/Anime/Fixture Beta/E01.mkv"),
            ],
            |_| 1,
        );

        assert!(plans[0].tied_maximum);
        assert_folder(plans[0].folder_path.as_deref(), "/data/Anime/Fixture Beta");
        assert_eq!(plans[0].unlink_media_file_ids, vec!["alpha"]);
    }

    #[test]
    fn valid_existing_folder_survives_when_no_media_group_is_valid() {
        let plans = build_plans(
            vec![title(Some("/data/Anime/Owned"))],
            vec![media("loose", "/data/Anime/E01.mkv")],
            |_| 0,
        );

        assert_eq!(plans[0].folder_path.as_deref(), Some("/data/Anime/Owned"));
        assert_eq!(plans[0].unlink_media_file_ids, vec!["loose"]);
    }

    #[test]
    fn duplicate_same_library_owner_with_fewer_media_rows_is_unlinked() {
        let plans = build_plans(
            vec![title_for("title-1", None), title_for("title-2", None)],
            vec![
                media_for("title-1", "first-1", "/data/Anime/Shared Fixture/E01.mkv"),
                media_for("title-1", "first-2", "/data/Anime/Shared Fixture/E02.mkv"),
                media_for("title-2", "second-1", "/data/Anime/Shared Fixture/E03.mkv"),
            ],
            |_| 0,
        );

        let first = plans
            .iter()
            .find(|plan| plan.title_id == "title-1")
            .expect("first plan");
        let second = plans
            .iter()
            .find(|plan| plan.title_id == "title-2")
            .expect("second plan");
        assert_folder(first.folder_path.as_deref(), "/data/Anime/Shared Fixture");
        assert!(first.unlink_media_file_ids.is_empty());
        assert_eq!(second.folder_path, None);
        assert_eq!(second.unlink_media_file_ids, vec!["second-1"]);
    }

    async fn fixture_pool() -> sqlx::SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory database");
        sqlx::raw_sql(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE library_roots (id TEXT PRIMARY KEY, path TEXT NOT NULL);
             CREATE TABLE titles (
                 id TEXT PRIMARY KEY,
                 library_id TEXT NOT NULL,
                 folder_path TEXT,
                 root_folder_id TEXT NOT NULL REFERENCES library_roots(id)
             );
             CREATE TABLE media_files (
                 id TEXT PRIMARY KEY,
                 title_id TEXT NOT NULL REFERENCES titles(id),
                 file_path TEXT NOT NULL,
                 role TEXT NOT NULL
             );
             CREATE TABLE file_episode_map (
                 file_id TEXT NOT NULL REFERENCES media_files(id) ON DELETE CASCADE,
                 episode_id TEXT NOT NULL
             );
             INSERT INTO library_roots (id, path) VALUES ('root-1', '/data/Anime');
             INSERT INTO titles (id, library_id, folder_path, root_folder_id)
             VALUES ('title-1', 'library-1', '/data/Anime/Old', 'root-1');
             INSERT INTO media_files (id, title_id, file_path, role) VALUES
                 ('winner-1', 'title-1', '/data/Anime/Winner/E01.mkv', 'primary'),
                 ('winner-2', 'title-1', '/data/Anime/Winner/E02.mkv', 'additional'),
                 ('loser-1', 'title-1', '/data/Anime/Loser/E03.mkv', 'primary');
             INSERT INTO file_episode_map (file_id, episode_id)
             VALUES ('loser-1', 'episode-3');",
        )
        .execute(&pool)
        .await
        .expect("fixture schema");
        pool
    }

    #[tokio::test]
    async fn sqlite_hook_updates_owner_and_cascades_only_catalog_rows() {
        let pool = fixture_pool().await;
        let mut tx = pool.begin().await.expect("transaction");
        migrate_title_folder_ownership_sqlite(&mut tx)
            .await
            .expect("migration hook");
        tx.commit().await.expect("commit");

        let folder_path: Option<String> =
            sqlx::query_scalar("SELECT folder_path FROM titles WHERE id = 'title-1'")
                .fetch_one(&pool)
                .await
                .expect("title folder");
        let media_ids: Vec<String> = sqlx::query_scalar("SELECT id FROM media_files ORDER BY id")
            .fetch_all(&pool)
            .await
            .expect("media ids");
        let link_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM file_episode_map")
            .fetch_one(&pool)
            .await
            .expect("link count");

        assert_folder(folder_path.as_deref(), "/data/Anime/Winner");
        assert_eq!(media_ids, vec!["winner-1", "winner-2"]);
        assert_eq!(link_count, 0);
    }

    #[tokio::test]
    async fn sqlite_hook_failure_rolls_back_folder_and_media_changes() {
        let pool = fixture_pool().await;
        sqlx::raw_sql(
            "CREATE TRIGGER reject_folder_update
             BEFORE UPDATE OF folder_path ON titles
             BEGIN
                 SELECT RAISE(ABORT, 'reject ownership update');
             END;",
        )
        .execute(&pool)
        .await
        .expect("failure trigger");

        let mut tx = pool.begin().await.expect("transaction");
        migrate_title_folder_ownership_sqlite(&mut tx)
            .await
            .expect_err("hook must fail");
        tx.rollback().await.expect("rollback");

        let folder_path: Option<String> =
            sqlx::query_scalar("SELECT folder_path FROM titles WHERE id = 'title-1'")
                .fetch_one(&pool)
                .await
                .expect("title folder");
        let media_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM media_files")
            .fetch_one(&pool)
            .await
            .expect("media count");
        assert_eq!(folder_path.as_deref(), Some("/data/Anime/Old"));
        assert_eq!(media_count, 3);
    }
}

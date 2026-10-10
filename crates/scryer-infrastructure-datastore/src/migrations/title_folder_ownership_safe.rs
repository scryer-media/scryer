//! 0160 — non-destructive replacement for the quarantined 0157
//! "single title folder ownership" migration.
//!
//! 0157 elected one folder per title (majority folder, random tie-break) and
//! **deleted** every `media_files` row outside it, in one long transaction.
//! It is recorded-but-never-executed via `migrations::known_bad`; this
//! migration performs the safe part of that work on every database — those
//! that never ran 0157 and those that already did:
//!
//! - never touches `media_files`;
//! - keeps a title's existing folder when it is a valid top-level folder under
//!   the title's root;
//! - otherwise sets the folder only when the title's media lives in exactly one
//!   top-level folder under its root, and no other title already owns that
//!   folder in the same library;
//! - leaves the folder unset and logs the title on any ambiguity (media in
//!   several folders, files outside the root, or a folder another title owns)
//!   so a later scan or an operator repairs it explicitly.
//!
//! The helpers below are deliberately re-implemented rather than shared with
//! `title_folder_ownership.rs`: a quarantined migration's source stays
//! byte-for-byte as it shipped.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Component;

use scryer_application::stored_paths::{
    folder_path_identity_key, folder_paths_match, path_to_stored_string, stored_path_to_path_buf,
};
use scryer_application::{AppError, AppResult};
use sqlx::Row;
use tracing::{info, warn};

#[derive(Clone, Debug)]
struct TitleRow {
    id: String,
    library_id: String,
    folder_path: Option<String>,
    root_path: String,
}

#[derive(Clone, Debug)]
struct MediaFileRow {
    title_id: String,
    file_path: String,
}

/// What 0160 decided for one title.
#[derive(Clone, Debug, PartialEq, Eq)]
enum FolderDecision {
    /// The stored folder is valid; nothing to do.
    KeepExisting,
    /// Exactly one unambiguous folder: assign it.
    Assign(String),
    /// Leave `folder_path` as it is; the reason is logged.
    Leave { reason: &'static str },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TitleFolderPlan {
    title_id: String,
    library_id: String,
    decision: FolderDecision,
    folder_counts: Vec<(String, usize)>,
    files_outside_root: usize,
}

pub async fn migrate_title_folder_ownership_safe_sqlite(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> AppResult<()> {
    let titles = sqlite_titles(tx).await?;
    let media_files = sqlite_media_files(tx).await?;
    let plans = build_plans(titles, media_files);
    log_plans(&plans);
    for plan in &plans {
        if let FolderDecision::Assign(folder_path) = &plan.decision {
            sqlx::query("UPDATE titles SET folder_path = ?1 WHERE id = ?2")
                .bind(folder_path)
                .bind(&plan.title_id)
                .execute(&mut **tx)
                .await
                .map_err(repo_err)?;
        }
    }
    Ok(())
}

pub async fn migrate_title_folder_ownership_safe_postgres(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> AppResult<()> {
    let titles = postgres_titles(tx).await?;
    let media_files = postgres_media_files(tx).await?;
    let plans = build_plans(titles, media_files);
    log_plans(&plans);
    for plan in &plans {
        if let FolderDecision::Assign(folder_path) = &plan.decision {
            sqlx::query("UPDATE titles SET folder_path = $1 WHERE id = $2")
                .bind(folder_path)
                .bind(&plan.title_id)
                .execute(&mut **tx)
                .await
                .map_err(repo_err)?;
        }
    }
    Ok(())
}

fn build_plans(titles: Vec<TitleRow>, media_files: Vec<MediaFileRow>) -> Vec<TitleFolderPlan> {
    let mut media_by_title = HashMap::<String, Vec<MediaFileRow>>::new();
    for media_file in media_files {
        media_by_title
            .entry(media_file.title_id.clone())
            .or_default()
            .push(media_file);
    }

    // Folders already owned by a title with a valid stored folder are taken:
    // a title without a folder never gets assigned one another title owns.
    let mut owned_folders = HashSet::<(String, String)>::new();
    for title in &titles {
        if let Some(existing) =
            valid_existing_folder(&title.root_path, title.folder_path.as_deref())
            && let Some(key) = folder_path_identity_key(&existing)
        {
            owned_folders.insert((title.library_id.clone(), key));
        }
    }

    let mut plans = Vec::with_capacity(titles.len());
    for title in titles {
        let media_files = media_by_title.remove(&title.id).unwrap_or_default();
        let plan = build_title_plan(title, media_files, &owned_folders);
        if let FolderDecision::Assign(folder_path) = &plan.decision
            && let Some(key) = folder_path_identity_key(folder_path)
        {
            // Two folderless titles sharing one folder: the first assignment
            // wins, the second is left for repair rather than doubling up.
            owned_folders.insert((plan.library_id.clone(), key));
        }
        plans.push(plan);
    }
    plans
}

fn build_title_plan(
    title: TitleRow,
    media_files: Vec<MediaFileRow>,
    owned_folders: &HashSet<(String, String)>,
) -> TitleFolderPlan {
    let mut groups = BTreeMap::<String, (String, usize)>::new();
    let mut files_outside_root = 0usize;
    for media_file in &media_files {
        let Some(folder_path) = top_level_title_folder(&title.root_path, &media_file.file_path)
        else {
            files_outside_root += 1;
            continue;
        };
        let Some(key) = folder_path_identity_key(&folder_path) else {
            files_outside_root += 1;
            continue;
        };
        groups
            .entry(key)
            .and_modify(|(_, count)| *count += 1)
            .or_insert((folder_path, 1));
    }
    let folder_counts = groups
        .values()
        .map(|(path, count)| (path.clone(), *count))
        .collect::<Vec<_>>();

    let decision =
        if valid_existing_folder(&title.root_path, title.folder_path.as_deref()).is_some() {
            FolderDecision::KeepExisting
        } else if files_outside_root > 0 {
            FolderDecision::Leave {
                reason: "media outside the title's library root",
            }
        } else {
            match groups.iter().next() {
                None => FolderDecision::Leave {
                    reason: "no media files to infer a folder from",
                },
                Some((key, (folder_path, _))) if groups.len() == 1 => {
                    if owned_folders.contains(&(title.library_id.clone(), key.clone())) {
                        FolderDecision::Leave {
                            reason: "folder is already owned by another title",
                        }
                    } else {
                        FolderDecision::Assign(folder_path.clone())
                    }
                }
                Some(_) => FolderDecision::Leave {
                    reason: "media spread across several folders",
                },
            }
        };

    TitleFolderPlan {
        title_id: title.id,
        library_id: title.library_id,
        decision,
        folder_counts,
        files_outside_root,
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

fn log_plans(plans: &[TitleFolderPlan]) {
    let mut assigned = 0usize;
    let mut kept = 0usize;
    for plan in plans {
        match &plan.decision {
            FolderDecision::KeepExisting => kept += 1,
            FolderDecision::Assign(folder_path) => {
                assigned += 1;
                info!(
                    title_id = %plan.title_id,
                    folder_path = %folder_path,
                    "0160: assigned the title folder from its single media folder"
                );
            }
            FolderDecision::Leave { reason } => {
                warn!(
                    title_id = %plan.title_id,
                    library_id = %plan.library_id,
                    reason,
                    folder_counts = ?plan.folder_counts,
                    files_outside_root = plan.files_outside_root,
                    "0160: left the title folder unset for explicit repair (no media rows were touched)"
                );
            }
        }
    }
    info!(
        titles = plans.len(),
        assigned,
        kept,
        left_for_repair = plans.len() - assigned - kept,
        "0160: title folder ownership normalized without deleting media"
    );
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
    rows.into_iter()
        .map(|row| {
            Ok(TitleRow {
                id: row.try_get("id").map_err(repo_err)?,
                library_id: row.try_get("library_id").map_err(repo_err)?,
                folder_path: row.try_get("folder_path").map_err(repo_err)?,
                root_path: row.try_get("root_path").map_err(repo_err)?,
            })
        })
        .collect()
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
    rows.into_iter()
        .map(|row| {
            Ok(TitleRow {
                id: row.try_get("id").map_err(repo_err)?,
                library_id: row.try_get("library_id").map_err(repo_err)?,
                folder_path: row.try_get("folder_path").map_err(repo_err)?,
                root_path: row.try_get("root_path").map_err(repo_err)?,
            })
        })
        .collect()
}

async fn sqlite_media_files(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> AppResult<Vec<MediaFileRow>> {
    let rows = sqlx::query("SELECT title_id, file_path FROM media_files ORDER BY title_id, id")
        .fetch_all(&mut **tx)
        .await
        .map_err(repo_err)?;
    rows.into_iter()
        .map(|row| {
            Ok(MediaFileRow {
                title_id: row.try_get("title_id").map_err(repo_err)?,
                file_path: row.try_get("file_path").map_err(repo_err)?,
            })
        })
        .collect()
}

async fn postgres_media_files(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> AppResult<Vec<MediaFileRow>> {
    let rows = sqlx::query("SELECT title_id, file_path FROM media_files ORDER BY title_id, id")
        .fetch_all(&mut **tx)
        .await
        .map_err(repo_err)?;
    rows.into_iter()
        .map(|row| {
            Ok(MediaFileRow {
                title_id: row.try_get("title_id").map_err(repo_err)?,
                file_path: row.try_get("file_path").map_err(repo_err)?,
            })
        })
        .collect()
}

fn repo_err(error: impl std::fmt::Display) -> AppError {
    AppError::Repository(format!(
        "0160 title folder ownership (safe) migration failed: {error}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn title(id: &str, folder_path: Option<&str>) -> TitleRow {
        TitleRow {
            id: id.to_string(),
            library_id: "library-1".to_string(),
            folder_path: folder_path.map(str::to_string),
            root_path: "/data/Anime".to_string(),
        }
    }

    fn media(title_id: &str, path: &str) -> MediaFileRow {
        MediaFileRow {
            title_id: title_id.to_string(),
            file_path: path.to_string(),
        }
    }

    fn decision(plans: &[TitleFolderPlan], title_id: &str) -> FolderDecision {
        plans
            .iter()
            .find(|plan| plan.title_id == title_id)
            .expect("plan for title")
            .decision
            .clone()
    }

    /// An assigned folder is the root joined with the platform separator, so a
    /// Unix-shaped expectation is compared component by component.
    fn assert_assigned(actual: FolderDecision, expected: &str) {
        match actual {
            FolderDecision::Assign(folder) => {
                assert_eq!(
                    std::path::Path::new(&folder),
                    std::path::Path::new(expected)
                )
            }
            other => panic!("expected Assign({expected:?}), got {other:?}"),
        }
    }

    #[test]
    fn single_folder_is_assigned_and_no_media_is_planned_for_deletion() {
        let plans = build_plans(
            vec![title("t1", None)],
            vec![
                media("t1", "/data/Anime/Show A/Season 1/e1.mkv"),
                media("t1", "/data/Anime/Show A/Season 2/e2.mkv"),
            ],
        );
        assert_assigned(decision(&plans, "t1"), "/data/Anime/Show A");
    }

    #[test]
    fn media_in_several_folders_is_left_for_repair() {
        // 0157 would have kept the majority folder and deleted the other rows.
        let plans = build_plans(
            vec![title("t1", None)],
            vec![
                media("t1", "/data/Anime/Show A/e1.mkv"),
                media("t1", "/data/Anime/Show A/e2.mkv"),
                media("t1", "/data/Anime/Show A (2019)/e3.mkv"),
            ],
        );
        assert_eq!(
            decision(&plans, "t1"),
            FolderDecision::Leave {
                reason: "media spread across several folders"
            }
        );
        assert_eq!(plans[0].folder_counts.len(), 2);
    }

    #[test]
    fn valid_existing_folder_is_kept_even_with_stray_media() {
        let plans = build_plans(
            vec![title("t1", Some("/data/Anime/Show A"))],
            vec![
                media("t1", "/data/Anime/Show A/e1.mkv"),
                media("t1", "/data/Anime/Show A - Copy/e1.mkv"),
            ],
        );
        assert_eq!(decision(&plans, "t1"), FolderDecision::KeepExisting);
    }

    #[test]
    fn media_outside_the_root_or_a_folder_owned_by_another_title_is_left_alone() {
        let plans = build_plans(
            vec![
                title("owner", Some("/data/Anime/Show A")),
                title("outside", None),
                title("collides", None),
            ],
            vec![
                media("owner", "/data/Anime/Show A/e1.mkv"),
                media("outside", "/mnt/other/Show B/e1.mkv"),
                media("collides", "/data/Anime/Show A/dupe.mkv"),
            ],
        );
        assert_eq!(decision(&plans, "owner"), FolderDecision::KeepExisting);
        assert_eq!(
            decision(&plans, "outside"),
            FolderDecision::Leave {
                reason: "media outside the title's library root"
            }
        );
        assert_eq!(
            decision(&plans, "collides"),
            FolderDecision::Leave {
                reason: "folder is already owned by another title"
            }
        );
    }

    #[test]
    fn two_folderless_titles_sharing_a_folder_assign_only_the_first() {
        let plans = build_plans(
            vec![title("a", None), title("b", None)],
            vec![
                media("a", "/data/Anime/Show A/e1.mkv"),
                media("b", "/data/Anime/Show A/e2.mkv"),
            ],
        );
        assert_assigned(decision(&plans, "a"), "/data/Anime/Show A");
        assert_eq!(
            decision(&plans, "b"),
            FolderDecision::Leave {
                reason: "folder is already owned by another title"
            }
        );
    }

    #[test]
    fn a_stale_stored_folder_outside_the_root_is_replaced_only_when_unambiguous() {
        let plans = build_plans(
            vec![title("t1", Some("/old/root/Show A"))],
            vec![media("t1", "/data/Anime/Show A/e1.mkv")],
        );
        assert_assigned(decision(&plans, "t1"), "/data/Anime/Show A");
    }
}

//! Give every title folder a comparison key and untangle folders that collide,
//! so a unique index on the key can be created.
//!
//! The repair only edits the catalog. It never touches the filesystem or a
//! `media_files` row:
//!
//! - a folder that is only whitespace becomes no folder;
//! - a folder that is a library root, or holds one, is cleared first, so it
//!   never wins a shared or nested folder from a real title folder;
//! - titles whose folders are the same folder, or lie one inside another, are
//!   decided by `scryer_application::title_folder_repair`, and each losing
//!   title has its folder cleared, with the reason code and the folder it lost
//!   stored beside it;
//! - every remaining folder gets its key.
//!
//! Running it again changes nothing.

use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{DateTime, NaiveDateTime, Utc};
use scryer_application::title_folder_keys::{
    TitleFolderKeyRelation, title_folder_key, title_folder_key_relation,
};
use scryer_application::title_folder_repair::{
    ClearedTitleFolder, TitleFolderRepairCandidate, TitleFolderRepairComponent,
    plan_title_folder_repair, title_folder_library_root_conflicts,
};
use scryer_application::{AppError, AppResult};
use sqlx::Row;
use tracing::info;

#[derive(Debug, Clone)]
struct TitleRow {
    id: String,
    folder_path: Option<String>,
    folder_path_key: Option<String>,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TitleFolderChange {
    /// The folder was blank; record no folder.
    ClearBlank { title_id: String },
    /// The title lost an overlapping folder to another title.
    ClearConflict {
        title_id: String,
        reason: &'static str,
        lost_folder: String,
    },
    /// The folder stays; its stored key is missing or stale.
    SetKey { title_id: String, key: String },
}

pub async fn repair_title_folder_keys_sqlite(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> AppResult<()> {
    let rows = sqlx::query(
        "SELECT id, folder_path, folder_path_key, created_at FROM titles \
         WHERE folder_path IS NOT NULL OR folder_path_key IS NOT NULL",
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(repo_err)?;
    let mut titles = Vec::with_capacity(rows.len());
    for row in rows {
        let created_at: Option<String> = row.try_get("created_at").map_err(repo_err)?;
        titles.push(TitleRow {
            id: row.try_get("id").map_err(repo_err)?,
            folder_path: row.try_get("folder_path").map_err(repo_err)?,
            folder_path_key: row.try_get("folder_path_key").map_err(repo_err)?,
            created_at: parse_sqlite_timestamp(created_at.as_deref()),
        });
    }
    let root_paths: Vec<String> = sqlx::query_scalar("SELECT path FROM library_roots")
        .fetch_all(&mut **tx)
        .await
        .map_err(repo_err)?;

    let reader = &mut *tx;
    let changes = plan_changes(titles, &root_paths, |title_ids| async move {
        let title_ids = serde_json::to_string(&title_ids).map_err(repo_err)?;
        let rows = sqlx::query(
            "SELECT id, title_id, file_path FROM media_files \
             WHERE role = 'primary' AND title_id IN (SELECT value FROM json_each(?1))",
        )
        .bind(title_ids)
        .fetch_all(&mut **reader)
        .await
        .map_err(repo_err)?;
        rows.iter()
            .map(|row| {
                Ok(MediaFileRow {
                    id: row.try_get("id").map_err(repo_err)?,
                    title_id: row.try_get("title_id").map_err(repo_err)?,
                    file_path: row.try_get("file_path").map_err(repo_err)?,
                })
            })
            .collect()
    })
    .await?;

    for change in &changes {
        let query = match change {
            TitleFolderChange::ClearBlank { title_id } => sqlx::query(
                "UPDATE titles SET folder_path = NULL, folder_path_key = NULL WHERE id = ?1",
            )
            .bind(title_id),
            TitleFolderChange::ClearConflict {
                title_id,
                reason,
                lost_folder,
            } => sqlx::query(
                "UPDATE titles SET folder_path = NULL, folder_path_key = NULL, \
                 folder_conflict_reason = ?1, folder_conflict_path = ?2 WHERE id = ?3",
            )
            .bind(*reason)
            .bind(lost_folder)
            .bind(title_id),
            TitleFolderChange::SetKey { title_id, key } => {
                sqlx::query("UPDATE titles SET folder_path_key = ?1 WHERE id = ?2")
                    .bind(key)
                    .bind(title_id)
            }
        };
        query.execute(&mut **tx).await.map_err(repo_err)?;
    }
    Ok(())
}

pub async fn repair_title_folder_keys_postgres(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> AppResult<()> {
    let rows = sqlx::query(
        "SELECT id, folder_path, folder_path_key, created_at FROM titles \
         WHERE folder_path IS NOT NULL OR folder_path_key IS NOT NULL",
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(repo_err)?;
    let mut titles = Vec::with_capacity(rows.len());
    for row in rows {
        titles.push(TitleRow {
            id: row.try_get("id").map_err(repo_err)?,
            folder_path: row.try_get("folder_path").map_err(repo_err)?,
            folder_path_key: row.try_get("folder_path_key").map_err(repo_err)?,
            created_at: row.try_get("created_at").map_err(repo_err)?,
        });
    }
    let root_paths: Vec<String> = sqlx::query_scalar("SELECT path FROM library_roots")
        .fetch_all(&mut **tx)
        .await
        .map_err(repo_err)?;

    let reader = &mut *tx;
    let changes = plan_changes(titles, &root_paths, |title_ids| async move {
        let rows = sqlx::query(
            "SELECT id, title_id, file_path FROM media_files \
             WHERE role = 'primary' AND title_id = ANY($1)",
        )
        .bind(title_ids)
        .fetch_all(&mut **reader)
        .await
        .map_err(repo_err)?;
        rows.iter()
            .map(|row| {
                Ok(MediaFileRow {
                    id: row.try_get("id").map_err(repo_err)?,
                    title_id: row.try_get("title_id").map_err(repo_err)?,
                    file_path: row.try_get("file_path").map_err(repo_err)?,
                })
            })
            .collect()
    })
    .await?;

    for change in &changes {
        let query = match change {
            TitleFolderChange::ClearBlank { title_id } => sqlx::query(
                "UPDATE titles SET folder_path = NULL, folder_path_key = NULL WHERE id = $1",
            )
            .bind(title_id),
            TitleFolderChange::ClearConflict {
                title_id,
                reason,
                lost_folder,
            } => sqlx::query(
                "UPDATE titles SET folder_path = NULL, folder_path_key = NULL, \
                 folder_conflict_reason = $1, folder_conflict_path = $2 WHERE id = $3",
            )
            .bind(*reason)
            .bind(lost_folder)
            .bind(title_id),
            TitleFolderChange::SetKey { title_id, key } => {
                sqlx::query("UPDATE titles SET folder_path_key = $1 WHERE id = $2")
                    .bind(key)
                    .bind(title_id)
            }
        };
        query.execute(&mut **tx).await.map_err(repo_err)?;
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct MediaFileRow {
    id: String,
    title_id: String,
    file_path: String,
}

/// Everything the repair changes, in a fixed order. `load_files` returns the
/// primary media files of the given titles; it is called only when some
/// folders overlap.
async fn plan_changes<F, Fut>(
    titles: Vec<TitleRow>,
    root_paths: &[String],
    load_files: F,
) -> AppResult<Vec<TitleFolderChange>>
where
    F: FnOnce(Vec<String>) -> Fut,
    Fut: std::future::Future<Output = AppResult<Vec<MediaFileRow>>>,
{
    let mut changes = Vec::new();
    let mut candidates = Vec::new();
    for title in titles {
        let key = title.folder_path.as_deref().and_then(title_folder_key);
        match (title.folder_path, key) {
            (Some(folder_path), Some(key)) => candidates.push((
                TitleFolderRepairCandidate {
                    title_id: title.id,
                    folder_path,
                    key,
                    own_file_count: 0,
                    created_at: title.created_at,
                },
                title.folder_path_key,
            )),
            (Some(_), None) => changes.push(TitleFolderChange::ClearBlank { title_id: title.id }),
            (None, _) => {
                if title.folder_path_key.is_some() {
                    changes.push(TitleFolderChange::ClearBlank { title_id: title.id });
                }
            }
        }
    }
    candidates.sort_by(|left, right| left.0.title_id.cmp(&right.0.title_id));

    let mut cleared_ids = HashSet::new();
    let all = candidates
        .iter()
        .map(|(candidate, _)| candidate.clone())
        .collect::<Vec<_>>();
    for cleared in title_folder_library_root_conflicts(&all, root_paths) {
        log_root_conflict(&cleared);
        cleared_ids.insert(cleared.title_id.clone());
        changes.push(TitleFolderChange::ClearConflict {
            title_id: cleared.title_id,
            reason: cleared.reason,
            lost_folder: cleared.lost_folder,
        });
    }
    candidates.retain(|(candidate, _)| !cleared_ids.contains(&candidate.title_id));

    let plain = candidates
        .iter()
        .map(|(candidate, _)| candidate.clone())
        .collect::<Vec<_>>();
    let overlapping = plan_title_folder_repair(&plain);
    if !overlapping.is_empty() {
        let members = overlapping
            .into_iter()
            .flat_map(|component| component.members)
            .map(|member| member.title_id)
            .collect::<Vec<_>>();
        let counts = own_file_counts(&plain, load_files(members).await?);
        for (candidate, _) in &mut candidates {
            candidate.own_file_count = counts.get(&candidate.title_id).copied().unwrap_or(0);
        }
        let counted = candidates
            .iter()
            .map(|(candidate, _)| candidate.clone())
            .collect::<Vec<_>>();
        for component in plan_title_folder_repair(&counted) {
            log_component(&component);
            for cleared in component.cleared {
                cleared_ids.insert(cleared.title_id.clone());
                changes.push(TitleFolderChange::ClearConflict {
                    title_id: cleared.title_id,
                    reason: cleared.reason,
                    lost_folder: cleared.lost_folder,
                });
            }
        }
    }

    for (candidate, stored_key) in candidates {
        if cleared_ids.contains(&candidate.title_id) {
            continue;
        }
        if stored_key.as_deref() != Some(candidate.key.as_str()) {
            changes.push(TitleFolderChange::SetKey {
                title_id: candidate.title_id,
                key: candidate.key,
            });
        }
    }
    Ok(changes)
}

/// Each title's distinct primary media files that lie under its own folder.
fn own_file_counts(
    titles: &[TitleFolderRepairCandidate],
    files: Vec<MediaFileRow>,
) -> HashMap<String, u64> {
    let keys = titles
        .iter()
        .map(|title| (title.title_id.as_str(), title.key.as_str()))
        .collect::<HashMap<_, _>>();
    let mut seen = HashMap::<String, HashSet<String>>::new();
    for file in files {
        let Some(folder_key) = keys.get(file.title_id.as_str()) else {
            continue;
        };
        let Some(file_key) = title_folder_key(&file.file_path) else {
            continue;
        };
        if title_folder_key_relation(&file_key, folder_key) == Some(TitleFolderKeyRelation::Inside)
        {
            seen.entry(file.title_id).or_default().insert(file.id);
        }
    }
    seen.into_iter()
        .map(|(title_id, ids)| (title_id, ids.len() as u64))
        .collect()
}

fn log_root_conflict(cleared: &ClearedTitleFolder) {
    info!(
        title_id = %cleared.title_id,
        folder_before = %cleared.lost_folder,
        folder_after = "",
        decision = "cleared",
        reason = cleared.reason,
        library_root = %cleared.kept_by,
        "title folder library root repair"
    );
}

fn log_component(component: &TitleFolderRepairComponent) {
    let cleared = component
        .cleared
        .iter()
        .map(|cleared| (cleared.title_id.as_str(), cleared))
        .collect::<BTreeMap<_, _>>();
    for member in &component.members {
        match cleared.get(member.title_id.as_str()) {
            Some(cleared) => info!(
                title_id = %member.title_id,
                folder_before = %member.folder_path,
                folder_after = "",
                own_file_count = member.own_file_count,
                decision = "cleared",
                reason = cleared.reason,
                kept_by = %cleared.kept_by,
                "title folder overlap repair"
            ),
            None => info!(
                title_id = %member.title_id,
                folder_before = %member.folder_path,
                folder_after = %member.folder_path,
                own_file_count = member.own_file_count,
                decision = "kept",
                "title folder overlap repair"
            ),
        }
    }
}

/// SQLite stores `created_at` as text. A value that cannot be read sorts as
/// the latest, so it never wins an earliest-added tie by accident.
fn parse_sqlite_timestamp(value: Option<&str>) -> DateTime<Utc> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return DateTime::<Utc>::MAX_UTC;
    };
    if let Ok(parsed) = DateTime::parse_from_rfc3339(value) {
        return parsed.with_timezone(&Utc);
    }
    for format in ["%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%dT%H:%M:%S%.f"] {
        if let Ok(parsed) = NaiveDateTime::parse_from_str(value, format) {
            return parsed.and_utc();
        }
    }
    DateTime::<Utc>::MAX_UTC
}

fn repo_err(error: impl std::fmt::Display) -> AppError {
    AppError::Repository(error.to_string())
}

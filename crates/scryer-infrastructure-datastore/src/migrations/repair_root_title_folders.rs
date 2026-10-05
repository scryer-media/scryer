//! Repair title folders that a rename recorded as a library root, an ancestor
//! of one, or somewhere outside the title's library.
//!
//! Applying a rename used to re-derive the title's folder from every moved
//! file by climbing one directory per level of the file's old location below
//! the recorded folder. Flattening a season-folder title, or moving a file out
//! of a deeper subfolder, made each climb overshoot, so the record walked up
//! through the library root and beyond. Everything that selects files "in the
//! title's folder" then acted on the whole library.
//!
//! This repair reads only the catalog. It never touches the filesystem, never
//! deletes a row, and only rewrites `titles.folder_path` (and the matching
//! `titles.root_folder_id`) when the right value is unambiguous:
//!
//! A title is a repair candidate when its non-empty recorded folder
//! - is a configured library root (of any library),
//! - contains a configured library root (of any library),
//! - is outside every root of the title's own library, or
//! - strictly contains the folder its media files imply.
//!
//! The implied folder is the library root plus the first path segment below
//! it, and is written only when
//! - every media file of the title (any role) implies the same folder,
//! - the title's files are spread across that folder rather than all sitting in
//!   one subfolder of it (which is what a nested `root/Group/Title` layout or
//!   a single season folder looks like, and the two cannot be told apart),
//! - the folder is itself a valid title folder,
//! - no other title records that folder, a folder inside it, or a folder
//!   containing it, and no other title has media files inside it.
//!
//! Every other candidate is left exactly as it was and logged, so an operator
//! (or a later rename, which now records the planned folder) can correct it.
//! Re-running finds nothing left to repair, so the repair is idempotent.
//!
//! A rename could also leave the record *deeper* than the title's real folder:
//! it re-derived the folder from each moved file, and a file landing in a
//! season folder whose name it did not recognise (`Specials`, or a custom
//! season template) recorded that season folder. Such a title has media files
//! outside its recorded folder. The record is moved up one level, to the
//! folder containing the recorded one, when every media file of the title is
//! inside that folder, it is a valid title folder, and no other title records
//! or has media in an overlapping folder. Otherwise it is only logged.

use std::collections::HashMap;

use scryer_application::stored_paths::{folder_path_identity_key, folder_paths_match};
use scryer_application::title_folder_rules::{
    containing_folder_keys, folder_containment_key, path_is_strictly_within,
    stored_path_is_inside_folder, title_folder_from_media_paths, title_folder_root_violation,
};
use scryer_application::{AppError, AppResult};
use sqlx::Row;
use tracing::{info, warn};

#[derive(Clone, Debug)]
struct RootRow {
    id: String,
    library_id: String,
    path: String,
}

#[derive(Clone, Debug)]
struct TitleRow {
    id: String,
    library_id: String,
    root_folder_id: Option<String>,
    folder_path: Option<String>,
}

#[derive(Clone, Debug)]
struct MediaFileRow {
    title_id: String,
    file_path: String,
}

/// One title's repair, when it gets one.
#[derive(Clone, Debug, PartialEq, Eq)]
struct TitleFolderRepair {
    title_id: String,
    folder_path: String,
    root_folder_id: Option<String>,
}

/// Why a repair candidate was left alone.
#[derive(Clone, Debug, PartialEq, Eq)]
struct LeftAlone {
    title_id: String,
    folder_path: String,
    flagged_because: &'static str,
    reason: &'static str,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct RepairPlan {
    repairs: Vec<TitleFolderRepair>,
    left_alone: Vec<LeftAlone>,
    /// Titles whose folder holds only some of their files and whose containing
    /// folder could not be recorded instead; logged only.
    too_deep: Vec<(String, String)>,
}

pub async fn repair_root_title_folders_sqlite(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> AppResult<()> {
    let roots = sqlite_roots(tx).await?;
    let titles = sqlite_titles(tx).await?;
    let media_files = sqlite_media_files(tx).await?;
    let plan = build_repair_plan(&roots, &titles, &media_files);
    log_plan(&plan);
    for repair in &plan.repairs {
        match &repair.root_folder_id {
            Some(root_folder_id) => {
                sqlx::query(
                    "UPDATE titles SET folder_path = ?1, root_folder_id = ?2 WHERE id = ?3",
                )
                .bind(&repair.folder_path)
                .bind(root_folder_id)
                .bind(&repair.title_id)
                .execute(&mut **tx)
                .await
                .map_err(repo_err)?;
            }
            None => {
                sqlx::query("UPDATE titles SET folder_path = ?1 WHERE id = ?2")
                    .bind(&repair.folder_path)
                    .bind(&repair.title_id)
                    .execute(&mut **tx)
                    .await
                    .map_err(repo_err)?;
            }
        }
    }
    Ok(())
}

pub async fn repair_root_title_folders_postgres(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> AppResult<()> {
    let roots = postgres_roots(tx).await?;
    let titles = postgres_titles(tx).await?;
    let media_files = postgres_media_files(tx).await?;
    let plan = build_repair_plan(&roots, &titles, &media_files);
    log_plan(&plan);
    for repair in &plan.repairs {
        match &repair.root_folder_id {
            Some(root_folder_id) => {
                sqlx::query(
                    "UPDATE titles SET folder_path = $1, root_folder_id = $2 WHERE id = $3",
                )
                .bind(&repair.folder_path)
                .bind(root_folder_id)
                .bind(&repair.title_id)
                .execute(&mut **tx)
                .await
                .map_err(repo_err)?;
            }
            None => {
                sqlx::query("UPDATE titles SET folder_path = $1 WHERE id = $2")
                    .bind(&repair.folder_path)
                    .bind(&repair.title_id)
                    .execute(&mut **tx)
                    .await
                    .map_err(repo_err)?;
            }
        }
    }
    Ok(())
}

fn recorded_folder(title: &TitleRow) -> Option<&str> {
    title
        .folder_path
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty())
}

fn build_repair_plan(
    roots: &[RootRow],
    titles: &[TitleRow],
    media_files: &[MediaFileRow],
) -> RepairPlan {
    let all_roots = roots
        .iter()
        .map(|root| root.path.clone())
        .collect::<Vec<_>>();
    let mut roots_by_library = HashMap::<&str, Vec<String>>::new();
    for root in roots {
        roots_by_library
            .entry(root.library_id.as_str())
            .or_default()
            .push(root.path.clone());
    }
    let mut media_by_title = HashMap::<&str, Vec<&str>>::new();
    for media_file in media_files {
        media_by_title
            .entry(media_file.title_id.as_str())
            .or_default()
            .push(media_file.file_path.as_str());
    }
    let library_roots_for =
        |library_id: &str| library_roots_or_all(&roots_by_library, &all_roots, library_id);

    // Pass one: which recorded folders are wrong, and what the files imply.
    let mut candidates = Vec::<(&TitleRow, &str, &'static str, Option<String>)>::new();
    let mut trusted_folders = Vec::<(&str, &str)>::new();
    let mut too_deep = Vec::<(&TitleRow, &str)>::new();
    let mut plan = RepairPlan::default();
    for title in titles {
        let Some(folder) = recorded_folder(title) else {
            continue;
        };
        let library_roots = library_roots_for(&title.library_id);
        let paths = media_by_title
            .get(title.id.as_str())
            .map(Vec::as_slice)
            .unwrap_or_default();
        let implied = title_folder_from_media_paths(library_roots, paths.iter().copied());
        let flagged_because = match title_folder_root_violation(folder, library_roots, &all_roots) {
            Some(violation) => Some(violation.as_str()),
            None => implied
                .as_deref()
                .filter(|implied| path_is_strictly_within(implied, folder))
                .map(|_| "contains_implied_title_folder"),
        };
        match flagged_because {
            Some(flagged_because) => candidates.push((title, folder, flagged_because, implied)),
            None => {
                if let Some(implied) = implied.as_deref()
                    && path_is_strictly_within(folder, implied)
                    && paths
                        .iter()
                        .any(|path| !stored_path_is_inside_folder(folder, path))
                {
                    too_deep.push((title, folder));
                }
                trusted_folders.push((title.id.as_str(), folder));
            }
        }
    }

    if candidates.is_empty() && too_deep.is_empty() {
        return plan;
    }

    // Pass two: repair each candidate whose implied folder is unambiguous and
    // free, in title order, so two candidates never claim one folder. Other
    // titles' folders and files are indexed once; a library can hold thousands
    // of candidates and far more files, and this runs inside the upgrade.
    let mut taken_folders = FolderIndex::default();
    for (title_id, folder) in &trusted_folders {
        taken_folders.insert(title_id, folder);
    }
    let media_owners = MediaOwnerIndex::build(media_files);
    for (title, folder, flagged_because, implied) in candidates {
        let leave = |reason: &'static str| LeftAlone {
            title_id: title.id.clone(),
            folder_path: folder.to_string(),
            flagged_because,
            reason,
        };
        let Some(implied) = implied else {
            plan.left_alone.push(leave(
                "media files do not imply one folder inside the library",
            ));
            continue;
        };
        if folder_paths_match(&implied, folder) {
            plan.left_alone
                .push(leave("the implied folder is the recorded one"));
            continue;
        }
        let library_roots = library_roots_for(&title.library_id);
        if title_folder_root_violation(&implied, library_roots, &all_roots).is_some() {
            plan.left_alone
                .push(leave("the implied folder is not a valid title folder"));
            continue;
        }
        let paths = media_by_title
            .get(title.id.as_str())
            .map(Vec::as_slice)
            .unwrap_or_default();
        // All files in one subfolder of the implied folder: either a single
        // season folder or a nested group layout, which look identical.
        if title_folder_from_media_paths(std::slice::from_ref(&implied), paths.iter().copied())
            .is_some()
        {
            plan.left_alone.push(leave(
                "every media file is in one subfolder; nested layouts are ambiguous",
            ));
            continue;
        }
        if taken_folders.overlaps_another_titles(&title.id, &implied) {
            plan.left_alone
                .push(leave("another title records an overlapping folder"));
            continue;
        }
        if media_owners.another_title_has_media_inside(&title.id, &implied) {
            plan.left_alone
                .push(leave("another title has media files in the implied folder"));
            continue;
        }

        let root_folder_id = most_specific_root_id(roots, &title.library_id, &implied)
            .filter(|root_id| title.root_folder_id.as_deref() != Some(root_id.as_str()));
        taken_folders.insert(&title.id, &implied);
        plan.repairs.push(TitleFolderRepair {
            title_id: title.id.clone(),
            folder_path: implied,
            root_folder_id,
        });
    }

    // Pass three: a record that holds only some of the title's files moves up
    // to the folder containing it, when that folder holds all of them and is
    // free.
    for (title, folder) in too_deep {
        let paths = media_by_title
            .get(title.id.as_str())
            .map(Vec::as_slice)
            .unwrap_or_default();
        let library_roots = library_roots_for(&title.library_id);
        let containing = containing_folder(folder).filter(|containing| {
            paths
                .iter()
                .all(|path| stored_path_is_inside_folder(containing, path))
                && title_folder_root_violation(containing, library_roots, &all_roots).is_none()
                && !taken_folders.overlaps_another_titles(&title.id, containing)
                && !media_owners.another_title_has_media_inside(&title.id, containing)
        });
        let Some(containing) = containing else {
            plan.too_deep.push((title.id.clone(), folder.to_string()));
            continue;
        };
        let root_folder_id = most_specific_root_id(roots, &title.library_id, &containing)
            .filter(|root_id| title.root_folder_id.as_deref() != Some(root_id.as_str()));
        taken_folders.insert(&title.id, &containing);
        plan.repairs.push(TitleFolderRepair {
            title_id: title.id.clone(),
            folder_path: containing,
            root_folder_id,
        });
    }
    plan
}

/// The folder directly containing `folder`, in the spelling `folder` is stored
/// in. Stored paths keep the separators of the host that wrote them, so the
/// cut is whichever separator leaves a folder `folder` is strictly within.
fn containing_folder(folder: &str) -> Option<String> {
    let trimmed = folder.trim().trim_end_matches(['/', '\\']);
    trimmed
        .rmatch_indices(['/', '\\'])
        .map(|(index, _)| &trimmed[..index])
        .find(|candidate| !candidate.is_empty() && path_is_strictly_within(folder, candidate))
        .map(str::to_string)
}

/// The titles that own one key of an index. Only "is there an owner other
/// than this title" is ever asked, so the first owner and whether there are
/// more is all it keeps.
#[derive(Debug, Default)]
struct KeyOwners {
    first: Option<String>,
    several: bool,
}

impl KeyOwners {
    fn add(&mut self, title_id: &str) {
        match &self.first {
            None => self.first = Some(title_id.to_string()),
            Some(first) if first != title_id => self.several = true,
            Some(_) => {}
        }
    }

    fn has_other_than(&self, title_id: &str) -> bool {
        self.several || self.first.as_deref().is_some_and(|first| first != title_id)
    }
}

fn add_owner(index: &mut HashMap<String, KeyOwners>, key: String, title_id: &str) {
    index.entry(key).or_default().add(title_id);
}

/// The folders titles record, indexed so one lookup answers whether a folder
/// is, lies inside, or contains another title's.
#[derive(Debug, Default)]
struct FolderIndex {
    /// Keyed by what [`folder_paths_match`] compares.
    by_identity: HashMap<String, KeyOwners>,
    /// Keyed by the folder itself.
    by_folder: HashMap<String, KeyOwners>,
    /// Keyed by the folder and every folder above it.
    by_containing_folder: HashMap<String, KeyOwners>,
    /// Folders in the escape form, which only compare natively.
    escaped: Vec<(String, String)>,
}

impl FolderIndex {
    fn insert(&mut self, title_id: &str, folder: &str) {
        if let Some(identity) = folder_path_identity_key(folder) {
            add_owner(&mut self.by_identity, identity, title_id);
        }
        let Some(key) = folder_containment_key(folder) else {
            self.escaped
                .push((title_id.to_string(), folder.to_string()));
            return;
        };
        add_owner(&mut self.by_folder, key, title_id);
        for key in containing_folder_keys(folder) {
            add_owner(&mut self.by_containing_folder, key, title_id);
        }
    }

    /// Whether a title other than `title_id` records `folder`, a folder inside
    /// it, or a folder containing it.
    fn overlaps_another_titles(&self, title_id: &str, folder: &str) -> bool {
        let same_folder = folder_path_identity_key(folder)
            .and_then(|identity| self.by_identity.get(&identity))
            .is_some_and(|owners| owners.has_other_than(title_id));
        if same_folder {
            return true;
        }
        let inside_it = folder_containment_key(folder)
            .and_then(|key| self.by_containing_folder.get(&key))
            .is_some_and(|owners| owners.has_other_than(title_id));
        if inside_it {
            return true;
        }
        let containing_it = containing_folder_keys(folder)
            .iter()
            .filter_map(|key| self.by_folder.get(key))
            .any(|owners| owners.has_other_than(title_id));
        if containing_it {
            return true;
        }
        self.escaped.iter().any(|(other_id, other)| {
            other_id != title_id
                && (path_is_strictly_within(other, folder)
                    || path_is_strictly_within(folder, other))
        })
    }
}

/// Which titles have media files in each folder.
#[derive(Debug, Default)]
struct MediaOwnerIndex<'a> {
    by_containing_folder: HashMap<String, KeyOwners>,
    /// Files in the escape form, which only compare natively.
    escaped: Vec<&'a MediaFileRow>,
}

impl<'a> MediaOwnerIndex<'a> {
    fn build(media_files: &'a [MediaFileRow]) -> Self {
        let mut index = Self::default();
        for media_file in media_files {
            let keys = containing_folder_keys(&media_file.file_path);
            if keys.is_empty() {
                index.escaped.push(media_file);
                continue;
            }
            for key in keys {
                add_owner(&mut index.by_containing_folder, key, &media_file.title_id);
            }
        }
        index
    }

    fn another_title_has_media_inside(&self, title_id: &str, folder: &str) -> bool {
        folder_containment_key(folder)
            .and_then(|key| self.by_containing_folder.get(&key))
            .is_some_and(|owners| owners.has_other_than(title_id))
            || self.escaped.iter().any(|media_file| {
                media_file.title_id != title_id
                    && stored_path_is_inside_folder(folder, &media_file.file_path)
            })
    }
}

/// The roots a title of `library_id` must sit inside. A library without roots
/// of its own falls back to every configured root: the folder only has to be
/// inside one of them.
fn library_roots_or_all<'a>(
    roots_by_library: &'a HashMap<&str, Vec<String>>,
    all_roots: &'a [String],
    library_id: &str,
) -> &'a [String] {
    match roots_by_library.get(library_id) {
        Some(roots) if !roots.is_empty() => roots.as_slice(),
        _ => all_roots,
    }
}

/// The id of the title's library root that contains `folder`, most specific
/// first.
fn most_specific_root_id(roots: &[RootRow], library_id: &str, folder: &str) -> Option<String> {
    roots
        .iter()
        .filter(|root| root.library_id == library_id)
        .filter(|root| path_is_strictly_within(folder, &root.path))
        .max_by_key(|root| root.path.trim().len())
        .map(|root| root.id.clone())
}

fn log_plan(plan: &RepairPlan) {
    for repair in &plan.repairs {
        info!(
            title_id = %repair.title_id,
            folder_path = %repair.folder_path,
            root_folder_id = ?repair.root_folder_id,
            "repaired a title folder that did not match where the title's files are"
        );
    }
    for left in &plan.left_alone {
        warn!(
            title_id = %left.title_id,
            folder_path = %left.folder_path,
            flagged_because = left.flagged_because,
            reason = left.reason,
            "left a suspect title folder unchanged; correct it from the title's folder settings"
        );
    }
    for (title_id, folder_path) in &plan.too_deep {
        warn!(
            title_id = %title_id,
            folder_path = %folder_path,
            "title folder holds only some of the title's files; left unchanged"
        );
    }
    info!(
        repaired = plan.repairs.len(),
        left_alone = plan.left_alone.len(),
        too_deep = plan.too_deep.len(),
        "title folder repair finished without touching any file"
    );
}

async fn sqlite_roots(tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>) -> AppResult<Vec<RootRow>> {
    let rows = sqlx::query("SELECT id, library_id, path FROM library_roots ORDER BY id")
        .fetch_all(&mut **tx)
        .await
        .map_err(repo_err)?;
    rows.into_iter().map(root_row_sqlite).collect()
}

async fn postgres_roots(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>) -> AppResult<Vec<RootRow>> {
    let rows = sqlx::query("SELECT id, library_id, path FROM library_roots ORDER BY id")
        .fetch_all(&mut **tx)
        .await
        .map_err(repo_err)?;
    rows.into_iter().map(root_row_postgres).collect()
}

fn root_row_sqlite(row: sqlx::sqlite::SqliteRow) -> AppResult<RootRow> {
    Ok(RootRow {
        id: row.try_get("id").map_err(repo_err)?,
        library_id: row.try_get("library_id").map_err(repo_err)?,
        path: row.try_get("path").map_err(repo_err)?,
    })
}

fn root_row_postgres(row: sqlx::postgres::PgRow) -> AppResult<RootRow> {
    Ok(RootRow {
        id: row.try_get("id").map_err(repo_err)?,
        library_id: row.try_get("library_id").map_err(repo_err)?,
        path: row.try_get("path").map_err(repo_err)?,
    })
}

const TITLES_QUERY: &str =
    "SELECT id, COALESCE(library_id, '') AS library_id, root_folder_id, folder_path
       FROM titles
      WHERE folder_path IS NOT NULL AND folder_path <> ''
      ORDER BY id";

async fn sqlite_titles(tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>) -> AppResult<Vec<TitleRow>> {
    let rows = sqlx::query(TITLES_QUERY)
        .fetch_all(&mut **tx)
        .await
        .map_err(repo_err)?;
    rows.into_iter()
        .map(|row| {
            Ok(TitleRow {
                id: row.try_get("id").map_err(repo_err)?,
                library_id: row.try_get("library_id").map_err(repo_err)?,
                root_folder_id: row.try_get("root_folder_id").map_err(repo_err)?,
                folder_path: row.try_get("folder_path").map_err(repo_err)?,
            })
        })
        .collect()
}

async fn postgres_titles(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> AppResult<Vec<TitleRow>> {
    let rows = sqlx::query(TITLES_QUERY)
        .fetch_all(&mut **tx)
        .await
        .map_err(repo_err)?;
    rows.into_iter()
        .map(|row| {
            Ok(TitleRow {
                id: row.try_get("id").map_err(repo_err)?,
                library_id: row.try_get("library_id").map_err(repo_err)?,
                root_folder_id: row.try_get("root_folder_id").map_err(repo_err)?,
                folder_path: row.try_get("folder_path").map_err(repo_err)?,
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
    AppError::Repository(format!("title folder repair migration failed: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(id: &str, library_id: &str, path: &str) -> RootRow {
        RootRow {
            id: id.to_string(),
            library_id: library_id.to_string(),
            path: path.to_string(),
        }
    }

    fn title(id: &str, folder_path: &str) -> TitleRow {
        TitleRow {
            id: id.to_string(),
            library_id: "series".to_string(),
            root_folder_id: Some("root-tv".to_string()),
            folder_path: Some(folder_path.to_string()),
        }
    }

    fn media(title_id: &str, path: &str) -> MediaFileRow {
        MediaFileRow {
            title_id: title_id.to_string(),
            file_path: path.to_string(),
        }
    }

    fn roots() -> Vec<RootRow> {
        vec![
            root("root-tv", "series", "/media/tv"),
            root("root-tv-2", "series", "/media/tv-2"),
            root("root-movies", "movies", "/media/movies"),
        ]
    }

    fn repaired(plan: &RepairPlan, title_id: &str) -> Option<String> {
        plan.repairs
            .iter()
            .find(|repair| repair.title_id == title_id)
            .map(|repair| repair.folder_path.clone())
    }

    fn left_reason(plan: &RepairPlan, title_id: &str) -> Option<&'static str> {
        plan.left_alone
            .iter()
            .find(|left| left.title_id == title_id)
            .map(|left| left.reason)
    }

    fn flattened(title_id: &str, folder: &str) -> Vec<MediaFileRow> {
        vec![
            media(title_id, &format!("{folder}/Show - S01E01.mkv")),
            media(title_id, &format!("{folder}/Show - S01E02.mkv")),
            media(title_id, &format!("{folder}/Show - S02E01.mkv")),
        ]
    }

    #[test]
    fn a_folder_recorded_as_the_root_is_repaired_to_the_implied_folder() {
        let titles = vec![title("t1", "/media/tv")];
        let plan = build_repair_plan(
            &roots(),
            &titles,
            &flattened("t1", "/media/tv/Synthetic Show"),
        );
        assert_eq!(
            repaired(&plan, "t1").as_deref(),
            Some("/media/tv/Synthetic Show")
        );
    }

    #[test]
    fn folders_above_the_root_are_repaired() {
        for corrupted in ["/media", "/"] {
            let titles = vec![title("t1", corrupted)];
            let plan = build_repair_plan(
                &roots(),
                &titles,
                &flattened("t1", "/media/tv/Synthetic Show"),
            );
            assert_eq!(
                repaired(&plan, "t1").as_deref(),
                Some("/media/tv/Synthetic Show"),
                "{corrupted}"
            );
        }
    }

    #[test]
    fn windows_style_records_are_repaired_in_their_own_spelling() {
        let roots = vec![root("root-d", "series", r"D:\Media\TV")];
        for corrupted in [r"D:\Media\TV", r"D:\Media", r"D:\"] {
            let mut row = title("t1", corrupted);
            row.root_folder_id = Some("root-d".to_string());
            let plan = build_repair_plan(
                &roots,
                &[row],
                &[
                    media("t1", r"D:\Media\TV\Synthetic Show\Show - S01E01.mkv"),
                    media("t1", r"D:\Media\TV\Synthetic Show\Show - S01E02.mkv"),
                ],
            );
            assert_eq!(
                repaired(&plan, "t1").as_deref(),
                Some(r"D:\Media\TV\Synthetic Show"),
                "{corrupted}"
            );
        }
    }

    #[test]
    fn a_folder_outside_the_library_is_repaired_and_its_root_healed() {
        let mut row = title("t1", "/media/movies");
        row.root_folder_id = Some("root-tv".to_string());
        let plan = build_repair_plan(
            &roots(),
            &[row],
            &flattened("t1", "/media/tv-2/Synthetic Show"),
        );
        assert_eq!(
            plan.repairs,
            vec![TitleFolderRepair {
                title_id: "t1".to_string(),
                folder_path: "/media/tv-2/Synthetic Show".to_string(),
                root_folder_id: Some("root-tv-2".to_string()),
            }]
        );
    }

    #[test]
    fn a_title_whose_root_id_is_already_right_keeps_it() {
        let plan = build_repair_plan(
            &roots(),
            &[title("t1", "/media/tv")],
            &flattened("t1", "/media/tv/Synthetic Show"),
        );
        assert_eq!(plan.repairs[0].root_folder_id, None);
    }

    #[test]
    fn mixed_flat_and_season_files_are_repaired() {
        let plan = build_repair_plan(
            &roots(),
            &[title("t1", "/media")],
            &[
                media("t1", "/media/tv/Synthetic Show/Season 01/Show - S01E01.mkv"),
                media("t1", "/media/tv/Synthetic Show/Show - S02E01.mkv"),
            ],
        );
        assert_eq!(
            repaired(&plan, "t1").as_deref(),
            Some("/media/tv/Synthetic Show")
        );
    }

    #[test]
    fn healthy_flat_and_season_folders_are_left_alone() {
        let titles = vec![
            title("flat", "/media/tv/Flat Show"),
            title("season", "/media/tv/Season Show"),
        ];
        let mut files = flattened("flat", "/media/tv/Flat Show");
        files.push(media(
            "season",
            "/media/tv/Season Show/Season 01/Show - S01E01.mkv",
        ));
        files.push(media(
            "season",
            "/media/tv/Season Show/Season 02/Show - S02E01.mkv",
        ));
        let plan = build_repair_plan(&roots(), &titles, &files);
        assert_eq!(plan, RepairPlan::default());
    }

    #[test]
    fn a_consistent_nested_layout_is_left_alone() {
        let titles = vec![
            title("nested-a", "/media/tv/Group/Show A"),
            title("nested-b", "/media/tv/Group/Show B"),
        ];
        let plan = build_repair_plan(
            &roots(),
            &titles,
            &[
                media("nested-a", "/media/tv/Group/Show A/Season 01/e1.mkv"),
                media("nested-b", "/media/tv/Group/Show B/e1.mkv"),
            ],
        );
        assert_eq!(plan, RepairPlan::default());
    }

    #[test]
    fn a_damaged_nested_layout_is_not_repaired_to_the_group_folder() {
        let plan = build_repair_plan(
            &roots(),
            &[title("nested", "/media/tv")],
            &[
                media("nested", "/media/tv/Group/Show A/e1.mkv"),
                media("nested", "/media/tv/Group/Show A/e2.mkv"),
            ],
        );
        assert!(plan.repairs.is_empty());
        assert_eq!(
            left_reason(&plan, "nested"),
            Some("every media file is in one subfolder; nested layouts are ambiguous")
        );
    }

    #[test]
    fn ambiguous_media_is_left_alone() {
        let plan = build_repair_plan(
            &roots(),
            &[title("t1", "/media/tv")],
            &[
                media("t1", "/media/tv/Show A/e1.mkv"),
                media("t1", "/media/tv/Show B/e2.mkv"),
            ],
        );
        assert!(plan.repairs.is_empty());
        assert_eq!(
            left_reason(&plan, "t1"),
            Some("media files do not imply one folder inside the library")
        );

        let plan = build_repair_plan(
            &roots(),
            &[title("t1", "/media/tv")],
            &[media("t1", "/media/tv/loose-file.mkv")],
        );
        assert!(plan.repairs.is_empty());
    }

    #[test]
    fn a_folder_owned_by_another_title_is_left_alone() {
        let titles = vec![
            title("damaged", "/media/tv"),
            title("owner", "/media/tv/Synthetic Show"),
        ];
        let mut files = flattened("damaged", "/media/tv/Synthetic Show");
        files.push(media("owner", "/media/tv/Synthetic Show/Other.mkv"));
        let plan = build_repair_plan(&roots(), &titles, &files);
        assert!(plan.repairs.is_empty());
        assert_eq!(
            left_reason(&plan, "damaged"),
            Some("another title records an overlapping folder")
        );

        // Another title's files in the folder are enough, recorded or not.
        let titles = vec![title("damaged", "/media/tv")];
        let mut files = flattened("damaged", "/media/tv/Synthetic Show");
        files.push(media("folderless", "/media/tv/Synthetic Show/Other.mkv"));
        let plan = build_repair_plan(&roots(), &titles, &files);
        assert_eq!(
            left_reason(&plan, "damaged"),
            Some("another title has media files in the implied folder")
        );
    }

    #[test]
    fn two_damaged_titles_never_claim_one_folder() {
        let titles = vec![title("a", "/media/tv"), title("b", "/media")];
        let mut files = flattened("a", "/media/tv/Synthetic Show");
        files.extend(flattened("b", "/media/tv/Synthetic Show"));
        let plan = build_repair_plan(&roots(), &titles, &files);
        // Each sees the other's files, so neither is repaired.
        assert!(plan.repairs.is_empty());
        assert_eq!(plan.left_alone.len(), 2);
    }

    #[test]
    fn a_damaged_title_without_media_is_left_alone() {
        let plan = build_repair_plan(&roots(), &[title("t1", "/media/tv")], &[]);
        assert!(plan.repairs.is_empty());
        assert_eq!(
            left_reason(&plan, "t1"),
            Some("media files do not imply one folder inside the library")
        );
    }

    #[test]
    fn a_folder_recorded_as_the_specials_folder_is_repaired_to_the_title_folder() {
        let plan = build_repair_plan(
            &roots(),
            &[title("t1", "/media/tv/Synthetic Show/Specials")],
            &[
                media("t1", "/media/tv/Synthetic Show/Season 1/e1.mkv"),
                media("t1", "/media/tv/Synthetic Show/Specials/s1.mkv"),
            ],
        );
        assert_eq!(
            plan.repairs,
            vec![TitleFolderRepair {
                title_id: "t1".to_string(),
                folder_path: "/media/tv/Synthetic Show".to_string(),
                root_folder_id: None,
            }]
        );
        assert!(plan.too_deep.is_empty());
    }

    #[test]
    fn a_windows_style_specials_record_is_repaired_in_its_own_spelling() {
        let roots = vec![root("root-d", "series", r"D:\Media\TV")];
        let mut row = title("t1", r"D:\Media\TV\Synthetic Show\Specials");
        row.root_folder_id = Some("root-d".to_string());
        let plan = build_repair_plan(
            &roots,
            &[row],
            &[
                media("t1", r"D:\Media\TV\Synthetic Show\Season 1\e1.mkv"),
                media("t1", r"D:\Media\TV\Synthetic Show\Specials\s1.mkv"),
            ],
        );
        assert_eq!(
            repaired(&plan, "t1").as_deref(),
            Some(r"D:\Media\TV\Synthetic Show")
        );
    }

    #[test]
    fn a_too_deep_record_in_a_nested_layout_moves_up_one_level_only() {
        let plan = build_repair_plan(
            &roots(),
            &[title("t1", "/media/tv/Group/Synthetic Show/Specials")],
            &[
                media("t1", "/media/tv/Group/Synthetic Show/Season 1/e1.mkv"),
                media("t1", "/media/tv/Group/Synthetic Show/Specials/s1.mkv"),
            ],
        );
        assert_eq!(
            repaired(&plan, "t1").as_deref(),
            Some("/media/tv/Group/Synthetic Show")
        );
    }

    #[test]
    fn a_too_deep_record_is_only_reported_when_the_containing_folder_is_not_free() {
        let deep = "/media/tv/Synthetic Show/Specials";
        let reported = vec![("t1".to_string(), deep.to_string())];

        // Files of the title outside the containing folder.
        let plan = build_repair_plan(
            &roots(),
            &[title("t1", "/media/tv/Group/Synthetic Show/Specials")],
            &[
                media("t1", "/media/tv/Group/Synthetic Show/Specials/s1.mkv"),
                media("t1", "/media/tv/Group/Elsewhere/e1.mkv"),
            ],
        );
        assert!(plan.repairs.is_empty());
        assert_eq!(plan.too_deep.len(), 1);

        // Another title has media in the containing folder.
        let plan = build_repair_plan(
            &roots(),
            &[title("t1", deep)],
            &[
                media("t1", "/media/tv/Synthetic Show/Season 1/e1.mkv"),
                media("t1", "/media/tv/Synthetic Show/Specials/s1.mkv"),
                media("other", "/media/tv/Synthetic Show/Season 1/other.mkv"),
            ],
        );
        assert!(plan.repairs.is_empty());
        assert_eq!(plan.too_deep, reported);

        // Another title records a folder inside the containing folder.
        let plan = build_repair_plan(
            &roots(),
            &[
                title("t1", deep),
                title("other", "/media/tv/Synthetic Show/Season 1"),
            ],
            &[
                media("t1", "/media/tv/Synthetic Show/Season 1/e1.mkv"),
                media("t1", "/media/tv/Synthetic Show/Specials/s1.mkv"),
            ],
        );
        assert!(plan.repairs.is_empty());
        assert_eq!(plan.too_deep, reported);
    }

    #[test]
    fn repairing_a_too_deep_record_twice_changes_nothing_the_second_time() {
        let files = [
            media("t1", "/media/tv/Synthetic Show/Season 1/e1.mkv"),
            media("t1", "/media/tv/Synthetic Show/Specials/s1.mkv"),
        ];
        let first = build_repair_plan(
            &roots(),
            &[title("t1", "/media/tv/Synthetic Show/Specials")],
            &files,
        );
        let repaired_folder = repaired(&first, "t1").expect("repaired");
        let second = build_repair_plan(&roots(), &[title("t1", &repaired_folder)], &files);
        assert_eq!(second, RepairPlan::default());
    }

    #[test]
    fn repairing_twice_changes_nothing_the_second_time() {
        let mut titles = vec![
            title("root", "/media/tv"),
            title("above", "/media"),
            title("healthy", "/media/tv/Healthy Show"),
        ];
        let mut files = flattened("root", "/media/tv/Root Show");
        files.extend(flattened("above", "/media/tv/Above Show"));
        files.extend(flattened("healthy", "/media/tv/Healthy Show"));
        let first = build_repair_plan(&roots(), &titles, &files);
        assert_eq!(first.repairs.len(), 2);
        for repair in &first.repairs {
            let row = titles
                .iter_mut()
                .find(|title| title.id == repair.title_id)
                .expect("repaired title");
            row.folder_path = Some(repair.folder_path.clone());
            if let Some(root_folder_id) = &repair.root_folder_id {
                row.root_folder_id = Some(root_folder_id.clone());
            }
        }
        let second = build_repair_plan(&roots(), &titles, &files);
        assert_eq!(second, RepairPlan::default());
    }
}

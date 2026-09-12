use std::path::Path;

use scryer_domain::Title;

use crate::library::workflow::{LibraryRootState, library_root_state};
use crate::stored_paths::{folder_paths_match, path_to_stored_string, stored_path_to_path_buf};
use crate::{AppError, AppResult, AppUseCase};

fn non_empty_folder_path(path: Option<&str>) -> Option<&str> {
    path.filter(|path| !path.is_empty())
}

pub(crate) fn title_owns_folder(title: &Title, folder_path: &Path) -> bool {
    let Some(owned_path) = non_empty_folder_path(title.folder_path.as_deref()) else {
        return false;
    };
    folder_paths_match(owned_path, &path_to_stored_string(folder_path))
}

pub(crate) fn title_owns_another_folder(title: &Title, folder_path: &Path) -> bool {
    non_empty_folder_path(title.folder_path.as_deref()).is_some()
        && !title_owns_folder(title, folder_path)
}

/// Whether the folder a title records as its own is still on disk.
///
/// `None` means the answer is unknown (an unreachable mount, a permission
/// problem). Callers treat that as "not proven" and take no action on it.
async fn owned_folder_present(owned_path: &str) -> Option<bool> {
    match tokio::fs::metadata(stored_path_to_path_buf(owned_path)).await {
        Ok(_) => Some(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Some(false),
        Err(_) => None,
    }
}

/// A title's recorded folder is stale when it no longer exists while the
/// directory that held it still has entries.
///
/// `folder_path` is written once when a title first claims a folder and is
/// never re-validated, so files relocated outside Scryer (a root split, a
/// drive migration, another tool's root-folder move) leave every affected
/// title pointing at a folder that is gone. The parent-directory check mirrors
/// the mount guard in [`library_root_state`]: a missing or empty parent is the
/// signature of a vanished mount, and nothing under it can be called gone.
pub(crate) async fn title_folder_is_stale(title: &Title) -> bool {
    let Some(owned_path) = non_empty_folder_path(title.folder_path.as_deref()) else {
        return false;
    };
    if owned_folder_present(owned_path).await != Some(false) {
        return false;
    }
    let owned_path = stored_path_to_path_buf(owned_path);
    let Some(parent) = owned_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    else {
        return false;
    };
    matches!(
        library_root_state(parent).await,
        Ok(LibraryRootState::Populated)
    )
}

/// Move a title's stale folder ownership to the folder a scan found its
/// files in. Returns `false`, changing nothing, when another title already
/// owns that folder — that is a real conflict, not a heal.
pub(crate) async fn reclaim_stale_title_folder(
    app: &AppUseCase,
    title: &mut Title,
    folder_path: &Path,
) -> AppResult<bool> {
    let previous_folder_path = title.folder_path.clone().unwrap_or_default();
    let folder_path = path_to_stored_string(folder_path);
    if find_other_folder_owner(app, title, &folder_path)
        .await?
        .is_some()
    {
        return Ok(false);
    }
    app.services
        .catalog
        .titles
        .set_folder_path(&title.id, &folder_path)
        .await?;
    title.folder_path = Some(folder_path.clone());
    tracing::info!(
        title_id = %title.id,
        previous_folder_path = %previous_folder_path,
        folder_path = %folder_path,
        "re-pointed stale title folder ownership to the scanned folder"
    );
    Ok(true)
}

async fn find_other_folder_owner(
    app: &AppUseCase,
    title: &Title,
    folder_path: &str,
) -> AppResult<Option<Title>> {
    let library_ids = vec![title.library_id.clone()];
    Ok(app
        .services
        .catalog
        .titles
        .list_for_libraries_without_external_ids(None, &library_ids, None)
        .await?
        .into_iter()
        .find(|candidate| {
            candidate.id != title.id
                && non_empty_folder_path(candidate.folder_path.as_deref())
                    .is_some_and(|owned_path| folder_paths_match(owned_path, folder_path))
        }))
}

pub(crate) async fn ensure_folder_move_available_to_title(
    app: &AppUseCase,
    title: &Title,
    folder_path: &Path,
) -> AppResult<()> {
    if folder_path.as_os_str().is_empty() {
        return Err(AppError::Validation("title folder path is required".into()));
    }
    let folder_path = path_to_stored_string(folder_path);
    if let Some(owner) = find_other_folder_owner(app, title, &folder_path).await? {
        return Err(AppError::Validation(format!(
            "folder {} is already owned by title {}",
            owner.folder_path.as_deref().unwrap_or_default(),
            owner.name
        )));
    }
    Ok(())
}

pub(crate) async fn ensure_folder_available_to_title(
    app: &AppUseCase,
    title: &Title,
    folder_path: &Path,
) -> AppResult<()> {
    if folder_path.as_os_str().is_empty() {
        return Err(AppError::Validation("title folder path is required".into()));
    }
    let folder_path = path_to_stored_string(folder_path);

    if non_empty_folder_path(title.folder_path.as_deref())
        .is_some_and(|owned_path| folder_paths_match(owned_path, &folder_path))
    {
        return Ok(());
    }

    if let Some(owner) = find_other_folder_owner(app, title, &folder_path).await? {
        let owned_path = owner.folder_path.as_deref().unwrap_or_default();
        return Err(AppError::Validation(format!(
            "folder {owned_path} is already owned by title {}",
            owner.name
        )));
    }

    if let Some(owned_path) = non_empty_folder_path(title.folder_path.as_deref()) {
        return Err(AppError::Validation(format!(
            "title {} already owns another folder: {owned_path}",
            title.name
        )));
    }

    Ok(())
}

pub(crate) async fn claim_title_folder_if_missing(
    app: &AppUseCase,
    title: &mut Title,
    folder_path: &Path,
) -> AppResult<()> {
    ensure_folder_available_to_title(app, title, folder_path).await?;
    if non_empty_folder_path(title.folder_path.as_deref()).is_some() {
        return Ok(());
    }

    let folder_path = path_to_stored_string(folder_path);
    app.services
        .catalog
        .titles
        .set_folder_path(&title.id, &folder_path)
        .await?;
    title.folder_path = Some(folder_path);
    Ok(())
}

pub(crate) async fn unlink_title_media_in_folder(
    app: &AppUseCase,
    title: &Title,
    folder_path: &Path,
) -> AppResult<u32> {
    if !title_owns_another_folder(title, folder_path) {
        return Ok(0);
    }
    // Only a title whose owned folder is confirmed on disk holds a real
    // second copy. An absent or unreachable owned folder is stale ownership
    // or an unmounted root; unlinking there turns a monitored title fileless
    // and exposes it to automatic re-download.
    let owned_path = title.folder_path.as_deref().unwrap_or_default();
    if owned_folder_present(owned_path).await != Some(true) {
        return Ok(0);
    }

    let folder_path = path_to_stored_string(folder_path);
    let media_file_ids = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await?
        .into_iter()
        .filter(|media_file| {
            crate::stored_paths::stored_path_is_within_folder(&folder_path, &media_file.file_path)
        })
        .map(|media_file| media_file.id)
        .collect::<Vec<_>>();
    for media_file_id in &media_file_ids {
        app.services
            .library
            .media_files
            .delete_media_file(media_file_id)
            .await?;
    }
    let deleted = media_file_ids.len() as u32;
    tracing::info!(
        title_id = %title.id,
        folder_path = %folder_path,
        unlinked_media_files = deleted,
        "unlinked catalog media outside the title-owned folder"
    );
    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use scryer_domain::MediaFacet;

    fn title(folder_path: Option<&str>) -> Title {
        let facet = MediaFacet::Anime;
        Title {
            id: "title-1".to_string(),
            library_id: scryer_domain::default_library_id_for_facet(&facet),
            name: "Case Split Fixture".to_string(),
            facet,
            monitored: true,
            tags: vec![],
            external_ids: vec![],
            root_folder_id: scryer_domain::root_folder_id_for_path("/data/Anime"),
            created_by: None,
            created_at: chrono::Utc::now(),
            year: None,
            overview: None,
            poster_url: None,
            poster_source_url: None,
            background_url: None,
            background_source_url: None,
            sort_title: None,
            catalog_sort_key: String::new(),
            slug: None,
            imdb_id: None,
            runtime_minutes: None,
            popularity: None,
            canonical_tags: vec![],
            content_status: None,
            language: None,
            first_aired: None,
            network: None,
            studio: None,
            country: None,
            aliases: vec![],
            tagged_aliases: vec![],
            metadata_language: None,
            metadata_fetched_at: None,
            min_availability: None,
            digital_release_date: None,
            folder_path: folder_path.map(str::to_string),
        }
    }

    #[test]
    fn native_title_folder_ownership_uses_platform_case_rules() {
        let title = title(Some("/data/Anime/CASE SPLIT FIXTURE"));
        let candidate = Path::new("/data/Anime/Case Split Fixture");

        if cfg!(windows) {
            assert!(title_owns_folder(&title, candidate));
        } else {
            assert!(title_owns_another_folder(&title, candidate));
        }
    }
}

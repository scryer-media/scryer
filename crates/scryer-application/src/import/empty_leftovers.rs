//! Removal of the empty files a failed copy import left at library names.
//!
//! An import that failed while it took its destination name could leave a
//! file of no bytes there. Such a file holds nothing, and it stops the next
//! import of the same name. The caller names the paths, from its own record
//! of failed imports; nothing here walks a library to find them.

use crate::disk::{Disk, EntryKind, Mark, Remove};
use crate::stored_paths::{path_to_stored_string, stored_path_to_path_buf};
use crate::{AppResult, AppUseCase};
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

/// What became of one named path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmptyImportLeftover {
    /// Not an absolute, plain path under a configured library root. Never
    /// read from disk.
    OutsideLibraryRoots,
    /// Nothing there, not a regular file, or a file with content.
    NotAnEmptyFile,
    /// The catalog records content for this path, so the empty file is not
    /// what the catalog knows. Left as it is.
    CatalogedWithContent,
    /// It stopped being the empty file that was read before its removal.
    Changed,
    /// The empty file is gone, and with it the catalog row that recorded an
    /// empty file at its path, when there was one.
    Removed { catalog_row: bool },
    /// The file could not be removed, or it was removed and its catalog row
    /// could not be. Logged with the cause.
    Failed,
}

/// The configured library roots, read once for a whole pass.
pub struct EmptyImportLeftoverRoots(Vec<PathBuf>);

impl AppUseCase {
    pub async fn empty_import_leftover_roots(&self) -> AppResult<EmptyImportLeftoverRoots> {
        Ok(EmptyImportLeftoverRoots(
            self.all_library_root_folders()
                .await?
                .into_iter()
                .map(|root| root.path.trim().to_string())
                .filter(|path| !path.is_empty())
                .map(|path| stored_path_to_path_buf(&path))
                .collect(),
        ))
    }

    /// Removes each of `paths` that is an empty regular file under a library
    /// root, one outcome per path in order.
    ///
    /// A link, a folder, a file with content and a path outside the roots are
    /// never touched. A file the catalog points at goes only when the catalog
    /// too records it as empty, and its row goes with it the way a scan drops
    /// the row of a file that vanished.
    ///
    /// Probes: one stat per path under a root, and a second only for a path
    /// that read as an empty file. One catalog read for the batch.
    pub async fn remove_empty_import_leftovers(
        &self,
        roots: &EmptyImportLeftoverRoots,
        paths: &[PathBuf],
    ) -> AppResult<Vec<EmptyImportLeftover>> {
        let mut outcomes = vec![EmptyImportLeftover::OutsideLibraryRoots; paths.len()];
        let mut empty: Vec<(usize, Mark)> = Vec::new();
        for (index, path) in paths.iter().enumerate() {
            if !is_plain_path_under_a_root(path, &roots.0) {
                continue;
            }
            outcomes[index] = EmptyImportLeftover::NotAnEmptyFile;
            if let Ok(mark) = Disk::process().fingerprint(path.clone()).await
                && is_empty_file(&mark)
            {
                empty.push((index, mark));
            }
        }
        if empty.is_empty() {
            return Ok(outcomes);
        }

        let stored: Vec<String> = empty
            .iter()
            .map(|(index, _)| path_to_stored_string(&paths[*index]))
            .collect();
        let cataloged: HashMap<String, crate::types::TitleMediaFile> = self
            .services
            .library
            .media_files
            .list_media_files_by_paths(&stored)
            .await?
            .into_iter()
            .collect();

        for ((index, mark), stored) in empty.into_iter().zip(stored) {
            let path = &paths[index];
            let row = cataloged.get(&stored);
            if row.is_some_and(|row| row.size_bytes != 0) {
                outcomes[index] = EmptyImportLeftover::CatalogedWithContent;
                tracing::info!(
                    path = %path.display(),
                    "an empty file at a failed import's destination is kept: the catalog records content for it"
                );
                continue;
            }
            let still_empty = Disk::process()
                .fingerprint(path.clone())
                .await
                .is_ok_and(|now| is_empty_file(&now) && mark.still_marks(&now));
            if !still_empty {
                outcomes[index] = EmptyImportLeftover::Changed;
                continue;
            }
            if let Err(error) = Disk::process().remove(path.clone(), Remove::File).await {
                outcomes[index] = EmptyImportLeftover::Failed;
                tracing::warn!(
                    path = %path.display(),
                    error = %error.words(),
                    "the empty file a failed import left could not be removed"
                );
                continue;
            }
            outcomes[index] = EmptyImportLeftover::Removed {
                catalog_row: row.is_some(),
            };
            if let Some(row) = row
                && let Err(error) = self.delete_media_file_record_with_dependents(&row.id).await
            {
                outcomes[index] = EmptyImportLeftover::Failed;
                tracing::warn!(
                    path = %path.display(),
                    media_file_id = %row.id,
                    %error,
                    "the empty file a failed import left was removed, but its catalog row was not; a scan of the title drops it"
                );
                continue;
            }
            tracing::info!(
                path = %path.display(),
                catalog_row = row.is_some(),
                "removed the empty file a failed import left at its destination"
            );
        }
        Ok(outcomes)
    }
}

fn is_empty_file(mark: &Mark) -> bool {
    mark.kind() == Some(EntryKind::File) && mark.is_len(0)
}

/// An absolute path of plain names only, under one of `roots` and not a root
/// itself. No probe.
fn is_plain_path_under_a_root(path: &Path, roots: &[PathBuf]) -> bool {
    path.is_absolute()
        && path
            .components()
            .all(|part| !matches!(part, Component::ParentDir | Component::CurDir))
        && roots.iter().any(|root| {
            root != path && crate::recycle_bin::path_is_under_configured_root(path, root)
        })
}

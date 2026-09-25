#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BurnedDataCleanupOutcome {
    /// The whole job directory was removed.
    DeletedDirectory(PathBuf),
    /// Only the rejected source file(s) were removed (shared/category root kept).
    DeletedFiles(Vec<PathBuf>),
    /// Nothing was deleted; the static reason says which guard refused.
    Skipped(&'static str),
    /// An I/O error while deleting (message).
    Failed(String),
}

/// Applies the filesystem policy for burned Usenet download data using explicit protected roots.
pub(crate) async fn delete_burned_download_data_with_roots(
    job_dir: &Path,
    rejected_sources: &[PathBuf],
    protected_roots: &[PathBuf],
    container_roots: &[PathBuf],
) -> BurnedDataCleanupOutcome {
    if job_dir.as_os_str().is_empty() {
        return skipped_burned_data_cleanup(job_dir, "job_dir_empty");
    }
    if !job_dir.is_absolute() {
        return skipped_burned_data_cleanup(job_dir, "job_dir_not_absolute");
    }
    if job_dir.parent().is_none() {
        return skipped_burned_data_cleanup(job_dir, "job_dir_is_filesystem_root");
    }

    let metadata = match tokio::fs::symlink_metadata(job_dir).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return skipped_burned_data_cleanup(job_dir, "job_dir_missing");
        }
        Err(error) => return failed_burned_data_cleanup(job_dir, error),
    };

    if metadata.file_type().is_symlink() {
        return skipped_burned_data_cleanup(job_dir, "job_dir_is_symlink");
    }

    if metadata.is_file() {
        if !rejected_sources.iter().any(|source| source == job_dir) {
            return skipped_burned_data_cleanup(job_dir, "job_file_not_rejected_source");
        }
        if job_dir_overlaps_protected_root(job_dir, protected_roots)
            || container_root_is_within_job_dir(job_dir, container_roots)
        {
            return skipped_burned_data_cleanup(job_dir, "job_dir_overlaps_protected_root");
        }

        return match tokio::fs::remove_file(job_dir).await {
            Ok(()) => deleted_burned_data_files(job_dir, vec![job_dir.to_path_buf()]),
            Err(error) => failed_burned_data_cleanup(job_dir, error),
        };
    }

    if !metadata.is_dir() {
        return skipped_burned_data_cleanup(job_dir, "job_dir_not_directory");
    }

    let rejected_sources_in_job_dir = rejected_sources
        .iter()
        .filter(|source| path_is_under_root(source, job_dir))
        .collect::<Vec<_>>();
    if rejected_sources_in_job_dir.is_empty() {
        return skipped_burned_data_cleanup(job_dir, "rejected_source_outside_job_dir");
    }

    if job_dir_overlaps_protected_root(job_dir, protected_roots)
        || container_root_is_within_job_dir(job_dir, container_roots)
    {
        return skipped_burned_data_cleanup(job_dir, "job_dir_overlaps_protected_root");
    }

    if rejected_sources_in_job_dir
        .iter()
        .any(|source| source.parent() == Some(job_dir))
    {
        return match delete_existing_rejected_files(job_dir, &rejected_sources_in_job_dir).await {
            Ok(deleted_files) if deleted_files.is_empty() => {
                skipped_burned_data_cleanup(job_dir, "rejected_source_missing")
            }
            Ok(deleted_files) => deleted_burned_data_files(job_dir, deleted_files),
            Err(error) => failed_burned_data_cleanup(job_dir, error),
        };
    }

    match crate::fs_safety::remove_dir_all_safely_if_exists(job_dir).await {
        Ok(()) => deleted_burned_data_directory(job_dir),
        Err(error) => failed_burned_data_cleanup(job_dir, error),
    }
}

/// Collects configured roots that must not be deleted, then applies burned-data cleanup.
pub(crate) async fn delete_burned_download_data(
    app: &AppUseCase,
    completed_local: &scryer_domain::CompletedDownload,
    rejected_sources: &[PathBuf],
) -> BurnedDataCleanupOutcome {
    let job_dir = stored_path_to_path_buf(&completed_local.dest_dir);
    let libraries = match app.services.catalog.libraries.list(None).await {
        Ok(libraries) => libraries,
        Err(error) => return failed_burned_data_cleanup(&job_dir, error),
    };
    let library_roots =
        crate::catalog_workflow::library_root_folders_from_libraries(&libraries, None)
            .into_iter()
            .map(|root| stored_path_to_path_buf(&root.path))
            .collect::<Vec<_>>();
    let recycle_bin_configs = app
        .recycle_bin_configs_for_media_roots(
            library_roots
                .iter()
                .map(|root| root.to_string_lossy().into_owned()),
        )
        .await;

    let download_client_configs = match app
        .services
        .integrations
        .download_client_configs
        .list(None)
        .await
    {
        Ok(configs) => configs,
        Err(error) => return failed_burned_data_cleanup(&job_dir, error),
    };

    let mut protected_roots = library_roots;
    protected_roots.extend(
        recycle_bin_configs
            .into_iter()
            .map(|(_, config)| config.base_path),
    );
    let mut container_roots = Vec::new();
    for config in download_client_configs {
        let mappings = match crate::parse_download_client_remote_path_mappings(&config.config_json)
        {
            Ok(mappings) => mappings,
            Err(error) => return failed_burned_data_cleanup(&job_dir, error),
        };
        container_roots.extend(
            mappings
                .into_iter()
                .map(|mapping| PathBuf::from(mapping.local_root())),
        );
    }

    delete_burned_download_data_with_roots(
        &job_dir,
        rejected_sources,
        &protected_roots,
        &container_roots,
    )
    .await
}

fn path_is_under_root(path: &Path, root: &Path) -> bool {
    crate::library::recycle_bin::path_is_under_configured_root(path, root)
}

fn job_dir_overlaps_protected_root(job_dir: &Path, protected_roots: &[PathBuf]) -> bool {
    protected_roots
        .iter()
        .filter(|root| !root.as_os_str().is_empty())
        .any(|root| path_is_under_root(job_dir, root) || path_is_under_root(root, job_dir))
}

fn container_root_is_within_job_dir(job_dir: &Path, container_roots: &[PathBuf]) -> bool {
    container_roots
        .iter()
        .filter(|root| !root.as_os_str().is_empty())
        .any(|root| path_is_under_root(root, job_dir))
}

async fn delete_existing_rejected_files(
    job_dir: &Path,
    rejected_sources: &[&PathBuf],
) -> Result<Vec<PathBuf>, String> {
    let mut deleted_files = Vec::new();
    for source in rejected_sources {
        let metadata = match tokio::fs::symlink_metadata(source).await {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(format!(
                    "failed to inspect rejected source {} in {}: {error}",
                    source.display(),
                    job_dir.display()
                ));
            }
        };
        if !metadata.is_file() && !metadata.file_type().is_symlink() {
            return Err(format!(
                "refusing to delete non-file rejected source {} in {}",
                source.display(),
                job_dir.display()
            ));
        }
        tokio::fs::remove_file(source).await.map_err(|error| {
            format!(
                "failed to delete rejected source {} in {}: {error}",
                source.display(),
                job_dir.display()
            )
        })?;
        deleted_files.push((*source).clone());
    }
    Ok(deleted_files)
}

fn deleted_burned_data_directory(job_dir: &Path) -> BurnedDataCleanupOutcome {
    tracing::info!(
        path = %job_dir.display(),
        outcome = "deleted_directory",
        "import: deleted burned download data"
    );
    BurnedDataCleanupOutcome::DeletedDirectory(job_dir.to_path_buf())
}

fn deleted_burned_data_files(
    job_dir: &Path,
    deleted_files: Vec<PathBuf>,
) -> BurnedDataCleanupOutcome {
    tracing::info!(
        path = %job_dir.display(),
        outcome = "deleted_files",
        deleted_files = ?deleted_files,
        "import: deleted burned download data"
    );
    BurnedDataCleanupOutcome::DeletedFiles(deleted_files)
}

fn skipped_burned_data_cleanup(job_dir: &Path, reason: &'static str) -> BurnedDataCleanupOutcome {
    tracing::warn!(
        path = %job_dir.display(),
        reason,
        "import: skipped burned download data cleanup"
    );
    BurnedDataCleanupOutcome::Skipped(reason)
}

fn failed_burned_data_cleanup(
    job_dir: &Path,
    error: impl std::fmt::Display,
) -> BurnedDataCleanupOutcome {
    let message = error.to_string();
    tracing::warn!(
        path = %job_dir.display(),
        error = %message,
        "import: burned download data cleanup failed"
    );
    BurnedDataCleanupOutcome::Failed(message)
}

#[cfg(test)]
mod burned_source_tests {
    use super::*;

    #[tokio::test]
    async fn deletes_dedicated_job_directory_and_sidecars() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let job_dir = temp_dir.path().join("completed/job");
        std::fs::create_dir_all(&job_dir).expect("create job directory");
        let video = job_dir.join("release/episode.mkv");
        std::fs::create_dir_all(video.parent().expect("video parent"))
            .expect("create video parent");
        std::fs::write(&video, "video").expect("write video");
        std::fs::write(job_dir.join("release/episode.nfo"), "sidecar").expect("write sidecar");

        let outcome = delete_burned_download_data_with_roots(&job_dir, &[video], &[], &[]).await;

        assert_eq!(
            outcome,
            BurnedDataCleanupOutcome::DeletedDirectory(job_dir.clone())
        );
        assert!(!job_dir.exists());
    }

    #[tokio::test]
    async fn deletes_only_rejected_files_from_shared_root() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let shared_root = temp_dir.path().join("complete");
        std::fs::create_dir_all(&shared_root).expect("create shared root");
        let video = shared_root.join("episode.mkv");
        let sibling = shared_root.join("keep.txt");
        std::fs::write(&video, "video").expect("write video");
        std::fs::write(&sibling, "keep").expect("write sibling");

        let outcome = delete_burned_download_data_with_roots(
            &shared_root,
            std::slice::from_ref(&video),
            &[],
            &[],
        )
        .await;

        assert_eq!(
            outcome,
            BurnedDataCleanupOutcome::DeletedFiles(vec![video.clone()])
        );
        assert!(!video.exists());
        assert!(shared_root.exists());
        assert!(sibling.exists());
    }

    #[tokio::test]
    async fn skips_when_job_directory_is_a_protected_root() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let job_dir = temp_dir.path().join("job");
        let video = job_dir.join("nested/episode.mkv");
        std::fs::create_dir_all(video.parent().expect("video parent"))
            .expect("create job directory");
        std::fs::write(&video, "video").expect("write video");

        let outcome = delete_burned_download_data_with_roots(
            &job_dir,
            &[video],
            std::slice::from_ref(&job_dir),
            &[],
        )
        .await;

        assert_eq!(
            outcome,
            BurnedDataCleanupOutcome::Skipped("job_dir_overlaps_protected_root")
        );
        assert!(job_dir.exists());
    }

    #[tokio::test]
    async fn skips_job_directory_inside_library_root() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let library_root = temp_dir.path().join("library");
        let job_dir = library_root.join("shows/example");
        let video = job_dir.join("nested/episode.mkv");
        std::fs::create_dir_all(video.parent().expect("video parent"))
            .expect("create job directory");
        std::fs::write(&video, "video").expect("write video");

        let outcome = delete_burned_download_data_with_roots(
            &job_dir,
            std::slice::from_ref(&video),
            std::slice::from_ref(&library_root),
            &[],
        )
        .await;

        assert_eq!(
            outcome,
            BurnedDataCleanupOutcome::Skipped("job_dir_overlaps_protected_root")
        );
        assert!(job_dir.exists());
        assert!(video.exists());
    }

    #[tokio::test]
    async fn skips_single_file_job_inside_protected_root() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let protected_root = temp_dir.path().join("library");
        let video = protected_root.join("show/episode.mkv");
        std::fs::create_dir_all(video.parent().expect("video parent"))
            .expect("create video parent");
        std::fs::write(&video, "video").expect("write video");

        let outcome = delete_burned_download_data_with_roots(
            &video,
            std::slice::from_ref(&video),
            std::slice::from_ref(&protected_root),
            &[],
        )
        .await;

        assert_eq!(
            outcome,
            BurnedDataCleanupOutcome::Skipped("job_dir_overlaps_protected_root")
        );
        assert!(video.exists());
    }

    #[tokio::test]
    async fn skips_when_a_protected_root_is_within_the_job_directory() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let job_dir = temp_dir.path().join("job");
        let protected_root = job_dir.join("protected");
        let video = job_dir.join("nested/episode.mkv");
        std::fs::create_dir_all(&protected_root).expect("create protected root");
        std::fs::create_dir_all(video.parent().expect("video parent"))
            .expect("create video parent");
        std::fs::write(&video, "video").expect("write video");

        let outcome = delete_burned_download_data_with_roots(
            &job_dir,
            &[video],
            std::slice::from_ref(&protected_root),
            &[],
        )
        .await;

        assert_eq!(
            outcome,
            BurnedDataCleanupOutcome::Skipped("job_dir_overlaps_protected_root")
        );
        assert!(job_dir.exists());
    }

    #[tokio::test]
    async fn deletes_job_directory_below_mapping_local_root() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let mapping_local_root = temp_dir.path().join("downloads");
        let job_dir = mapping_local_root.join("complete/series/show");
        let video = job_dir.join("nested/episode.mkv");
        std::fs::create_dir_all(video.parent().expect("video parent"))
            .expect("create job directory");
        std::fs::write(&video, "video").expect("write video");

        let outcome = delete_burned_download_data_with_roots(
            &job_dir,
            &[video],
            &[],
            std::slice::from_ref(&mapping_local_root),
        )
        .await;

        assert_eq!(
            outcome,
            BurnedDataCleanupOutcome::DeletedDirectory(job_dir.clone())
        );
        assert!(!job_dir.exists());
        assert!(mapping_local_root.exists());
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn skips_symlink_job_directory_without_touching_target() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let target = temp_dir.path().join("target");
        let job_dir = temp_dir.path().join("job-link");
        let video = target.join("episode.mkv");
        std::fs::create_dir_all(&target).expect("create target");
        std::fs::write(&video, "video").expect("write video");
        std::os::unix::fs::symlink(&target, &job_dir).expect("create symlink");

        let outcome = delete_burned_download_data_with_roots(
            &job_dir,
            std::slice::from_ref(&video),
            &[],
            &[],
        )
        .await;

        assert_eq!(
            outcome,
            BurnedDataCleanupOutcome::Skipped("job_dir_is_symlink")
        );
        assert!(target.exists());
        assert!(video.exists());
    }

    #[tokio::test]
    async fn skips_when_rejected_source_is_outside_job_directory() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let job_dir = temp_dir.path().join("job");
        let outside_video = temp_dir.path().join("outside/episode.mkv");
        std::fs::create_dir_all(&job_dir).expect("create job directory");
        std::fs::create_dir_all(outside_video.parent().expect("outside parent"))
            .expect("create outside parent");
        std::fs::write(&outside_video, "video").expect("write outside video");

        let outcome = delete_burned_download_data_with_roots(
            &job_dir,
            std::slice::from_ref(&outside_video),
            &[],
            &[],
        )
        .await;

        assert_eq!(
            outcome,
            BurnedDataCleanupOutcome::Skipped("rejected_source_outside_job_dir")
        );
        assert!(job_dir.exists());
        assert!(outside_video.exists());
    }

    #[tokio::test]
    async fn skips_missing_job_directory() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let job_dir = temp_dir.path().join("missing");
        let video = job_dir.join("episode.mkv");

        let outcome = delete_burned_download_data_with_roots(&job_dir, &[video], &[], &[]).await;

        assert_eq!(
            outcome,
            BurnedDataCleanupOutcome::Skipped("job_dir_missing")
        );
    }

    #[tokio::test]
    async fn deletes_rejected_single_file_job() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let video = temp_dir.path().join("episode.mkv");
        std::fs::write(&video, "video").expect("write video");

        let outcome =
            delete_burned_download_data_with_roots(&video, std::slice::from_ref(&video), &[], &[])
                .await;

        assert_eq!(
            outcome,
            BurnedDataCleanupOutcome::DeletedFiles(vec![video.clone()])
        );
        assert!(!video.exists());
    }
}

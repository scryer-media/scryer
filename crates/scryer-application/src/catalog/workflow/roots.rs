/// How long one root-folder stat may take before its usage reports as
/// unavailable. Generous for a healthy mount; a dead network mount never
/// answers at all.
const STORAGE_ROOT_STAT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LibraryRootFolder {
    pub library_id: String,
    pub library_name: String,
    pub facet: MediaFacet,
    pub path: String,
    pub normalized_path: String,
}
pub(crate) fn normalize_library_root_path(path: &str) -> String {
    scryer_domain::normalize_library_root_path(path)
}
pub(crate) fn library_path_is_under_root(path: &str, root: &str) -> bool {
    let normalized_path = normalize_library_root_path(path);
    let normalized_root = normalize_library_root_path(root);
    if normalized_path.is_empty() || normalized_root.is_empty() {
        return false;
    }

    #[cfg(windows)]
    let separator = "\\";
    #[cfg(not(windows))]
    let separator = "/";

    let descendant_prefix = if normalized_root.ends_with(separator) {
        normalized_root.clone()
    } else {
        format!("{normalized_root}{separator}")
    };

    normalized_path == normalized_root || normalized_path.starts_with(&descendant_prefix)
}
pub(crate) fn library_root_paths_overlap(left: &str, right: &str) -> bool {
    library_path_is_under_root(left, right) || library_path_is_under_root(right, left)
}
pub(crate) fn library_root_folders_from_libraries(
    libraries: &[Library],
    facet: Option<&MediaFacet>,
) -> Vec<LibraryRootFolder> {
    let mut roots = Vec::new();
    for library in libraries {
        if facet.is_some_and(|facet| library.facet != *facet) {
            continue;
        }

        for root in &library.roots {
            let path = root.path.trim();
            let normalized_path = normalize_library_root_path(path);
            if normalized_path.is_empty() {
                continue;
            }
            roots.push(LibraryRootFolder {
                library_id: library.id.clone(),
                library_name: library.name.clone(),
                facet: library.facet.clone(),
                path: path.to_string(),
                normalized_path,
            });
        }
    }
    roots
}
pub(crate) fn submission_scopes_overlap(
    title_id: &str,
    existing: &SubmissionScope,
    requested: &SubmissionScope,
    episodes: &[Episode],
) -> bool {
    let existing_submission = submission_for_scope(title_id, existing);
    if wanted_item_candidates_for_submission_scope(title_id, requested, episodes)
        .iter()
        .any(|(item, collection_id)| {
            submission_blocks_wanted_item(&existing_submission, item, collection_id.as_deref())
        })
    {
        return true;
    }

    let requested_submission = submission_for_scope(title_id, requested);
    wanted_item_candidates_for_submission_scope(title_id, existing, episodes)
        .iter()
        .any(|(item, collection_id)| {
            submission_blocks_wanted_item(&requested_submission, item, collection_id.as_deref())
        })
}
impl AppUseCase {
    /// Return the configured root folders for a facet.
    ///
    /// Reads canonical roots from the facet's default library. Legacy
    /// `<facet>.root_folders` and `<facet>.path` settings are maintained only
    /// as compatibility mirrors and are reconciled during startup.
    pub async fn root_folders_for_facet(
        &self,
        facet: &scryer_domain::MediaFacet,
    ) -> AppResult<Vec<scryer_domain::RootFolderEntry>> {
        let handler = self.facet_registry.get(facet);
        let default_path = handler.map(|h| h.default_library_path()).unwrap_or("/data");

        if let Some(library) = self
            .services
            .catalog
            .libraries
            .default_for_facet(facet.clone())
            .await?
        {
            let entries = root_folder_entries_from_library_roots(&library.roots);

            if !entries.is_empty() {
                return Ok(entries);
            }
        }

        Ok(vec![scryer_domain::RootFolderEntry {
            path: default_path.to_string(),
            is_default: true,
        }])
    }
}
impl AppUseCase {
    pub(crate) async fn resolve_title_root_folder_id_for_library(
        &self,
        library_id: &str,
        root_folder_id: Option<&str>,
    ) -> AppResult<String> {
        let library = self
            .services
            .catalog
            .libraries
            .get_by_id(library_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("library {library_id}")))?;
        let root = match root_folder_id
            .map(str::trim)
            .filter(|root_folder_id| !root_folder_id.is_empty())
        {
            Some(root_folder_id) => library
                .roots
                .iter()
                .find(|root| root.id == root_folder_id)
                .ok_or_else(|| {
                    AppError::Validation(
                        "rootFolderId must reference a root on the title library".to_string(),
                    )
                })?,
            None => library
                .roots
                .iter()
                .find(|root| root.is_default)
                .or_else(|| library.roots.first())
                .ok_or_else(|| {
                    AppError::Validation(
                        "title library must have at least one root folder".to_string(),
                    )
                })?,
        };
        Ok(root.id.clone())
    }

    pub(crate) async fn title_root_folder_path_override(
        &self,
        title: &scryer_domain::Title,
    ) -> AppResult<String> {
        self.title_root_folder_path_for_parts(
            &title.root_folder_id,
            &title.library_id,
            &title.facet,
        )
        .await
    }

    pub async fn title_root_folder_path_for_parts(
        &self,
        root_folder_id: &str,
        library_id: &str,
        facet: &MediaFacet,
    ) -> AppResult<String> {
        let root_folder_id = root_folder_id.trim();
        if root_folder_id.is_empty() {
            return Err(AppError::Repository(
                "title root folder id cannot be empty".to_string(),
            ));
        }
        let library = self
            .services
            .catalog
            .libraries
            .get_by_id(library_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("library {library_id}")))?;
        if library.facet != *facet {
            return Err(AppError::Repository(format!(
                "title root folder library {library_id} does not match title facet {}",
                facet.as_str()
            )));
        }
        library
            .roots
            .into_iter()
            .find(|root| root.id == root_folder_id)
            .map(|root| root.path)
            .ok_or_else(|| {
                AppError::Repository(format!(
                    "title root folder id {root_folder_id} is not configured on library {library_id}"
                ))
            })
    }
}
impl AppUseCase {
    /// Return every configured library root for a facet across all libraries.
    pub(crate) async fn all_library_root_folders_for_facet(
        &self,
        facet: &scryer_domain::MediaFacet,
    ) -> AppResult<Vec<LibraryRootFolder>> {
        let libraries = self.services.catalog.libraries.list(None).await?;
        Ok(library_root_folders_from_libraries(&libraries, Some(facet)))
    }
}
impl AppUseCase {
    /// Return every configured library root across all facets and libraries.
    pub(crate) async fn all_library_root_folders(&self) -> AppResult<Vec<LibraryRootFolder>> {
        let libraries = self.services.catalog.libraries.list(None).await?;
        Ok(library_root_folders_from_libraries(&libraries, None))
    }
}
impl AppUseCase {
    /// Return one usage row per (library, root) pair the caller may view.
    ///
    /// Visibility is resolved with the same
    /// [`AppUseCase::list_libraries_for_permission`] call the `libraries` query
    /// uses, so roots of libraries the caller cannot view are never returned.
    /// Rows are deduplicated by normalized path within each library, and each
    /// unique filesystem path is stat'ed at most once per call even when several
    /// libraries share it. `used_bytes` and `total_bytes` are `None` when the
    /// filesystem cannot be inspected (a failed stat, or a non-unix build).
    pub async fn storage_root_usage(&self, actor: &User) -> AppResult<Vec<StorageRootUsage>> {
        let libraries = self
            .list_libraries_for_permission(actor, None, scryer_domain::LibraryPermission::View)
            .await?;
        let roots = library_root_folders_from_libraries(&libraries, None);

        let mut seen = HashSet::new();
        let mut unique_paths: HashMap<String, String> = HashMap::new();
        let mut dedup_roots = Vec::new();
        for root in roots {
            if !seen.insert((root.library_id.clone(), root.normalized_path.clone())) {
                continue;
            }
            unique_paths
                .entry(root.normalized_path.clone())
                .or_insert_with(|| root.path.clone());
            dedup_roots.push(root);
        }

        // Stat each unique filesystem path once, on blocking threads and each
        // behind its own timeout: a dead network mount (an unresponsive NFS or
        // SMB server) blocks the stat syscall indefinitely, and one such mount
        // must not hang the dashboard or blank the healthy roots beside it.
        let mut stats = tokio::task::JoinSet::new();
        for (normalized, path) in unique_paths {
            stats.spawn(async move {
                let usage = tokio::time::timeout(
                    STORAGE_ROOT_STAT_TIMEOUT,
                    tokio::task::spawn_blocking(move || library_root_filesystem_usage(&path)),
                )
                .await
                .ok()
                .and_then(Result::ok)
                .flatten();
                (normalized, usage)
            });
        }
        let mut usage_by_path: HashMap<String, Option<(i64, i64)>> = HashMap::new();
        while let Some(joined) = stats.join_next().await {
            if let Ok((normalized, usage)) = joined {
                usage_by_path.insert(normalized, usage);
            }
        }

        Ok(dedup_roots
            .into_iter()
            .map(|root| {
                let usage = usage_by_path.get(&root.normalized_path).copied().flatten();
                StorageRootUsage {
                    path: root.path,
                    library_id: root.library_id,
                    library_name: root.library_name,
                    facet: root.facet,
                    used_bytes: usage.map(|(used, _)| used),
                    total_bytes: usage.map(|(_, total)| total),
                }
            })
            .collect())
    }
}

/// Used and total bytes for the filesystem backing `path`, or `None` when it
/// cannot be inspected. It is the *available* figure that excludes the blocks
/// reserved for root, so the used figure reported here (total minus available)
/// includes them.
fn library_root_filesystem_usage(path: &str) -> Option<(i64, i64)> {
    let space = crate::filesystem_space(path)?;
    let used = space.total_bytes.saturating_sub(space.available_bytes);
    Some((
        i64::try_from(used).ok()?,
        i64::try_from(space.total_bytes).ok()?,
    ))
}
impl AppUseCase {
    /// Return the configured root folders for a concrete library.
    ///
    /// If a stale title points at a missing or empty library, fall back to the
    /// facet default roots so existing data remains importable.
    pub(crate) async fn root_folders_for_library(
        &self,
        library_id: &str,
        fallback_facet: &scryer_domain::MediaFacet,
    ) -> AppResult<Vec<scryer_domain::RootFolderEntry>> {
        if let Some(library) = self
            .services
            .catalog
            .libraries
            .get_by_id(library_id)
            .await?
        {
            if library.facet != *fallback_facet {
                warn!(
                    library_id = %library.id,
                    library_facet = library.facet.as_str(),
                    title_facet = fallback_facet.as_str(),
                    "library facet does not match title facet; falling back to facet default roots"
                );
                return self.root_folders_for_facet(fallback_facet).await;
            }

            let entries = root_folder_entries_from_library_roots(&library.roots);
            if !entries.is_empty() {
                return Ok(entries);
            }
            warn!(
                library_id = %library.id,
                facet = library.facet.as_str(),
                "library has no roots; falling back to facet default roots"
            );
        } else {
            warn!(
                library_id,
                facet = fallback_facet.as_str(),
                "library is missing; falling back to facet default roots"
            );
        }

        self.root_folders_for_facet(fallback_facet).await
    }
}

#[cfg(test)]
mod tests {
    use super::library_path_is_under_root;

    #[test]
    fn root_containment_handles_unix_root() {
        assert!(library_path_is_under_root("/media/movies", "/"));
    }

    #[test]
    fn root_containment_handles_windows_drive_root() {
        assert!(library_path_is_under_root("C:\\Media\\Movies", "C:\\"));
    }

    #[test]
    fn root_containment_handles_unc_share_root() {
        assert!(library_path_is_under_root(
            "\\\\server\\share\\Movies",
            "\\\\server\\share\\"
        ));
    }
}

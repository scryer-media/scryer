use std::collections::{HashMap, HashSet};
use std::path::Path;

use chrono::Utc;
use scryer_domain::MediaFacet;

use crate::library_scan_metadata::{
    METADATA_TYPE_MOVIE, METADATA_TYPE_SERIES, MetadataSearchResults,
    PreparedMovieLibraryScanCandidate, PreparedSeriesLibraryScanCandidate,
    build_library_scan_unmatched_search_attempts, library_scan_unmatched_reason_code,
    movie_candidate_year_hint_variants,
};
use crate::{
    AppResult, AppUseCase, LibraryScanUnmatchedItem, LibraryScanUnmatchedSearchAttempt,
    PendingImportStatus,
};

pub(crate) const LIBRARY_SCAN_SKIPPED_UNUSABLE_TITLE_EVIDENCE: &str =
    "skipped_unusable_title_evidence";
/// The scanned folder is an organizing folder, a mixed layout, or a link; it
/// cannot be bound to a title as-is.
pub(crate) const LIBRARY_SCAN_INVALID_TITLE_BOUNDARY: &str = "library_scan_invalid_title_boundary";
pub(crate) const LIBRARY_SCAN_SKIPPED_FILE_METADATA_UNREADABLE: &str =
    "skipped_file_metadata_unreadable";
pub(crate) const LIBRARY_SCAN_TITLE_ALREADY_OWNS_ANOTHER_FOLDER: &str =
    "title_already_owns_another_folder";
/// A user took this folder away from the title that owned it (FR-007, SC-008).
///
/// The displaced title keeps its identity, monitoring, and history but owns no
/// folder, so it surfaces in the same unmatched/repair experience a scan uses —
/// with a reason that says a person did this, not a failed match.
pub(crate) const LIBRARY_SCAN_FOLDER_OWNERSHIP_CHANGED_BY_USER: &str =
    "folder_ownership_changed_by_user";

#[derive(Clone, Debug)]
struct MovieUnmatchedScanRecord {
    path: String,
    display_name: String,
    query: String,
    year_hint: Option<u32>,
    reason: &'static str,
    search_attempts: Vec<LibraryScanUnmatchedSearchAttempt>,
    size_bytes: Option<i64>,
}

struct LibraryScanUnmatchedScope<'a> {
    facet: &'a MediaFacet,
    library_id: &'a str,
    title_id: Option<&'a str>,
    session_id: &'a str,
    library_path: &'a str,
}

struct LibraryScanUnmatchedItemArgs {
    status: PendingImportStatus,
    item_path: String,
    display_name: String,
    query: String,
    year_hint: Option<u32>,
    reason_code: String,
    error_message: Option<String>,
    search_attempts: Vec<LibraryScanUnmatchedSearchAttempt>,
    /// File size when the scanner has a concrete file in hand. Folder-shaped
    /// candidates leave this `None`.
    size_bytes: Option<i64>,
}

fn normalize_library_scan_root(library_path: &str) -> String {
    Path::new(library_path).to_string_lossy().trim().to_string()
}

pub(crate) fn normalize_library_scan_item_path(path: &str) -> String {
    path.trim().to_string()
}

fn build_library_scan_unmatched_item_id(
    facet: &MediaFacet,
    library_id: &str,
    item_path: &str,
) -> String {
    let fingerprint = crate::helpers::blake3_identity_hex(
        crate::helpers::HashDomain::LibraryScanUnmatchedItem,
        format!("{}:{library_id}:{item_path}", facet.as_str()),
    );
    format!("library_scan_unmatched:{}", &fingerprint[..24])
}

fn build_library_scan_unmatched_item(
    scope: LibraryScanUnmatchedScope<'_>,
    args: LibraryScanUnmatchedItemArgs,
) -> LibraryScanUnmatchedItem {
    let item_path = normalize_library_scan_item_path(&args.item_path);
    let timestamp = Utc::now().to_rfc3339();

    LibraryScanUnmatchedItem {
        id: build_library_scan_unmatched_item_id(scope.facet, scope.library_id, &item_path),
        library_id: scope.library_id.to_string(),
        facet: scope.facet.clone(),
        status: args.status,
        title_id: scope.title_id.map(str::to_string),
        scan_session_id: scope.session_id.to_string(),
        scan_root: normalize_library_scan_root(scope.library_path),
        item_path,
        display_name: args.display_name,
        query: args.query,
        year_hint: args.year_hint.map(|value| value as i32),
        reason_code: args.reason_code,
        error_message: args.error_message,
        search_attempts: args.search_attempts,
        size_bytes: args.size_bytes,
        created_at: timestamp.clone(),
        updated_at: timestamp,
    }
}

pub(crate) fn series_unmatched_display_name(
    candidate: &PreparedSeriesLibraryScanCandidate,
) -> String {
    candidate
        .folder_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            candidate
                .folder_path
                .file_name()
                .and_then(|value| value.to_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| candidate.folder_path.to_string_lossy().to_string())
}

fn build_movie_unmatched_scan_record(
    candidate: &PreparedMovieLibraryScanCandidate,
    batch_search_results: &MetadataSearchResults,
) -> MovieUnmatchedScanRecord {
    let search_attempts = build_library_scan_unmatched_search_attempts(
        METADATA_TYPE_MOVIE,
        &candidate.search_candidates,
        &movie_candidate_year_hint_variants(candidate),
        candidate.identity_hint.as_ref(),
        batch_search_results,
    );
    let reason =
        library_scan_unmatched_reason_code(candidate.identity_hint.as_ref(), &search_attempts);

    MovieUnmatchedScanRecord {
        path: candidate.file.path.clone(),
        display_name: candidate.file.display_name.clone(),
        query: candidate.query.clone(),
        year_hint: candidate.year_hint,
        reason,
        search_attempts,
        size_bytes: candidate.file.size_bytes,
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "movie unmatched items still mirror the caller's scan context and persisted payload fields"
)]
pub(crate) fn build_movie_unmatched_scan_item(
    facet: &MediaFacet,
    library_id: &str,
    session_id: &str,
    library_path: &str,
    candidate: &PreparedMovieLibraryScanCandidate,
    batch_search_results: &MetadataSearchResults,
    reason_override: Option<&str>,
    error_message: Option<String>,
) -> LibraryScanUnmatchedItem {
    let record = build_movie_unmatched_scan_record(candidate, batch_search_results);
    build_library_scan_unmatched_item(
        LibraryScanUnmatchedScope {
            facet,
            library_id,
            title_id: None,
            session_id,
            library_path,
        },
        LibraryScanUnmatchedItemArgs {
            status: PendingImportStatus::Pending,
            item_path: record.path,
            display_name: record.display_name,
            query: record.query,
            year_hint: record.year_hint,
            reason_code: reason_override.unwrap_or(record.reason).to_string(),
            error_message,
            search_attempts: record.search_attempts,
            size_bytes: record.size_bytes,
        },
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "series unmatched items still mirror the caller's scan context and persisted payload fields"
)]
pub(crate) fn build_series_unmatched_scan_item(
    facet: &MediaFacet,
    library_id: &str,
    session_id: &str,
    library_path: &str,
    candidate: &PreparedSeriesLibraryScanCandidate,
    batch_search_results: &MetadataSearchResults,
    reason_override: Option<&str>,
    error_message: Option<String>,
) -> LibraryScanUnmatchedItem {
    let search_attempts = build_library_scan_unmatched_search_attempts(
        METADATA_TYPE_SERIES,
        &candidate.search_candidates,
        &[candidate.year_hint],
        candidate.identity_hint.as_ref(),
        batch_search_results,
    );
    let reason_code = reason_override.unwrap_or_else(|| {
        library_scan_unmatched_reason_code(candidate.identity_hint.as_ref(), &search_attempts)
    });

    build_library_scan_unmatched_item(
        LibraryScanUnmatchedScope {
            facet,
            library_id,
            title_id: None,
            session_id,
            library_path,
        },
        LibraryScanUnmatchedItemArgs {
            status: PendingImportStatus::Pending,
            item_path: candidate.item_path().to_string(),
            display_name: series_unmatched_display_name(candidate),
            query: candidate.query.clone(),
            year_hint: candidate.year_hint,
            reason_code: reason_code.to_string(),
            error_message,
            search_attempts,
            // Series candidates are folder-shaped, so there is no single file
            // size to record here.
            size_bytes: None,
        },
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "title-bound unmatched items still mirror the caller's scan context and persisted payload fields"
)]
pub(crate) fn build_title_bound_unmatched_scan_item(
    facet: &MediaFacet,
    library_id: &str,
    title_id: &str,
    session_id: Option<&str>,
    title_scan_root: &str,
    item_path: &str,
    display_name: &str,
    query: &str,
    year_hint: Option<u32>,
    reason_code: &str,
    size_bytes: Option<i64>,
) -> LibraryScanUnmatchedItem {
    build_library_scan_unmatched_item(
        LibraryScanUnmatchedScope {
            facet,
            library_id,
            title_id: Some(title_id),
            session_id: session_id.unwrap_or_default(),
            library_path: title_scan_root,
        },
        LibraryScanUnmatchedItemArgs {
            status: PendingImportStatus::Pending,
            item_path: item_path.to_string(),
            display_name: display_name.to_string(),
            query: query.to_string(),
            year_hint,
            reason_code: reason_code.to_string(),
            error_message: None,
            search_attempts: Vec::new(),
            size_bytes,
        },
    )
}

pub(crate) struct IgnoredLibraryScanItemArgs<'a> {
    pub title_id: Option<&'a str>,
    pub session_id: Option<&'a str>,
    pub library_path: &'a str,
    pub item_path: &'a str,
    pub display_name: &'a str,
    pub query: &'a str,
    pub year_hint: Option<u32>,
    pub reason_code: &'a str,
    pub error_message: Option<String>,
    pub size_bytes: Option<i64>,
}

pub(crate) fn build_ignored_library_scan_item(
    facet: &MediaFacet,
    library_id: &str,
    args: IgnoredLibraryScanItemArgs<'_>,
) -> LibraryScanUnmatchedItem {
    build_library_scan_unmatched_item(
        LibraryScanUnmatchedScope {
            facet,
            library_id,
            title_id: args.title_id,
            session_id: args.session_id.unwrap_or_default(),
            library_path: args.library_path,
        },
        LibraryScanUnmatchedItemArgs {
            status: PendingImportStatus::Ignored,
            item_path: args.item_path.to_string(),
            display_name: args.display_name.to_string(),
            query: args.query.to_string(),
            year_hint: args.year_hint,
            reason_code: args.reason_code.to_string(),
            error_message: args.error_message,
            search_attempts: Vec::new(),
            size_bytes: args.size_bytes,
        },
    )
}

pub(crate) fn format_library_scan_unmatched_search_attempts(
    attempts: &[LibraryScanUnmatchedSearchAttempt],
) -> String {
    attempts
        .iter()
        .map(|attempt| {
            let top_results = if attempt.top_results.is_empty() {
                "[]".to_string()
            } else {
                format!("[{}]", attempt.top_results.join(" | "))
            };
            format!("{}:{}:{}", attempt.query, attempt.result_count, top_results)
        })
        .collect::<Vec<_>>()
        .join("; ")
}

pub(crate) async fn persist_library_scan_unmatched_item(
    app: &AppUseCase,
    item: &LibraryScanUnmatchedItem,
) -> AppResult<()> {
    app.services
        .library
        .library_scan_unmatched_items
        .upsert_library_scan_unmatched_item(item)
        .await?;
    Ok(())
}

pub(crate) async fn persist_ignored_library_scan_item(
    app: &AppUseCase,
    facet: &MediaFacet,
    library_id: &str,
    args: IgnoredLibraryScanItemArgs<'_>,
) -> AppResult<()> {
    let item = build_ignored_library_scan_item(facet, library_id, args);
    persist_library_scan_unmatched_item(app, &item).await
}

pub(crate) async fn clear_library_scan_unmatched_item(
    app: &AppUseCase,
    facet: &MediaFacet,
    library_id: &str,
    item_path: &str,
) -> AppResult<()> {
    let item_path = normalize_library_scan_item_path(item_path);
    if item_path.is_empty() {
        return Ok(());
    }

    app.services
        .library
        .library_scan_unmatched_items
        .delete_library_scan_unmatched_item(library_id, facet.clone(), &item_path)
        .await
}

/// A title walk's cleared rows, removed together when the walk ends.
///
/// A row that is persisted again before the walk ends is removed first, at
/// that point, so it is never removed after it was written. A walk that is
/// dropped before it ends still removes the rows it cleared.
pub(crate) struct ClearedLibraryScanUnmatchedItems {
    app: AppUseCase,
    title_id: String,
    /// (library id, facet, normalized item path), in the order they were
    /// cleared.
    items: Vec<(String, MediaFacet, String)>,
    keys: HashSet<(String, MediaFacet, String)>,
}

impl ClearedLibraryScanUnmatchedItems {
    pub(crate) fn new(app: &AppUseCase, title_id: &str) -> Self {
        Self {
            app: app.clone(),
            title_id: title_id.to_string(),
            items: Vec::new(),
            keys: HashSet::new(),
        }
    }

    pub(crate) fn push(&mut self, facet: &MediaFacet, library_id: &str, item_path: &str) {
        let item_path = normalize_library_scan_item_path(item_path);
        if item_path.is_empty() {
            return;
        }
        let key = (library_id.to_string(), facet.clone(), item_path);
        if self.keys.insert(key.clone()) {
            self.items.push(key);
        }
    }

    /// Call before persisting `item_path` as unmatched: a pending clear of
    /// that row runs now, so the row persisted next is kept, as when every
    /// clear ran at once.
    pub(crate) async fn clear_before_persist(
        &mut self,
        facet: &MediaFacet,
        library_id: &str,
        item_path: &str,
    ) {
        let key = (
            library_id.to_string(),
            facet.clone(),
            normalize_library_scan_item_path(item_path),
        );
        if !self.keys.remove(&key) {
            return;
        }
        self.items.retain(|item| *item != key);
        let (library_id, facet, item_path) = key;
        clear_library_scan_unmatched_items_now(
            &self.app,
            &self.title_id,
            vec![(library_id, facet, vec![item_path])],
        )
        .await;
    }

    /// Removes every collected row: one statement per library and facet, and
    /// when that fails, `clear_library_scan_unmatched_item` per row, warning
    /// and carrying on past each failure, as clearing them one at a time did.
    pub(crate) async fn clear(&mut self) {
        let groups = self.take_groups();
        clear_library_scan_unmatched_items_now(&self.app, &self.title_id, groups).await;
    }

    fn take_groups(&mut self) -> Vec<(String, MediaFacet, Vec<String>)> {
        self.keys.clear();
        let mut groups: Vec<(String, MediaFacet, Vec<String>)> = Vec::new();
        let mut group_index: HashMap<(String, MediaFacet), usize> = HashMap::new();
        for (library_id, facet, item_path) in std::mem::take(&mut self.items) {
            match group_index.get(&(library_id.clone(), facet.clone())) {
                Some(index) => groups[*index].2.push(item_path),
                None => {
                    group_index.insert((library_id.clone(), facet.clone()), groups.len());
                    groups.push((library_id, facet, vec![item_path]));
                }
            }
        }
        groups
    }
}

impl Drop for ClearedLibraryScanUnmatchedItems {
    fn drop(&mut self) {
        if self.items.is_empty() {
            return;
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let groups = self.take_groups();
        let app = self.app.clone();
        let title_id = std::mem::take(&mut self.title_id);
        runtime.spawn(async move {
            clear_library_scan_unmatched_items_now(&app, &title_id, groups).await;
        });
    }
}

async fn clear_library_scan_unmatched_items_now(
    app: &AppUseCase,
    title_id: &str,
    groups: Vec<(String, MediaFacet, Vec<String>)>,
) {
    for (library_id, facet, paths) in groups {
        if app
            .services
            .library
            .library_scan_unmatched_items
            .delete_library_scan_unmatched_items(&library_id, facet.clone(), &paths)
            .await
            .is_ok()
        {
            continue;
        }
        for item_path in &paths {
            if let Err(error) =
                clear_library_scan_unmatched_item(app, &facet, &library_id, item_path).await
            {
                tracing::warn!(
                    error = %error,
                    title_id = %title_id,
                    file_path = %item_path,
                    "failed to clear unmatched title scan item"
                );
            }
        }
    }
}

pub(crate) async fn reconcile_library_scan_unmatched_items(
    app: &AppUseCase,
    facet: &MediaFacet,
    library_path: &str,
    seen_paths: &HashSet<String>,
) -> AppResult<()> {
    let scan_root = normalize_library_scan_root(library_path);
    let count = app
        .services
        .library
        .library_scan_unmatched_items
        .count_library_scan_unmatched_items(Some(facet.clone()), Some(&scan_root), None)
        .await?;
    if count <= 0 {
        return Ok(());
    }

    let existing = app
        .services
        .library
        .library_scan_unmatched_items
        .list_library_scan_unmatched_items(Some(facet.clone()), Some(&scan_root), None, count, 0)
        .await?;

    // A rejected or unreadable boundary hides its descendants from discovery.
    // Their absence is not evidence that previously recorded items disappeared.
    let unresolved_boundaries: Vec<_> = existing
        .iter()
        .filter(|item| {
            item.status == PendingImportStatus::Ignored && seen_paths.contains(&item.item_path)
        })
        .collect();

    for item in &existing {
        if !seen_paths.contains(&item.item_path)
            && !unresolved_boundaries.iter().any(|boundary| {
                boundary.library_id == item.library_id
                    && crate::stored_paths::stored_path_is_within_folder(
                        &boundary.item_path,
                        &item.item_path,
                    )
            })
        {
            app.services
                .library
                .library_scan_unmatched_items
                .delete_library_scan_unmatched_item(
                    &item.library_id,
                    facet.clone(),
                    &item.item_path,
                )
                .await?;
        }
    }

    Ok(())
}

/// Drop title-scan rows left behind by a title that no longer owns them.
///
/// A title scan records a file it cannot place with the title's folder as the
/// row's scan root. [`reconcile_library_scan_unmatched_items`] lists rows by
/// scan root, so the library pass never sees these and the title pass only
/// sees the folder the title has now. Once the folder is renamed or the title
/// is deleted, nothing lists the old rows again and they sit in pending
/// imports forever. Rows under `library_path` (but not at it) are therefore
/// dropped here when their title is gone or records a different folder.
pub(crate) async fn reconcile_title_scan_unmatched_items(
    app: &AppUseCase,
    facet: &MediaFacet,
    library_id: &str,
    library_path: &str,
) -> AppResult<()> {
    let repo = &app.services.library.library_scan_unmatched_items;
    let count = repo
        .count_library_scan_unmatched_items(Some(facet.clone()), None, None)
        .await?;
    if count <= 0 {
        return Ok(());
    }
    let library_root =
        crate::stored_paths::stored_path_to_path_buf(&normalize_library_scan_root(library_path));
    let existing = repo
        .list_library_scan_unmatched_items(Some(facet.clone()), None, None, count, 0)
        .await?;

    let mut title_folders: HashMap<String, Option<Option<String>>> = HashMap::new();
    for item in existing {
        let Some(title_id) = item.title_id.as_deref() else {
            continue;
        };
        if item.library_id != library_id {
            continue;
        }
        let scan_root = crate::stored_paths::stored_path_to_path_buf(&item.scan_root);
        if scan_root == library_root || !scan_root.starts_with(&library_root) {
            continue;
        }

        if !title_folders.contains_key(title_id) {
            let folder = app
                .services
                .catalog
                .titles
                .get_by_id(title_id)
                .await?
                .map(|title| {
                    title
                        .folder_path
                        .as_deref()
                        .map(str::trim)
                        .filter(|folder| !folder.is_empty())
                        .map(|folder| {
                            normalize_library_scan_root(
                                &crate::stored_paths::path_to_stored_string(
                                    crate::stored_paths::stored_path_to_path_buf(folder),
                                ),
                            )
                        })
                });
            title_folders.insert(title_id.to_string(), folder);
        }
        let stale = match &title_folders[title_id] {
            None => true,
            Some(Some(folder)) => *folder != item.scan_root,
            // No recorded folder to compare against; the title scan owns it.
            Some(None) => false,
        };
        if stale {
            tracing::debug!(
                title_id,
                item_path = %item.item_path,
                scan_root = %item.scan_root,
                "dropping pending import left behind by a deleted or moved title"
            );
            repo.delete_library_scan_unmatched_item(
                &item.library_id,
                facet.clone(),
                &item.item_path,
            )
            .await?;
        }
    }

    Ok(())
}

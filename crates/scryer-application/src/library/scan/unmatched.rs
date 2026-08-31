use std::collections::HashSet;
use std::path::Path;

use chrono::Utc;
use scryer_domain::MediaFacet;

use crate::library_scan_metadata::{
    METADATA_TYPE_MOVIE, METADATA_TYPE_SERIES, MetadataSearchResults,
    PreparedMovieLibraryScanCandidate, PreparedSeriesLibraryScanCandidate,
    build_library_scan_unmatched_search_attempts, library_scan_unmatched_reason_code,
};
use crate::{
    AppResult, AppUseCase, LibraryScanUnmatchedItem, LibraryScanUnmatchedSearchAttempt,
    PendingImportStatus,
};

pub(crate) const LIBRARY_SCAN_SKIPPED_UNUSABLE_TITLE_EVIDENCE: &str =
    "skipped_unusable_title_evidence";
pub(crate) const LIBRARY_SCAN_SKIPPED_FILE_METADATA_UNREADABLE: &str =
    "skipped_file_metadata_unreadable";
pub(crate) const LIBRARY_SCAN_TITLE_ALREADY_OWNS_ANOTHER_FOLDER: &str =
    "title_already_owns_another_folder";

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
        candidate.year_hint,
        candidate.identity_hint.as_ref(),
        batch_search_results,
    );
    let reason = library_scan_unmatched_reason_code(&search_attempts);

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
        candidate.year_hint,
        candidate.identity_hint.as_ref(),
        batch_search_results,
    );
    let reason_code =
        reason_override.unwrap_or_else(|| library_scan_unmatched_reason_code(&search_attempts));

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

    for item in existing {
        if !seen_paths.contains(&item.item_path) {
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

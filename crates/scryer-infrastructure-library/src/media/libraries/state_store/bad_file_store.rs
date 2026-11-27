//! Bad-file reports and the content identity a blocklist entry carries.
//!
//! The [`BlocklistRepository`](scryer_application::BlocklistRepository) impl
//! in the parent module calls these.

use std::collections::{HashMap, HashSet};

use chrono::Utc;
use scryer_application::{
    AppError, AppResult, ConfirmMediaIssue, MediaIssueRecycleIntent, NewMediaIssue,
    UnmarkedMediaIssue, normalize_release_name,
};
use scryer_domain::{
    BlocklistContentIdentity, Id, MediaIssue, MediaIssueExecutionStatus, MediaIssueReason,
    MediaIssueRemediation, MediaIssueStatus,
};
use scryer_plugin_sdk::torrent::normalize_info_hash;

use crate::media::id_list_statements::{id_list_statements, max_binds};
use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRow, SqlRuntime, StoreDatastore};
use crate::storage::sql::json::canonical_json_text;

/// The most reports one page returns.
const MAX_MEDIA_ISSUE_PAGE: usize = 200;

/// Ids bound per statement at most.
const ID_CHUNK: usize = 500;

const MEDIA_ISSUE_COLUMNS: &str = "id, library_id, title_id, media_file_id, file_path_snapshot,
    file_version, content_scheme, size_bytes, sample_bytes, sample_blake3, reporter_user_id,
    reason, details, remediation, authorized_remediation, status, affected_episode_ids_json,
    affected_series_movie_link_ids_json, rule_decision_id, blocklist_id, reviewed_by_user_id,
    reviewed_at, execution_status, execution_error_code, created_at, updated_at";

fn invalid(what: &str, value: &str) -> AppError {
    AppError::Repository(format!("invalid stored media issue {what}: {value}"))
}

fn to_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// The content identity a blocklist row carries, when it carries one.
pub(super) fn blocklist_row_content(row: &SqlRow) -> AppResult<Option<BlocklistContentIdentity>> {
    let Some(sample_blake3) = row.opt_text("content_sample_blake3")? else {
        return Ok(None);
    };
    Ok(Some(BlocklistContentIdentity {
        scheme: row.opt_text("content_scheme")?.unwrap_or_default(),
        size_bytes: row.opt_i64("content_size_bytes")?.unwrap_or(0).max(0) as u64,
        sample_bytes: row.opt_i64("content_sample_bytes")?.unwrap_or(0).max(0) as u64,
        sample_blake3,
    }))
}

fn media_issue_row(row: &SqlRow) -> AppResult<MediaIssue> {
    let text_enum = |column: &str| row.text(column);
    let reason = text_enum("reason")?;
    let remediation = text_enum("remediation")?;
    let status = text_enum("status")?;
    let execution_status = text_enum("execution_status")?;
    let decode_ids = |column: &str| -> AppResult<Vec<String>> {
        let json = row.text(column)?;
        serde_json::from_str(&json).map_err(|_| invalid(column, &json))
    };
    Ok(MediaIssue {
        id: row.text("id")?,
        library_id: row.text("library_id")?,
        title_id: row.text("title_id")?,
        media_file_id: row.text("media_file_id")?,
        file_path_snapshot: row.text("file_path_snapshot")?,
        file_version: row.text("file_version")?,
        content: BlocklistContentIdentity {
            scheme: row.text("content_scheme")?,
            size_bytes: row.i64("size_bytes")?.max(0) as u64,
            sample_bytes: row.i64("sample_bytes")?.max(0) as u64,
            sample_blake3: row.text("sample_blake3")?,
        },
        reporter_user_id: row.text("reporter_user_id")?,
        reason: MediaIssueReason::parse(&reason).ok_or_else(|| invalid("reason", &reason))?,
        details: row.opt_text("details")?,
        remediation: MediaIssueRemediation::parse(&remediation)
            .ok_or_else(|| invalid("remediation", &remediation))?,
        authorized_remediation: row
            .opt_text("authorized_remediation")?
            .as_deref()
            .and_then(MediaIssueRemediation::parse),
        status: MediaIssueStatus::parse(&status).ok_or_else(|| invalid("status", &status))?,
        affected_episode_ids: decode_ids("affected_episode_ids_json")?,
        affected_series_movie_link_ids: decode_ids("affected_series_movie_link_ids_json")?,
        rule_decision_id: row.opt_text("rule_decision_id")?,
        blocklist_id: row.opt_text("blocklist_id")?,
        reviewed_by_user_id: row.opt_text("reviewed_by_user_id")?,
        reviewed_at: row.opt_timestamp_text("reviewed_at")?,
        execution_status: MediaIssueExecutionStatus::parse(&execution_status)
            .ok_or_else(|| invalid("execution status", &execution_status))?,
        execution_error_code: row.opt_text("execution_error_code")?,
        created_at: row.timestamp_text("created_at")?,
        updated_at: row.timestamp_text("updated_at")?,
    })
}

fn placeholders(count: usize) -> String {
    std::iter::repeat_n("{}", count)
        .collect::<Vec<_>>()
        .join(", ")
}

pub(super) async fn list_content_identities_for_title(
    datastore: &StoreDatastore,
    title_id: &str,
    only_blocking_imports: bool,
) -> AppResult<Vec<(String, BlocklistContentIdentity)>> {
    // `idx_blocklist_content_identity_unique` serves this read; the query
    // repeats its predicate so both engines can use the partial index.
    let (sql, args) = if only_blocking_imports {
        (
            "SELECT id, content_scheme, content_size_bytes, content_sample_bytes,
                    content_sample_blake3
               FROM blocklist
              WHERE title_id = {} AND content_sample_blake3 IS NOT NULL
                AND content_blocks_imports = {}",
            vec![SqlArg::Text(title_id.to_string()), SqlArg::Bool(true)],
        )
    } else {
        (
            "SELECT id, content_scheme, content_size_bytes, content_sample_bytes,
                    content_sample_blake3
               FROM blocklist
              WHERE title_id = {} AND content_sample_blake3 IS NOT NULL",
            vec![SqlArg::Text(title_id.to_string())],
        )
    };
    let rows = SqlRuntime::fetch_all(datastore.read_exec(), sql, &args).await?;
    let mut identities = Vec::with_capacity(rows.len());
    for row in &rows {
        if let Some(identity) = blocklist_row_content(row)? {
            identities.push((row.text("id")?, identity));
        }
    }
    Ok(identities)
}

/// Which of these files carry a bad-file mark. Reads by primary key.
pub(super) async fn marked_bad_file_ids(
    datastore: &StoreDatastore,
    file_ids: &[String],
) -> AppResult<HashSet<String>> {
    let mut marked = HashSet::new();
    if file_ids.is_empty() {
        return Ok(marked);
    }
    let exec = datastore.read_exec();
    let (file_ids, per_statement) = id_list_statements(file_ids, 1, 0, max_binds(&exec), ID_CHUNK);
    for chunk in file_ids.chunks(per_statement) {
        let sql = format!(
            "SELECT id FROM media_files
              WHERE id IN ({}) AND bad_file_blocklist_id IS NOT NULL",
            placeholders(chunk.len())
        );
        let args: Vec<SqlArg> = chunk.iter().cloned().map(SqlArg::Text).collect();
        for row in SqlRuntime::fetch_all(datastore.read_exec(), &sql, &args).await? {
            marked.insert(row.text("id")?);
        }
    }
    Ok(marked)
}

fn identity_args(title_id: &str, identity: &BlocklistContentIdentity) -> Vec<SqlArg> {
    vec![
        SqlArg::Text(title_id.to_string()),
        SqlArg::Text(identity.scheme.clone()),
        SqlArg::I64(to_i64(identity.size_bytes)),
        SqlArg::I64(to_i64(identity.sample_bytes)),
        SqlArg::Text(identity.sample_blake3.clone()),
    ]
}

/// Matches one title's entry by content identity, through the unique index.
pub(super) const CONTENT_IDENTITY_PREDICATE: &str = "title_id = {} AND content_scheme = {}
    AND content_size_bytes = {} AND content_sample_bytes = {} AND content_sample_blake3 = {}
    AND content_sample_blake3 IS NOT NULL";

pub(super) fn content_identity_lookup_args(
    title_id: &str,
    identity: &BlocklistContentIdentity,
) -> Vec<SqlArg> {
    identity_args(title_id, identity)
}

pub(super) async fn remove_and_unmark(
    datastore: &StoreDatastore,
    id: &str,
) -> AppResult<Vec<UnmarkedMediaIssue>> {
    let id = id.to_string();
    SqlRuntime::run_in_transaction(datastore, "remove_bad_file_blocklist_entry", move |tx| {
        let id = id.clone();
        Box::pin(async move {
            let args = [SqlArg::Text(id.clone())];
            let reports = SqlRuntime::fetch_all(
                SqlExec::Tx(tx),
                "SELECT id, library_id, title_id FROM media_issues
                  WHERE blocklist_id = {} AND blocklist_id IS NOT NULL",
                &args,
            )
            .await?
            .iter()
            .map(|row| {
                Ok(UnmarkedMediaIssue {
                    issue_id: row.text("id")?,
                    library_id: row.text("library_id")?,
                    title_id: row.text("title_id")?,
                })
            })
            .collect::<AppResult<Vec<_>>>()?;
            // The foreign keys clear both as well; doing it here does not
            // depend on them being enforced.
            SqlRuntime::execute(
                SqlExec::Tx(tx),
                "UPDATE media_files SET bad_file_blocklist_id = NULL
                  WHERE bad_file_blocklist_id = {}",
                &args,
            )
            .await?;
            SqlRuntime::execute(
                SqlExec::Tx(tx),
                "UPDATE media_issues SET blocklist_id = NULL WHERE blocklist_id = {}",
                &args,
            )
            .await?;
            SqlRuntime::execute(
                SqlExec::Tx(tx),
                "DELETE FROM blocklist WHERE id = {}",
                &args,
            )
            .await?;
            Ok(reports)
        })
    })
    .await
}

pub(super) async fn submit_media_issue(
    datastore: &StoreDatastore,
    issue: &NewMediaIssue,
) -> AppResult<(MediaIssue, bool)> {
    let id = Id::new().0;
    let now = datastore.timestamp_text_arg(&Utc::now().to_rfc3339())?;
    let identity = [
        SqlArg::Text(issue.media_file_id.clone()),
        SqlArg::Text(issue.file_version.clone()),
        SqlArg::Text(issue.reporter_user_id.clone()),
        SqlArg::Text(issue.remediation.as_str().to_string()),
    ];
    let args = vec![
        SqlArg::Text(id.clone()),
        SqlArg::Text(issue.library_id.clone()),
        SqlArg::Text(issue.title_id.clone()),
        SqlArg::Text(issue.media_file_id.clone()),
        SqlArg::Text(issue.file_path_snapshot.clone()),
        SqlArg::Text(issue.file_version.clone()),
        SqlArg::Text(issue.content.scheme.clone()),
        SqlArg::I64(to_i64(issue.content.size_bytes)),
        SqlArg::I64(to_i64(issue.content.sample_bytes)),
        SqlArg::Text(issue.content.sample_blake3.clone()),
        SqlArg::Text(issue.reporter_user_id.clone()),
        SqlArg::Text(issue.reason.as_str().to_string()),
        SqlArg::OptText(issue.details.clone()),
        SqlArg::Text(issue.remediation.as_str().to_string()),
        SqlArg::Text(canonical_json_text(&issue.affected_episode_ids)?),
        SqlArg::Text(canonical_json_text(&issue.affected_series_movie_link_ids)?),
        now.clone(),
        now,
    ];
    let inserted = SqlRuntime::execute_write(
        datastore,
        "insert_media_issue",
        "INSERT INTO media_issues
             (id, library_id, title_id, media_file_id, file_path_snapshot, file_version,
              content_scheme, size_bytes, sample_bytes, sample_blake3, reporter_user_id,
              reason, details, remediation, status, affected_episode_ids_json,
              affected_series_movie_link_ids_json, execution_status, created_at, updated_at)
         VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, 'pending', {}, {},
                 'none', {}, {})
         ON CONFLICT DO NOTHING",
        args,
    )
    .await?;
    // A reporter sending the same report twice gets the first one back.
    let id = if inserted == 0 {
        SqlRuntime::fetch_optional(
            datastore.read_exec(),
            "SELECT id FROM media_issues
              WHERE media_file_id = {} AND file_version = {} AND reporter_user_id = {}
                AND remediation = {} AND status <> 'canceled'",
            &identity,
        )
        .await?
        .ok_or_else(|| AppError::Repository("existing media issue could not be found".into()))?
        .text("id")?
    } else {
        id
    };
    let issue = get_media_issue(datastore, &id)
        .await?
        .ok_or_else(|| AppError::Repository("inserted media issue could not be reloaded".into()))?;
    Ok((issue, inserted > 0))
}

pub(super) async fn get_media_issue(
    datastore: &StoreDatastore,
    id: &str,
) -> AppResult<Option<MediaIssue>> {
    let row = SqlRuntime::fetch_optional(
        datastore.read_exec(),
        &format!("SELECT {MEDIA_ISSUE_COLUMNS} FROM media_issues WHERE id = {{}}"),
        &[SqlArg::Text(id.to_string())],
    )
    .await?;
    let Some(issue) = row.as_ref().map(media_issue_row).transpose()? else {
        return Ok(None);
    };
    let mut issues = [issue];
    mark_replacements_imported(datastore, &mut issues).await?;
    let [issue] = issues;
    Ok(Some(issue))
}

pub(super) async fn set_media_issue_rule_decision(
    datastore: &StoreDatastore,
    id: &str,
    decision_id: &str,
) -> AppResult<()> {
    SqlRuntime::execute_write(
        datastore,
        "set_media_issue_rule_decision",
        "UPDATE media_issues SET rule_decision_id = {}
          WHERE id = {} AND status = 'pending'",
        vec![
            SqlArg::Text(decision_id.to_string()),
            SqlArg::Text(id.to_string()),
        ],
    )
    .await?;
    Ok(())
}

pub(super) async fn list_media_issues(
    datastore: &StoreDatastore,
    library_id: &str,
    status: Option<MediaIssueStatus>,
    limit: usize,
    offset: usize,
) -> AppResult<Vec<MediaIssue>> {
    let limit = limit.clamp(1, MAX_MEDIA_ISSUE_PAGE);
    let mut args = vec![SqlArg::Text(library_id.to_string())];
    let status_filter = if let Some(status) = status {
        args.push(SqlArg::Text(status.as_str().to_string()));
        " AND status = {}"
    } else {
        ""
    };
    args.push(SqlArg::I64(limit as i64));
    args.push(SqlArg::I64(i64::try_from(offset).unwrap_or(i64::MAX)));
    // `idx_media_issues_library_status` serves the filter and the order.
    let sql = format!(
        "SELECT {MEDIA_ISSUE_COLUMNS}
           FROM media_issues
          WHERE library_id = {{}}{status_filter}
          ORDER BY created_at DESC, id DESC
          LIMIT {{}} OFFSET {{}}"
    );
    let mut issues = SqlRuntime::fetch_all(datastore.read_exec(), &sql, &args)
        .await?
        .iter()
        .map(media_issue_row)
        .collect::<AppResult<Vec<_>>>()?;
    mark_replacements_imported(datastore, &mut issues).await?;
    Ok(issues)
}

/// A report whose replacement search ran reads as imported once a file not
/// marked bad was added for its media after the report last changed. One
/// read covers the whole page.
async fn mark_replacements_imported(
    datastore: &StoreDatastore,
    issues: &mut [MediaIssue],
) -> AppResult<()> {
    let waiting: Vec<usize> = issues
        .iter()
        .enumerate()
        .filter(|(_, issue)| {
            matches!(
                issue.execution_status,
                MediaIssueExecutionStatus::Searching | MediaIssueExecutionStatus::DownloadQueued
            )
        })
        .map(|(index, _)| index)
        .collect();
    let Some(oldest) = waiting
        .iter()
        .map(|index| issues[*index].updated_at.as_str())
        .min()
        .map(str::to_string)
    else {
        return Ok(());
    };
    let mut title_ids: Vec<String> = waiting
        .iter()
        .map(|index| issues[*index].title_id.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    title_ids.sort_unstable();
    // file id -> (title id, created at, episode ids)
    let mut files: HashMap<String, (String, String, Vec<String>)> = HashMap::new();
    let exec = datastore.read_exec();
    let (title_ids, per_statement) =
        id_list_statements(&title_ids, 1, 1, max_binds(&exec), ID_CHUNK);
    for chunk in title_ids.chunks(per_statement) {
        let sql = format!(
            "SELECT mf.id AS id, mf.title_id AS title_id, mf.created_at AS created_at,
                    m.episode_id AS episode_id
               FROM media_files mf
               LEFT JOIN file_episode_map m ON m.file_id = mf.id
              WHERE mf.title_id IN ({})
                AND mf.created_at > {{}}
                AND mf.bad_file_blocklist_id IS NULL",
            placeholders(chunk.len())
        );
        let mut args: Vec<SqlArg> = chunk.iter().cloned().map(SqlArg::Text).collect();
        args.push(datastore.timestamp_text_arg(&oldest)?);
        for row in SqlRuntime::fetch_all(datastore.read_exec(), &sql, &args).await? {
            let entry = files
                .entry(row.text("id")?)
                .or_insert_with(|| (String::new(), String::new(), Vec::new()));
            entry.0 = row.text("title_id")?;
            entry.1 = row.timestamp_text("created_at")?;
            if let Some(episode_id) = row.opt_text("episode_id")? {
                entry.2.push(episode_id);
            }
        }
    }
    let mut by_title: HashMap<&str, Vec<(&str, &str, &[String])>> = HashMap::new();
    for (file_id, (title_id, created_at, episodes)) in &files {
        by_title.entry(title_id.as_str()).or_default().push((
            file_id.as_str(),
            created_at.as_str(),
            episodes.as_slice(),
        ));
    }
    for index in waiting {
        let issue = &issues[index];
        let Some(candidates) = by_title.get(issue.title_id.as_str()) else {
            continue;
        };
        let wanted: HashSet<&str> = issue
            .affected_episode_ids
            .iter()
            .map(String::as_str)
            .collect();
        let replaced = candidates.iter().any(|(file_id, created_at, episodes)| {
            *file_id != issue.media_file_id
                && *created_at > issue.updated_at.as_str()
                && (wanted.is_empty() || episodes.iter().any(|id| wanted.contains(id.as_str())))
        });
        if replaced {
            issues[index].execution_status = MediaIssueExecutionStatus::ReplacementImported;
        }
    }
    Ok(())
}

pub(super) async fn confirm_media_issue(
    datastore: &StoreDatastore,
    confirm: ConfirmMediaIssue<'_>,
) -> AppResult<bool> {
    let now = datastore.timestamp_text_arg(&Utc::now().to_rfc3339())?;
    let execution_status = match confirm.remediation {
        MediaIssueRemediation::MarkBad => MediaIssueExecutionStatus::Blocked,
        MediaIssueRemediation::RecycleAndSearch => MediaIssueExecutionStatus::Recycling,
    };
    // A pending report, or one a rule marked bad whose reporter asked for a
    // recycle that an operator now confirms.
    let review_args = vec![
        SqlArg::OptText(confirm.reviewer_user_id.map(str::to_string)),
        now.clone(),
        SqlArg::Text(confirm.remediation.as_str().to_string()),
        SqlArg::Text(execution_status.as_str().to_string()),
        SqlArg::OptText(confirm.recycle_fingerprint.map(str::to_string)),
        now.clone(),
        SqlArg::Text(confirm.issue_id.to_string()),
        SqlArg::Text(confirm.remediation.as_str().to_string()),
    ];
    let release = confirm.release.and_then(|entry| {
        let normalized = normalize_release_name(Some(&entry.release_name))?;
        Some((
            entry.release_name.trim().to_string(),
            normalized,
            entry.indexer_id.trim().to_string(),
            normalize_info_hash(entry.info_hash.as_deref()),
        ))
    });
    let title_id = confirm.title_id.to_string();
    let content = confirm.content.clone();
    let reason = confirm.reason.to_string();
    let blocks_imports = confirm.blocks_imports;
    let issue_id = confirm.issue_id.to_string();
    let file_ids = confirm.matching_media_file_ids.to_vec();
    SqlRuntime::run_in_transaction(datastore, "confirm_media_issue", move |tx| {
        let review_args = review_args.clone();
        let release = release.clone();
        let title_id = title_id.clone();
        let content = content.clone();
        let reason = reason.clone();
        let issue_id = issue_id.clone();
        let file_ids = file_ids.clone();
        let now = now.clone();
        Box::pin(async move {
            let reviewed = SqlRuntime::execute(
                SqlExec::Tx(tx),
                "UPDATE media_issues
                    SET status = 'approved',
                        reviewed_by_user_id = COALESCE({}, reviewed_by_user_id),
                        reviewed_at = {},
                        authorized_remediation = {},
                        execution_status = {},
                        recycle_preview_fingerprint = {},
                        updated_at = {}
                  WHERE id = {}
                    AND (status = 'pending'
                         OR (status = 'approved'
                             AND {} = 'recycle_and_search'
                             AND remediation = 'recycle_and_search'
                             AND authorized_remediation = 'mark_bad'
                             AND execution_status = 'blocked'))",
                &review_args,
            )
            .await?;
            if reviewed == 0 {
                return Ok(false);
            }

            // The entry that already carries this content, else the grab's
            // release entry when it carries no content yet, else a new row.
            let mut lookup = identity_args(&title_id, &content);
            let existing = SqlRuntime::fetch_optional(
                SqlExec::Tx(tx),
                &format!("SELECT id FROM blocklist WHERE {CONTENT_IDENTITY_PREDICATE}"),
                &lookup,
            )
            .await?
            .map(|row| row.text("id"))
            .transpose()?;
            let blocklist_id = match existing {
                Some(id) => id,
                None => {
                    let release_row = match &release {
                        Some((_, normalized, indexer_id, info_hash)) => {
                            let (sql, args) = match info_hash {
                                Some(hash) => (
                                    "SELECT id, content_sample_blake3 FROM blocklist
                                      WHERE title_id = {} AND info_hash = {}",
                                    vec![
                                        SqlArg::Text(title_id.clone()),
                                        SqlArg::Text(hash.clone()),
                                    ],
                                ),
                                None => (
                                    "SELECT id, content_sample_blake3 FROM blocklist
                                      WHERE title_id = {} AND indexer_id = {}
                                        AND normalized_release_name = {}
                                        AND info_hash IS NULL AND release_name IS NOT NULL",
                                    vec![
                                        SqlArg::Text(title_id.clone()),
                                        SqlArg::Text(indexer_id.clone()),
                                        SqlArg::Text(normalized.clone()),
                                    ],
                                ),
                            };
                            SqlRuntime::fetch_optional(SqlExec::Tx(tx), sql, &args).await?
                        }
                        None => None,
                    };
                    match release_row {
                        Some(row) if row.opt_text("content_sample_blake3")?.is_none() => {
                            let id = row.text("id")?;
                            let mut args = vec![
                                SqlArg::Text(content.scheme.clone()),
                                SqlArg::I64(to_i64(content.size_bytes)),
                                SqlArg::I64(to_i64(content.sample_bytes)),
                                SqlArg::Text(content.sample_blake3.clone()),
                                SqlArg::Bool(blocks_imports),
                            ];
                            args.push(SqlArg::Text(id.clone()));
                            SqlRuntime::execute(
                                SqlExec::Tx(tx),
                                "UPDATE blocklist
                                    SET content_scheme = {}, content_size_bytes = {},
                                        content_sample_bytes = {}, content_sample_blake3 = {},
                                        content_blocks_imports = {}
                                  WHERE id = {}",
                                &args,
                            )
                            .await?;
                            id
                        }
                        // A release entry that already carries another
                        // file's content keeps it; this file gets its own
                        // content-only entry. With no entry yet, one row
                        // carries both identities.
                        found => {
                            let id = Id::new().0;
                            let named = if found.is_none() { release.clone() } else { None };
                            let (release_name, normalized, indexer_id, info_hash) = named
                                .map(|(name, normalized, indexer, hash)| {
                                    (Some(name), normalized, indexer, hash)
                                })
                                .unwrap_or((None, String::new(), String::new(), None));
                            let inserted = SqlRuntime::execute(
                                SqlExec::Tx(tx),
                                "INSERT INTO blocklist
                                     (id, title_id, release_name, normalized_release_name,
                                      indexer_id, info_hash, reason, created_at, content_scheme,
                                      content_size_bytes, content_sample_bytes,
                                      content_sample_blake3, content_blocks_imports)
                                 VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})
                                 ON CONFLICT DO NOTHING",
                                &[
                                    SqlArg::Text(id.clone()),
                                    SqlArg::Text(title_id.clone()),
                                    SqlArg::OptText(release_name),
                                    SqlArg::Text(normalized),
                                    SqlArg::Text(indexer_id),
                                    SqlArg::OptText(info_hash),
                                    SqlArg::Text(reason.clone()),
                                    now.clone(),
                                    SqlArg::Text(content.scheme.clone()),
                                    SqlArg::I64(to_i64(content.size_bytes)),
                                    SqlArg::I64(to_i64(content.sample_bytes)),
                                    SqlArg::Text(content.sample_blake3.clone()),
                                    SqlArg::Bool(blocks_imports),
                                ],
                            )
                            .await?;
                            if inserted > 0 {
                                id
                            } else {
                                // A concurrent mark wrote this entry first;
                                // use the row it wrote.
                                SqlRuntime::fetch_optional(
                                    SqlExec::Tx(tx),
                                    &format!(
                                        "SELECT id FROM blocklist \
                                          WHERE {CONTENT_IDENTITY_PREDICATE}"
                                    ),
                                    &lookup,
                                )
                                .await?
                                .map(|row| row.text("id"))
                                .transpose()?
                                .ok_or_else(|| {
                                    AppError::Repository(
                                        "the title's blocklist changed while the file was \
                                         being marked bad; retry"
                                            .into(),
                                    )
                                })?
                            }
                        }
                    }
                }
            };
            lookup.clear();
            // A report that gates imports turns the gate on for an entry a
            // wrong-content report wrote first; nothing turns it off.
            if blocks_imports {
                SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "UPDATE blocklist SET content_blocks_imports = {}
                      WHERE id = {} AND content_blocks_imports = {}",
                    &[
                        SqlArg::Bool(true),
                        SqlArg::Text(blocklist_id.clone()),
                        SqlArg::Bool(false),
                    ],
                )
                .await?;
            }

            let (ids, per_statement) =
                id_list_statements(&file_ids, 1, 2, max_binds(&SqlExec::Tx(tx)), ID_CHUNK);
            let mut stamped = 0u64;
            for chunk in ids.chunks(per_statement) {
                let sql = format!(
                    "UPDATE media_files SET bad_file_blocklist_id = {{}}
                      WHERE title_id = {{}} AND id IN ({})",
                    placeholders(chunk.len())
                );
                let mut args = vec![
                    SqlArg::Text(blocklist_id.clone()),
                    SqlArg::Text(title_id.clone()),
                ];
                args.extend(chunk.iter().cloned().map(SqlArg::Text));
                stamped += SqlRuntime::execute(SqlExec::Tx(tx), &sql, &args).await?;
            }
            if stamped == 0 {
                return Err(AppError::validation_refused(
                    scryer_application::error_reasons::media::REGISTERED_MEDIA_PATH_CHANGED_BEFORE_INSPECTION,
                    "the reported file changed before approval",
                ));
            }
            SqlRuntime::execute(
                SqlExec::Tx(tx),
                "UPDATE media_issues SET blocklist_id = {} WHERE id = {}",
                &[SqlArg::Text(blocklist_id), SqlArg::Text(issue_id)],
            )
            .await?;
            Ok(true)
        })
    })
    .await
}

pub(super) async fn reject_media_issue(
    datastore: &StoreDatastore,
    id: &str,
    reviewer_user_id: Option<&str>,
) -> AppResult<Option<MediaIssue>> {
    let now = datastore.timestamp_text_arg(&Utc::now().to_rfc3339())?;
    SqlRuntime::execute_write(
        datastore,
        "reject_media_issue",
        "UPDATE media_issues
            SET status = 'rejected', reviewed_by_user_id = {}, reviewed_at = {},
                updated_at = {}
          WHERE id = {} AND status = 'pending'",
        vec![
            SqlArg::OptText(reviewer_user_id.map(str::to_string)),
            now.clone(),
            now,
            SqlArg::Text(id.to_string()),
        ],
    )
    .await?;
    get_media_issue(datastore, id).await
}

pub(super) async fn set_media_issue_execution_status(
    datastore: &StoreDatastore,
    id: &str,
    status: MediaIssueExecutionStatus,
    error_code: Option<&str>,
) -> AppResult<bool> {
    if status == MediaIssueExecutionStatus::ReplacementImported {
        return Err(AppError::Repository(
            "replacement_imported is derived when reports are read".into(),
        ));
    }
    let now = datastore.timestamp_text_arg(&Utc::now().to_rfc3339())?;
    // Writing the same status and error again changes nothing.
    let changed = SqlRuntime::execute_write(
        datastore,
        "set_media_issue_execution_status",
        "UPDATE media_issues
            SET execution_status = {}, execution_error_code = {}, updated_at = {}
          WHERE id = {}
            AND NOT (execution_status = {} AND COALESCE(execution_error_code, '') = {})",
        vec![
            SqlArg::Text(status.as_str().to_string()),
            SqlArg::OptText(error_code.map(str::to_string)),
            now,
            SqlArg::Text(id.to_string()),
            SqlArg::Text(status.as_str().to_string()),
            SqlArg::Text(error_code.unwrap_or_default().to_string()),
        ],
    )
    .await?;
    Ok(changed > 0)
}

pub(super) async fn interrupted_media_issue_recycles(
    datastore: &StoreDatastore,
    limit: usize,
) -> AppResult<Vec<MediaIssueRecycleIntent>> {
    // `idx_media_issues_recycle_recovery` serves this; with nothing to
    // resume it is an empty index read.
    SqlRuntime::fetch_all(
        datastore.read_exec(),
        "SELECT id, reviewed_by_user_id, recycle_preview_fingerprint
           FROM media_issues
          WHERE execution_status = 'recycling'
            AND status = 'approved'
            AND authorized_remediation = 'recycle_and_search'
            AND recycle_preview_fingerprint IS NOT NULL
            AND reviewed_by_user_id IS NOT NULL
          ORDER BY updated_at, id
          LIMIT {}",
        &[SqlArg::I64(limit.clamp(1, 1_000) as i64)],
    )
    .await?
    .iter()
    .map(|row| {
        Ok(MediaIssueRecycleIntent {
            issue_id: row.text("id")?,
            actor_id: row.text("reviewed_by_user_id")?,
            preview_fingerprint: row.text("recycle_preview_fingerprint")?,
        })
    })
    .collect()
}

pub(super) async fn save_media_issue_recycle_receipt(
    datastore: &StoreDatastore,
    id: &str,
    receipt_json: &str,
) -> AppResult<()> {
    let written = SqlRuntime::execute_write(
        datastore,
        "save_media_issue_recycle_receipt",
        "UPDATE media_issues SET recycle_receipt_json = {}
          WHERE id = {} AND status = 'approved' AND execution_status = 'recycling'",
        vec![
            SqlArg::Text(receipt_json.to_string()),
            SqlArg::Text(id.to_string()),
        ],
    )
    .await?;
    if written == 0 {
        // The report stopped being an authorized recycle; nothing may move.
        return Err(AppError::validation_refused(
            scryer_application::error_reasons::media::REMEDIATION_NOT_AVAILABLE,
            "the report is no longer an authorized recycle",
        ));
    }
    Ok(())
}

pub(super) async fn get_media_issue_recycle_receipt(
    datastore: &StoreDatastore,
    id: &str,
) -> AppResult<Option<String>> {
    Ok(SqlRuntime::fetch_optional(
        datastore.read_exec(),
        "SELECT recycle_receipt_json FROM media_issues
          WHERE id = {} AND recycle_receipt_json IS NOT NULL",
        &[SqlArg::Text(id.to_string())],
    )
    .await?
    .map(|row| row.text("recycle_receipt_json"))
    .transpose()?)
}

pub(super) async fn get_media_issue_recycle_fingerprint(
    datastore: &StoreDatastore,
    id: &str,
) -> AppResult<Option<String>> {
    Ok(SqlRuntime::fetch_optional(
        datastore.read_exec(),
        "SELECT recycle_preview_fingerprint FROM media_issues
          WHERE id = {} AND recycle_preview_fingerprint IS NOT NULL",
        &[SqlArg::Text(id.to_string())],
    )
    .await?
    .map(|row| row.text("recycle_preview_fingerprint"))
    .transpose()?)
}

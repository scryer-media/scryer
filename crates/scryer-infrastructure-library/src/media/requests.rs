use async_trait::async_trait;
use chrono::Utc;
use scryer_application::{
    AppError, AppResult, MediaRequestCounts, MediaRequestQualityProfileReferenceCounts,
    MediaRequestQuery, MediaRequestRepository, MediaRequestResolution,
    MediaRequestResolutionResult, MediaRequestSubmissionResult, MediaRequestUpdateResult,
    NewMediaRequest,
};
use scryer_domain::{
    ExternalId, MediaFacet, MediaRequest, MediaRequestRequester, MediaRequestStatus,
    MonitorSelection, NewDomainEvent, User,
};

use crate::media::monitor_selections::{
    OWNER_KIND_MEDIA_REQUEST, load_monitor_selection, load_monitor_selections_for_owners,
    replace_monitor_selection_tx,
};
use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRow, SqlRuntime, SqlTx, StoreDatastore};
use crate::storage::sql::json::{canonical_json_text, json_text_or};
use crate::workflow::stores::append_domain_event_tx;

#[derive(Clone)]
pub struct MediaRequestStore {
    datastore: StoreDatastore,
}

impl MediaRequestStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }
}

#[async_trait]
impl MediaRequestRepository for MediaRequestStore {
    async fn count_quality_profile_references(
        &self,
        profile_id: &str,
    ) -> AppResult<MediaRequestQualityProfileReferenceCounts> {
        let profile_id = profile_id.trim();
        if profile_id.is_empty() {
            return Ok(MediaRequestQualityProfileReferenceCounts::default());
        }
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT COUNT(*) AS pending_requested_count
               FROM media_requests
              WHERE status = 'pending'
                AND LOWER(requested_quality_profile_id) = LOWER({})",
            &[SqlArg::Text(profile_id.to_string())],
        )
        .await?;
        let Some(row) = row else {
            return Ok(MediaRequestQualityProfileReferenceCounts::default());
        };
        Ok(MediaRequestQualityProfileReferenceCounts {
            pending_requested: row.i64("pending_requested_count")?.max(0) as u64,
        })
    }

    async fn submit(
        &self,
        request: NewMediaRequest,
        requester: &User,
        submitted_event: NewDomainEvent,
    ) -> AppResult<MediaRequestSubmissionResult> {
        let requester = requester.clone();
        SqlRuntime::run_in_transaction(&self.datastore, "submit_media_request", move |tx| {
            let request = request.clone();
            let requester = requester.clone();
            let submitted_event = submitted_event.clone();
            Box::pin(async move {
                let now = Utc::now();
                insert_media_request_tx(tx, &request, now).await?;
                insert_media_request_external_ids_tx(
                    tx,
                    &request.id,
                    &request.library_id,
                    &request.external_ids,
                    now,
                )
                .await?;
                insert_media_request_requester_tx(tx, &request.id, &requester.id, now).await?;
                replace_monitor_selection_tx(
                    tx,
                    OWNER_KIND_MEDIA_REQUEST,
                    &request.id,
                    request.requested_monitor_selection.as_ref(),
                )
                .await?;
                let event = append_domain_event_tx(tx, submitted_event).await?;

                let request = load_media_request_tx(tx, &request.id)
                    .await?
                    .ok_or_else(|| AppError::NotFound(format!("media request {}", request.id)))?;
                Ok(MediaRequestSubmissionResult { request, event })
            })
        })
        .await
    }

    /// Two reads rather than a UNION: the submitter and the additional
    /// requesters live in different tables with different orderings, and
    /// merging them in Rust keeps the SQL dialect-neutral instead of relying on
    /// SQLite and Postgres agreeing on how a UNION orders mixed timestamp
    /// columns.
    async fn requester_user_ids_by_title_ids(
        &self,
        title_ids: &[String],
    ) -> AppResult<std::collections::HashMap<String, Vec<String>>> {
        let mut by_title: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        if title_ids.is_empty() {
            return Ok(by_title);
        }

        let placeholders = std::iter::repeat_n("{}", title_ids.len())
            .collect::<Vec<_>>()
            .join(", ");
        let args: Vec<SqlArg> = title_ids.iter().cloned().map(SqlArg::Text).collect();

        // The submitter comes first for every request, so a title with a
        // request is a key here even when it has no extra requester rows.
        let submitters = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            &format!(
                "SELECT created_title_id, created_by_user_id
                   FROM media_requests
                  WHERE created_title_id IN ({placeholders})
                  ORDER BY created_at ASC, id ASC"
            ),
            &args,
        )
        .await?;
        for row in &submitters {
            by_title
                .entry(row.text("created_title_id")?)
                .or_default()
                .push(row.text("created_by_user_id")?);
        }

        let requesters = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            &format!(
                "SELECT mr.created_title_id AS created_title_id, mrr.user_id
                   FROM media_request_requesters mrr
                   JOIN media_requests mr ON mr.id = mrr.request_id
                  WHERE mr.created_title_id IN ({placeholders})
                  ORDER BY mrr.requested_at ASC, mrr.user_id ASC"
            ),
            &args,
        )
        .await?;
        for row in &requesters {
            by_title
                .entry(row.text("created_title_id")?)
                .or_default()
                .push(row.text("user_id")?);
        }

        for user_ids in by_title.values_mut() {
            let mut seen = std::collections::HashSet::new();
            user_ids.retain(|user_id| seen.insert(user_id.clone()));
        }
        Ok(by_title)
    }

    async fn get(&self, request_id: &str) -> AppResult<Option<MediaRequest>> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &format!(
                "SELECT {REQUEST_COLUMNS}
                   FROM media_requests
                  WHERE id = {{}}"
            ),
            &[SqlArg::Text(request_id.to_string())],
        )
        .await?;

        let Some(row) = row else {
            return Ok(None);
        };

        let mut request = row_to_media_request(&row)?;
        request.external_ids =
            load_media_request_external_ids(self.datastore.read_exec(), &request.id).await?;
        request.requesters =
            load_media_request_requesters(self.datastore.read_exec(), &request.id).await?;
        request.requested_monitor_selection = load_monitor_selection(
            self.datastore.read_exec(),
            OWNER_KIND_MEDIA_REQUEST,
            &request.id,
        )
        .await?;
        Ok(Some(request))
    }

    async fn resolve_pending_overlapping(
        &self,
        request: &MediaRequest,
        resolution: MediaRequestResolution,
    ) -> AppResult<MediaRequestResolutionResult> {
        let request = request.clone();
        let resolution = resolution.clone();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "resolve_pending_media_request_group",
            move |tx| {
                let request = request.clone();
                let resolution = resolution.clone();
                Box::pin(
                    async move { resolve_pending_overlapping_tx(tx, &request, resolution).await },
                )
            },
        )
        .await
    }

    async fn resolve_pending(
        &self,
        request_id: &str,
        resolution: MediaRequestResolution,
    ) -> AppResult<MediaRequestResolutionResult> {
        let request_id = request_id.to_string();
        let resolution = resolution.clone();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "resolve_pending_media_request",
            move |tx| {
                let request_id = request_id.clone();
                let resolution = resolution.clone();
                Box::pin(async move { resolve_pending_tx(tx, &request_id, resolution).await })
            },
        )
        .await
    }

    async fn update_pending_request_preferences(
        &self,
        request_id: &str,
        requested_quality_profile_id: String,
        requested_quality_profile_name: String,
        requested_monitor_type: Option<String>,
        requested_monitor_selection: Option<MonitorSelection>,
        requested_lease_days: Option<i64>,
        updated_event: NewDomainEvent,
    ) -> AppResult<MediaRequestUpdateResult> {
        let request_id = request_id.to_string();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "update_pending_media_request_preferences",
            move |tx| {
                let request_id = request_id.clone();
                let requested_quality_profile_id = requested_quality_profile_id.clone();
                let requested_quality_profile_name = requested_quality_profile_name.clone();
                let requested_monitor_type = requested_monitor_type.clone();
                let requested_monitor_selection = requested_monitor_selection.clone();
                let updated_event = updated_event.clone();
                Box::pin(async move {
                    update_pending_request_preferences_tx(
                        tx,
                        &request_id,
                        requested_quality_profile_id,
                        requested_quality_profile_name,
                        requested_monitor_type,
                        requested_monitor_selection,
                        requested_lease_days,
                        updated_event,
                    )
                    .await
                })
            },
        )
        .await
    }

    async fn count_pending_by_facet(
        &self,
        library_ids: &[String],
    ) -> AppResult<MediaRequestCounts> {
        if library_ids.is_empty() {
            return Ok(MediaRequestCounts::default());
        }

        let placeholders = std::iter::repeat_n("{}", library_ids.len())
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT facet, COUNT(DISTINCT library_id || ':' || identity_fingerprint) AS count
               FROM media_requests
              WHERE status = {{}} AND library_id IN ({placeholders})
              GROUP BY facet"
        );
        let mut args = vec![SqlArg::Text(
            MediaRequestStatus::Pending.as_str().to_string(),
        )];
        args.extend(library_ids.iter().cloned().map(SqlArg::Text));

        let rows = SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args).await?;
        let mut counts = MediaRequestCounts::default();
        for row in rows {
            let facet_raw = row.text("facet")?;
            let count = row.i64("count")?;
            match MediaFacet::parse(&facet_raw) {
                Some(MediaFacet::Movie) => counts.movie = count,
                Some(MediaFacet::Series) => counts.series = count,
                Some(MediaFacet::Anime) => counts.anime = count,
                None => {
                    return Err(AppError::Repository(format!(
                        "unknown media request facet {facet_raw}"
                    )));
                }
            }
        }
        Ok(counts)
    }

    async fn list(&self, query: MediaRequestQuery) -> AppResult<Vec<MediaRequest>> {
        if matches!(&query.library_ids, Some(library_ids) if library_ids.is_empty()) {
            return Ok(Vec::new());
        }

        let (sql, args) = build_media_request_list_sql(&query);
        let rows = SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args).await?;
        let mut requests = Vec::with_capacity(rows.len());
        for row in rows {
            let mut request = row_to_media_request(&row)?;
            request.external_ids =
                load_media_request_external_ids(self.datastore.read_exec(), &request.id).await?;
            request.requesters =
                load_media_request_requesters(self.datastore.read_exec(), &request.id).await?;
            requests.push(request);
        }
        // One query for the whole page rather than one per request.
        let owner_ids = requests
            .iter()
            .map(|request| request.id.clone())
            .collect::<Vec<_>>();
        let mut selections = load_monitor_selections_for_owners(
            self.datastore.read_exec(),
            OWNER_KIND_MEDIA_REQUEST,
            &owner_ids,
        )
        .await?;
        for request in &mut requests {
            request.requested_monitor_selection = selections.remove(&request.id);
        }
        Ok(requests)
    }

    /// Counts submissions, not joins: `created_by_user_id` is who asked, and a
    /// user who seconded someone else's request did not ask for it.
    async fn count_for_requester(
        &self,
        user_id: &str,
        status: Option<MediaRequestStatus>,
        since: Option<chrono::DateTime<Utc>>,
    ) -> AppResult<u64> {
        let mut sql = String::from(
            "SELECT COUNT(*) AS request_count
               FROM media_requests
              WHERE created_by_user_id = {}",
        );
        let mut args = vec![SqlArg::Text(user_id.to_string())];
        if let Some(status) = status {
            sql.push_str(" AND status = {}");
            args.push(SqlArg::Text(status.as_str().to_string()));
        }
        if let Some(since) = since {
            sql.push_str(" AND created_at >= {}");
            args.push(SqlArg::Timestamp(since));
        }
        let row = SqlRuntime::fetch_optional(self.datastore.read_exec(), &sql, &args).await?;
        let Some(row) = row else {
            return Ok(0);
        };
        Ok(row.i64("request_count")?.max(0) as u64)
    }

    /// Every status, every requester: the identity-history facts ask what has
    /// been asked for this title before, and a previous denial is exactly the
    /// row a status filter would hide.
    async fn history_for_fingerprint(
        &self,
        identity_fingerprint: &str,
    ) -> AppResult<Vec<MediaRequest>> {
        let sql = format!(
            "SELECT {REQUEST_COLUMNS}
               FROM media_requests
              WHERE identity_fingerprint = {{}}
              ORDER BY created_at DESC, id DESC"
        );
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::Text(identity_fingerprint.to_string())],
        )
        .await?;
        let mut requests = Vec::with_capacity(rows.len());
        for row in rows {
            let mut request = row_to_media_request(&row)?;
            request.external_ids =
                load_media_request_external_ids(self.datastore.read_exec(), &request.id).await?;
            requests.push(request);
        }
        Ok(requests)
    }

    async fn latest_request_at_for_user(
        &self,
        user_id: &str,
    ) -> AppResult<Option<chrono::DateTime<Utc>>> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT created_at
               FROM media_requests
              WHERE created_by_user_id = {}
              ORDER BY created_at DESC, id DESC
              LIMIT 1",
            &[SqlArg::Text(user_id.to_string())],
        )
        .await?;
        row.map(|row| row.timestamp("created_at")).transpose()
    }

    async fn record_decision_on_request(
        &self,
        request_id: &str,
        decision_id: Option<&str>,
        rule_set_ids: &[String],
        tags: &[String],
    ) -> AppResult<()> {
        SqlRuntime::execute_write(
            &self.datastore,
            "record_media_request_decision",
            "UPDATE media_requests
                SET decision_id = {},
                    decided_by_rule_set_ids = {},
                    policy_tags_json = {},
                    updated_at = {}
              WHERE id = {}",
            vec![
                SqlArg::OptText(decision_id.map(str::to_string)),
                SqlArg::Text(json_array_text(rule_set_ids)?),
                SqlArg::Text(json_array_text(tags)?),
                SqlArg::Timestamp(Utc::now()),
                SqlArg::Text(request_id.to_string()),
            ],
        )
        .await?;
        Ok(())
    }
}

/// Every read of `media_requests` selects the same column list, so the policy
/// columns 0220 added cannot be picked up by one path and missed by another.
const REQUEST_COLUMNS: &str = "id, library_id, facet, status, identity_fingerprint, title,
        sort_title, slug, poster_url, background_url, year, overview, runtime_minutes, language,
        content_status, rating_summary_json,
        requested_quality_profile_id, requested_quality_profile_name, requested_monitor_type,
        resolved_by_user_id, resolved_at, created_title_id,
        approved_quality_profile_id, approved_quality_profile_name,
        requested_lease_days, approved_lease_days, decision_id, decided_by_rule_set_ids,
        policy_tags_json, metadata_snapshot_json,
        created_by_user_id, created_at, updated_at";

async fn insert_media_request_tx(
    tx: &mut SqlTx<'_>,
    request: &NewMediaRequest,
    now: chrono::DateTime<Utc>,
) -> AppResult<()> {
    let rating_summary_json = serde_json::to_string(&request.rating_summary).map_err(|error| {
        AppError::Repository(format!("serialize media request rating summary: {error}"))
    })?;
    // The column is NOT NULL with a `{}` default; an empty snapshot string from
    // a caller that captured nothing must land as that same empty object rather
    // than as invalid JSON.
    let metadata_snapshot_json = if request.metadata_snapshot_json.trim().is_empty() {
        "{}".to_string()
    } else {
        request.metadata_snapshot_json.clone()
    };
    tx.execute(
        "INSERT INTO media_requests (
            id, library_id, facet, status, identity_fingerprint, title, sort_title, slug,
            poster_url, background_url, year, overview, runtime_minutes, language, content_status,
            rating_summary_json,
            requested_quality_profile_id, requested_quality_profile_name,
            requested_monitor_type,
            requested_lease_days, metadata_snapshot_json,
            created_by_user_id, created_at, updated_at
        ) VALUES (
            {}, {}, {}, {}, {}, {}, {}, {},
            {}, {}, {}, {}, {}, {}, {},
            {},
            {}, {}, {},
            {}, {},
            {}, {}, {}
        )",
        &[
            SqlArg::Text(request.id.clone()),
            SqlArg::Text(request.library_id.clone()),
            SqlArg::Text(request.facet.as_str().to_string()),
            SqlArg::Text(MediaRequestStatus::Pending.as_str().to_string()),
            SqlArg::Text(request.identity_fingerprint.clone()),
            SqlArg::Text(request.title.clone()),
            SqlArg::OptText(request.sort_title.clone()),
            SqlArg::OptText(request.slug.clone()),
            SqlArg::OptText(request.poster_url.clone()),
            SqlArg::OptText(request.background_url.clone()),
            SqlArg::OptI32(request.year),
            SqlArg::OptText(request.overview.clone()),
            SqlArg::OptI32(request.runtime_minutes),
            SqlArg::OptText(request.language.clone()),
            SqlArg::OptText(request.content_status.clone()),
            SqlArg::Text(rating_summary_json),
            SqlArg::OptText(request.requested_quality_profile_id.clone()),
            SqlArg::OptText(request.requested_quality_profile_name.clone()),
            SqlArg::OptText(request.requested_monitor_type.clone()),
            SqlArg::OptI64(request.requested_lease_days),
            SqlArg::Text(metadata_snapshot_json),
            SqlArg::Text(request.created_by_user_id.clone()),
            SqlArg::Timestamp(now),
            SqlArg::Timestamp(now),
        ],
    )
    .await?;
    Ok(())
}

async fn insert_media_request_external_ids_tx(
    tx: &mut SqlTx<'_>,
    request_id: &str,
    library_id: &str,
    external_ids: &[ExternalId],
    now: chrono::DateTime<Utc>,
) -> AppResult<()> {
    for external_id in external_ids {
        tx.execute(
            "INSERT INTO media_request_external_ids (
                request_id, library_id, source, external_id, created_at
            ) VALUES ({}, {}, {}, {}, {})
            ON CONFLICT (request_id, source, external_id) DO NOTHING",
            &[
                SqlArg::Text(request_id.to_string()),
                SqlArg::Text(library_id.to_string()),
                SqlArg::Text(external_id.source.clone()),
                SqlArg::Text(external_id.value.clone()),
                SqlArg::Timestamp(now),
            ],
        )
        .await?;
    }
    Ok(())
}

async fn insert_media_request_requester_tx(
    tx: &mut SqlTx<'_>,
    request_id: &str,
    user_id: &str,
    requested_at: chrono::DateTime<Utc>,
) -> AppResult<bool> {
    let rows = tx
        .execute(
            "INSERT INTO media_request_requesters (request_id, user_id, requested_at)
             VALUES ({}, {}, {})
             ON CONFLICT (request_id, user_id) DO NOTHING",
            &[
                SqlArg::Text(request_id.to_string()),
                SqlArg::Text(user_id.to_string()),
                SqlArg::Timestamp(requested_at),
            ],
        )
        .await?;
    Ok(rows > 0)
}

async fn resolve_pending_overlapping_tx(
    tx: &mut SqlTx<'_>,
    request: &MediaRequest,
    resolution: MediaRequestResolution,
) -> AppResult<MediaRequestResolutionResult> {
    let resolved_at = resolution.resolved_at;
    let mut args = vec![
        SqlArg::Text(resolution.status.as_str().to_string()),
        SqlArg::OptText(resolution.resolved_by_user_id),
        SqlArg::Timestamp(resolved_at),
        SqlArg::OptText(resolution.created_title_id),
        SqlArg::OptText(resolution.approved_quality_profile_id),
        SqlArg::OptText(resolution.approved_quality_profile_name),
        SqlArg::OptI64(resolution.approved_lease_days),
        SqlArg::OptText(resolution.decision_id),
        SqlArg::Text(json_array_text(&resolution.decided_by_rule_set_ids)?),
        SqlArg::Text(json_array_text(&resolution.policy_tags)?),
        SqlArg::Timestamp(resolved_at),
    ];

    let where_clause = if request.external_ids.is_empty() {
        args.push(SqlArg::Text(request.id.clone()));
        args.push(SqlArg::Text(
            MediaRequestStatus::Pending.as_str().to_string(),
        ));
        "id = {} AND status = {}".to_string()
    } else {
        args.push(SqlArg::Text(
            MediaRequestStatus::Pending.as_str().to_string(),
        ));
        args.push(SqlArg::Text(request.library_id.clone()));
        args.push(SqlArg::Text(request.facet.as_str().to_string()));
        args.push(SqlArg::Text(request.library_id.clone()));
        let overlap_clauses = std::iter::repeat_n(
            "(source = {} AND external_id = {})",
            request.external_ids.len(),
        )
        .collect::<Vec<_>>()
        .join(" OR ");
        for external_id in &request.external_ids {
            args.push(SqlArg::Text(external_id.source.clone()));
            args.push(SqlArg::Text(external_id.value.clone()));
        }
        format!(
            "status = {{}}
             AND library_id = {{}}
             AND facet = {{}}
             AND id IN (
                 SELECT DISTINCT request_id
                   FROM media_request_external_ids
                  WHERE library_id = {{}}
                    AND ({overlap_clauses})
             )"
        )
    };

    let sql = format!(
        "UPDATE media_requests
            SET status = {{}},
                resolved_by_user_id = {{}},
                resolved_at = {{}},
                created_title_id = {{}},
                approved_quality_profile_id = {{}},
                approved_quality_profile_name = {{}},
                approved_lease_days = {{}},
                decision_id = {{}},
                decided_by_rule_set_ids = {{}},
                policy_tags_json = {{}},
                updated_at = {{}}
          WHERE {where_clause}"
    );
    let rows = tx.execute(&sql, &args).await?;
    let event = if rows > 0 {
        Some(append_domain_event_tx(tx, resolution.event).await?)
    } else {
        None
    };
    Ok(MediaRequestResolutionResult {
        updated: rows,
        event,
    })
}

async fn resolve_pending_tx(
    tx: &mut SqlTx<'_>,
    request_id: &str,
    resolution: MediaRequestResolution,
) -> AppResult<MediaRequestResolutionResult> {
    let resolved_at = resolution.resolved_at;
    let rows = tx
        .execute(
            "UPDATE media_requests
                SET status = {},
                    resolved_by_user_id = {},
                    resolved_at = {},
                    created_title_id = {},
                    approved_quality_profile_id = {},
                    approved_quality_profile_name = {},
                    approved_lease_days = {},
                    decision_id = {},
                    decided_by_rule_set_ids = {},
                    policy_tags_json = {},
                    updated_at = {}
              WHERE id = {} AND status = {}",
            &[
                SqlArg::Text(resolution.status.as_str().to_string()),
                SqlArg::OptText(resolution.resolved_by_user_id),
                SqlArg::Timestamp(resolved_at),
                SqlArg::OptText(resolution.created_title_id),
                SqlArg::OptText(resolution.approved_quality_profile_id),
                SqlArg::OptText(resolution.approved_quality_profile_name),
                SqlArg::OptI64(resolution.approved_lease_days),
                SqlArg::OptText(resolution.decision_id),
                SqlArg::Text(json_array_text(&resolution.decided_by_rule_set_ids)?),
                SqlArg::Text(json_array_text(&resolution.policy_tags)?),
                SqlArg::Timestamp(resolved_at),
                SqlArg::Text(request_id.to_string()),
                SqlArg::Text(MediaRequestStatus::Pending.as_str().to_string()),
            ],
        )
        .await?;
    let event = if rows > 0 {
        Some(append_domain_event_tx(tx, resolution.event).await?)
    } else {
        None
    };
    Ok(MediaRequestResolutionResult {
        updated: rows,
        event,
    })
}

// One parameter per column the edit writes, mirroring the port method it
// implements; a struct here would only rename the same eight values.
#[allow(clippy::too_many_arguments)]
async fn update_pending_request_preferences_tx(
    tx: &mut SqlTx<'_>,
    request_id: &str,
    requested_quality_profile_id: String,
    requested_quality_profile_name: String,
    requested_monitor_type: Option<String>,
    requested_monitor_selection: Option<MonitorSelection>,
    requested_lease_days: Option<i64>,
    updated_event: NewDomainEvent,
) -> AppResult<MediaRequestUpdateResult> {
    let now = Utc::now();
    let rows = tx
        .execute(
            "UPDATE media_requests
                SET requested_quality_profile_id = {},
                    requested_quality_profile_name = {},
                    requested_monitor_type = {},
                    requested_lease_days = {},
                    updated_at = {}
              WHERE id = {} AND status = {}",
            &[
                SqlArg::Text(requested_quality_profile_id),
                SqlArg::Text(requested_quality_profile_name),
                SqlArg::OptText(requested_monitor_type),
                SqlArg::OptI64(requested_lease_days),
                SqlArg::Timestamp(now),
                SqlArg::Text(request_id.to_string()),
                SqlArg::Text(MediaRequestStatus::Pending.as_str().to_string()),
            ],
        )
        .await?;
    if rows == 0 {
        return Err(AppError::Validation(
            "media request is no longer pending".into(),
        ));
    }

    replace_monitor_selection_tx(
        tx,
        OWNER_KIND_MEDIA_REQUEST,
        request_id,
        requested_monitor_selection.as_ref(),
    )
    .await?;

    let event = append_domain_event_tx(tx, updated_event).await?;
    let request = load_media_request_tx(tx, request_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("media request {request_id}")))?;
    Ok(MediaRequestUpdateResult { request, event })
}

async fn load_media_request_tx(
    tx: &mut SqlTx<'_>,
    request_id: &str,
) -> AppResult<Option<MediaRequest>> {
    let row = SqlRuntime::fetch_optional(
        SqlExec::Tx(tx),
        &format!(
            "SELECT {REQUEST_COLUMNS}
               FROM media_requests
              WHERE id = {{}}"
        ),
        &[SqlArg::Text(request_id.to_string())],
    )
    .await?;

    let Some(row) = row else {
        return Ok(None);
    };

    let mut request = row_to_media_request(&row)?;
    request.external_ids = load_media_request_external_ids(SqlExec::Tx(tx), request_id).await?;
    request.requesters = load_media_request_requesters(SqlExec::Tx(tx), request_id).await?;
    request.requested_monitor_selection =
        load_monitor_selection(SqlExec::Tx(tx), OWNER_KIND_MEDIA_REQUEST, request_id).await?;
    Ok(Some(request))
}

async fn load_media_request_external_ids(
    exec: SqlExec<'_, '_>,
    request_id: &str,
) -> AppResult<Vec<ExternalId>> {
    let rows = SqlRuntime::fetch_all(
        exec,
        "SELECT source, external_id
           FROM media_request_external_ids
          WHERE request_id = {}
          ORDER BY source, external_id",
        &[SqlArg::Text(request_id.to_string())],
    )
    .await?;
    rows.iter()
        .map(|row| {
            Ok(ExternalId {
                source: row.text("source")?,
                value: row.text("external_id")?,
            })
        })
        .collect()
}

async fn load_media_request_requesters(
    exec: SqlExec<'_, '_>,
    request_id: &str,
) -> AppResult<Vec<MediaRequestRequester>> {
    let rows = SqlRuntime::fetch_all(
        exec,
        "SELECT mrr.user_id,
                users.username,
                (
                    SELECT account.avatar_url
                      FROM user_external_accounts account
                     WHERE account.user_id = mrr.user_id
                       AND account.status = 'active'
                       AND account.avatar_url IS NOT NULL
                       AND account.avatar_url <> ''
                     ORDER BY COALESCE(
                         account.last_login_at,
                         account.verified_at,
                         account.updated_at,
                         account.created_at
                     ) DESC
                     LIMIT 1
                ) AS avatar_url,
                mrr.requested_at
           FROM media_request_requesters mrr
           JOIN users ON users.id = mrr.user_id
          WHERE mrr.request_id = {}
          ORDER BY mrr.requested_at ASC, users.username ASC",
        &[SqlArg::Text(request_id.to_string())],
    )
    .await?;
    rows.iter()
        .map(|row| {
            Ok(MediaRequestRequester {
                user_id: row.text("user_id")?,
                username: row.text("username")?,
                avatar_url: row.opt_text("avatar_url")?,
                requested_at: row.timestamp("requested_at")?,
            })
        })
        .collect()
}

fn build_media_request_list_sql(query: &MediaRequestQuery) -> (String, Vec<SqlArg>) {
    let mut sql = format!(
        "SELECT {REQUEST_COLUMNS}
           FROM media_requests
          WHERE 1 = 1"
    );
    let mut args = Vec::new();

    if let Some(facet) = &query.facet {
        sql.push_str(" AND facet = {}");
        args.push(SqlArg::Text(facet.as_str().to_string()));
    }

    if let Some(status) = query.status {
        sql.push_str(" AND status = {}");
        args.push(SqlArg::Text(status.as_str().to_string()));
    }

    if let Some(library_ids) = &query.library_ids {
        let placeholders = std::iter::repeat_n("{}", library_ids.len())
            .collect::<Vec<_>>()
            .join(", ");
        sql.push_str(&format!(" AND library_id IN ({placeholders})"));
        args.extend(library_ids.iter().cloned().map(SqlArg::Text));
    }

    if let Some(requester_user_id) = &query.requester_user_id {
        sql.push_str(
            " AND id IN (
                SELECT request_id
                  FROM media_request_requesters
                 WHERE user_id = {}
            )",
        );
        args.push(SqlArg::Text(requester_user_id.clone()));
    }

    sql.push_str(" ORDER BY updated_at DESC, created_at DESC");
    (sql, args)
}

fn row_to_media_request(row: &SqlRow) -> AppResult<MediaRequest> {
    let facet_raw = row.text("facet")?;
    let facet = MediaFacet::parse(&facet_raw)
        .ok_or_else(|| AppError::Repository(format!("unknown media request facet {facet_raw}")))?;
    let status_raw = row.text("status")?;
    let status = MediaRequestStatus::parse(&status_raw).ok_or_else(|| {
        AppError::Repository(format!("unknown media request status {status_raw}"))
    })?;

    Ok(MediaRequest {
        id: row.text("id")?,
        library_id: row.text("library_id")?,
        facet,
        status,
        identity_fingerprint: row.text("identity_fingerprint")?,
        title: row.text("title")?,
        sort_title: row.opt_text("sort_title")?,
        slug: row.opt_text("slug")?,
        poster_url: row.opt_text("poster_url")?,
        background_url: row.opt_text("background_url")?,
        year: row.opt_i32("year")?,
        overview: row.opt_text("overview")?,
        runtime_minutes: row.opt_i32("runtime_minutes")?,
        language: row.opt_text("language")?,
        content_status: row.opt_text("content_status")?,
        rating_summary: serde_json::from_str(&row.text("rating_summary_json")?).map_err(
            |error| AppError::Repository(format!("parse media request rating summary: {error}")),
        )?,
        requested_quality_profile_id: row.opt_text("requested_quality_profile_id")?,
        requested_quality_profile_name: row.opt_text("requested_quality_profile_name")?,
        requested_monitor_type: row.opt_text("requested_monitor_type")?,
        requested_monitor_selection: None,
        resolved_by_user_id: row.opt_text("resolved_by_user_id")?,
        resolved_at: row.opt_timestamp("resolved_at")?,
        created_title_id: row.opt_text("created_title_id")?,
        approved_quality_profile_id: row.opt_text("approved_quality_profile_id")?,
        approved_quality_profile_name: row.opt_text("approved_quality_profile_name")?,
        requested_lease_days: row.opt_i64("requested_lease_days")?,
        approved_lease_days: row.opt_i64("approved_lease_days")?,
        decision_id: row.opt_text("decision_id")?,
        // A JSON column that somehow holds something other than a string array
        // reads back empty rather than failing the whole listing: policy
        // provenance is explanatory, and losing it must not hide the request.
        decided_by_rule_set_ids: parse_json_array(row, "decided_by_rule_set_ids")?,
        policy_tags: parse_json_array(row, "policy_tags_json")?,
        metadata_snapshot_json: json_text_or(row, "metadata_snapshot_json", "{}")?,
        external_ids: Vec::new(),
        requesters: Vec::new(),
        created_by_user_id: row.text("created_by_user_id")?,
        created_at: row.timestamp("created_at")?,
        updated_at: row.timestamp("updated_at")?,
    })
}

/// Serialize a string list for one of the JSON text columns 0220 added.
fn json_array_text(values: &[String]) -> AppResult<String> {
    canonical_json_text(&values)
}

fn parse_json_array(row: &SqlRow, column: &str) -> AppResult<Vec<String>> {
    let raw = json_text_or(row, column, "[]")?;
    Ok(serde_json::from_str(&raw).unwrap_or_default())
}

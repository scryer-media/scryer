use scryer_application::{
    AcquisitionScopeState, AcquisitionScopeStatesQuery, AppError, AppResult, ReleaseDecision,
};
use sqlx::sqlite::SqliteRow;
use sqlx::{QueryBuilder, Row, Sqlite, SqlitePool};

use crate::media::libraries::state_store::decode_release_decision_explanation;

pub async fn list_wanted_items_query(
    pool: &SqlitePool,
    query: &AcquisitionScopeStatesQuery,
) -> AppResult<Vec<AcquisitionScopeState>> {
    let AcquisitionScopeStatesQuery {
        statuses,
        media_types,
        title_id,
        library_ids,
        title_search,
        latest_decision_codes,
        limit,
        offset,
    } = query;
    let search_plan = title_search
        .as_deref()
        .and_then(|search| super::title_search::build_title_search_plan(None, search));
    let mut builder = QueryBuilder::<Sqlite>::new("");
    if let Some(plan) = search_plan.as_ref() {
        super::title_search::push_ranked_title_matches_cte(&mut builder, plan);
    }

    builder.push(
        "SELECT w.id, w.title_id, t.name AS title_name, t.slug AS title_slug,
                t.facet AS title_facet, t.library_id AS library_id,
                libraries.name AS library_name, libraries.slug AS library_slug,
                w.episode_id, w.collection_id, w.series_movie_link_id,
                e.season_number, e.episode_number, w.media_type,
                w.last_search_at, w.status,
                w.grabbed_release,
                latest_decision.id AS latest_decision_id,
                latest_decision.wanted_item_id AS latest_decision_wanted_item_id,
                latest_decision.title_id AS latest_decision_title_id,
                latest_decision.release_title AS latest_decision_release_title,
                latest_decision.release_url AS latest_decision_release_url,
                latest_decision.release_size_bytes AS latest_decision_release_size_bytes,
                latest_decision.decision_code AS latest_decision_decision_code,
                latest_decision.candidate_score AS latest_decision_candidate_score,
                latest_decision.current_score AS latest_decision_current_score,
                latest_decision.score_delta AS latest_decision_score_delta,
                latest_decision.explanation_json AS latest_decision_explanation_json,
                latest_decision.created_at AS latest_decision_created_at,
                CASE
                    WHEN w.status = 'wanted'
                     AND EXISTS (
                         SELECT 1
                         FROM release_decisions mismatch_any
                         WHERE mismatch_any.wanted_item_id = w.id
                     )
                     AND NOT EXISTS (
                         SELECT 1
                         FROM release_decisions mismatch_other
                         WHERE mismatch_other.wanted_item_id = w.id
                           AND mismatch_other.decision_code <> 'title_mismatch'
                     )
                    THEN 1
                    ELSE 0
                END AS mismatch_recovery_eligible,
                w.created_at, w.updated_at
         FROM wanted_items w
         LEFT JOIN titles t ON t.id = w.title_id
         LEFT JOIN libraries ON libraries.id = t.library_id
         LEFT JOIN episodes e ON e.id = w.episode_id
         LEFT JOIN release_decisions latest_decision ON latest_decision.id = (
             SELECT rd.id
             FROM release_decisions rd
             WHERE rd.wanted_item_id = w.id
             ORDER BY rd.created_at DESC
             LIMIT 1
         )
         ",
    );

    if search_plan.is_some() {
        builder.push("JOIN ranked_title_matches rtm ON rtm.title_id = w.title_id ");
    }

    builder.push("WHERE 1=1");

    if !statuses.is_empty() {
        builder.push(" AND w.status IN (");
        for (index, status) in statuses.iter().enumerate() {
            if index > 0 {
                builder.push(", ");
            }
            builder.push_bind(status);
        }
        builder.push(")");
    }
    if !media_types.is_empty() {
        builder.push(" AND w.media_type IN (");
        for (index, media_type) in media_types.iter().enumerate() {
            if index > 0 {
                builder.push(", ");
            }
            builder.push_bind(media_type);
        }
        builder.push(")");
    }
    if let Some(tid) = title_id.as_deref() {
        builder.push(" AND w.title_id = ");
        builder.push_bind(tid.to_string());
    }
    if !library_ids.is_empty() {
        builder.push(" AND t.library_id IN (");
        for (index, library_id) in library_ids.iter().enumerate() {
            if index > 0 {
                builder.push(", ");
            }
            builder.push_bind(library_id);
        }
        builder.push(")");
    }
    if !latest_decision_codes.is_empty() {
        builder.push(" AND latest_decision.decision_code IN (");
        for (index, code) in latest_decision_codes.iter().enumerate() {
            if index > 0 {
                builder.push(", ");
            }
            builder.push_bind(code);
        }
        builder.push(")");
    }

    builder.push(" ORDER BY w.updated_at DESC LIMIT ");
    builder.push_bind(limit);
    builder.push(" OFFSET ");
    builder.push_bind(offset);

    let rows: Vec<SqliteRow> = builder
        .build()
        .fetch_all(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        out.push(row_to_wanted_item(row)?);
    }
    Ok(out)
}

pub async fn count_wanted_items_query(
    pool: &SqlitePool,
    query: &AcquisitionScopeStatesQuery,
) -> AppResult<i64> {
    let AcquisitionScopeStatesQuery {
        statuses,
        media_types,
        title_id,
        library_ids,
        title_search,
        latest_decision_codes,
        ..
    } = query;
    let search_plan = title_search
        .as_deref()
        .and_then(|search| super::title_search::build_title_search_plan(None, search));
    let mut builder = QueryBuilder::<Sqlite>::new("");
    if let Some(plan) = search_plan.as_ref() {
        super::title_search::push_ranked_title_matches_cte(&mut builder, plan);
    }

    builder.push(
        "SELECT COUNT(*) as cnt
         FROM wanted_items w
         LEFT JOIN titles t ON t.id = w.title_id
         LEFT JOIN release_decisions latest_decision ON latest_decision.id = (
             SELECT rd.id
             FROM release_decisions rd
             WHERE rd.wanted_item_id = w.id
             ORDER BY rd.created_at DESC
             LIMIT 1
         )
         ",
    );

    if search_plan.is_some() {
        builder.push("JOIN ranked_title_matches rtm ON rtm.title_id = w.title_id ");
    }

    builder.push("WHERE 1=1");

    if !statuses.is_empty() {
        builder.push(" AND w.status IN (");
        for (index, status) in statuses.iter().enumerate() {
            if index > 0 {
                builder.push(", ");
            }
            builder.push_bind(status);
        }
        builder.push(")");
    }
    if !media_types.is_empty() {
        builder.push(" AND w.media_type IN (");
        for (index, media_type) in media_types.iter().enumerate() {
            if index > 0 {
                builder.push(", ");
            }
            builder.push_bind(media_type);
        }
        builder.push(")");
    }
    if let Some(tid) = title_id.as_deref() {
        builder.push(" AND w.title_id = ");
        builder.push_bind(tid.to_string());
    }
    if !library_ids.is_empty() {
        builder.push(" AND t.library_id IN (");
        for (index, library_id) in library_ids.iter().enumerate() {
            if index > 0 {
                builder.push(", ");
            }
            builder.push_bind(library_id);
        }
        builder.push(")");
    }
    if !latest_decision_codes.is_empty() {
        builder.push(" AND latest_decision.decision_code IN (");
        for (index, code) in latest_decision_codes.iter().enumerate() {
            if index > 0 {
                builder.push(", ");
            }
            builder.push_bind(code);
        }
        builder.push(")");
    }

    let row: SqliteRow = builder
        .build()
        .fetch_one(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    let count: i64 = row
        .try_get("cnt")
        .map_err(|e| AppError::Repository(e.to_string()))?;
    Ok(count)
}

fn row_to_wanted_item(row: &SqliteRow) -> AppResult<AcquisitionScopeState> {
    let latest_release_decision = match row.try_get::<Option<String>, _>("latest_decision_id") {
        Ok(Some(id)) => Some(ReleaseDecision {
            explanation_json: latest_release_decision_explanation(row, &id)?,
            id,
            wanted_item_id: row
                .try_get("latest_decision_wanted_item_id")
                .map_err(|e| AppError::Repository(e.to_string()))?,
            title_id: row
                .try_get("latest_decision_title_id")
                .map_err(|e| AppError::Repository(e.to_string()))?,
            release_title: row
                .try_get("latest_decision_release_title")
                .map_err(|e| AppError::Repository(e.to_string()))?,
            release_url: row.try_get("latest_decision_release_url").unwrap_or(None),
            release_size_bytes: row
                .try_get("latest_decision_release_size_bytes")
                .unwrap_or(None),
            decision_code: row
                .try_get("latest_decision_decision_code")
                .map_err(|e| AppError::Repository(e.to_string()))?,
            candidate_score: row
                .try_get("latest_decision_candidate_score")
                .map_err(|e| AppError::Repository(e.to_string()))?,
            current_score: row.try_get("latest_decision_current_score").unwrap_or(None),
            score_delta: row.try_get("latest_decision_score_delta").unwrap_or(None),
            created_at: row
                .try_get("latest_decision_created_at")
                .map_err(|e| AppError::Repository(e.to_string()))?,
        }),
        _ => None,
    };

    Ok(AcquisitionScopeState {
        id: row
            .try_get("id")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        title_id: row
            .try_get("title_id")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        title_name: row.try_get("title_name").unwrap_or(None),
        title_slug: row.try_get("title_slug").unwrap_or(None),
        title_facet: row.try_get("title_facet").unwrap_or(None),
        library_id: row.try_get("library_id").unwrap_or(None),
        library_name: row.try_get("library_name").unwrap_or(None),
        library_slug: row.try_get("library_slug").unwrap_or(None),
        episode_id: row.try_get("episode_id").unwrap_or(None),
        collection_id: row.try_get("collection_id").unwrap_or(None),
        series_movie_link_id: row.try_get("series_movie_link_id").unwrap_or(None),
        season_number: row.try_get("season_number").unwrap_or(None),
        episode_number: row.try_get("episode_number").unwrap_or(None),
        media_type: row
            .try_get("media_type")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        last_search_at: row.try_get("last_search_at").unwrap_or(None),
        status: {
            let s: String = row
                .try_get("status")
                .map_err(|e| AppError::Repository(e.to_string()))?;
            scryer_application::AcquisitionScopeStatus::parse(&s).unwrap_or_default()
        },
        grabbed_release: row.try_get("grabbed_release").unwrap_or(None),
        // Resolved from the library by the caller, never stored.
        landed_bar: None,
        latest_release_decision,
        mismatch_recovery_eligible: row
            .try_get::<i64, _>("mismatch_recovery_eligible")
            .map(|value| value != 0)
            .unwrap_or(false),
        created_at: row
            .try_get("created_at")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        updated_at: row
            .try_get("updated_at")
            .map_err(|e| AppError::Repository(e.to_string()))?,
    })
}

fn latest_release_decision_explanation(
    row: &SqliteRow,
    decision_id: &str,
) -> AppResult<Option<String>> {
    let encoded = row
        .try_get::<Option<Vec<u8>>, _>("latest_decision_explanation_json")
        .map_err(|error| AppError::Repository(error.to_string()))?;
    match decode_release_decision_explanation(encoded.as_deref()) {
        Ok(explanation) => Ok(explanation),
        Err(error) => {
            tracing::warn!(
                decision_id,
                error = %error,
                "release decision explanation could not be decoded"
            );
            Ok(None)
        }
    }
}

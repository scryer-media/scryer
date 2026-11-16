//! A library title's content ratings, as SMG returns them.

use std::collections::BTreeMap;

use scryer_application::{AppError, AppResult, ContentCertification, ContentRating};

use crate::media::id_list_statements::{id_list_statements, max_binds};
use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRuntime, SqlTx};

/// Owners read per statement once a list is too long for one.
const CONTENT_RATING_OWNER_CHUNK: usize = 900;

/// Replace a title's content ratings with `ratings`, in SMG's order. An empty
/// slice clears them.
pub async fn replace_title_content_ratings_tx(
    tx: &mut SqlTx<'_>,
    title_id: &str,
    ratings: &[ContentRating],
) -> AppResult<()> {
    tx.execute(
        "DELETE FROM title_content_ratings WHERE title_id = {}",
        &[SqlArg::Text(title_id.to_string())],
    )
    .await?;
    for (sort_index, rating) in ratings.iter().enumerate() {
        let certifications = serde_json::to_string(&rating.certifications).map_err(|error| {
            AppError::Repository(format!("encode content certifications: {error}"))
        })?;
        tx.execute(
            "INSERT INTO title_content_ratings (
                title_id, sort_index, country, certifications_json, age_rating, age_rating_source
             ) VALUES ({}, {}, {}, {}, {}, {})",
            &[
                SqlArg::Text(title_id.to_string()),
                SqlArg::I32(sort_index as i32),
                SqlArg::Text(rating.country.clone()),
                SqlArg::Text(certifications),
                SqlArg::OptI32(rating.age_rating),
                SqlArg::OptText(rating.age_rating_source.clone()),
            ],
        )
        .await?;
    }
    Ok(())
}

/// The stored content ratings of each title in `title_ids`, in SMG's order. A
/// title with none is absent from the map.
pub async fn load_title_content_ratings(
    mut exec: SqlExec<'_, '_>,
    title_ids: &[String],
) -> AppResult<BTreeMap<String, Vec<ContentRating>>> {
    let (title_ids, per_statement) = id_list_statements(
        title_ids,
        1,
        0,
        max_binds(&exec),
        CONTENT_RATING_OWNER_CHUNK,
    );
    let mut by_title = BTreeMap::new();
    for chunk in title_ids.chunks(per_statement) {
        let exec = match &mut exec {
            SqlExec::Target(target) => SqlExec::Target(*target),
            SqlExec::Tx(tx) => SqlExec::Tx(tx),
        };
        by_title.extend(load_chunk(exec, chunk).await?);
    }
    Ok(by_title)
}

async fn load_chunk(
    exec: SqlExec<'_, '_>,
    title_ids: &[String],
) -> AppResult<BTreeMap<String, Vec<ContentRating>>> {
    if title_ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    let placeholders = std::iter::repeat_n("{}", title_ids.len())
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT title_id, country, certifications_json, age_rating, age_rating_source
           FROM title_content_ratings
          WHERE title_id IN ({placeholders})
          ORDER BY title_id, sort_index"
    );
    let args = title_ids
        .iter()
        .cloned()
        .map(SqlArg::Text)
        .collect::<Vec<_>>();
    let rows = SqlRuntime::fetch_all(exec, &sql, &args).await?;

    let mut by_title = BTreeMap::<String, Vec<ContentRating>>::new();
    for row in &rows {
        let certifications: Vec<ContentCertification> =
            serde_json::from_str(&row.text("certifications_json")?).map_err(|error| {
                AppError::Repository(format!("decode content certifications: {error}"))
            })?;
        by_title
            .entry(row.text("title_id")?)
            .or_default()
            .push(ContentRating {
                country: row.text("country")?,
                certifications,
                age_rating: row.opt_i32("age_rating")?,
                age_rating_source: row.opt_text("age_rating_source")?,
            });
    }
    Ok(by_title)
}

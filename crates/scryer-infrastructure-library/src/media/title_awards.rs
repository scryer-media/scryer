//! A library title's awards, as SMG returns them.

use std::collections::BTreeMap;

use chrono::Utc;
use scryer_application::{AppResult, TitleAward};

use crate::media::id_list_statements::{id_list_statements, max_binds};
use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRuntime, SqlTx};

/// Owners read per statement once a list is too long for one.
const AWARD_OWNER_CHUNK: usize = 900;

/// Replace a title's awards with `awards`, in SMG's order, and record that
/// they are stored. An empty slice stores that it has none.
pub async fn replace_title_awards_tx(
    tx: &mut SqlTx<'_>,
    title_id: &str,
    awards: &[TitleAward],
) -> AppResult<()> {
    delete_title_awards_tx(tx, title_id).await?;
    let rows = awards
        .iter()
        .enumerate()
        .map(|(sort_index, award)| {
            vec![
                SqlArg::Text(title_id.to_string()),
                SqlArg::I32(sort_index as i32),
                SqlArg::Text(award.award_qid.clone()),
                SqlArg::Text(award.award_label.clone()),
                SqlArg::OptI32(award.year),
                SqlArg::Text(award.recipient_qid.clone()),
                SqlArg::Text(award.recipient_label.clone()),
                SqlArg::Text(award.claim_side.clone()),
            ]
        })
        .collect();
    SqlRuntime::execute_batch_insert(
        tx,
        "INSERT INTO title_awards (
            title_id, sort_index, award_qid, award_label, award_year, recipient_qid,
            recipient_label, claim_side
         )",
        8,
        rows,
        "",
    )
    .await?;
    tx.execute(
        "UPDATE titles SET awards_stored_at = {} WHERE id = {}",
        &[
            SqlArg::Timestamp(Utc::now()),
            SqlArg::Text(title_id.to_string()),
        ],
    )
    .await?;
    Ok(())
}

/// Remove a title's awards and mark them not stored, so they read as unknown
/// until the title is hydrated again.
pub async fn clear_title_awards_tx(tx: &mut SqlTx<'_>, title_id: &str) -> AppResult<()> {
    delete_title_awards_tx(tx, title_id).await?;
    tx.execute(
        "UPDATE titles SET awards_stored_at = NULL WHERE id = {}",
        &[SqlArg::Text(title_id.to_string())],
    )
    .await?;
    Ok(())
}

async fn delete_title_awards_tx(tx: &mut SqlTx<'_>, title_id: &str) -> AppResult<()> {
    tx.execute(
        "DELETE FROM title_awards WHERE title_id = {}",
        &[SqlArg::Text(title_id.to_string())],
    )
    .await?;
    Ok(())
}

/// The stored awards of each title in `title_ids`, in SMG's order. A title
/// whose awards are stored maps to them, an empty list included; a title whose
/// awards are not stored, or that does not exist, is absent.
pub async fn load_title_awards(
    mut exec: SqlExec<'_, '_>,
    title_ids: &[String],
) -> AppResult<BTreeMap<String, Vec<TitleAward>>> {
    let (title_ids, per_statement) =
        id_list_statements(title_ids, 1, 0, max_binds(&exec), AWARD_OWNER_CHUNK);
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
) -> AppResult<BTreeMap<String, Vec<TitleAward>>> {
    if title_ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    let placeholders = std::iter::repeat_n("{}", title_ids.len())
        .collect::<Vec<_>>()
        .join(", ");
    // One row per stored award, and one row with no award for a title whose
    // stored set is empty.
    let sql = format!(
        "SELECT t.id AS title_id, a.award_qid, a.award_label, a.award_year, a.recipient_qid,
                a.recipient_label, a.claim_side
           FROM titles t
           LEFT JOIN title_awards a ON a.title_id = t.id
          WHERE t.id IN ({placeholders})
            AND t.awards_stored_at IS NOT NULL
          ORDER BY t.id, a.sort_index"
    );
    let args = title_ids
        .iter()
        .cloned()
        .map(SqlArg::Text)
        .collect::<Vec<_>>();
    let rows = SqlRuntime::fetch_all(exec, &sql, &args).await?;

    let mut by_title = BTreeMap::<String, Vec<TitleAward>>::new();
    for row in &rows {
        let awards = by_title.entry(row.text("title_id")?).or_default();
        let Some(award_qid) = row.opt_text("award_qid")? else {
            continue;
        };
        awards.push(TitleAward {
            award_qid,
            award_label: row.text("award_label")?,
            year: row.opt_i32("award_year")?,
            recipient_qid: row.text("recipient_qid")?,
            recipient_label: row.text("recipient_label")?,
            claim_side: row.text("claim_side")?,
        });
    }
    Ok(by_title)
}

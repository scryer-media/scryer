//! Membership rows: one per (subscription, provider item).
//!
//! Departure is detected by time, not by set difference. A sync refreshes every
//! present row's `last_seen_at`; whatever the subscription still has with an
//! older `last_seen_at` and no `left_at` is what left. That keeps the write
//! bounded by the list's size rather than by an `IN (...)` of every key.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use scryer_application::AppResult;
use scryer_application::lists::ListMembershipRepository;
use scryer_domain::{ListMembership, ListMembershipState, MediaFacet};

use super::{ListStore, json_arg, json_column, parse_or_repo_err, placeholders};
use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRow, SqlRuntime};

const MEMBERSHIP_COLUMNS: &str = "subscription_id, item_key, rank, season, display_title, year,
    external_ids_json,
    smg_title_id, title_id, request_id, kind, state, state_reason, added_by_list,
    first_seen_at, last_seen_at, left_at, left_handled";

const MEMBERSHIP_WIDTH: usize = 18;

/// `first_seen_at` is deliberately absent: a refreshed row is the same item.
/// `left_at` and `left_handled` reset because the item is present again.
const MEMBERSHIP_UPSERT_SUFFIX: &str = "ON CONFLICT (subscription_id, item_key) DO UPDATE SET
         rank = excluded.rank,
         season = excluded.season,
         display_title = excluded.display_title,
         year = excluded.year,
         external_ids_json = excluded.external_ids_json,
         smg_title_id = excluded.smg_title_id,
         title_id = excluded.title_id,
         request_id = excluded.request_id,
         kind = excluded.kind,
         state = excluded.state,
         state_reason = excluded.state_reason,
         added_by_list = excluded.added_by_list,
         last_seen_at = excluded.last_seen_at,
         left_at = NULL,
         left_handled = excluded.left_handled";

#[async_trait]
impl ListMembershipRepository for ListStore {
    async fn upsert_many(&self, memberships: &[ListMembership]) -> AppResult<u64> {
        if memberships.is_empty() {
            return Ok(0);
        }
        let rows = memberships
            .iter()
            .map(membership_args)
            .collect::<AppResult<Vec<_>>>()?;
        SqlRuntime::run_in_transaction(&self.datastore, "upsert_list_memberships", move |tx| {
            let rows = rows.clone();
            Box::pin(async move {
                SqlRuntime::execute_batch_insert(
                    tx,
                    &format!("INSERT INTO list_memberships ({MEMBERSHIP_COLUMNS})"),
                    MEMBERSHIP_WIDTH,
                    rows,
                    MEMBERSHIP_UPSERT_SUFFIX,
                )
                .await
            })
        })
        .await
    }

    async fn list_by_subscription(&self, subscription_id: &str) -> AppResult<Vec<ListMembership>> {
        self.fetch_memberships(
            "subscription_id = {}",
            vec![SqlArg::Text(subscription_id.to_string())],
        )
        .await
    }

    async fn list_by_title(&self, title_id: &str) -> AppResult<Vec<ListMembership>> {
        self.fetch_memberships("title_id = {}", vec![SqlArg::Text(title_id.to_string())])
            .await
    }

    async fn list_by_titles(&self, title_ids: &[String]) -> AppResult<Vec<ListMembership>> {
        if title_ids.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = std::iter::repeat_n("{}", title_ids.len())
            .collect::<Vec<_>>()
            .join(", ");
        self.fetch_memberships(
            &format!("title_id IN ({placeholders})"),
            title_ids.iter().cloned().map(SqlArg::Text).collect(),
        )
        .await
    }

    async fn list_by_request(&self, request_id: &str) -> AppResult<Vec<ListMembership>> {
        self.fetch_memberships(
            "request_id = {}",
            vec![SqlArg::Text(request_id.to_string())],
        )
        .await
    }

    async fn mark_left(
        &self,
        subscription_id: &str,
        seen_before: DateTime<Utc>,
        left_at: DateTime<Utc>,
    ) -> AppResult<Vec<ListMembership>> {
        let subscription_id = subscription_id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "mark_list_memberships_left", move |tx| {
            let subscription_id = subscription_id.clone();
            Box::pin(async move {
                SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "UPDATE list_memberships
                        SET left_at = {}, left_handled = {}
                      WHERE subscription_id = {}
                        AND left_at IS NULL
                        AND last_seen_at < {}",
                    &[
                        SqlArg::Timestamp(left_at),
                        SqlArg::Bool(false),
                        SqlArg::Text(subscription_id.clone()),
                        SqlArg::Timestamp(seen_before),
                    ],
                )
                .await?;
                let rows = SqlRuntime::fetch_all(
                    SqlExec::Tx(tx),
                    &format!(
                        "SELECT {MEMBERSHIP_COLUMNS} FROM list_memberships
                          WHERE subscription_id = {{}} AND left_at = {{}}
                          ORDER BY rank, item_key"
                    ),
                    &[SqlArg::Text(subscription_id), SqlArg::Timestamp(left_at)],
                )
                .await?;
                rows.iter().map(row_to_membership).collect()
            })
        })
        .await
    }

    async fn set_left_handled(
        &self,
        subscription_id: &str,
        item_keys: &[String],
    ) -> AppResult<u64> {
        if item_keys.is_empty() {
            return Ok(0);
        }
        let mut args = vec![
            SqlArg::Bool(true),
            SqlArg::Text(subscription_id.to_string()),
        ];
        args.extend(item_keys.iter().cloned().map(SqlArg::Text));
        SqlRuntime::execute_write(
            &self.datastore,
            "set_list_memberships_left_handled",
            &format!(
                "UPDATE list_memberships SET left_handled = {{}}
                  WHERE subscription_id = {{}} AND item_key IN ({})",
                placeholders(item_keys.len())
            ),
            args,
        )
        .await
    }
}

impl ListStore {
    async fn fetch_memberships(
        &self,
        predicate: &str,
        args: Vec<SqlArg>,
    ) -> AppResult<Vec<ListMembership>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            &format!(
                "SELECT {MEMBERSHIP_COLUMNS} FROM list_memberships
                  WHERE {predicate}
                  ORDER BY subscription_id, rank, item_key"
            ),
            &args,
        )
        .await?;
        rows.iter().map(row_to_membership).collect()
    }
}

fn membership_args(membership: &ListMembership) -> AppResult<Vec<SqlArg>> {
    Ok(vec![
        SqlArg::Text(membership.subscription_id.clone()),
        SqlArg::Text(membership.item_key.clone()),
        SqlArg::OptI64(membership.rank),
        SqlArg::OptI32(membership.season),
        SqlArg::OptText(membership.display_title.clone()),
        SqlArg::OptI32(membership.year),
        json_arg(&membership.external_ids)?,
        SqlArg::OptI64(membership.smg_title_id),
        SqlArg::OptText(membership.title_id.clone()),
        SqlArg::OptText(membership.request_id.clone()),
        SqlArg::Text(membership.kind.as_str().to_string()),
        SqlArg::Text(membership.state.as_str().to_string()),
        SqlArg::OptText(membership.state_reason.clone()),
        SqlArg::Bool(membership.added_by_list),
        SqlArg::Timestamp(membership.first_seen_at),
        SqlArg::Timestamp(membership.last_seen_at),
        SqlArg::OptTimestamp(membership.left_at),
        SqlArg::Bool(membership.left_handled),
    ])
}

fn row_to_membership(row: &SqlRow) -> AppResult<ListMembership> {
    Ok(ListMembership {
        subscription_id: row.text("subscription_id")?,
        item_key: row.text("item_key")?,
        rank: row.opt_i64("rank")?,
        season: row.opt_i32("season")?,
        display_title: row.opt_text("display_title")?,
        year: row.opt_i32("year")?,
        external_ids: json_column(row, "external_ids_json", "[]")?,
        smg_title_id: row.opt_i64("smg_title_id")?,
        title_id: row.opt_text("title_id")?,
        request_id: row.opt_text("request_id")?,
        kind: parse_or_repo_err("kind", &row.text("kind")?, MediaFacet::parse)?,
        state: parse_or_repo_err("state", &row.text("state")?, ListMembershipState::parse)?,
        state_reason: row.opt_text("state_reason")?,
        added_by_list: row.bool("added_by_list")?,
        first_seen_at: row.timestamp("first_seen_at")?,
        last_seen_at: row.timestamp("last_seen_at")?,
        left_at: row.opt_timestamp("left_at")?,
        left_handled: row.bool("left_handled")?,
    })
}

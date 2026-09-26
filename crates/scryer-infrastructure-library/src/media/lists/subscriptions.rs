//! Subscriptions, their routing cards, and their sync runs.
//!
//! Routes live in a child table keyed by (subscription, kind) and are replaced
//! wholesale on every update: a subscription has at most one card per kind, so
//! "replace all" is the only write that cannot leave a stale card behind.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use scryer_application::lists::{ListSubscriptionQuery, ListSubscriptionRepository};
use scryer_application::{AppError, AppResult};
use scryer_domain::{
    ListCounts, ListMode, ListOnLeave, ListRoute, ListScope, ListSource, ListSourceOrigin,
    ListSubscription, ListSyncRun, ListSyncRunOutcome, ListSyncState, ListSyncStatus, MediaFacet,
};
use std::collections::HashMap;

use super::{ListStore, json_arg, json_column, parse_or_repo_err, placeholders};
use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRow, SqlRuntime, SqlTx};

const SUBSCRIPTION_COLUMNS: &str = "id, scope, owner_user_id, provider, source_type,
    source_params_json, source_origin, chart_key, chart_scope, name, provider_url, kinds_json,
    enabled, mode, filters_json, max_per_sync, on_leave, interval_seconds, credential_id,
    sync_state, last_sync_at, next_sync_at, error_message, error_at, paused_until,
    fetch_fingerprint, count_total, count_in_library, count_added, count_requested, count_held,
    count_filtered, count_excluded, count_unresolved, created_at, updated_at";

const ROUTE_COLUMNS: &str = "subscription_id, kind, library_id, quality_profile_id,
    root_folder_id, monitor_type, min_availability, use_season_folders, release_numbering,
    tags_json";

const RUN_COLUMNS: &str =
    "id, subscription_id, job_run_id, started_at, finished_at, outcome, counts_json, error_message";

#[async_trait]
impl ListSubscriptionRepository for ListStore {
    async fn create(&self, subscription: ListSubscription) -> AppResult<ListSubscription> {
        let insert_args = subscription_insert_args(&subscription)?;
        let route_rows = route_rows(&subscription)?;
        SqlRuntime::run_in_transaction(&self.datastore, "create_list_subscription", move |tx| {
            let insert_args = insert_args.clone();
            let route_rows = route_rows.clone();
            let subscription = subscription.clone();
            Box::pin(async move {
                SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    &format!(
                        "INSERT INTO list_subscriptions ({SUBSCRIPTION_COLUMNS})
                         VALUES ({})",
                        placeholders(SUBSCRIPTION_WIDTH)
                    ),
                    &insert_args,
                )
                .await?;
                replace_routes_tx(tx, &subscription.id, route_rows).await?;
                Ok(subscription)
            })
        })
        .await
    }

    async fn update(&self, subscription: ListSubscription) -> AppResult<ListSubscription> {
        let args = vec![
            SqlArg::Text(subscription.name.clone()),
            SqlArg::OptText(subscription.provider_url.clone()),
            json_arg(&subscription.kinds)?,
            SqlArg::Bool(subscription.enabled),
            SqlArg::Text(subscription.mode.as_str().to_string()),
            json_arg(&subscription.filters)?,
            SqlArg::OptI64(subscription.max_per_sync.map(i64::from)),
            SqlArg::Text(subscription.on_leave.as_str().to_string()),
            SqlArg::I64(subscription.interval_seconds),
            SqlArg::OptText(subscription.credential_id.clone()),
            SqlArg::Text(subscription.source.source_type.clone()),
            json_arg(&subscription.source.params)?,
            SqlArg::Timestamp(subscription.updated_at),
            SqlArg::Text(subscription.id.clone()),
        ];
        let route_rows = route_rows(&subscription)?;
        SqlRuntime::run_in_transaction(&self.datastore, "update_list_subscription", move |tx| {
            let args = args.clone();
            let route_rows = route_rows.clone();
            let subscription = subscription.clone();
            Box::pin(async move {
                let changed = SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "UPDATE list_subscriptions
                        SET name = {}, provider_url = {}, kinds_json = {}, enabled = {},
                            mode = {}, filters_json = {}, max_per_sync = {}, on_leave = {},
                            interval_seconds = {}, credential_id = {}, source_type = {},
                            source_params_json = {}, updated_at = {}
                      WHERE id = {}",
                    &args,
                )
                .await?;
                if changed == 0 {
                    return Err(AppError::NotFound(format!(
                        "list subscription {}",
                        subscription.id
                    )));
                }
                replace_routes_tx(tx, &subscription.id, route_rows).await?;
                load_subscription_tx(tx, &subscription.id)
                    .await?
                    .ok_or_else(|| {
                        AppError::NotFound(format!("list subscription {}", subscription.id))
                    })
            })
        })
        .await
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<ListSubscription>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            &format!("SELECT {SUBSCRIPTION_COLUMNS} FROM list_subscriptions WHERE id = {{}}"),
            &[SqlArg::Text(id.to_string())],
        )
        .await?;
        let mut subscriptions = self.hydrate_subscriptions(rows).await?;
        Ok(subscriptions.pop())
    }

    async fn list(&self, query: ListSubscriptionQuery) -> AppResult<Vec<ListSubscription>> {
        let mut clauses = Vec::new();
        let mut args = Vec::new();
        if let Some(scope) = query.scope {
            clauses.push("scope = {}");
            args.push(SqlArg::Text(scope.as_str().to_string()));
        }
        if let Some(owner) = query.owner_user_id {
            clauses.push("owner_user_id = {}");
            args.push(SqlArg::Text(owner));
        }
        if let Some(provider) = query.provider {
            clauses.push("provider = {}");
            args.push(SqlArg::Text(provider));
        }
        if let Some(enabled) = query.enabled {
            clauses.push("enabled = {}");
            args.push(SqlArg::Bool(enabled));
        }
        let where_clause = if clauses.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", clauses.join(" AND "))
        };
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            &format!(
                "SELECT {SUBSCRIPTION_COLUMNS} FROM list_subscriptions{where_clause}
                  ORDER BY created_at, id"
            ),
            &args,
        )
        .await?;
        self.hydrate_subscriptions(rows).await
    }

    async fn list_due(&self, now: DateTime<Utc>, limit: usize) -> AppResult<Vec<ListSubscription>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            &format!(
                "SELECT {SUBSCRIPTION_COLUMNS} FROM list_subscriptions
                  WHERE enabled = {{}}
                    AND (next_sync_at IS NULL OR next_sync_at <= {{}})
                    AND (paused_until IS NULL OR paused_until <= {{}})
                  ORDER BY next_sync_at, created_at, id
                  LIMIT {{}}"
            ),
            &[
                SqlArg::Bool(true),
                SqlArg::Timestamp(now),
                SqlArg::Timestamp(now),
                SqlArg::I64(i64::try_from(limit).unwrap_or(i64::MAX)),
            ],
        )
        .await?;
        self.hydrate_subscriptions(rows).await
    }

    async fn record_sync(
        &self,
        id: &str,
        sync: &ListSyncStatus,
        counts: &ListCounts,
    ) -> AppResult<()> {
        let args = vec![
            SqlArg::Text(sync.state.as_str().to_string()),
            SqlArg::OptTimestamp(sync.last_at),
            SqlArg::OptTimestamp(sync.next_at),
            SqlArg::OptText(sync.error_message.clone()),
            SqlArg::OptTimestamp(sync.error_at),
            SqlArg::OptTimestamp(sync.paused_until),
            SqlArg::OptText(sync.fetch_fingerprint.clone()),
            count_arg(counts.total),
            count_arg(counts.in_library),
            count_arg(counts.added),
            count_arg(counts.requested),
            count_arg(counts.held),
            count_arg(counts.filtered),
            count_arg(counts.excluded),
            count_arg(counts.unresolved),
            SqlArg::Timestamp(Utc::now()),
            SqlArg::Text(id.to_string()),
        ];
        let changed = SqlRuntime::execute_write(
            &self.datastore,
            "record_list_sync",
            "UPDATE list_subscriptions
                SET sync_state = {}, last_sync_at = {}, next_sync_at = {}, error_message = {},
                    error_at = {}, paused_until = {}, fetch_fingerprint = {},
                    count_total = {}, count_in_library = {}, count_added = {},
                    count_requested = {}, count_held = {}, count_filtered = {},
                    count_excluded = {}, count_unresolved = {}, updated_at = {}
              WHERE id = {}",
            args,
        )
        .await?;
        if changed == 0 {
            return Err(AppError::NotFound(format!("list subscription {id}")));
        }
        Ok(())
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        let changed = SqlRuntime::execute_write(
            &self.datastore,
            "delete_list_subscription",
            "DELETE FROM list_subscriptions WHERE id = {}",
            vec![SqlArg::Text(id.to_string())],
        )
        .await?;
        if changed == 0 {
            return Err(AppError::NotFound(format!("list subscription {id}")));
        }
        Ok(())
    }

    async fn record_sync_run(&self, run: ListSyncRun) -> AppResult<ListSyncRun> {
        let args = vec![
            SqlArg::Text(run.id.clone()),
            SqlArg::Text(run.subscription_id.clone()),
            SqlArg::OptText(run.job_run_id.clone()),
            SqlArg::Timestamp(run.started_at),
            SqlArg::OptTimestamp(run.finished_at),
            SqlArg::Text(run.outcome.as_str().to_string()),
            json_arg(&run.counts)?,
            SqlArg::OptText(run.error_message.clone()),
        ];
        SqlRuntime::execute_write(
            &self.datastore,
            "record_list_sync_run",
            &format!(
                "INSERT INTO list_sync_runs ({RUN_COLUMNS}) VALUES ({})
                 ON CONFLICT (id) DO UPDATE SET
                     finished_at = excluded.finished_at,
                     outcome = excluded.outcome,
                     counts_json = excluded.counts_json,
                     error_message = excluded.error_message",
                placeholders(8)
            ),
            args,
        )
        .await?;
        Ok(run)
    }

    async fn list_sync_runs(
        &self,
        subscription_id: &str,
        limit: usize,
    ) -> AppResult<Vec<ListSyncRun>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            &format!(
                "SELECT {RUN_COLUMNS} FROM list_sync_runs
                  WHERE subscription_id = {{}}
                  ORDER BY started_at DESC, id DESC
                  LIMIT {{}}"
            ),
            &[
                SqlArg::Text(subscription_id.to_string()),
                SqlArg::I64(i64::try_from(limit).unwrap_or(i64::MAX)),
            ],
        )
        .await?;
        rows.iter().map(row_to_run).collect()
    }
}

impl ListStore {
    /// Attach routes to a page of subscription rows with one child read.
    async fn hydrate_subscriptions(&self, rows: Vec<SqlRow>) -> AppResult<Vec<ListSubscription>> {
        let mut subscriptions = rows
            .iter()
            .map(row_to_subscription)
            .collect::<AppResult<Vec<_>>>()?;
        if subscriptions.is_empty() {
            return Ok(subscriptions);
        }
        let ids: Vec<SqlArg> = subscriptions
            .iter()
            .map(|subscription| SqlArg::Text(subscription.id.clone()))
            .collect();
        let route_rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            &format!(
                "SELECT {ROUTE_COLUMNS} FROM list_subscription_routes
                  WHERE subscription_id IN ({})
                  ORDER BY subscription_id, kind",
                placeholders(ids.len())
            ),
            &ids,
        )
        .await?;
        let mut routes_by_subscription: HashMap<String, Vec<ListRoute>> = HashMap::new();
        for row in &route_rows {
            let subscription_id = row.text("subscription_id")?;
            routes_by_subscription
                .entry(subscription_id)
                .or_default()
                .push(row_to_route(row)?);
        }
        for subscription in &mut subscriptions {
            subscription.routes = routes_by_subscription
                .remove(&subscription.id)
                .unwrap_or_default();
        }
        Ok(subscriptions)
    }
}

const SUBSCRIPTION_WIDTH: usize = 36;

fn count_arg(value: u64) -> SqlArg {
    SqlArg::I64(i64::try_from(value).unwrap_or(i64::MAX))
}

fn count_from(row: &SqlRow, column: &str) -> AppResult<u64> {
    Ok(row.i64(column)?.max(0) as u64)
}

fn subscription_insert_args(subscription: &ListSubscription) -> AppResult<Vec<SqlArg>> {
    let (chart_key, chart_scope) = match &subscription.source.origin {
        ListSourceOrigin::SmgChart { chart_key, scope } => {
            (Some(chart_key.clone()), Some(scope.clone()))
        }
        ListSourceOrigin::ProviderFetch | ListSourceOrigin::SmgImdbList => (None, None),
    };
    let sync = &subscription.sync;
    let counts = &subscription.counts;
    Ok(vec![
        SqlArg::Text(subscription.id.clone()),
        SqlArg::Text(subscription.scope.as_str().to_string()),
        SqlArg::Text(subscription.owner_user_id.clone()),
        SqlArg::Text(subscription.source.provider.clone()),
        SqlArg::Text(subscription.source.source_type.clone()),
        json_arg(&subscription.source.params)?,
        SqlArg::Text(subscription.source.origin.as_str().to_string()),
        SqlArg::OptText(chart_key),
        SqlArg::OptText(chart_scope),
        SqlArg::Text(subscription.name.clone()),
        SqlArg::OptText(subscription.provider_url.clone()),
        json_arg(&subscription.kinds)?,
        SqlArg::Bool(subscription.enabled),
        SqlArg::Text(subscription.mode.as_str().to_string()),
        json_arg(&subscription.filters)?,
        SqlArg::OptI64(subscription.max_per_sync.map(i64::from)),
        SqlArg::Text(subscription.on_leave.as_str().to_string()),
        SqlArg::I64(subscription.interval_seconds),
        SqlArg::OptText(subscription.credential_id.clone()),
        SqlArg::Text(sync.state.as_str().to_string()),
        SqlArg::OptTimestamp(sync.last_at),
        SqlArg::OptTimestamp(sync.next_at),
        SqlArg::OptText(sync.error_message.clone()),
        SqlArg::OptTimestamp(sync.error_at),
        SqlArg::OptTimestamp(sync.paused_until),
        SqlArg::OptText(sync.fetch_fingerprint.clone()),
        count_arg(counts.total),
        count_arg(counts.in_library),
        count_arg(counts.added),
        count_arg(counts.requested),
        count_arg(counts.held),
        count_arg(counts.filtered),
        count_arg(counts.excluded),
        count_arg(counts.unresolved),
        SqlArg::Timestamp(subscription.created_at),
        SqlArg::Timestamp(subscription.updated_at),
    ])
}

fn route_rows(subscription: &ListSubscription) -> AppResult<Vec<Vec<SqlArg>>> {
    subscription
        .routes
        .iter()
        .map(|route| {
            Ok(vec![
                SqlArg::Text(subscription.id.clone()),
                SqlArg::Text(route.kind.as_str().to_string()),
                SqlArg::Text(route.library_id.clone()),
                SqlArg::OptText(route.quality_profile_id.clone()),
                SqlArg::OptText(route.root_folder_id.clone()),
                SqlArg::Text(route.monitor_type.clone()),
                SqlArg::OptText(route.min_availability.clone()),
                SqlArg::OptBool(route.use_season_folders),
                SqlArg::OptText(route.release_numbering.clone()),
                json_arg(&route.tags)?,
            ])
        })
        .collect()
}

async fn replace_routes_tx(
    tx: &mut SqlTx<'_>,
    subscription_id: &str,
    rows: Vec<Vec<SqlArg>>,
) -> AppResult<()> {
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "DELETE FROM list_subscription_routes WHERE subscription_id = {}",
        &[SqlArg::Text(subscription_id.to_string())],
    )
    .await?;
    if rows.is_empty() {
        return Ok(());
    }
    SqlRuntime::execute_batch_insert(
        tx,
        &format!("INSERT INTO list_subscription_routes ({ROUTE_COLUMNS})"),
        10,
        rows,
        "",
    )
    .await?;
    Ok(())
}

async fn load_subscription_tx(tx: &mut SqlTx<'_>, id: &str) -> AppResult<Option<ListSubscription>> {
    let row = SqlRuntime::fetch_optional(
        SqlExec::Tx(tx),
        &format!("SELECT {SUBSCRIPTION_COLUMNS} FROM list_subscriptions WHERE id = {{}}"),
        &[SqlArg::Text(id.to_string())],
    )
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let mut subscription = row_to_subscription(&row)?;
    let route_rows = SqlRuntime::fetch_all(
        SqlExec::Tx(tx),
        &format!(
            "SELECT {ROUTE_COLUMNS} FROM list_subscription_routes
              WHERE subscription_id = {{}} ORDER BY kind"
        ),
        &[SqlArg::Text(id.to_string())],
    )
    .await?;
    subscription.routes = route_rows
        .iter()
        .map(row_to_route)
        .collect::<AppResult<Vec<_>>>()?;
    Ok(Some(subscription))
}

fn row_to_subscription(row: &SqlRow) -> AppResult<ListSubscription> {
    let origin = match row.text("source_origin")?.as_str() {
        "smg_chart" => ListSourceOrigin::SmgChart {
            chart_key: row.opt_text("chart_key")?.unwrap_or_default(),
            scope: row.opt_text("chart_scope")?.unwrap_or_default(),
        },
        "provider_fetch" => ListSourceOrigin::ProviderFetch,
        "smg_imdb_list" => ListSourceOrigin::SmgImdbList,
        other => {
            return Err(AppError::Repository(format!(
                "unknown source_origin value '{other}'"
            )));
        }
    };
    Ok(ListSubscription {
        id: row.text("id")?,
        scope: parse_or_repo_err("scope", &row.text("scope")?, ListScope::parse)?,
        owner_user_id: row.text("owner_user_id")?,
        source: ListSource {
            provider: row.text("provider")?,
            source_type: row.text("source_type")?,
            params: json_column(row, "source_params_json", "{}")?,
            origin,
        },
        name: row.text("name")?,
        provider_url: row.opt_text("provider_url")?,
        kinds: json_column(row, "kinds_json", "[]")?,
        enabled: row.bool("enabled")?,
        mode: parse_or_repo_err("mode", &row.text("mode")?, ListMode::parse)?,
        routes: Vec::new(),
        filters: json_column(row, "filters_json", "[]")?,
        max_per_sync: row
            .opt_i64("max_per_sync")?
            .and_then(|value| u32::try_from(value).ok()),
        on_leave: parse_or_repo_err("on_leave", &row.text("on_leave")?, ListOnLeave::parse)?,
        interval_seconds: row.i64("interval_seconds")?,
        sync: ListSyncStatus {
            state: parse_or_repo_err("sync_state", &row.text("sync_state")?, ListSyncState::parse)?,
            last_at: row.opt_timestamp("last_sync_at")?,
            next_at: row.opt_timestamp("next_sync_at")?,
            error_message: row.opt_text("error_message")?,
            error_at: row.opt_timestamp("error_at")?,
            paused_until: row.opt_timestamp("paused_until")?,
            fetch_fingerprint: row.opt_text("fetch_fingerprint")?,
        },
        counts: ListCounts {
            total: count_from(row, "count_total")?,
            in_library: count_from(row, "count_in_library")?,
            added: count_from(row, "count_added")?,
            requested: count_from(row, "count_requested")?,
            held: count_from(row, "count_held")?,
            filtered: count_from(row, "count_filtered")?,
            excluded: count_from(row, "count_excluded")?,
            unresolved: count_from(row, "count_unresolved")?,
        },
        credential_id: row.opt_text("credential_id")?,
        created_at: row.timestamp("created_at")?,
        updated_at: row.timestamp("updated_at")?,
    })
}

fn row_to_route(row: &SqlRow) -> AppResult<ListRoute> {
    Ok(ListRoute {
        kind: parse_or_repo_err("kind", &row.text("kind")?, MediaFacet::parse)?,
        library_id: row.text("library_id")?,
        quality_profile_id: row.opt_text("quality_profile_id")?,
        root_folder_id: row.opt_text("root_folder_id")?,
        monitor_type: row.text("monitor_type")?,
        min_availability: row.opt_text("min_availability")?,
        use_season_folders: row.opt_bool("use_season_folders")?,
        release_numbering: row.opt_text("release_numbering")?,
        tags: json_column(row, "tags_json", "[]")?,
    })
}

fn row_to_run(row: &SqlRow) -> AppResult<ListSyncRun> {
    Ok(ListSyncRun {
        id: row.text("id")?,
        subscription_id: row.text("subscription_id")?,
        job_run_id: row.opt_text("job_run_id")?,
        started_at: row.timestamp("started_at")?,
        finished_at: row.opt_timestamp("finished_at")?,
        outcome: parse_or_repo_err("outcome", &row.text("outcome")?, ListSyncRunOutcome::parse)?,
        counts: json_column(row, "counts_json", "{}")?,
        error_message: row.opt_text("error_message")?,
    })
}

use std::sync::Arc;

use async_graphql::{Context, Error, Result as GqlResult, Schema};
use scryer_application::AppError;
use scryer_application::AppUseCase;
use scryer_domain::User;
use scryer_infrastructure::SqliteServices;
use tokio::sync::broadcast;

use crate::{mutation::MutationRoot, query::QueryRoot, subscription::SubscriptionRoot};

/// Opaque handle to a log snapshot provider and subscription source.
/// The `scryer` crate constructs this from its `LogRingBuffer`.
#[derive(Clone)]
pub struct LogBuffer {
    snapshot_fn: Arc<dyn Fn(usize) -> Vec<String> + Send + Sync>,
    subscribe_fn: Arc<dyn Fn() -> broadcast::Receiver<String> + Send + Sync>,
}

impl LogBuffer {
    pub fn new(
        snapshot: impl Fn(usize) -> Vec<String> + Send + Sync + 'static,
        subscribe: impl Fn() -> broadcast::Receiver<String> + Send + Sync + 'static,
    ) -> Self {
        Self {
            snapshot_fn: Arc::new(snapshot),
            subscribe_fn: Arc::new(subscribe),
        }
    }

    pub fn snapshot(&self, limit: usize) -> Vec<String> {
        (self.snapshot_fn)(limit)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<String> {
        (self.subscribe_fn)()
    }
}

pub type ApiSchema = Schema<QueryRoot, MutationRoot, SubscriptionRoot>;

#[derive(Clone)]
pub struct ApiContext {
    pub app: AppUseCase,
    pub settings_db: SqliteServices,
    pub dev_auto_login: bool,
}

pub fn build_schema(
    app: AppUseCase,
    settings_db: SqliteServices,
    dev_auto_login: bool,
) -> ApiSchema {
    build_schema_with_log_buffer(app, settings_db, dev_auto_login, None)
}

pub fn build_schema_with_log_buffer(
    app: AppUseCase,
    settings_db: SqliteServices,
    dev_auto_login: bool,
    log_buffer: Option<LogBuffer>,
) -> ApiSchema {
    let mut builder =
        Schema::build(QueryRoot, MutationRoot::default(), SubscriptionRoot).data(ApiContext {
            app,
            settings_db,
            dev_auto_login,
        });
    if let Some(buf) = log_buffer {
        builder = builder.data(buf);
    }
    builder.finish()
}

pub(crate) fn app_from_ctx(ctx: &Context<'_>) -> GqlResult<AppUseCase> {
    Ok(ctx.data_unchecked::<ApiContext>().app.clone())
}

pub(crate) fn settings_db_from_ctx(ctx: &Context<'_>) -> GqlResult<SqliteServices> {
    Ok(ctx.data_unchecked::<ApiContext>().settings_db.clone())
}

pub(crate) fn to_gql_error(err: AppError) -> Error {
    Error::new(err.to_string())
}

pub(crate) fn actor_from_ctx(ctx: &Context<'_>) -> GqlResult<User> {
    ctx.data_opt::<User>()
        .cloned()
        .ok_or_else(|| Error::new("authentication required"))
}

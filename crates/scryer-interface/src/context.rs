use async_graphql::{Context, Error, Result as GqlResult, Schema};
use scryer_application::AppError;
use scryer_application::AppUseCase;
use scryer_domain::User;
use scryer_infrastructure::SqliteServices;

use crate::{mutation::MutationRoot, query::QueryRoot, subscription::SubscriptionRoot};

pub type ApiSchema = Schema<QueryRoot, MutationRoot, SubscriptionRoot>;

#[derive(Clone)]
pub struct ApiContext {
    pub app: AppUseCase,
    pub settings_db: SqliteServices,
    pub dev_auto_login: bool,
}

pub fn build_schema(app: AppUseCase, settings_db: SqliteServices, dev_auto_login: bool) -> ApiSchema {
    Schema::build(QueryRoot, MutationRoot::default(), SubscriptionRoot)
        .data(ApiContext { app, settings_db, dev_auto_login })
        .finish()
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

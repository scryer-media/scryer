pub mod downloads;
pub mod indexers;
pub mod proxy_config_store;
pub mod upstream_scheduler;

/// HELP text for the indexer metric families owned by this crate. Called from
/// the binary's recorder setup, after the recorder is installed.
pub use indexers::search_client::describe_indexer_metrics;

pub use downloads::clients::describe_download_client_router_metrics;

pub mod config_store {
    pub use scryer_infrastructure_crypto::config::*;
}

pub mod encryption {
    pub use scryer_infrastructure_crypto::*;
}

pub mod graphql {
    pub use crate::downloads::clients::weaver_graphql as weaver;
}

pub mod queries {
    pub use crate::indexers::db as indexer;
    pub use scryer_infrastructure_sql::runtime as sql_runtime;
}

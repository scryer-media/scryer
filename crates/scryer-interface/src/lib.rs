#![recursion_limit = "256"]

//! GraphQL API module boundaries.
//!
//! The monolithic `lib.rs` implementation was split into focused modules to align
//! with the architecture guidance while preserving the same public schema and
//! resolver behavior.

pub mod context;
pub mod metrics_extension;
pub mod mutation;
pub mod utils;

pub use metrics_extension::{GraphqlMetricsExtension, describe_graphql_metrics};
pub use scryer_interface_core::loaders::RequestLoaders;
pub use scryer_interface_media::{mappers, types};
pub use scryer_interface_query as query;
pub use scryer_interface_subscription as subscription;

pub use context::{
    ApiContext, ApiSchema, GRAPHQL_RECURSIVE_DEPTH_LIMIT, LogBuffer, LoginAttemptPrincipal,
    RestoreContext, RestoreRestartHandle, build_schema, build_schema_with_log_buffer,
    build_schema_with_log_buffer_and_restore,
    build_schema_with_log_buffer_and_restore_and_application_upgrade, export_schema_sdl,
};

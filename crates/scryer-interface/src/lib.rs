//! GraphQL API module boundaries.
//!
//! The monolithic `lib.rs` implementation was split into focused modules to align
//! with the architecture guidance while preserving the same public schema and
//! resolver behavior.

pub mod context;
pub mod mappers;
pub mod mutation;
pub mod query;
pub mod settings_graph;
pub mod subscription;
pub mod types;
pub mod utils;

pub use context::{ApiContext, ApiSchema, LogBuffer, build_schema, build_schema_with_log_buffer};

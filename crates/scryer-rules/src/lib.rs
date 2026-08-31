//! Scryer policy engine crate.
//!
//! Hosts every Rego policy family on one shared Regorus runtime:
//!
//! - [`release`] (re-exported at the crate root for compatibility) scores
//!   indexer release candidates for acquisition.
//! - [`maintenance`] evaluates existing library media against maintenance
//!   rules and returns a match / no-match / unknown decision. It never
//!   selects or performs an action.
//! - [`runtime`] owns the shared engine mechanics: construction, execution
//!   limits, input bounds, and content hashing. It understands serialized
//!   input/output and Regorus — never media deletion, requests, jobs,
//!   GraphQL, or stores.

pub(crate) mod builtins;
pub mod maintenance;
mod release;
pub mod runtime;
pub mod validation;

pub use release::*;

pub(crate) use release::{
    score_entry_wrapper_policy_path, score_entry_wrapper_rule_path, score_entry_wrapper_source,
};

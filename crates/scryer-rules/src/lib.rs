//! Scryer policy engine crate.
//!
//! Hosts every Rego policy family on one shared Regorus runtime:
//!
//! - [`release`] (re-exported at the crate root for compatibility) scores
//!   indexer release candidates for acquisition.
//! - [`maintenance`] evaluates existing library media against maintenance
//!   rules and returns a match / no-match / unknown decision. It never
//!   selects or performs an action.
//! - [`request`] decides a media request at submit time — approve, deny, or
//!   send it to a human — and may stamp tags on the title it creates. It runs
//!   synchronously while the requester waits, and nothing it can do fails the
//!   submission.
//! - [`policy`] is the family-agnostic core every family rides on: the build
//!   and evaluation loops, observation envelopes, host-derived holds, generated
//!   wrappers, and the bounded reason/tag decoders.
//! - [`runtime`] owns the shared engine mechanics: construction, execution
//!   limits, input bounds, and content hashing. It understands serialized
//!   input/output and Regorus — never media deletion, requests, jobs,
//!   GraphQL, or stores.

pub(crate) mod builtins;
pub mod maintenance;
pub mod policy;
mod release;
pub mod request;
pub mod runtime;
pub mod validation;

pub use release::*;

pub(crate) use release::{
    score_entry_wrapper_policy_path, score_entry_wrapper_rule_path, score_entry_wrapper_source,
};

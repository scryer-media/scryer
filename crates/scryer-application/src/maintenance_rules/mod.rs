//! Maintenance rules (RFC 137, "Policy Automation: Maintenance Rules,
//! Maintainerr Migration, and Future Request Rules").
//!
//! # Dormant module — RFC 137 Track A2
//!
//! This module currently holds *only* the static maintenance action catalog:
//! the closed action kinds, their descriptors (subjects, effect classes, risk
//! class, timing mode, allowed repeat modes), and the schema-versioned,
//! host-validated [`MaintenanceActionSpec`] a rule revision would store.
//!
//! Per Track A2 ("Dormant registry and capability model") **no reachable
//! handler is registered**. Nothing outside this module's own tests calls any
//! item declared here: there is no executor, job, scheduler hook, repository,
//! GraphQL resolver, or command wiring. Instance gates remain off, and the
//! contract tests in [`action_catalog`] prove that raw Rego output cannot
//! select an action — policy evaluation yields only `match`, `no_match`, or
//! `unknown` (RFC 9.1), and there is no API anywhere that turns a free-form
//! action name into a [`MaintenanceActionSpec`].
//!
//! Later waves add the remaining RFC 6.2 files (`commands.rs`, `queries.rs`,
//! `evaluation.rs`, `candidates.rs`, `action_execution.rs`, `claims.rs`,
//! `conflicts.rs`, `exclusions.rs`, `import.rs`) alongside this one.

pub mod action_catalog;

pub use action_catalog::{
    MAINTENANCE_ACTION_SCHEMA_VERSION, MaintenanceActionDescriptor, MaintenanceActionKind,
    MaintenanceActionParameters, MaintenanceActionSpec, MaintenanceActionSpecError,
    MaintenanceEffectClass, MaintenanceRepeatMode, MaintenanceRiskClass, MaintenanceSubjectKind,
    MaintenanceTimingMode, action_catalog, descriptor_for,
};

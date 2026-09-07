pub(crate) use crate::*;

pub(crate) mod admission;
pub(crate) mod anime_numbering;
pub(crate) mod submission;

/// Contract tests for the shared admission gate. Written from the gate's own
/// contract rather than from either caller, because the whole point is that
/// grab and import cannot disagree with it.
#[cfg(test)]
#[path = "admission_tests.rs"]
mod admission_tests;

pub(crate) mod convergence;
pub(crate) mod coverage;
pub(crate) mod decision_helpers;
pub(crate) mod delay_profile;
pub(crate) mod pending;
pub(crate) mod policy;
pub(crate) mod release_search;
pub(crate) mod rss;
pub(crate) mod scoring;
pub(crate) mod search_queries;
pub(crate) mod seed_goals;
pub(crate) mod targets;
pub(crate) mod wanted_views;
pub(crate) mod workflow;

pub(crate) use workflow as acquisition;

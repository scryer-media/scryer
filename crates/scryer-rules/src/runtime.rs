//! Shared Regorus runtime mechanics for every Scryer policy family.
//!
//! This module owns engine construction, execution limits, host-side input
//! bounds, package rewriting, and policy content hashing. It understands
//! serialized input/output and Regorus — never domain contracts. Each policy
//! family (release scoring, maintenance rules) layers its own input schema,
//! fixed query entry point, and closed output contract on top.

use core::num::{NonZeroU32, NonZeroUsize};
use core::time::Duration;

use regorus::utils::limits::ExecutionTimerConfig;
use regorus::{Engine, PolicyLengthConfig};
use serde::Serialize;

use crate::RulesError;

/// Host-enforced limits applied to every engine a policy family builds.
///
/// The execution timer is a per-evaluation wall-clock budget: Regorus resets
/// and restarts it on each `eval_rule`/`eval_query` call. Policy length limits
/// are enforced when a policy is added. Input bounds are enforced by the host
/// before serialized input reaches the engine.
#[derive(Debug, Clone, Copy)]
pub struct RuntimeLimits {
    /// Maximum wall-clock time for one evaluation call.
    pub max_execution_time: Duration,
    /// Interpreter work units between wall-clock checks.
    pub timer_check_interval: NonZeroU32,
    /// Maximum policy source size in bytes.
    pub max_policy_bytes: NonZeroUsize,
    /// Maximum number of lines per policy source.
    pub max_policy_lines: NonZeroUsize,
    /// Maximum column width per policy line.
    pub max_policy_col: NonZeroU32,
    /// Maximum serialized input document size in bytes.
    pub max_input_bytes: usize,
}

impl RuntimeLimits {
    /// Limits for release-scoring engines. The generous evaluation budget
    /// exists to stop pathological rules, not to constrain legitimate ones:
    /// existing release rules must keep producing identical decisions.
    pub fn release_defaults() -> Self {
        Self {
            max_execution_time: Duration::from_secs(1),
            timer_check_interval: NonZeroU32::new(4096).expect("non-zero"),
            max_policy_bytes: NonZeroUsize::new(1024 * 1024).expect("non-zero"),
            max_policy_lines: NonZeroUsize::new(20_000).expect("non-zero"),
            max_policy_col: NonZeroU32::new(1024).expect("non-zero"),
            max_input_bytes: 1024 * 1024,
        }
    }

    /// Limits for maintenance-rule engines. Tighter evaluation budget because
    /// scheduled evaluation fans out across many subjects, and a larger input
    /// bound because fact snapshots are richer than release documents.
    pub fn maintenance_defaults() -> Self {
        Self {
            max_execution_time: Duration::from_millis(250),
            timer_check_interval: NonZeroU32::new(4096).expect("non-zero"),
            max_policy_bytes: NonZeroUsize::new(256 * 1024).expect("non-zero"),
            max_policy_lines: NonZeroUsize::new(5_000).expect("non-zero"),
            max_policy_col: NonZeroU32::new(1024).expect("non-zero"),
            max_input_bytes: 4 * 1024 * 1024,
        }
    }
}

/// Build an engine with Scryer builtins registered and the given limits
/// applied. Every engine in this crate must be constructed through here so no
/// policy family evaluates without an execution budget.
pub(crate) fn configured_engine(limits: &RuntimeLimits) -> Engine {
    let mut engine = Engine::new();
    crate::builtins::register_builtins(&mut engine);
    engine.set_execution_timer_config(ExecutionTimerConfig {
        limit: limits.max_execution_time,
        check_interval: limits.timer_check_interval,
    });
    engine.set_policy_length_config(PolicyLengthConfig {
        max_col: limits.max_policy_col,
        max_file_bytes: limits.max_policy_bytes,
        max_lines: limits.max_policy_lines,
    });
    engine
}

/// Serialize an input document, enforcing the host-side size bound before it
/// reaches the engine.
pub(crate) fn bounded_input_value<T: Serialize>(
    input: &T,
    limits: &RuntimeLimits,
) -> Result<regorus::Value, RulesError> {
    let bytes = serde_json::to_vec(input)?;
    if bytes.len() > limits.max_input_bytes {
        return Err(RulesError::InputTooLarge {
            size: bytes.len(),
            limit: limits.max_input_bytes,
        });
    }
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    Ok(value.into())
}

/// Stable content hash of a policy source. Used to attribute evaluation
/// results to the exact policy revision that produced them.
pub fn content_hash(source: &str) -> String {
    blake3::hash(source.as_bytes()).to_hex().to_string()
}

/// Rewrite (or insert) the package declaration so the stored source always
/// carries the family package prefix plus the system-assigned rule ID, and
/// ensure `import rego.v1` is present. Family-specific wrappers delegate here.
pub(crate) fn rewrite_package_declaration_with_prefix(
    rego_source: &str,
    package_prefix: &str,
    rule_id: &str,
) -> String {
    let pkg_line = format!("package {package_prefix}.{rule_id}");
    let has_import = rego_source.lines().any(|l| l.trim() == "import rego.v1");
    let mut output = String::with_capacity(rego_source.len() + pkg_line.len() + 20);
    let mut found = false;

    for line in rego_source.lines() {
        if !found && line.trim().starts_with("package ") {
            output.push_str(&pkg_line);
            output.push('\n');
            if !has_import {
                output.push_str("import rego.v1\n");
            }
            found = true;
        } else {
            output.push_str(line);
            output.push('\n');
        }
    }

    if !found {
        let mut header = format!("{pkg_line}\n");
        if !has_import {
            header.push_str("import rego.v1\n");
        }
        return format!("{header}{output}");
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_hash_is_stable_and_distinguishes_sources() {
        let a = content_hash("package a\nmatch := true\n");
        let b = content_hash("package a\nmatch := true\n");
        let c = content_hash("package a\nmatch := false\n");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(a.len(), 64);
    }

    #[test]
    fn bounded_input_rejects_oversized_documents() {
        let mut limits = RuntimeLimits::maintenance_defaults();
        limits.max_input_bytes = 8;
        let err = bounded_input_value(&serde_json::json!({"key": "a long enough value"}), &limits)
            .unwrap_err();
        assert!(matches!(err, RulesError::InputTooLarge { .. }), "{err:?}");
    }

    #[test]
    fn configured_engine_rejects_oversized_policies() {
        let mut limits = RuntimeLimits::maintenance_defaults();
        limits.max_policy_bytes = NonZeroUsize::new(64).expect("non-zero");
        let mut engine = configured_engine(&limits);
        let big = format!(
            "package t\nimport rego.v1\n# {}\nmatch := true\n",
            "x".repeat(256)
        );
        assert!(engine.add_policy("t.rego".to_string(), big).is_err());
    }
}

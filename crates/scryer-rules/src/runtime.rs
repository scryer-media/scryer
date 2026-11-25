//! Shared Regorus runtime mechanics for every Scryer policy family.
//!
//! This module owns engine construction, execution limits, host-side input
//! bounds, package rewriting, and policy content hashing. It understands
//! serialized input/output and Regorus — never domain contracts. Each policy
//! family (release scoring, lifecycle rules) layers its own input schema,
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
            max_policy_bytes: NonZeroUsize::new(16 * 1024 * 1024).expect("non-zero"),
            max_policy_lines: NonZeroUsize::new(200_000).expect("non-zero"),
            max_policy_col: NonZeroU32::new(1024).expect("non-zero"),
            max_input_bytes: 1024 * 1024,
        }
    }

    /// Limits for lifecycle-rule engines. Tighter evaluation budget because
    /// scheduled evaluation fans out across many subjects, and a larger input
    /// bound because fact snapshots are richer than release documents.
    pub fn lifecycle_defaults() -> Self {
        Self {
            max_execution_time: Duration::from_millis(250),
            timer_check_interval: NonZeroU32::new(4096).expect("non-zero"),
            max_policy_bytes: NonZeroUsize::new(256 * 1024).expect("non-zero"),
            max_policy_lines: NonZeroUsize::new(5_000).expect("non-zero"),
            max_policy_col: NonZeroU32::new(1024).expect("non-zero"),
            max_input_bytes: 4 * 1024 * 1024,
        }
    }

    /// Limits for request-rule engines.
    ///
    /// The tightest evaluation budget of the three, because this family alone
    /// runs *synchronously on the submit path*: a requester is waiting on the
    /// answer, and the pre-flight banner re-evaluates as they type. 100 ms is
    /// far more than a well-written rule needs and still short enough that a
    /// runaway one cannot make submitting a request feel broken — and a rule
    /// that does exceed it becomes a per-rule error, which arbitrates to manual
    /// review rather than to an approval or a denial.
    ///
    /// The policy bounds match lifecycle (256 KiB, 5 000 lines): these are
    /// hand-authored matchers of the same shape. The input bound is a quarter of
    /// lifecycle's, because a request document describes one draft request and
    /// one title's metadata rather than a whole library subject's file list.
    pub fn request_defaults() -> Self {
        Self {
            max_execution_time: Duration::from_millis(100),
            timer_check_interval: NonZeroU32::new(4096).expect("non-zero"),
            max_policy_bytes: NonZeroUsize::new(256 * 1024).expect("non-zero"),
            max_policy_lines: NonZeroUsize::new(5_000).expect("non-zero"),
            max_policy_col: NonZeroU32::new(1024).expect("non-zero"),
            max_input_bytes: 1024 * 1024,
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
///
/// The document is returned as `serde_json::Value` rather than a Regorus value
/// because the policy core inspects it — observation envelopes are read off the
/// serialized form, not the typed one — before handing it to the engine.
pub(crate) fn bounded_input_document<T: Serialize + ?Sized>(
    input: &T,
    limits: &RuntimeLimits,
) -> Result<serde_json::Value, RulesError> {
    // The size is that of the serialized text, counted without keeping it.
    // The document is then built straight from the input: for the bounded
    // families' inputs, which hold no `f32` and no raw JSON, that is the value
    // reading the text back would give.
    let size = serialized_len(input)?;
    if size > limits.max_input_bytes {
        return Err(RulesError::InputTooLarge {
            size,
            limit: limits.max_input_bytes,
        });
    }
    Ok(serde_json::to_value(input)?)
}

/// The length of a value's serialized text, counted without keeping it.
pub(crate) fn serialized_len<T: Serialize + ?Sized>(value: &T) -> Result<usize, RulesError> {
    let mut counter = ByteCounter(0);
    serde_json::to_writer(&mut counter, value)?;
    Ok(counter.0)
}

/// Counts the bytes written to it and keeps none.
struct ByteCounter(usize);

impl std::io::Write for ByteCounter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 += bytes.len();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Serialize an input document straight into the engine's value type, enforcing
/// the same host-side size bound.
#[cfg(test)]
pub(crate) fn bounded_input_value<T: Serialize>(
    input: &T,
    limits: &RuntimeLimits,
) -> Result<regorus::Value, RulesError> {
    Ok(bounded_input_document(input, limits)?.into())
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
    let has_package = rego_source
        .lines()
        .any(|line| line.trim().starts_with("package "));
    let mut output = String::with_capacity(rego_source.len() + pkg_line.len() + 20);
    if !has_package {
        output.push_str(&pkg_line);
        output.push('\n');
        if !has_import {
            output.push_str("import rego.v1\n");
        }
    }

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
        let mut limits = RuntimeLimits::lifecycle_defaults();
        limits.max_input_bytes = 8;
        let err = bounded_input_value(&serde_json::json!({"key": "a long enough value"}), &limits)
            .unwrap_err();
        assert!(matches!(err, RulesError::InputTooLarge { .. }), "{err:?}");
    }

    /// What the bound did before: keep the serialized text, measure it, and
    /// read it back.
    fn reparsed_bounded_document<T: Serialize + ?Sized>(
        input: &T,
        limits: &RuntimeLimits,
    ) -> Result<serde_json::Value, RulesError> {
        let bytes = serde_json::to_vec(input)?;
        if bytes.len() > limits.max_input_bytes {
            return Err(RulesError::InputTooLarge {
                size: bytes.len(),
                limit: limits.max_input_bytes,
            });
        }
        Ok(serde_json::from_slice(&bytes)?)
    }

    fn assert_bounds_as_before<T: Serialize + ?Sized>(input: &T) {
        let size = serde_json::to_vec(input).unwrap().len();
        let mut limits = RuntimeLimits::lifecycle_defaults();
        for limit in [size.saturating_sub(1), size, size + 1] {
            limits.max_input_bytes = limit;
            match (
                bounded_input_document(input, &limits),
                reparsed_bounded_document(input, &limits),
            ) {
                (Ok(built), Ok(reparsed)) => {
                    assert_eq!(built, reparsed);
                    assert_eq!(
                        serde_json::to_vec(&built).unwrap(),
                        serde_json::to_vec(&reparsed).unwrap()
                    );
                }
                (
                    Err(RulesError::InputTooLarge { size, limit }),
                    Err(RulesError::InputTooLarge {
                        size: old_size,
                        limit: old_limit,
                    }),
                ) => assert_eq!((size, limit), (old_size, old_limit)),
                (built, reparsed) => panic!("bounded {built:?}, before {reparsed:?}"),
            }
        }
    }

    #[test]
    fn bounded_documents_match_reading_the_text_back() {
        assert_bounds_as_before(&crate::lifecycle::synthetic_lifecycle_input());
        assert_bounds_as_before(&crate::request::synthetic_request_input());
        assert_bounds_as_before(&serde_json::json!({
            "text": "Synthetic \"quoted\" \\ é \u{1} \u{1F600}",
            "numbers": [0, -1, i64::MIN, u64::MAX, 0.1, -0.0, 1e300, 5e-324, f64::NAN],
            "nested": {"z": null, "a": [true, false, {}], "m": {"1": 2}},
        }));
        let map: std::collections::BTreeMap<i64, Option<String>> =
            [(3, None), (-7, Some("synthetic".to_owned()))].into();
        assert_bounds_as_before(&map);
    }

    #[test]
    fn configured_engine_rejects_oversized_policies() {
        let mut limits = RuntimeLimits::lifecycle_defaults();
        limits.max_policy_bytes = NonZeroUsize::new(64).expect("non-zero");
        let mut engine = configured_engine(&limits);
        let big = format!(
            "package t\nimport rego.v1\n# {}\nmatch := true\n",
            "x".repeat(256)
        );
        assert!(engine.add_policy("t.rego".to_string(), big).is_err());
    }

    #[test]
    fn release_defaults_expand_source_bounds_without_widening_columns() {
        let release = RuntimeLimits::release_defaults();
        assert_eq!(release.max_policy_bytes.get(), 16 * 1024 * 1024);
        assert_eq!(release.max_policy_lines.get(), 200_000);
        assert_eq!(release.max_policy_col.get(), 1024);

        let lifecycle = RuntimeLimits::lifecycle_defaults();
        assert_eq!(lifecycle.max_policy_bytes.get(), 256 * 1024);
        assert_eq!(lifecycle.max_policy_lines.get(), 5_000);
        assert_eq!(lifecycle.max_policy_col.get(), 1024);

        let request = RuntimeLimits::request_defaults();
        assert_eq!(request.max_policy_bytes.get(), 256 * 1024);
        assert_eq!(request.max_policy_lines.get(), 5_000);
        assert_eq!(request.max_policy_col.get(), 1024);
    }
}

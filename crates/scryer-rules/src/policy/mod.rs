//! The shared policy core every Rego family in Scryer rides on.
//!
//! Release scoring, maintenance matching, and everything that comes after them
//! do the same thing: build a Regorus engine under host limits, add each user
//! policy plus a generated wrapper that projects its optional heads into one
//! closed object, serialize an input document once per subject, evaluate each
//! rule in isolation, and decode what came back into a family-shaped decision.
//! Only three things genuinely differ — the input document, the heads the
//! wrapper projects, and the decoder — so those are what a family supplies.
//!
//! Everything else is here: engine construction, the build loop, per-rule error
//! isolation, observation envelopes, host-derived holds, and the bounded
//! decoders for reason codes and tags. A family cannot opt out of the parts
//! that keep an unevaluable rule from deciding, because it never gets to write
//! them.

pub mod decode;
pub mod engine;
pub mod observation;
pub(crate) mod wrapper;

#[cfg(test)]
mod tests;

use std::collections::BTreeSet;

use regorus::Value;
use serde::Serialize;

use crate::RulesError;
use crate::runtime::RuntimeLimits;

pub use decode::{
    MAX_REASON_CODE_LEN, MAX_REASON_CODES, MAX_TAG_LEN, MAX_TAGS, RESERVED_TAG_PREFIX,
    decode_reasons, decode_tags,
};
pub use engine::{EvalOutcome, EvalRecord, PolicyEngine, PolicyEvaluator, RuleHandle};
pub use observation::{
    Observation, SerializedFacts, held_reason_codes, serialize_fact_namespaces, unobservable_facts,
};

/// The stored shape of one policy, whatever else a family hangs off it.
pub trait PolicyRecord {
    /// System-assigned rule ID. Package names and policy paths are derived from
    /// it, so it is part of the stored contract.
    fn id(&self) -> &str;
    /// Human-readable name, carried through to every record and error.
    fn name(&self) -> &str;
    /// The complete stored Rego module, package declaration and all.
    fn rego_source(&self) -> &str;
}

/// Everything one policy family has to say for itself.
///
/// Implementors are zero-sized markers: the family is a set of decisions about
/// packages, paths, heads, and decoding, not a value anyone constructs.
pub trait PolicyFamily: Sized + 'static {
    /// Family name, used in logs and diagnostics.
    const NAME: &'static str;
    /// Package prefix for user-authored policies of this family. Stored source
    /// carries it, so it can never change for a shipped family.
    const USER_PACKAGE_PREFIX: &'static str;
    /// Package prefix for the generated wrapper module.
    const WRAPPER_PACKAGE_PREFIX: &'static str;
    /// Whether the host resolves each policy's `input.facts.*` references at
    /// build time and holds a policy whose facts it could not observe.
    const TRACKS_REFERENCED_FACTS: bool = false;
    /// Whether the serialized input document is checked against
    /// [`RuntimeLimits::max_input_bytes`] before it reaches the engine.
    ///
    /// On by default: a family written against this core should refuse an
    /// oversized document rather than hand it to Regorus. It exists as a knob
    /// only so a family that shipped *without* the bound can keep its exact
    /// prior behaviour until someone deliberately decides to add one.
    const BOUND_INPUT: bool = true;

    /// The stored policy record.
    type Policy: PolicyRecord;
    /// The input document, serialized once per subject.
    type Input: Serialize;
    /// The closed output contract of one policy.
    type Decision;
    /// Per-rule metadata resolved at build time (facets, origin, …).
    type RuleExtra: Clone;
    /// Per-evaluation context that decides which rules apply. `()` for a family
    /// where every rule always applies.
    type EvalContext: ?Sized;
    /// The family's per-rule failure record.
    type EvalError;

    /// Host limits every engine of this family is built under.
    fn limits() -> RuntimeLimits;

    /// Path a stored policy is registered under. Part of the diagnostics
    /// contract: parse errors quote it.
    fn user_policy_path(rule_id: &str) -> String;

    /// Path the generated wrapper is registered under. Never persisted.
    fn wrapper_policy_path(rule_id: &str) -> String;

    /// Source of the generated wrapper. Build it with
    /// [`wrapper::object_wrapper_source`] so every family's wrapper defaults its
    /// heads the same way.
    fn wrapper_source(rule_id: &str) -> String;

    /// Query path the evaluator asks for, once per rule per subject.
    fn wrapper_rule_path(rule_id: &str) -> String;

    /// Per-rule metadata to carry alongside the loaded policy.
    fn rule_extra(policy: &Self::Policy) -> Self::RuleExtra;

    /// Family checks that must pass before a policy is added to the engine at
    /// all — a managed pack's origin contract, for instance.
    fn prepare_policy(_policy: &Self::Policy) -> Result<(), RulesError> {
        Ok(())
    }

    /// The `input.facts.*` names this policy reads, resolved statically.
    /// Meaningful only when [`Self::TRACKS_REFERENCED_FACTS`] is set; a family
    /// that returns facts here must set it, or nothing will ever be held.
    fn referenced_facts(
        _policy: &Self::Policy,
        _policy_path: &str,
    ) -> Result<BTreeSet<String>, String> {
        Ok(BTreeSet::new())
    }

    /// The Rego head an author writes to hold a subject themselves —
    /// `unknown if { … }` for maintenance, `manual if { … }` for requests.
    /// `None` for a family with no hold at all.
    fn hold_rule_name() -> Option<&'static str> {
        None
    }

    /// The decision a rule yields when the host held it before consulting it.
    /// Never called unless [`Self::TRACKS_REFERENCED_FACTS`] is set.
    fn held_decision(reason_codes: Vec<String>) -> Self::Decision;

    /// Whether this rule applies to the subject under evaluation. A rule that
    /// does not apply is skipped entirely: no record, no error.
    fn applies(_extra: &Self::RuleExtra, _ctx: &Self::EvalContext) -> bool {
        true
    }

    /// Turn the wrapper's value into the family's closed output contract.
    /// Fails closed: anything that is not exactly the declared shape is an
    /// error for that rule instead of a coerced decision.
    fn decode(
        value: &Value,
        rule_id: &str,
        rule_name: &str,
        extra: &Self::RuleExtra,
    ) -> Result<Self::Decision, String>;

    /// Family checks on a decoded decision — a managed pack's score bounds,
    /// for instance. A rejection turns the record into an error.
    fn post_decode(_decision: &Self::Decision, _extra: &Self::RuleExtra) -> Result<(), String> {
        Ok(())
    }

    /// Build the family's failure record for one rule.
    fn eval_error(rule: &RuleHandle<Self::RuleExtra>, message: String) -> Self::EvalError;
}

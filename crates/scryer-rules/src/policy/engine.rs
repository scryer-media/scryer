//! The family-agnostic engine and evaluator.
//!
//! One build loop, one evaluation loop, one error-isolation policy, one place
//! where a rule is held before it is consulted. A family supplies its input
//! document, its wrapper heads, and its decoder; it does not get to have its
//! own opinion about any of the mechanics here, which is the point — three
//! copies of this loop would be three chances to diverge on the one property
//! that matters, that a rule which could not be evaluated never decides.

use core::marker::PhantomData;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use regorus::Engine;
use tracing::warn;

use super::PolicyFamily;
use super::PolicyRecord;
use super::observation::{held_reason_codes, unobservable_facts};
use crate::RulesError;
use crate::runtime::{self, RuntimeLimits};

/// One loaded policy, with everything the evaluator needs to decide whether to
/// consult it and how to attribute what it says.
#[derive(Debug, Clone)]
pub struct RuleHandle<X> {
    pub id: String,
    pub name: String,
    pub content_hash: String,
    /// Facts this policy reads through `input.facts.*`, resolved statically at
    /// build time. Empty for families that do not track fact references, and
    /// for a policy that reads only always-known sections of the input.
    pub referenced_facts: BTreeSet<String>,
    /// Per-family rule metadata (facets, origin, …).
    pub extra: X,
}

/// One policy's decision, attributed to the exact policy revision that produced
/// it via `policy_content_hash`.
#[derive(Debug, Clone)]
pub struct EvalRecord<D> {
    pub rule_set_id: String,
    pub rule_set_name: String,
    pub policy_content_hash: String,
    pub decision: D,
}

/// Everything one evaluation pass produced: the decisions that were reached and
/// the per-rule failures that were isolated.
#[derive(Debug, Clone)]
pub struct EvalOutcome<D, E> {
    pub records: Vec<EvalRecord<D>>,
    pub errors: Vec<E>,
}

impl<D, E> Default for EvalOutcome<D, E> {
    fn default() -> Self {
        Self {
            records: Vec::new(),
            errors: Vec::new(),
        }
    }
}

/// Pre-compiled engine holding every active policy of one family.
///
/// Built once per rule-set revision and shared; evaluators are cheap clones
/// created per evaluation run.
pub struct PolicyEngine<F: PolicyFamily> {
    pub(crate) template: Arc<Engine>,
    pub(crate) rules: Vec<RuleHandle<F::RuleExtra>>,
    pub(crate) limits: RuntimeLimits,
    pub(crate) family: PhantomData<fn() -> F>,
}

impl<F: PolicyFamily> Clone for PolicyEngine<F> {
    fn clone(&self) -> Self {
        Self {
            template: Arc::clone(&self.template),
            rules: self.rules.clone(),
            limits: self.limits,
            family: PhantomData,
        }
    }
}

impl<F: PolicyFamily> PolicyEngine<F> {
    /// Build an engine under the family's standard limits.
    pub fn build(policies: &[F::Policy]) -> Result<Self, RulesError> {
        Self::build_with_limits(policies, F::limits())
    }

    /// Build an engine under caller-supplied limits. Exists so tests can prove
    /// the execution budget is enforced without waiting out the real one.
    pub fn build_with_limits(
        policies: &[F::Policy],
        limits: RuntimeLimits,
    ) -> Result<Self, RulesError> {
        let mut engine = runtime::configured_engine(&limits);
        let mut rules = Vec::with_capacity(policies.len());

        for policy in policies {
            F::prepare_policy(policy)?;

            let policy_path = F::user_policy_path(policy.id());
            engine
                .add_policy(policy_path.clone(), policy.rego_source().to_string())
                .map_err(|e| RulesError::Compilation(format!("{}: {e}", policy.id())))?;
            engine
                .add_policy(
                    F::wrapper_policy_path(policy.id()),
                    F::wrapper_source(policy.id()),
                )
                .map_err(|e| RulesError::Compilation(format!("{}: {e}", policy.id())))?;

            // A policy whose fact dependencies cannot be read off its source
            // must not load at all: the host could not then tell whether it is
            // deciding on evidence Scryer actually has.
            let referenced_facts = F::referenced_facts(policy, &policy_path)
                .map_err(|e| RulesError::Compilation(format!("{}: {e}", policy.id())))?;

            rules.push(RuleHandle {
                id: policy.id().to_string(),
                name: policy.name().to_string(),
                content_hash: runtime::content_hash(policy.rego_source()),
                referenced_facts,
                extra: F::rule_extra(policy),
            });
        }

        Ok(Self {
            template: Arc::new(engine),
            rules,
            limits,
            family: PhantomData,
        })
    }

    /// Build an empty engine (no policies loaded).
    pub fn empty() -> Self {
        let limits = F::limits();
        Self {
            template: Arc::new(runtime::configured_engine(&limits)),
            rules: Vec::new(),
            limits,
            family: PhantomData,
        }
    }

    /// True when no policies are loaded. Callers should skip evaluation.
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Number of loaded policies.
    pub fn rule_count(&self) -> usize {
        self.rules.len()
    }

    /// The loaded policies, in policy order.
    pub fn rules(&self) -> &[RuleHandle<F::RuleExtra>] {
        &self.rules
    }

    /// The limits this engine was built under.
    pub fn limits(&self) -> RuntimeLimits {
        self.limits
    }

    /// Create an evaluator for a single evaluation run.
    pub fn evaluator(&self) -> PolicyEvaluator<F> {
        PolicyEvaluator {
            engine: (*self.template).clone(),
            rules: self.rules.clone(),
            limits: self.limits,
            family: PhantomData,
        }
    }
}

/// Evaluates every loaded policy against one input document at a time.
pub struct PolicyEvaluator<F: PolicyFamily> {
    pub(crate) engine: Engine,
    pub(crate) rules: Vec<RuleHandle<F::RuleExtra>>,
    pub(crate) limits: RuntimeLimits,
    pub(crate) family: PhantomData<fn() -> F>,
}

impl<F: PolicyFamily> PolicyEvaluator<F> {
    /// Evaluate every loaded policy against one input document.
    ///
    /// Per-rule failures — compilation-clean rules that error at runtime,
    /// exceed the execution budget, or return a malformed decision — are
    /// collected and never abort the batch. A failing rule contributes no
    /// record, so it can never decide anything.
    ///
    /// For a family that tracks fact references, a rule reading a fact Scryer
    /// could not observe for this subject is held *before* it is consulted at
    /// all. On the simple surface an unknown fact is simply a missing key,
    /// which a policy would otherwise read as a decisive "no" — so the host,
    /// not the author, is what makes an unobservable fact fail closed. A held
    /// rule is not evaluated, so its reason codes are the observations' own:
    /// the operator sees *which fact* Scryer could not read, which is the
    /// actionable half.
    ///
    /// The serialized document is checked against the family's
    /// `max_input_bytes` unless the family opted out through
    /// [`PolicyFamily::BOUND_INPUT`], in which case it is handed to the engine
    /// whatever its size — see that constant for why the escape hatch exists.
    pub fn evaluate_policies(
        &mut self,
        input: &F::Input,
        ctx: &F::EvalContext,
    ) -> Result<EvalOutcome<F::Decision, F::EvalError>, RulesError> {
        let mut outcome = EvalOutcome::default();

        if self.rules.is_empty() {
            return Ok(outcome);
        }

        let document = if F::BOUND_INPUT {
            runtime::bounded_input_document(input, &self.limits)?
        } else {
            serde_json::to_value(input)?
        };
        let unobservable = if F::TRACKS_REFERENCED_FACTS {
            unobservable_facts(&document)
        } else {
            BTreeMap::new()
        };
        self.engine.set_input(document.into());

        for rule in &self.rules {
            if !F::applies(&rule.extra, ctx) {
                continue;
            }

            if F::TRACKS_REFERENCED_FACTS {
                let held_by = held_reason_codes(&rule.referenced_facts, &unobservable);
                if !held_by.is_empty() {
                    outcome.records.push(EvalRecord {
                        rule_set_id: rule.id.clone(),
                        rule_set_name: rule.name.clone(),
                        policy_content_hash: rule.content_hash.clone(),
                        decision: F::held_decision(held_by),
                    });
                    continue;
                }
            }

            match self.engine.eval_rule(F::wrapper_rule_path(&rule.id)) {
                Ok(value) => match F::decode(&value, &rule.id, &rule.name, &rule.extra) {
                    Ok(decision) => match F::post_decode(&decision, &rule.extra) {
                        Ok(()) => outcome.records.push(EvalRecord {
                            rule_set_id: rule.id.clone(),
                            rule_set_name: rule.name.clone(),
                            policy_content_hash: rule.content_hash.clone(),
                            decision,
                        }),
                        Err(message) => {
                            warn!(
                                family = F::NAME,
                                rule_id = rule.id.as_str(),
                                %message,
                                "policy rule output rejected"
                            );
                            outcome.errors.push(F::eval_error(rule, message));
                        }
                    },
                    Err(message) => {
                        warn!(
                            family = F::NAME,
                            rule_id = rule.id.as_str(),
                            %message,
                            "policy rule produced a malformed decision"
                        );
                        outcome.errors.push(F::eval_error(rule, message));
                    }
                },
                Err(e) => {
                    warn!(
                        family = F::NAME,
                        rule_id = rule.id.as_str(),
                        error = %e,
                        "policy rule evaluation failed, skipping"
                    );
                    outcome.errors.push(F::eval_error(rule, e.to_string()));
                }
            }
        }

        Ok(outcome)
    }
}

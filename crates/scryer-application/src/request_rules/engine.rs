//! The cached request-rules engine and its rebuild (spec 0003 FR-013).
//!
//! Compiling Rego is expensive and a request is decided while its requester
//! waits, so the engine is built once and swapped, exactly as the release
//! scoring engine is (`rules/rules.rs::rebuild_user_rules_engine`). Every
//! mutating authoring call rebuilds it, so an operator who saves a rule and
//! immediately submits a request is judged by what they just saved.
//!
//! # Why library scope is an application filter
//!
//! A rule set can be confined to specific libraries. That could be expressed
//! three ways: build one engine per library, pass the library through the
//! family's `EvalContext`, or build one engine and discard the votes of rules
//! whose scope does not cover the request. The third is what this does.
//!
//! One engine per library multiplies compile cost and memory by the library
//! count for a filter that touches one field. Threading it through
//! `EvalContext` would put a host authorization concern inside the policy
//! runtime, where the request family deliberately has none (`EvalContext = ()`,
//! WP2 report §9.4). Discarding out-of-scope votes costs one map lookup per
//! rule, is impossible to get subtly wrong, and keeps the scope readable in the
//! trace: an out-of-scope rule simply does not appear.
//!
//! The scope map is captured **with** the engine, in the same swap, so a rule
//! and the scope it was compiled under can never be read from different
//! generations.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use scryer_domain::RequestRuleEvaluationMode;
use scryer_rules::request::{RequestPolicy, RequestRulesEngine};

use crate::{AppError, AppResult, AppUseCase};

/// What the engine knows about one loaded rule set beyond its compiled policy.
#[derive(Clone, Debug)]
pub struct RequestRuleScope {
    pub name: String,
    pub mode: RequestRuleEvaluationMode,
    /// Empty means every library.
    pub library_ids: Vec<String>,
    pub revision_number: i64,
    pub content_hash: String,
}

impl RequestRuleScope {
    /// Whether this rule's vote counts for a request targeting `library_id`.
    pub fn covers(&self, library_id: &str) -> bool {
        self.library_ids.is_empty() || self.library_ids.iter().any(|id| id == library_id)
    }
}

/// The compiled engine plus the per-rule metadata the evaluation and trace need.
#[derive(Clone)]
pub struct RequestRulesEngineCache {
    pub engine: RequestRulesEngine,
    pub scopes: HashMap<String, RequestRuleScope>,
}

impl Default for RequestRulesEngineCache {
    fn default() -> Self {
        Self {
            engine: RequestRulesEngine::empty(),
            scopes: HashMap::new(),
        }
    }
}

impl RequestRulesEngineCache {
    pub fn is_empty(&self) -> bool {
        self.scopes.is_empty()
    }

    /// The rules whose scope covers `library_id`, as `(id, scope)` pairs.
    pub fn scopes_for_library(&self, library_id: &str) -> Vec<(&String, &RequestRuleScope)> {
        self.scopes
            .iter()
            .filter(|(_, scope)| scope.covers(library_id))
            .collect()
    }

    /// The strictest mode among the rules that were consulted, or `Disabled`
    /// when none were. Recorded on the trace so a mixed shadow/enforce
    /// evaluation reads honestly: one enforcing rule makes the evaluation an
    /// enforcing one.
    pub fn strictest_mode(&self, library_id: &str) -> RequestRuleEvaluationMode {
        let mut mode = RequestRuleEvaluationMode::Disabled;
        for (_, scope) in self.scopes_for_library(library_id) {
            mode = match (mode, scope.mode) {
                (RequestRuleEvaluationMode::Enforce, _)
                | (_, RequestRuleEvaluationMode::Enforce) => RequestRuleEvaluationMode::Enforce,
                (RequestRuleEvaluationMode::Shadow, _) | (_, RequestRuleEvaluationMode::Shadow) => {
                    RequestRuleEvaluationMode::Shadow
                }
                _ => RequestRuleEvaluationMode::Disabled,
            };
        }
        mode
    }
}

/// Shared handle the service bundle hangs on to.
pub type RequestRulesEngineHandle = Arc<RwLock<RequestRulesEngineCache>>;

impl AppUseCase {
    /// Read the current engine generation.
    ///
    /// A poisoned lock degrades to an empty engine rather than failing the
    /// submission: no rules means the legacy permission check decides, which is
    /// the safe direction, and the next rebuild replaces the poisoned value.
    pub(crate) fn request_rules_engine_snapshot(&self) -> RequestRulesEngineCache {
        self.services
            .customization
            .request_rules_engine
            .read()
            .map(|guard| guard.clone())
            .unwrap_or_else(|error| {
                tracing::warn!(
                    error = %error,
                    "request rules engine lock poisoned; evaluating with no rules"
                );
                RequestRulesEngineCache::default()
            })
    }

    /// Rebuild the engine from every rule set in `shadow` or `enforce`.
    ///
    /// Disabled rule sets are not loaded at all: a disabled rule must cost
    /// nothing, not even a compile, and it must not be able to fail the rebuild
    /// of the rules that *are* live.
    ///
    /// A rule whose stored source no longer compiles is skipped with a warning
    /// rather than failing the whole rebuild. It was validated when it was
    /// written, so reaching this branch means the runtime changed under it, and
    /// the alternative — refusing to build any engine — would silently disarm
    /// every other rule on the instance.
    pub async fn rebuild_request_rules_engine(&self) -> AppResult<()> {
        let rule_sets = self
            .services
            .customization
            .request_rule_sets
            .list_rule_sets()
            .await?;

        let mut policies: Vec<RequestPolicy> = Vec::new();
        let mut scopes: HashMap<String, RequestRuleScope> = HashMap::new();
        for rule_set in rule_sets
            .into_iter()
            .filter(|rule_set| rule_set.evaluation_mode != RequestRuleEvaluationMode::Disabled)
        {
            let Some(revision) = self
                .services
                .customization
                .request_rule_sets
                .get_revision(&rule_set.id, rule_set.current_revision_number)
                .await?
            else {
                tracing::warn!(
                    rule_set_id = rule_set.id.as_str(),
                    revision_number = rule_set.current_revision_number,
                    "request rule set has no current revision; skipping it"
                );
                continue;
            };

            let policy = RequestPolicy {
                id: rule_set.id.clone(),
                name: rule_set.name.clone(),
                rego_source: revision.rego_source.clone(),
            };
            scopes.insert(
                rule_set.id.clone(),
                RequestRuleScope {
                    name: rule_set.name,
                    mode: rule_set.evaluation_mode,
                    library_ids: rule_set.library_ids,
                    revision_number: revision.revision_number,
                    content_hash: revision.matcher_content_hash,
                },
            );
            policies.push(policy);
        }

        // Fast path: one compile for the whole set. Only when that fails does
        // it cost a per-rule compile to find out which one is at fault.
        let engine = match RequestRulesEngine::build(&policies) {
            Ok(engine) => engine,
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "a stored request rule no longer compiles; rebuilding without it"
                );
                policies.retain(|policy| {
                    match RequestRulesEngine::build(std::slice::from_ref(policy)) {
                        Ok(_) => true,
                        Err(error) => {
                            tracing::warn!(
                                rule_set_id = policy.id.as_str(),
                                error = %error,
                                "request rule dropped from the engine; it will not be evaluated"
                            );
                            scopes.remove(&policy.id);
                            false
                        }
                    }
                });
                RequestRulesEngine::build(&policies).map_err(|error| {
                    AppError::Validation(format!(
                        "failed to build the request rules engine: {error}"
                    ))
                })?
            }
        };
        let rule_count = policies.len();

        let mut guard = self
            .services
            .customization
            .request_rules_engine
            .write()
            .map_err(|error| {
                AppError::Repository(format!("request rules engine lock poisoned: {error}"))
            })?;
        *guard = RequestRulesEngineCache { engine, scopes };
        drop(guard);

        tracing::debug!(rule_count, "request rules engine rebuilt");
        Ok(())
    }
}

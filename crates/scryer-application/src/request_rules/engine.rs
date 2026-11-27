//! The cached request-rules engine and its rebuild.
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
//!
//! # Request rules and report rules
//!
//! A rule decides media requests or bad-file reports, never both: validation
//! refuses a rule that defines heads of both kinds. Each loaded rule's kind is
//! read once, at rebuild, from the heads its source defines, and an evaluation
//! runs only the rules of its own kind. A rule whose kind cannot be read, such
//! as a stored source that no longer parses, counts for both, so its failure
//! holds requests and reports alike.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use scryer_rules::request::{RequestPolicy, RequestRulesEngine};

use crate::{AppError, AppResult, AppUseCase};

/// What a loaded rule decides.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestRuleKind {
    /// Media requests: the acquisition and shaping heads.
    Request,
    /// Bad-file reports: `mark_bad` and `reject_report`.
    Report,
    /// The heads could not be read; the rule counts for both.
    Unknown,
}

impl RequestRuleKind {
    /// The kind of a stored rule source, from the heads it defines.
    pub fn of_source(rego_source: &str, rule_set_id: &str) -> Self {
        match scryer_rules::request_actions::scan_request_action_heads(rego_source, rule_set_id) {
            Ok(scan) => match (scan.decides_request(), scan.decides_report()) {
                (false, true) => Self::Report,
                (true, true) => Self::Unknown,
                _ => Self::Request,
            },
            Err(_) => Self::Unknown,
        }
    }

    pub fn judges_requests(self) -> bool {
        matches!(self, Self::Request | Self::Unknown)
    }

    pub fn judges_reports(self) -> bool {
        matches!(self, Self::Report | Self::Unknown)
    }
}

/// What the engine knows about one loaded rule set beyond its compiled policy.
#[derive(Clone, Debug)]
pub struct RequestRuleScope {
    pub name: String,
    /// Empty means every library.
    pub library_ids: Vec<String>,
    pub revision_number: i64,
    pub content_hash: String,
    pub kind: RequestRuleKind,
}

impl RequestRuleScope {
    /// Whether this rule's vote counts for a request targeting `library_id`.
    pub fn covers(&self, library_id: &str) -> bool {
        self.kind.judges_requests() && self.covers_library(library_id)
    }

    /// Whether this rule's vote counts for a bad-file report in `library_id`.
    pub fn covers_report(&self, library_id: &str) -> bool {
        self.kind.judges_reports() && self.covers_library(library_id)
    }

    fn covers_library(&self, library_id: &str) -> bool {
        self.library_ids.is_empty() || self.library_ids.iter().any(|id| id == library_id)
    }
}

/// The compiled engine plus the per-rule metadata the evaluation and trace need.
#[derive(Clone)]
pub struct RequestRulesEngineCache {
    pub engine: RequestRulesEngine,
    pub scopes: HashMap<String, RequestRuleScope>,
    /// Persisted policies that cannot be loaded still participate as scoped errors.
    pub load_errors: HashMap<String, String>,
    /// Monotonically increasing refresh identity. A rebuild may install its
    /// result only while this still identifies its own refresh.
    pub generation: u64,
    /// True after a persisted rule mutation could not be reflected in this
    /// generation. Evaluation must hold rather than consult stale policy.
    pub unavailable: bool,
    /// The fact families some loaded request rule may read. Evaluation leaves
    /// the others unread.
    pub fact_reads: crate::request_rules::facts::RequestFactReads,
    /// Whether some loaded report rule may read whether the reporter asked
    /// for the reported media. A report reads it only then.
    pub report_reads_requester_relationship: bool,
}

impl Default for RequestRulesEngineCache {
    fn default() -> Self {
        Self {
            engine: RequestRulesEngine::empty(),
            scopes: HashMap::new(),
            load_errors: HashMap::new(),
            generation: 0,
            unavailable: false,
            fact_reads: crate::request_rules::facts::RequestFactReads::ALL,
            report_reads_requester_relationship: true,
        }
    }
}

impl RequestRulesEngineCache {
    pub fn is_empty(&self) -> bool {
        self.scopes.is_empty()
    }

    /// The request rules whose scope covers `library_id`, as `(id, scope)`
    /// pairs.
    pub fn scopes_for_library(&self, library_id: &str) -> Vec<(&String, &RequestRuleScope)> {
        self.scopes
            .iter()
            .filter(|(_, scope)| scope.covers(library_id))
            .collect()
    }

    /// The report rules whose scope covers `library_id`, as `(id, scope)`
    /// pairs.
    pub fn report_scopes_for_library(&self, library_id: &str) -> Vec<(&String, &RequestRuleScope)> {
        self.scopes
            .iter()
            .filter(|(_, scope)| scope.covers_report(library_id))
            .collect()
    }
}

/// Shared handle the service bundle hangs on to. A generation is installed
/// whole and never changed in place, so a reader takes the `Arc` and drops the
/// lock without copying the engine or its maps.
pub type RequestRulesEngineHandle = Arc<RwLock<Arc<RequestRulesEngineCache>>>;

impl AppUseCase {
    /// Read the current engine generation.
    ///
    /// A poisoned lock produces an unavailable generation. Request evaluation
    /// holds rather than deciding against an empty engine that might omit
    /// persisted enabled rules.
    pub(crate) fn request_rules_engine_snapshot(&self) -> Arc<RequestRulesEngineCache> {
        self.services
            .customization
            .request_rules_engine
            .read()
            .map(|guard| Arc::clone(&guard))
            .unwrap_or_else(|error| {
                tracing::warn!(
                    error = %error,
                    "request rules engine lock poisoned; request rules are unavailable"
                );
                Arc::new(RequestRulesEngineCache {
                    unavailable: true,
                    ..RequestRulesEngineCache::default()
                })
            })
    }

    /// Begin a new refresh by invalidating the current policy before any store
    /// read can suspend. A later refresh owns a distinct generation, so an
    /// earlier in-flight read cannot replace a newer unavailable generation.
    fn begin_request_rules_engine_rebuild(&self) -> AppResult<u64> {
        let mut guard = self
            .services
            .customization
            .request_rules_engine
            .write()
            .map_err(|error| {
                AppError::Repository(format!("request rules engine lock poisoned: {error}"))
            })?;
        let generation = guard.generation.checked_add(1).ok_or_else(|| {
            AppError::Repository("request rules engine generation exhausted".to_string())
        })?;
        // Readers keep the generation they already hold; the unavailable one
        // replaces it, copied once per rebuild rather than once per read.
        *guard = Arc::new(RequestRulesEngineCache {
            unavailable: true,
            generation,
            ..RequestRulesEngineCache::clone(&guard)
        });
        Ok(generation)
    }

    /// Rebuild the engine from every enabled rule set.
    ///
    /// Disabled rule sets are not loaded at all: a disabled rule must cost
    /// nothing, not even a compile, and it must not be able to fail the rebuild
    /// of the rules that *are* live.
    ///
    /// A rule whose stored source no longer compiles remains a scoped error.
    /// Healthy rules still run, while a failed enabled policy holds requests
    /// in its libraries instead of silently falling back to auto-approval.
    pub async fn rebuild_request_rules_engine(&self) -> AppResult<()> {
        let generation = self.begin_request_rules_engine_rebuild()?;
        let rule_sets = self
            .services
            .customization
            .request_rule_sets
            .list_rule_sets()
            .await?;

        let rule_sets: Vec<_> = rule_sets
            .into_iter()
            .filter(|rule_set| rule_set.enabled)
            .collect();
        let revision_keys: Vec<(String, i64)> = rule_sets
            .iter()
            .map(|rule_set| (rule_set.id.clone(), rule_set.current_revision_number))
            .collect();
        let revisions = self
            .services
            .customization
            .request_rule_sets
            .get_revisions(&revision_keys)
            .await?;

        let mut policies: Vec<RequestPolicy> = Vec::new();
        let mut scopes: HashMap<String, RequestRuleScope> = HashMap::new();
        let mut load_errors = HashMap::new();
        for (rule_set, revision) in rule_sets.into_iter().zip(revisions) {
            let Some(revision) = revision else {
                tracing::warn!(
                    rule_set_id = rule_set.id.as_str(),
                    revision_number = rule_set.current_revision_number,
                    "request rule set has no current revision; retaining a scoped error"
                );
                scopes.insert(
                    rule_set.id.clone(),
                    RequestRuleScope {
                        name: rule_set.name,
                        library_ids: rule_set.library_ids,
                        revision_number: rule_set.current_revision_number,
                        content_hash: String::new(),
                        kind: RequestRuleKind::Unknown,
                    },
                );
                load_errors.insert(
                    rule_set.id,
                    "request rule current revision is missing".into(),
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
                    library_ids: rule_set.library_ids,
                    revision_number: revision.revision_number,
                    content_hash: revision.source_content_hash,
                    kind: RequestRuleKind::of_source(&revision.rego_source, &rule_set.id),
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
                                "request rule cannot compile; retaining a scoped error"
                            );
                            load_errors.insert(policy.id.clone(), error.to_string());
                            false
                        }
                    }
                });
                RequestRulesEngine::build(&policies).map_err(|error| {
                    AppError::validation_refused(
                        crate::error_reasons::request_rules::ENGINE_BUILD_FAILED,
                        format!("failed to build the request rules engine: {error}"),
                    )
                })?
            }
        };
        let rule_count = policies.len();
        // Every loaded rule of a kind is evaluated, whichever libraries it
        // covers, so the reads are the union over that kind.
        let kind_of = |policy: &&RequestPolicy| {
            scopes
                .get(&policy.id)
                .map_or(RequestRuleKind::Unknown, |scope| scope.kind)
        };
        let fact_reads = crate::request_rules::facts::RequestFactReads::for_policies(
            policies
                .iter()
                .filter(|policy| kind_of(policy).judges_requests())
                .map(|policy| (policy.rego_source.as_str(), policy.id.as_str())),
        );
        let report_reads_requester_relationship =
            crate::request_rules::facts::report_rules_read_requester_relationship(
                policies
                    .iter()
                    .filter(|policy| kind_of(policy).judges_reports())
                    .map(|policy| (policy.rego_source.as_str(), policy.id.as_str())),
            );

        let mut guard = self
            .services
            .customization
            .request_rules_engine
            .write()
            .map_err(|error| {
                AppError::Repository(format!("request rules engine lock poisoned: {error}"))
            })?;
        if guard.generation != generation {
            tracing::debug!(
                generation,
                current_generation = guard.generation,
                "discarding stale request rules engine rebuild"
            );
            return Ok(());
        }
        *guard = Arc::new(RequestRulesEngineCache {
            engine,
            scopes,
            load_errors,
            generation,
            unavailable: false,
            fact_reads,
            report_reads_requester_relationship,
        });
        drop(guard);

        tracing::debug!(rule_count, "request rules engine rebuilt");
        Ok(())
    }
}

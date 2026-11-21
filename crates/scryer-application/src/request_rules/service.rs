//! Authoring, previewing, and enabling request rule sets.
//!
//! Every call here needs [`AppPermission::ManageCatalogSettings`]. Authoring or
//! previewing a rule that reads `input.requester.*` needs
//! [`AppPermission::ManagePermissions`] on top, for the reason lifecycle's
//! person-fact gate gives: writing a rule about *media* is catalog
//! administration, writing one about a named person is a use of the instance's
//! identity records.
//!
//! Rule sets are created disabled. Enabling one is always a deliberate second
//! call ([`AppUseCase::set_request_rule_enabled`]).

use chrono::Utc;
use scryer_domain::{
    AppPermission, ExternalId, Id, MediaFacet, RequestRuleRevision, RequestRuleSet, User,
};
use scryer_rules::diagnostic_codes::{
    RULE_AUDIO_LANGUAGE_UNKNOWN, RULE_MONITOR_UNSUPPORTED, RULE_QUALITY_PROFILE_AMBIGUOUS,
    RULE_QUALITY_PROFILE_UNKNOWN,
};
use scryer_rules::policy::{MAX_TAGS, TagProblem, tag_problem};
use scryer_rules::request::{
    RequestPolicy, RequestRulesEngine, RequestVote, rewrite_package_declaration,
};
use scryer_rules::request_actions::{
    RequestActionHead, RequestActionScan, RequestDeclaredActions, RequestShaping,
    scan_request_action_heads,
};
use scryer_rules::rule_scan::RuleLiteral;
use scryer_rules::runtime::content_hash;
use scryer_rules::validation::{
    ValidationDiagnostic, ValidationResult, request_person_targeted_paths, validate_request_rule,
};

use crate::media_requests::snapshot::MediaRequestMetadataSnapshot;
use crate::request_rules::evaluation::RequestDecisionReason;
use crate::request_rules::facts::{RequestDraft, build_request_input};
use crate::{AppError, AppResult, AppUseCase};

/// Upper bound on a requested lease, in days — ten years.
///
/// The same bound the lifecycle grace period uses, and for the same reason:
/// an approved lease is materialized into `starts_at + Duration::days(n)`, and
/// `chrono` panics rather than saturating once that addition leaves the
/// representable range. A nonsense lease would therefore not be a long lease; it
/// would take down the import that tried to start its clock.
pub const REQUEST_MAX_LEASE_DAYS: i64 = 3650;

// ── Request models ──────────────────────────────────────────────────────────

/// A new rule set plus the source of its first revision.
#[derive(Clone, Debug)]
pub struct RequestRuleDraft {
    pub name: String,
    pub description: String,
    pub rego_source: String,
    /// Empty means every library.
    pub library_ids: Vec<String>,
    /// The simple editor's document for this source, kept opaque. `None`
    /// stores none.
    pub builder_model: Option<serde_json::Value>,
}

/// Which rule source a preview should run.
#[derive(Clone, Debug)]
pub enum RequestRulePreviewSource {
    /// The current revision of a stored rule set.
    Stored { rule_set_id: String },
    /// An unsaved draft from the editor. Validated and compiled, never stored.
    Inline { rego_source: String },
}

/// The hypothetical request a preview is evaluated against.
///
/// It names a real requester and a real library on purpose: the point of the
/// preview is to answer "what would this rule do to *this person* asking for
/// *this title*", and a synthetic requester would not exercise the permission,
/// history, or linked-provider facts that make request rules useful.
#[derive(Clone, Debug)]
pub struct RequestRuleSample {
    pub user_id: String,
    pub library_id: String,
    pub external_ids: Vec<ExternalId>,
    pub quality_profile_id: Option<String>,
    pub monitor_type: Option<String>,
    /// `Some(None)` is an explicit "forever"; `None` means the sample did not
    /// say. The preview evaluates both as a request for forever.
    pub lease_days: Option<Option<i64>>,
    /// `None` leaves it to the library; an empty list asks for no requirement.
    pub required_audio_languages: Option<Vec<String>>,
    /// `None` leaves it to the library.
    pub preferred_audio_languages: Option<Vec<String>>,
}

#[derive(Clone, Debug)]
pub struct RequestRulePreviewRequest {
    pub source: RequestRulePreviewSource,
    pub sample: RequestRuleSample,
}

// ── Read models ─────────────────────────────────────────────────────────────

/// A rule set with the revision currently in force.
#[derive(Clone, Debug)]
pub struct RequestRuleSetDetail {
    pub rule_set: RequestRuleSet,
    pub revision: RequestRuleRevision,
    /// The stored builder document. Read by the single-rule read and returned
    /// by a write that set it; `None` everywhere else.
    pub builder_model: Option<serde_json::Value>,
}

impl RequestRuleSetDetail {
    /// What the current revision's source can do, read without running it.
    /// `None` when the source cannot be parsed.
    pub fn declared_actions(&self) -> Option<RequestDeclaredActions> {
        scan_request_action_heads(&self.revision.rego_source, &self.rule_set.id)
            .ok()
            .map(|scan| scan.declared_actions())
    }
}

/// A draft's validation, with what its source can do.
#[derive(Clone, Debug)]
pub struct RequestRuleValidation {
    pub result: ValidationResult,
    /// `None` when the source cannot be parsed.
    pub declared_actions: Option<RequestDeclaredActions>,
}

/// What one rule did to one sample.
#[derive(Clone, Debug)]
pub struct RequestRulePreviewResult {
    /// The stored rule set's id, or the throwaway id an inline draft compiled
    /// under.
    pub rule_set_id: String,
    pub source_content_hash: String,
    pub evaluated_at: chrono::DateTime<Utc>,
    /// `None` exactly when `error` is set: a rule that failed produced no vote.
    pub vote: Option<RequestVote>,
    /// True when the host held the rule because a fact it reads was
    /// unobservable, rather than the author writing `manual if`.
    pub held: bool,
    pub reasons: Vec<RequestDecisionReason>,
    pub tags: Vec<String>,
    /// The subset of `tags` the title tag registry does not define.
    ///
    /// A request rule cannot be validated statically the way a lifecycle
    /// action's tag list can — the labels only exist inside the compiled Rego —
    /// so the preview is the one moment an author can be told that a tag they
    /// emit will be dropped at approval until somebody defines it.
    pub undefined_tags: Vec<String>,
    /// What the rule's shaping heads set. Empty when it failed or was held.
    pub shaping: RequestShaping,
    /// The text `notify` produced. A preview never sends it.
    pub notify: Option<String>,
    pub error: Option<String>,
    /// The exact document the rule saw, pretty-printed. An author debugging
    /// "why did this not match" needs the facts, not just the verdict.
    pub input_json: String,
    /// True when the sample's metadata could not be fully established.
    pub metadata_partial: bool,
}

/// A rule set with its current revision; a missing revision is a repository
/// fault.
fn request_rule_detail(
    rule_set: RequestRuleSet,
    revision: Option<RequestRuleRevision>,
) -> AppResult<RequestRuleSetDetail> {
    let revision = revision.ok_or_else(|| {
        AppError::Repository(format!(
            "request rule set {} has no revision {}",
            rule_set.id, rule_set.current_revision_number
        ))
    })?;
    Ok(RequestRuleSetDetail {
        rule_set,
        revision,
        builder_model: None,
    })
}

// ── Service ─────────────────────────────────────────────────────────────────

impl AppUseCase {
    pub async fn list_request_rule_sets(&self, actor: &User) -> AppResult<Vec<RequestRuleSet>> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;
        self.services
            .customization
            .request_rule_sets
            .list_rule_sets()
            .await
    }

    /// Every rule set with the revision currently in force, so a list view can
    /// show what each rule actually says without one round trip per row.
    pub async fn list_request_rule_set_details(
        &self,
        actor: &User,
    ) -> AppResult<Vec<RequestRuleSetDetail>> {
        let rule_sets = self.list_request_rule_sets(actor).await?;
        let keys: Vec<(String, i64)> = rule_sets
            .iter()
            .map(|rule_set| (rule_set.id.clone(), rule_set.current_revision_number))
            .collect();
        let revisions = self
            .services
            .customization
            .request_rule_sets
            .get_revisions(&keys)
            .await?;
        rule_sets
            .into_iter()
            .zip(revisions)
            .map(|(rule_set, revision)| request_rule_detail(rule_set, revision))
            .collect()
    }

    /// One rule set with its revision in force. `with_builder_model` reads
    /// the stored builder document too, for the one view that shows it.
    pub async fn get_request_rule_set(
        &self,
        actor: &User,
        id: &str,
        with_builder_model: bool,
    ) -> AppResult<Option<RequestRuleSetDetail>> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;
        let Some(rule_set) = self
            .services
            .customization
            .request_rule_sets
            .get_rule_set(id)
            .await?
        else {
            return Ok(None);
        };
        let mut detail = self.load_request_rule_detail(rule_set).await?;
        if !with_builder_model {
            return Ok(Some(detail));
        }
        // A document that no longer parses is dropped rather than failing the
        // read: the source is what the rule is, the document is an editor aid.
        detail.builder_model = self
            .services
            .customization
            .request_rule_sets
            .get_builder_model(&detail.rule_set.id)
            .await?
            .and_then(|json| serde_json::from_str(&json).ok());
        Ok(Some(detail))
    }

    pub async fn list_request_rule_revisions(
        &self,
        actor: &User,
        rule_set_id: &str,
    ) -> AppResult<Vec<RequestRuleRevision>> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;
        self.services
            .customization
            .request_rule_sets
            .list_revisions(rule_set_id)
            .await
    }

    /// What a rule set's revision in force can do, for a write that returns
    /// the rule set without its revision. One revision read.
    pub async fn request_rule_declared_actions(
        &self,
        actor: &User,
        rule_set: &RequestRuleSet,
    ) -> AppResult<Option<RequestDeclaredActions>> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;
        Ok(self
            .load_request_rule_detail(rule_set.clone())
            .await?
            .declared_actions())
    }

    pub async fn create_request_rule_set(
        &self,
        actor: &User,
        draft: RequestRuleDraft,
    ) -> AppResult<RequestRuleSetDetail> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;

        let name = require_non_empty(&draft.name, "name")?;
        let builder_model_json = draft
            .builder_model
            .as_ref()
            .map(validated_builder_model_json)
            .transpose()?;
        let id = Id::new_rego_safe().0;
        let prepared = prepare_source(&id, &draft.rego_source)?;
        self.require_request_person_fact_authority(actor, &prepared.rego_source, &id)
            .await?;
        self.require_request_shaping_values(&prepared.scan, &draft.library_ids)
            .await?;

        let now = Utc::now();
        let rule_set = RequestRuleSet {
            id: id.clone(),
            name,
            description: draft.description,
            // Created disabled: enabling is a second, deliberate call.
            enabled: false,
            library_ids: draft.library_ids,
            current_revision_number: 1,
            created_at: now,
            updated_at: now,
        };
        let revision = build_revision(&id, 1, &prepared, actor, now);

        self.services
            .customization
            .request_rule_sets
            .create_rule_set(&rule_set, &revision, builder_model_json.as_deref())
            .await?;
        self.rebuild_request_rules_engine().await?;

        Ok(RequestRuleSetDetail {
            rule_set,
            revision,
            builder_model: draft.builder_model,
        })
    }

    /// Appends revision N+1. Revision N is left exactly as written, so a
    /// decision recorded against it stays attributable.
    ///
    /// `builder_model` is `Some(Some(doc))` to replace the stored builder
    /// document and `Some(None)` to clear it. `None` keeps the stored document
    /// only when the source is unchanged; a changed source with no document
    /// clears it, so a later builder save cannot bring back a source that was
    /// edited by hand.
    pub async fn update_request_rule_source(
        &self,
        actor: &User,
        rule_set_id: &str,
        rego_source: &str,
        builder_model: Option<Option<serde_json::Value>>,
    ) -> AppResult<RequestRuleSetDetail> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;

        let mut builder_model_json = match &builder_model {
            None => None,
            Some(None) => Some(None),
            Some(Some(model)) => Some(Some(validated_builder_model_json(model)?)),
        };
        let mut rule_set = self.require_request_rule_set(rule_set_id).await?;
        let prepared = prepare_source(&rule_set.id, rego_source)?;
        self.require_request_person_fact_authority(actor, &prepared.rego_source, &rule_set.id)
            .await?;
        self.require_request_shaping_values(&prepared.scan, &rule_set.library_ids)
            .await?;
        if builder_model_json.is_none() {
            let current = self
                .services
                .customization
                .request_rule_sets
                .get_revision(&rule_set.id, rule_set.current_revision_number)
                .await?;
            let source_unchanged = current
                .is_some_and(|revision| revision.source_content_hash == prepared.content_hash);
            if !source_unchanged {
                builder_model_json = Some(None);
            }
        }

        let now = Utc::now();
        let revision_number = rule_set.current_revision_number + 1;
        let revision = build_revision(&rule_set.id, revision_number, &prepared, actor, now);

        self.services
            .customization
            .request_rule_sets
            .add_revision(
                &revision,
                now,
                builder_model_json.as_ref().map(|model| model.as_deref()),
            )
            .await?;

        rule_set.current_revision_number = revision_number;
        rule_set.updated_at = now;
        self.rebuild_request_rules_engine().await?;
        Ok(RequestRuleSetDetail {
            rule_set,
            revision,
            builder_model: builder_model.flatten(),
        })
    }

    /// Renames and re-scopes without touching the source, so no revision is
    /// created. The engine still rebuilds: library scope is part of what the
    /// cache holds.
    pub async fn update_request_rule_metadata(
        &self,
        actor: &User,
        rule_set_id: &str,
        name: String,
        description: String,
        library_ids: Vec<String>,
    ) -> AppResult<RequestRuleSet> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;

        let mut rule_set = self.require_request_rule_set(rule_set_id).await?;
        let name = require_non_empty(&name, "name")?;
        // A new scope can drop the only library that accepted a shaped value,
        // so the revision in force is checked against it as a save would be.
        let mut stored_scope = rule_set.library_ids.clone();
        stored_scope.sort();
        let mut new_scope = library_ids.clone();
        new_scope.sort();
        if stored_scope != new_scope {
            let detail = self.load_request_rule_detail(rule_set.clone()).await?;
            let scan = scan_request_action_heads(&detail.revision.rego_source, &rule_set.id)
                .map_err(|error| {
                    AppError::validation_refused(
                        crate::error_reasons::request_rules::RULE_VALIDATION_FAILED,
                        format!("rule validation failed: {error}"),
                    )
                })?;
            self.require_request_shaping_values(&scan, &library_ids)
                .await?;
        }
        let now = Utc::now();

        self.services
            .customization
            .request_rule_sets
            .update_rule_set_metadata(&rule_set.id, &name, &description, &library_ids, now)
            .await?;

        rule_set.name = name;
        rule_set.description = description;
        rule_set.library_ids = library_ids;
        rule_set.updated_at = now;
        self.rebuild_request_rules_engine().await?;
        Ok(rule_set)
    }

    /// Enable or disable a rule set.
    pub async fn set_request_rule_enabled(
        &self,
        actor: &User,
        rule_set_id: &str,
        enabled: bool,
    ) -> AppResult<RequestRuleSet> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;

        let mut rule_set = self.require_request_rule_set(rule_set_id).await?;
        let now = Utc::now();

        self.services
            .customization
            .request_rule_sets
            .update_rule_set_enabled(&rule_set.id, enabled, now)
            .await?;

        rule_set.enabled = enabled;
        rule_set.updated_at = now;
        self.rebuild_request_rules_engine().await?;
        Ok(rule_set)
    }

    /// Delete a rule set.
    ///
    /// Unconditional, unlike a lifecycle rule. A lifecycle rule's deletion
    /// would cascade away the action runs that are the only record of what it
    /// did to someone's library; a request rule executes nothing, and its
    /// decisions live in `request_rule_decisions`, which deliberately has no
    /// foreign key to the rule. The trace outlives the rule, so there
    /// is nothing to protect by refusing.
    pub async fn delete_request_rule_set(&self, actor: &User, rule_set_id: &str) -> AppResult<()> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;
        let rule_set = self.require_request_rule_set(rule_set_id).await?;
        self.services
            .customization
            .request_rule_sets
            .delete_rule_set(&rule_set.id)
            .await?;
        self.rebuild_request_rules_engine().await
    }

    /// Editor support: compile and check a draft without storing it, with
    /// the same checks a save runs. `library_ids` is the scope the draft will
    /// be saved with; empty means every library.
    pub async fn validate_request_rule_source(
        &self,
        actor: &User,
        rego_source: &str,
        library_ids: &[String],
    ) -> AppResult<RequestRuleValidation> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;

        let scratch_id = Id::new_rego_safe().0;
        let rewritten = rewrite_package_declaration(rego_source, &scratch_id);
        let mut result = validate_request_rule(&rewritten, &scratch_id).map_err(|error| {
            AppError::validation_refused(
                crate::error_reasons::request_rules::RULE_VALIDATION_FAILED,
                format!("rule validation failed: {error}"),
            )
        })?;
        let scan = scan_request_action_heads(&rewritten, &scratch_id).ok();
        if result.valid
            && let Some(scan) = &scan
        {
            let diagnostics = self
                .request_shaping_value_diagnostics(scan, library_ids)
                .await?;
            if !diagnostics.is_empty() {
                result.valid = false;
                result.errors.extend(
                    diagnostics
                        .iter()
                        .map(|diagnostic| diagnostic.message.clone()),
                );
                result.diagnostics.extend(diagnostics);
            }
        }
        Ok(RequestRuleValidation {
            result,
            declared_actions: scan.map(|scan| scan.declared_actions()),
        })
    }

    /// Run one rule against one hypothetical request. Nothing is written.
    pub async fn preview_request_rule(
        &self,
        actor: &User,
        request: RequestRulePreviewRequest,
    ) -> AppResult<RequestRulePreviewResult> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;

        let policy = self.resolve_request_preview_policy(request.source).await?;
        // Preview evaluates against a real requester and reports the verdict,
        // so an ungated one is a person-fact oracle: ask "would this approve
        // for user x" and read the answer. The same bar authoring meets applies
        // here, for a stored rule as much as for a draft.
        self.require_request_person_fact_authority(actor, &policy.rego_source, &policy.id)
            .await?;

        let sample = request.sample;
        let requester = self
            .services
            .identity
            .users
            .get_by_id(sample.user_id.trim())
            .await?
            .ok_or_else(|| {
                AppError::not_found_reason(
                    crate::error_reasons::request_rules::SAMPLE_REQUESTER_NOT_FOUND,
                    "sample requester not found",
                )
            })?;
        let requester = self.attach_user_authorization(requester).await?;
        let library = self
            .services
            .catalog
            .libraries
            .get_by_id(sample.library_id.trim())
            .await?
            .ok_or_else(|| {
                AppError::not_found_reason(
                    crate::error_reasons::request_rules::SAMPLE_LIBRARY_NOT_FOUND,
                    "sample library not found",
                )
            })?;

        let external_ids =
            crate::media_requests::normalize_media_request_external_ids(sample.external_ids)?;
        let (snapshot, enriched_ids) = if external_ids.is_empty() {
            // A sample with no identifiers has no title to look up. That is a
            // legitimate preview — "what does this rule do on requester facts
            // alone" — and every metadata fact is unknown, which is exactly
            // what the rule would see for a title SMG cannot resolve.
            (
                MediaRequestMetadataSnapshot::unavailable("no_sample_external_ids"),
                Vec::new(),
            )
        } else {
            let enrichment = self
                .enrich_request_draft(&library.facet, external_ids.clone())
                .await;
            (enrichment.snapshot, enrichment.external_ids)
        };
        let external_ids = if enriched_ids.is_empty() {
            external_ids
        } else {
            enriched_ids
        };

        let (requested_required_audio_languages, requested_preferred_audio_languages) =
            crate::validated_title_audio_overrides(
                sample.required_audio_languages.clone(),
                sample.preferred_audio_languages.clone(),
            )?;
        let quality_profile_id = sample.quality_profile_id.clone();
        let quality_profile_name = match quality_profile_id.as_deref() {
            None => None,
            Some(profile_id) => {
                let settings = self.load_quality_profile_settings().await?;
                crate::settings::runtime::quality_profile_by_id(&settings.profiles, profile_id)?
                    .map(|profile| profile.name.clone())
            }
        };

        let draft = RequestDraft {
            facet: library.facet.clone(),
            title: preview_sample_title(&library.facet),
            year: None,
            identity_fingerprint: crate::media_requests::media_request_identity_fingerprint(
                &external_ids,
            ),
            external_ids,
            quality_profile_id,
            quality_profile_name,
            // Normalized as submit stores it, so the sample matches what a
            // real request with this monitor type would give the rule.
            monitor_type: crate::media_requests::normalize_requested_monitor_type(
                &library.facet,
                sample.monitor_type.clone(),
            )?,
            monitor_selection: None,
            requested_lease_days: sample.lease_days.flatten(),
            requested_required_audio_languages,
            requested_preferred_audio_languages,
            origin: scryer_domain::MediaRequestOrigin::Manual,
            list_rank: None,
        };

        let evaluated_at = Utc::now();
        let metadata_partial = snapshot.partial;
        let context = self
            .assemble_request_input_context(
                &requester,
                &library,
                &draft,
                snapshot,
                evaluated_at,
                None,
                // The preview shows the whole input document.
                crate::request_rules::facts::RequestFactReads::ALL,
            )
            .await?;
        let input = build_request_input(context);
        let input_json = serde_json::to_string_pretty(&input).unwrap_or_else(|_| "{}".to_string());

        let source_content_hash = content_hash(&policy.rego_source);
        let rule_set_id = policy.id.clone();
        let engine = RequestRulesEngine::build(std::slice::from_ref(&policy)).map_err(|error| {
            AppError::validation_refused(
                crate::error_reasons::request_rules::COMPILE_FAILED,
                format!("rule failed to compile: {error}"),
            )
        })?;
        let mut evaluator = engine.evaluator();
        let outcome = evaluator.evaluate(&input).map_err(|error| {
            AppError::validation_refused(
                crate::error_reasons::request_rules::RULE_EVALUATION_FAILED,
                format!("rule evaluation failed: {error}"),
            )
        })?;

        let (vote, held, reasons, tags, shaping, notify, error) =
            match (outcome.records.first(), outcome.errors.first()) {
                (Some(record), _) => (
                    Some(record.decision.vote),
                    record.decision.held,
                    record
                        .decision
                        .reason_codes
                        .iter()
                        .map(|code| RequestDecisionReason {
                            code: code.clone(),
                            rule_name: record.rule_set_name.clone(),
                        })
                        .collect(),
                    record.decision.tags.clone(),
                    record.decision.shaping.clone(),
                    record.decision.notify.clone(),
                    None,
                ),
                (None, Some(failure)) => (
                    None,
                    false,
                    Vec::new(),
                    Vec::new(),
                    RequestShaping::default(),
                    None,
                    Some(failure.message.clone()),
                ),
                (None, None) => (
                    None,
                    false,
                    Vec::new(),
                    Vec::new(),
                    RequestShaping::default(),
                    None,
                    Some("rule produced no decision".to_string()),
                ),
            };

        // The preview keeps returning every emitted label; `undefined_tags`
        // only says which of them cannot land yet.
        let undefined_tags = self.undefined_title_tag_labels(&tags).await?;

        Ok(RequestRulePreviewResult {
            rule_set_id,
            source_content_hash,
            evaluated_at,
            vote,
            held,
            reasons,
            tags,
            undefined_tags,
            shaping,
            notify,
            error,
            input_json,
            metadata_partial,
        })
    }

    /// Gate authoring — or previewing — a rule that reads the requester
    /// document.
    ///
    /// Catalog-settings management is enough to write a rule about *media*.
    /// Writing one about *people* — who is asking, what they hold, which
    /// provider accounts they linked — is a use of the instance's identity
    /// records, so it asks for the authority that hands out permissions.
    ///
    /// The bar is on reaching the facts, not on running: a stored revision keeps
    /// evaluating under the system principal, because revoking an author's
    /// permission must not silently stop a rule an operator enabled.
    ///
    /// The path list comes from the same static extraction the engine holds
    /// rules on. A source it cannot resolve is **refused**, not treated as
    /// person-free: an unreadable source has no person list either, and reading
    /// the empty answer as "asks about nobody" is exactly the mistake that would
    /// let one through.
    async fn require_request_person_fact_authority(
        &self,
        actor: &User,
        rego_source: &str,
        rule_set_id: &str,
    ) -> AppResult<()> {
        let paths = request_person_targeted_paths(rego_source, rule_set_id).map_err(|error| {
            AppError::validation_refused(
                crate::error_reasons::request_rules::RULE_VALIDATION_FAILED,
                format!("rule validation failed: {error}"),
            )
        })?;
        if paths.is_empty() {
            return Ok(());
        }

        if self
            .has_app_permission(actor, AppPermission::ManagePermissions)
            .await?
        {
            return Ok(());
        }

        Err(AppError::unauthorized_reason(
            crate::error_reasons::request_rules::PERSON_TARGETING_PERMISSION_REQUIRED,
            format!(
                "This rule reads facts about specific people ({}), so saving it needs permission to manage permissions. Ask an administrator to save it, or rewrite the rule without those facts.",
                paths.join(", ")
            ),
        ))
    }

    /// Refuse a source whose shaping heads name a value this instance cannot
    /// apply: a quality profile that does not exist, a monitor type no library
    /// the rule applies to accepts, or an audio language Scryer does not know.
    async fn require_request_shaping_values(
        &self,
        scan: &RequestActionScan,
        library_ids: &[String],
    ) -> AppResult<()> {
        let diagnostics = self
            .request_shaping_value_diagnostics(scan, library_ids)
            .await?;
        if diagnostics.is_empty() {
            return Ok(());
        }
        Err(AppError::validation_refused_with_diagnostics(
            crate::error_reasons::request_rules::RULE_VALIDATION_FAILED,
            format!(
                "Request rule validation failed:\n- {}",
                diagnostics
                    .iter()
                    .map(|diagnostic| diagnostic.message.as_str())
                    .collect::<Vec<_>>()
                    .join("\n- ")
            ),
            diagnostics,
        ))
    }

    /// One diagnostic per shaping literal this instance cannot apply, located
    /// at the literal. Each catalog is read once, and only when a head that
    /// needs it is written.
    pub(crate) async fn request_shaping_value_diagnostics(
        &self,
        scan: &RequestActionScan,
        library_ids: &[String],
    ) -> AppResult<Vec<ValidationDiagnostic>> {
        let mut diagnostics = Vec::new();

        let profiles = scan.literals(RequestActionHead::SetQualityProfile);
        if !profiles.is_empty() {
            let settings = self.load_quality_profile_settings().await?;
            for literal in profiles {
                match quality_profiles_named(&settings.profiles, &literal.value).count() {
                    1 => {}
                    0 => diagnostics.push(literal_diagnostic(
                        RULE_QUALITY_PROFILE_UNKNOWN,
                        RequestActionHead::SetQualityProfile,
                        literal,
                        format!("no quality profile is called '{}'", literal.value),
                    )),
                    _ => diagnostics.push(literal_diagnostic(
                        RULE_QUALITY_PROFILE_AMBIGUOUS,
                        RequestActionHead::SetQualityProfile,
                        literal,
                        format!(
                            "more than one quality profile is called '{}'; rename one so the rule names a single profile",
                            literal.value
                        ),
                    )),
                }
            }
        }

        let monitors = scan.literals(RequestActionHead::SetMonitor);
        if !monitors.is_empty() {
            let facets = self.request_rule_scope_facets(library_ids).await?;
            for literal in monitors {
                if !facets
                    .iter()
                    .any(|facet| shaping_monitor_for_facet(facet, &literal.value).is_some())
                {
                    diagnostics.push(literal_diagnostic(
                        RULE_MONITOR_UNSUPPORTED,
                        RequestActionHead::SetMonitor,
                        literal,
                        format!(
                            "no library this rule applies to accepts the monitor type '{}'",
                            literal.value
                        ),
                    ));
                }
            }
        }

        for head in [
            RequestActionHead::SetRequiredAudioLanguages,
            RequestActionHead::SetPreferredAudioLanguages,
        ] {
            for literal in scan.literals(head) {
                if crate::settings::runtime::validated_preference_languages(
                    "audio language",
                    vec![literal.value.clone()],
                )
                .is_err()
                {
                    diagnostics.push(literal_diagnostic(
                        RULE_AUDIO_LANGUAGE_UNKNOWN,
                        head,
                        literal,
                        format!("'{}' is not an audio language Scryer knows", literal.value),
                    ));
                }
            }
        }
        Ok(diagnostics)
    }

    /// The facets of the libraries a rule applies to: the named ones, or
    /// every library for a rule with no scope. One read either way.
    async fn request_rule_scope_facets(
        &self,
        library_ids: &[String],
    ) -> AppResult<Vec<MediaFacet>> {
        let libraries = self.services.catalog.libraries.list(None).await?;
        let mut facets: Vec<MediaFacet> = Vec::new();
        for library in libraries {
            if (library_ids.is_empty() || library_ids.contains(&library.id))
                && !facets.contains(&library.facet)
            {
                facets.push(library.facet);
            }
        }
        Ok(facets)
    }

    pub(crate) async fn require_request_rule_set(&self, id: &str) -> AppResult<RequestRuleSet> {
        self.services
            .customization
            .request_rule_sets
            .get_rule_set(id)
            .await?
            .ok_or_else(|| {
                AppError::not_found_reason(
                    crate::error_reasons::request_rules::SET_NOT_FOUND,
                    format!("request rule set {id} not found"),
                )
            })
    }

    pub(crate) async fn load_request_rule_detail(
        &self,
        rule_set: RequestRuleSet,
    ) -> AppResult<RequestRuleSetDetail> {
        let revision = self
            .services
            .customization
            .request_rule_sets
            .get_revision(&rule_set.id, rule_set.current_revision_number)
            .await?;
        request_rule_detail(rule_set, revision)
    }

    async fn resolve_request_preview_policy(
        &self,
        source: RequestRulePreviewSource,
    ) -> AppResult<RequestPolicy> {
        match source {
            RequestRulePreviewSource::Stored { rule_set_id } => {
                let rule_set = self.require_request_rule_set(&rule_set_id).await?;
                let detail = self.load_request_rule_detail(rule_set).await?;
                Ok(RequestPolicy {
                    id: detail.rule_set.id,
                    name: detail.rule_set.name,
                    rego_source: detail.revision.rego_source,
                })
            }
            RequestRulePreviewSource::Inline { rego_source } => {
                let scratch_id = Id::new_rego_safe().0;
                let prepared = prepare_source(&scratch_id, &rego_source)?;
                Ok(RequestPolicy {
                    id: scratch_id,
                    name: "preview".to_string(),
                    rego_source: prepared.rego_source,
                })
            }
        }
    }
}

// ── Validation helpers ──────────────────────────────────────────────────────

/// A rule source that passed every authoring-time check, ready to persist.
pub(crate) struct PreparedSource {
    pub(crate) rego_source: String,
    pub(crate) content_hash: String,
    /// What the source can do, for the checks that need this instance's
    /// catalogs.
    pub(crate) scan: RequestActionScan,
}

/// The quality profiles whose trimmed name is `name`. Profile names are not
/// unique, so a caller that needs one profile treats more than one match as
/// no answer rather than picking one.
pub(crate) fn quality_profiles_named<'a>(
    profiles: &'a [crate::QualityProfile],
    name: &'a str,
) -> impl Iterator<Item = &'a crate::QualityProfile> + 'a {
    profiles
        .iter()
        .filter(move |profile| profile.name.trim() == name.trim())
}

/// The monitor type a shaping head would write for one facet, or `None` when
/// the facet does not accept it. The accepted values are the monitoring
/// choices a library of that facet may hold as its default, so a movie takes
/// only `monitored` or `unmonitored` and a series or anime takes only the
/// episode choices. A rule never selects seasons or movies, so `advanced` is
/// refused for every facet. A requester's own pick is checked elsewhere and
/// is not narrowed by this.
pub(crate) fn shaping_monitor_for_facet(facet: &MediaFacet, monitor: &str) -> Option<String> {
    let normalized =
        crate::media_requests::normalize_requested_monitor_type(facet, Some(monitor.to_string()))
            .ok()
            .flatten()?;
    crate::settings::runtime::default_monitor_type_choices(facet)
        .iter()
        .any(|choice| choice.replace('_', "").eq_ignore_ascii_case(&normalized))
        .then_some(normalized)
}

fn literal_diagnostic(
    code: &'static str,
    head: RequestActionHead,
    literal: &RuleLiteral,
    message: String,
) -> ValidationDiagnostic {
    ValidationDiagnostic {
        code,
        message,
        line: Some(literal.line),
        column: Some(literal.column),
        path: Some(head.rule_name().to_string()),
    }
}

/// The builder document as stored, checked the way every rule family's is:
/// an object naming the request family and a version, within the shared size
/// bound. The server never reads inside it.
pub(crate) fn validated_builder_model_json(model: &serde_json::Value) -> AppResult<String> {
    crate::rules::builder_model::checked_builder_model(
        model,
        crate::rules::builder_model::BuilderFamily::Request,
    )
}

/// Rewrite the package to the system-assigned id, then run every check that has
/// to hold before a revision is written.
fn prepare_source(rule_id: &str, rego_source: &str) -> AppResult<PreparedSource> {
    let rewritten = rewrite_package_declaration(rego_source, rule_id);
    let validation = validate_request_rule(&rewritten, rule_id).map_err(|error| {
        AppError::validation_refused(
            crate::error_reasons::request_rules::RULE_VALIDATION_FAILED,
            format!("rule validation failed: {error}"),
        )
    })?;
    if !validation.valid {
        return Err(AppError::validation_refused_with_diagnostics(
            if validation
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == crate::error_reasons::rules::TARGETS_PERSON)
            {
                crate::error_reasons::rules::TARGETS_PERSON
            } else {
                crate::error_reasons::request_rules::RULE_VALIDATION_FAILED
            },
            format!(
                "Request rule validation failed:\n- {}",
                validation.errors.join("\n- ")
            ),
            validation.diagnostics,
        ));
    }
    let scan = scan_request_action_heads(&rewritten, rule_id).map_err(|error| {
        AppError::validation_refused(
            crate::error_reasons::request_rules::RULE_VALIDATION_FAILED,
            format!("rule validation failed: {error}"),
        )
    })?;
    Ok(PreparedSource {
        content_hash: content_hash(&rewritten),
        rego_source: rewritten,
        scan,
    })
}

fn build_revision(
    rule_set_id: &str,
    revision_number: i64,
    prepared: &PreparedSource,
    actor: &User,
    created_at: chrono::DateTime<Utc>,
) -> RequestRuleRevision {
    RequestRuleRevision {
        id: Id::new().0,
        rule_set_id: rule_set_id.to_string(),
        revision_number,
        rego_source: prepared.rego_source.clone(),
        source_content_hash: prepared.content_hash.clone(),
        created_by: Some(actor.id.clone()),
        created_at,
    }
}

fn require_non_empty(value: &str, field: &str) -> AppResult<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(AppError::validation_refused(
            crate::error_reasons::request_rules::FIELD_REQUIRED,
            format!("{field} must not be empty"),
        ));
    }
    Ok(trimmed.to_string())
}

/// A stand-in title for the preview's draft document. Preview resolves the
/// sample's facts from its external identifiers, not its name, so the name only
/// has to be something a rule reading `input.request.title` sees as a title.
fn preview_sample_title(facet: &MediaFacet) -> String {
    match facet {
        MediaFacet::Movie => "Sample movie".to_string(),
        MediaFacet::Series => "Sample series".to_string(),
        MediaFacet::Anime => "Sample anime".to_string(),
    }
}

/// Validate a tag list a *human* supplied — an approver's override of the
/// policy's tags.
///
/// The bounds are the policy family's own ([`MAX_TAGS`] and [`tag_problem`]),
/// because a tag an approver types and a tag a rule emits end up in the same
/// column on the same title, and a rule may not mint one an approver could
/// not. Each tag is trimmed first, and each problem refuses with its own
/// reason.
pub fn validate_tag_list(tags: Vec<String>) -> AppResult<Vec<String>> {
    if tags.len() > MAX_TAGS {
        return Err(AppError::validation_refused(
            crate::error_reasons::request_rules::TAG_LIMIT_EXCEEDED,
            format!(
                "a request may carry at most {MAX_TAGS} tags, {} were supplied",
                tags.len()
            ),
        ));
    }
    let mut validated: Vec<String> = Vec::with_capacity(tags.len());
    for tag in tags {
        let tag = tag.trim().to_string();
        if let Some(problem) = tag_problem(&tag) {
            use crate::error_reasons::request_rules as reasons;
            let reason = match problem {
                TagProblem::Empty => reasons::TAG_REQUIRED,
                TagProblem::TooLong => reasons::TAG_TOO_LONG,
                TagProblem::ReservedPrefix => reasons::TAG_USES_RESERVED_PREFIX,
                TagProblem::UnsupportedCharacter(_) => reasons::TAG_CONTAINS_UNSUPPORTED_CHARACTER,
            };
            return Err(AppError::validation_refused(reason, problem.message(&tag)));
        }
        if !validated.contains(&tag) {
            validated.push(tag);
        }
    }
    Ok(validated)
}

/// Validate a requested or approved lease, in days. `None` is forever.
pub fn validate_lease_days(lease_days: Option<i64>) -> AppResult<Option<i64>> {
    match lease_days {
        None => Ok(None),
        Some(days) if days < 1 => Err(AppError::validation_refused(
            crate::error_reasons::request_rules::LEASE_OUT_OF_RANGE,
            "a lease must be at least one day, or forever".to_string(),
        )),
        Some(days) if days > REQUEST_MAX_LEASE_DAYS => Err(AppError::validation_refused(
            crate::error_reasons::request_rules::LEASE_OUT_OF_RANGE,
            format!(
                "a lease must be at most {REQUEST_MAX_LEASE_DAYS} days (ten years), or forever"
            ),
        )),
        Some(days) => Ok(Some(days)),
    }
}

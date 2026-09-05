//! Authoring, previewing, and arming request rule sets (spec 0003 FR-010,
//! FR-013, FR-014, FR-015).
//!
//! Every call here needs [`AppPermission::ManageCatalogSettings`]. Authoring or
//! previewing a matcher that reads `input.requester.*` needs
//! [`AppPermission::ManagePermissions`] on top, for the reason maintenance's
//! person-fact gate gives: writing a rule about *media* is catalog
//! administration, writing one about a named person is a use of the instance's
//! identity records.
//!
//! Rule sets are created disabled. Arming is always a deliberate second call
//! ([`AppUseCase::set_request_rule_mode`]), and the instance gate is a third
//! switch on top of that.

use chrono::Utc;
use scryer_domain::{
    AppPermission, ExternalId, Id, MediaFacet, RequestRuleEvaluationMode, RequestRuleRevision,
    RequestRuleSet, User,
};
use scryer_rules::policy::{MAX_TAG_LEN, MAX_TAGS, RESERVED_TAG_PREFIX};
use scryer_rules::request::{
    RequestPolicy, RequestRulesEngine, RequestVote, rewrite_package_declaration,
};
use scryer_rules::runtime::content_hash;
use scryer_rules::validation::{
    ValidationResult, request_person_targeted_paths, validate_request_rule,
};

use crate::media_requests::snapshot::MediaRequestMetadataSnapshot;
use crate::request_rules::evaluation::RequestDecisionReason;
use crate::request_rules::facts::{RequestDraft, build_request_input};
use crate::{AppError, AppResult, AppUseCase};

/// Upper bound on a requested lease, in days — ten years.
///
/// The same bound the maintenance grace period uses, and for the same reason:
/// an approved lease is materialized into `starts_at + Duration::days(n)`, and
/// `chrono` panics rather than saturating once that addition leaves the
/// representable range. A nonsense lease would therefore not be a long lease; it
/// would take down the import that tried to start its clock.
pub const REQUEST_MAX_LEASE_DAYS: i64 = 3650;

// ── Request models ──────────────────────────────────────────────────────────

/// A new rule set plus its first matcher revision.
#[derive(Clone, Debug)]
pub struct RequestRuleDraft {
    pub name: String,
    pub description: String,
    pub rego_source: String,
    /// Empty means every library.
    pub library_ids: Vec<String>,
}

/// Which matcher a preview should run.
#[derive(Clone, Debug)]
pub enum RequestRulePreviewMatcher {
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
    /// say, which is also treated as forever. The double option exists so a
    /// client can distinguish the two even though they behave alike today.
    pub lease_days: Option<Option<i64>>,
}

#[derive(Clone, Debug)]
pub struct RequestRulePreviewRequest {
    pub matcher: RequestRulePreviewMatcher,
    pub sample: RequestRuleSample,
}

// ── Read models ─────────────────────────────────────────────────────────────

/// A rule set with the revision currently in force.
#[derive(Clone, Debug)]
pub struct RequestRuleSetDetail {
    pub rule_set: RequestRuleSet,
    pub revision: RequestRuleRevision,
}

/// What one matcher did to one sample.
#[derive(Clone, Debug)]
pub struct RequestRulePreviewResult {
    /// The stored rule set's id, or the throwaway id an inline draft compiled
    /// under.
    pub rule_set_id: String,
    pub matcher_content_hash: String,
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
    /// A request rule cannot be validated statically the way a maintenance
    /// action's tag list can — the labels only exist inside the compiled Rego —
    /// so the preview is the one moment an author can be told that a tag they
    /// emit will be dropped at approval until somebody defines it.
    pub undefined_tags: Vec<String>,
    pub error: Option<String>,
    /// The exact document the rule saw, pretty-printed. An author debugging
    /// "why did this not match" needs the facts, not just the verdict.
    pub input_json: String,
    /// True when the sample's metadata could not be fully established.
    pub metadata_partial: bool,
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
        let mut details = Vec::with_capacity(rule_sets.len());
        for rule_set in rule_sets {
            details.push(self.load_request_rule_detail(rule_set).await?);
        }
        Ok(details)
    }

    pub async fn get_request_rule_set(
        &self,
        actor: &User,
        id: &str,
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
        self.load_request_rule_detail(rule_set).await.map(Some)
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

    pub async fn create_request_rule_set(
        &self,
        actor: &User,
        draft: RequestRuleDraft,
    ) -> AppResult<RequestRuleSetDetail> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;

        let name = require_non_empty(&draft.name, "name")?;
        let id = Id::new_rego_safe().0;
        let prepared = prepare_matcher(&id, &draft.rego_source)?;
        self.require_request_person_fact_authority(actor, &prepared.rego_source, &id)
            .await?;

        let now = Utc::now();
        let rule_set = RequestRuleSet {
            id: id.clone(),
            name,
            description: draft.description,
            // Created disabled: arming is a second, deliberate call, and the
            // instance gate is a third.
            enabled: false,
            evaluation_mode: RequestRuleEvaluationMode::Disabled,
            library_ids: draft.library_ids,
            current_revision_number: 1,
            created_at: now,
            updated_at: now,
        };
        let revision = build_revision(&id, 1, &prepared, actor, now);

        self.services
            .customization
            .request_rule_sets
            .create_rule_set(&rule_set, &revision)
            .await?;
        self.rebuild_request_rules_engine().await?;

        Ok(RequestRuleSetDetail { rule_set, revision })
    }

    /// Appends revision N+1. Revision N is left exactly as written, so a
    /// decision recorded against it stays attributable (FR-016).
    pub async fn update_request_rule_matcher(
        &self,
        actor: &User,
        rule_set_id: &str,
        rego_source: &str,
    ) -> AppResult<RequestRuleSetDetail> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;

        let mut rule_set = self.require_request_rule_set(rule_set_id).await?;
        let prepared = prepare_matcher(&rule_set.id, rego_source)?;
        self.require_request_person_fact_authority(actor, &prepared.rego_source, &rule_set.id)
            .await?;

        let now = Utc::now();
        let revision_number = rule_set.current_revision_number + 1;
        let revision = build_revision(&rule_set.id, revision_number, &prepared, actor, now);

        self.services
            .customization
            .request_rule_sets
            .add_revision(&revision, now)
            .await?;

        rule_set.current_revision_number = revision_number;
        rule_set.updated_at = now;
        self.rebuild_request_rules_engine().await?;
        Ok(RequestRuleSetDetail { rule_set, revision })
    }

    /// Renames and re-scopes without touching the matcher, so no revision is
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

    /// Move a rule set between disabled, shadow, and enforce.
    ///
    /// `enabled` is derived from the mode rather than accepted separately: an
    /// enabled-but-disabled-mode row would be a state the evaluator has no
    /// reading for.
    pub async fn set_request_rule_mode(
        &self,
        actor: &User,
        rule_set_id: &str,
        mode: RequestRuleEvaluationMode,
    ) -> AppResult<RequestRuleSetDetail> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;

        let mut rule_set = self.require_request_rule_set(rule_set_id).await?;
        let enabled = mode != RequestRuleEvaluationMode::Disabled;
        let now = Utc::now();

        self.services
            .customization
            .request_rule_sets
            .update_rule_set_evaluation_mode(&rule_set.id, mode, enabled, now)
            .await?;

        rule_set.evaluation_mode = mode;
        rule_set.enabled = enabled;
        rule_set.updated_at = now;
        self.rebuild_request_rules_engine().await?;
        self.load_request_rule_detail(rule_set).await
    }

    /// Delete a rule set.
    ///
    /// Unconditional, unlike a maintenance rule. A maintenance rule's deletion
    /// would cascade away the action runs that are the only record of what it
    /// did to someone's library; a request rule executes nothing, and its
    /// decisions live in `request_rule_decisions`, which deliberately has no
    /// foreign key to the rule (FR-016). The trace outlives the rule, so there
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

    /// Editor support: compile and check a draft without storing it.
    pub async fn validate_request_rule_source(
        &self,
        actor: &User,
        rego_source: &str,
    ) -> AppResult<ValidationResult> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;

        let scratch_id = Id::new_rego_safe().0;
        let rewritten = rewrite_package_declaration(rego_source, &scratch_id);
        validate_request_rule(&rewritten, &scratch_id)
            .map_err(|error| AppError::Validation(format!("rule validation failed: {error}")))
    }

    /// Run one matcher against one hypothetical request. Nothing is written.
    pub async fn preview_request_rule(
        &self,
        actor: &User,
        request: RequestRulePreviewRequest,
    ) -> AppResult<RequestRulePreviewResult> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;

        let policy = self.resolve_request_preview_policy(request.matcher).await?;
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
            .ok_or_else(|| AppError::NotFound("sample requester not found".into()))?;
        let requester = self.attach_user_authorization(requester).await?;
        let library = self
            .services
            .catalog
            .libraries
            .get_by_id(sample.library_id.trim())
            .await?
            .ok_or_else(|| AppError::NotFound("sample library not found".into()))?;

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
            monitor_type: sample.monitor_type.clone(),
            monitor_selection: None,
            requested_lease_days: sample.lease_days.flatten(),
        };

        let evaluated_at = Utc::now();
        let metadata_partial = snapshot.partial;
        let context = self
            .assemble_request_input_context(&requester, &library, &draft, snapshot, evaluated_at)
            .await?;
        let input = build_request_input(context);
        let input_json = serde_json::to_string_pretty(&input).unwrap_or_else(|_| "{}".to_string());

        let matcher_content_hash = content_hash(&policy.rego_source);
        let rule_set_id = policy.id.clone();
        let engine = RequestRulesEngine::build(std::slice::from_ref(&policy))
            .map_err(|error| AppError::Validation(format!("rule failed to compile: {error}")))?;
        let mut evaluator = engine.evaluator();
        let outcome = evaluator
            .evaluate(&input)
            .map_err(|error| AppError::Validation(format!("rule evaluation failed: {error}")))?;

        let (vote, held, reasons, tags, error) =
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
                    None,
                ),
                (None, Some(failure)) => (
                    None,
                    false,
                    Vec::new(),
                    Vec::new(),
                    Some(failure.message.clone()),
                ),
                (None, None) => (
                    None,
                    false,
                    Vec::new(),
                    Vec::new(),
                    Some("rule produced no decision".to_string()),
                ),
            };

        // The preview keeps returning every emitted label; `undefined_tags`
        // only says which of them cannot land yet.
        let undefined_tags = self.undefined_title_tag_labels(&tags).await?;

        Ok(RequestRulePreviewResult {
            rule_set_id,
            matcher_content_hash,
            evaluated_at,
            vote,
            held,
            reasons,
            tags,
            undefined_tags,
            error,
            input_json,
            metadata_partial,
        })
    }

    /// Gate authoring — or previewing — a matcher that reads the requester
    /// document (FR-014).
    ///
    /// Catalog-settings management is enough to write a rule about *media*.
    /// Writing one about *people* — who is asking, what they hold, which
    /// provider accounts they linked — is a use of the instance's identity
    /// records, so it asks for the authority that hands out permissions.
    ///
    /// The bar is on reaching the facts, not on running: a stored revision keeps
    /// evaluating under the system principal, because revoking an author's
    /// permission must not silently stop a rule an operator armed.
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
        let paths = request_person_targeted_paths(rego_source, rule_set_id)
            .map_err(|error| AppError::Validation(format!("rule validation failed: {error}")))?;
        if paths.is_empty() {
            return Ok(());
        }

        if self
            .has_app_permission(actor, AppPermission::ManagePermissions)
            .await?
        {
            return Ok(());
        }

        Err(AppError::Unauthorized(format!(
            "This rule reads facts about specific people ({}), so saving it needs permission to manage permissions. Ask an administrator to save it, or rewrite the rule without those facts.",
            paths.join(", ")
        )))
    }

    pub(crate) async fn require_request_rule_set(&self, id: &str) -> AppResult<RequestRuleSet> {
        self.services
            .customization
            .request_rule_sets
            .get_rule_set(id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("request rule set {id} not found")))
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
            .await?
            .ok_or_else(|| {
                AppError::Repository(format!(
                    "request rule set {} has no revision {}",
                    rule_set.id, rule_set.current_revision_number
                ))
            })?;
        Ok(RequestRuleSetDetail { rule_set, revision })
    }

    async fn resolve_request_preview_policy(
        &self,
        matcher: RequestRulePreviewMatcher,
    ) -> AppResult<RequestPolicy> {
        match matcher {
            RequestRulePreviewMatcher::Stored { rule_set_id } => {
                let rule_set = self.require_request_rule_set(&rule_set_id).await?;
                let detail = self.load_request_rule_detail(rule_set).await?;
                Ok(RequestPolicy {
                    id: detail.rule_set.id,
                    name: detail.rule_set.name,
                    rego_source: detail.revision.rego_source,
                })
            }
            RequestRulePreviewMatcher::Inline { rego_source } => {
                let scratch_id = Id::new_rego_safe().0;
                let prepared = prepare_matcher(&scratch_id, &rego_source)?;
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

/// A matcher that passed every authoring-time check, ready to persist.
pub(crate) struct PreparedMatcher {
    pub(crate) rego_source: String,
    pub(crate) content_hash: String,
}

/// Rewrite the package to the system-assigned id, then run every check that has
/// to hold before a revision is written.
fn prepare_matcher(rule_id: &str, rego_source: &str) -> AppResult<PreparedMatcher> {
    let rewritten = rewrite_package_declaration(rego_source, rule_id);
    let validation = validate_request_rule(&rewritten, rule_id)
        .map_err(|error| AppError::Validation(format!("rule validation failed: {error}")))?;
    if !validation.valid {
        return Err(AppError::Validation(format!(
            "Request rule validation failed:\n- {}",
            validation.errors.join("\n- ")
        )));
    }
    Ok(PreparedMatcher {
        content_hash: content_hash(&rewritten),
        rego_source: rewritten,
    })
}

fn build_revision(
    rule_set_id: &str,
    revision_number: i64,
    prepared: &PreparedMatcher,
    actor: &User,
    created_at: chrono::DateTime<Utc>,
) -> RequestRuleRevision {
    RequestRuleRevision {
        id: Id::new().0,
        rule_set_id: rule_set_id.to_string(),
        revision_number,
        rego_source: prepared.rego_source.clone(),
        matcher_content_hash: prepared.content_hash.clone(),
        created_by: Some(actor.id.clone()),
        created_at,
    }
}

fn require_non_empty(value: &str, field: &str) -> AppResult<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(AppError::Validation(format!("{field} must not be empty")));
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
/// The bounds are the policy family's own ([`MAX_TAGS`], [`MAX_TAG_LEN`],
/// [`RESERVED_TAG_PREFIX`], the same charset), because a tag an approver types
/// and a tag a rule emits end up in the same column on the same title, and a
/// rule may not mint one an approver could not. `decode_tags` itself cannot be
/// reused: it takes a Rego value.
pub fn validate_tag_list(tags: Vec<String>) -> AppResult<Vec<String>> {
    if tags.len() > MAX_TAGS {
        return Err(AppError::Validation(format!(
            "a request may carry at most {MAX_TAGS} tags, {} were supplied",
            tags.len()
        )));
    }
    let mut validated: Vec<String> = Vec::with_capacity(tags.len());
    for tag in tags {
        let tag = tag.trim().to_string();
        if tag.is_empty() {
            return Err(AppError::Validation("tag must not be empty".to_string()));
        }
        if tag.len() > MAX_TAG_LEN {
            return Err(AppError::Validation(format!(
                "tag is {} characters, at most {MAX_TAG_LEN} are allowed",
                tag.len()
            )));
        }
        if tag.starts_with(RESERVED_TAG_PREFIX) {
            return Err(AppError::Validation(format!(
                "tag '{tag}' uses the reserved '{RESERVED_TAG_PREFIX}' prefix"
            )));
        }
        if let Some(bad) = tag
            .chars()
            .find(|c| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ' ' | '-')))
        {
            return Err(AppError::Validation(format!(
                "tag '{tag}' contains unsupported character '{bad}'; use letters, digits, '.', \
                 '_', '-', or spaces"
            )));
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
        Some(days) if days < 1 => Err(AppError::Validation(
            "a lease must be at least one day, or forever".to_string(),
        )),
        Some(days) if days > REQUEST_MAX_LEASE_DAYS => Err(AppError::Validation(format!(
            "a lease must be at most {REQUEST_MAX_LEASE_DAYS} days (ten years), or forever"
        ))),
        Some(days) => Ok(Some(days)),
    }
}

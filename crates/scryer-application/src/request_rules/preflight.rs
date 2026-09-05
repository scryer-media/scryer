//! Pre-flight: tell a requester what will happen before they submit
//! (spec 0003 FR-020, FR-021).
//!
//! The dialog calls this as the requester changes library, profile, monitor, or
//! lease. It runs the *same* enrichment and the *same* evaluation the submit
//! will run — literally the same functions, through the same cache — so the
//! banner and the outcome cannot disagree.
//!
//! Two things it deliberately does not do:
//!
//! - **It never returns rule internals.** A requester sees an outcome, the
//!   reason codes with the rule *names* that produced them, and the tags that
//!   would land on the title. Not the vote table, not the input document, not a
//!   line of Rego. The authoring preview is where those live, behind
//!   catalog-settings authority.
//! - **It never errors for an evaluation failure.** A preview that 500s teaches
//!   a requester nothing; a preview that says "this will need approval" is both
//!   true (an unevaluable policy never approves) and useful.

use scryer_domain::{LibraryPermission, RequestDecisionOutcome, RequestRuleEvaluationMode, User};

use crate::media_requests::SubmitMediaRequestInput;
use crate::request_rules::evaluation::{RequestDecisionReason, RequestEvaluationPurpose};
use crate::request_rules::facts::RequestDraft;
use crate::{AppError, AppResult, AppUseCase};

/// What the request dialog renders.
#[derive(Clone, Debug)]
pub struct RequestPreflight {
    /// What would actually happen if this draft were submitted now.
    pub outcome: RequestDecisionOutcome,
    pub reasons: Vec<RequestDecisionReason>,
    /// Tags the policy would stamp on the created title.
    pub tags: Vec<String>,
    /// True when the metadata could not be fully established, so some facts
    /// were unknown. The dialog says "metadata unavailable — will need
    /// approval" rather than pretending to a verdict it does not have.
    pub metadata_partial: bool,
    /// Strictest mode among the rules that were consulted.
    pub evaluation_mode: RequestRuleEvaluationMode,
    /// What the policy concluded, which differs from `outcome` exactly when the
    /// gate is off or every deciding rule is in shadow. The dialog labels it
    /// "preview" beside the effective verdict.
    pub policy_outcome: RequestDecisionOutcome,
    /// Why the verdict fell back to needing approval, when it did: one of the
    /// arbitration codes (`rule_manual`, `held`, `error`, `no_rule_matched`),
    /// or `None` when a rule decided outright. It names the *shape* of the
    /// fallback and never a rule, so it is safe on the requester's surface.
    pub fallback_reason: Option<String>,
}

impl AppUseCase {
    /// Evaluate a draft without persisting it.
    ///
    /// Requires exactly what submit requires — [`LibraryPermission::Request`] on
    /// the target library — and refuses **before** any rule runs. A requester
    /// who may not ask for anything in a library may not use its rules as an
    /// oracle about that library's contents either.
    pub async fn preview_my_request_decision(
        &self,
        actor: &User,
        input: SubmitMediaRequestInput,
    ) -> AppResult<RequestPreflight> {
        let library = self
            .services
            .catalog
            .libraries
            .get_by_id(input.library_id.trim())
            .await?
            .ok_or_else(|| AppError::NotFound("library not found".into()))?;
        if library.facet != input.facet {
            return Err(AppError::Validation(
                "library facet does not match requested media facet".into(),
            ));
        }
        self.require_library_permission(actor, &library.id, LibraryPermission::Request)
            .await?;

        let external_ids =
            crate::media_requests::normalize_media_request_external_ids(input.external_ids)?;
        if external_ids.is_empty() {
            return Err(AppError::Validation(
                "media requests must include SMG external identifiers".into(),
            ));
        }
        let requested_lease_days =
            crate::request_rules::service::validate_lease_days(input.requested_lease_days)?;

        // The same read-through cache the submit uses, so the two share one SMG
        // call and therefore one set of facts (FR-021).
        let enrichment = self.enrich_request_draft(&input.facet, external_ids).await;
        let external_ids = enrichment.external_ids.clone();

        // The profile is resolved exactly as submit resolves it, including the
        // library's default when the draft names none — otherwise a preview of
        // "whatever the default is" would evaluate with no profile facts at all.
        let (quality_profile_id, quality_profile_name) = match self
            .request_quality_profile_snapshot_for_submission(
                &library,
                input.requested_quality_profile_id.clone(),
            )
            .await
        {
            Ok((id, name)) => (Some(id), Some(name)),
            // A profile the library does not allow is a validation error at
            // submit; here it degrades to "no profile facts", because the
            // dialog is mid-edit and a hard error would replace the banner.
            Err(_) => (None, None),
        };

        let draft = RequestDraft {
            facet: input.facet.clone(),
            title: input.title.trim().to_string(),
            year: input.year,
            identity_fingerprint: crate::media_requests::media_request_identity_fingerprint(
                &external_ids,
            ),
            external_ids,
            quality_profile_id,
            quality_profile_name,
            monitor_type: input.requested_monitor_type.clone(),
            monitor_selection: input.requested_monitor_selection.clone(),
            requested_lease_days,
        };

        let evaluation = self
            .evaluate_request_draft(
                actor,
                &library,
                &draft,
                &enrichment.snapshot,
                RequestEvaluationPurpose::Preflight,
            )
            .await?;

        Ok(RequestPreflight {
            outcome: evaluation.effective_outcome,
            reasons: evaluation.reasons,
            // Tags are shown whatever the outcome — a requester should see what
            // an approval would stamp — but they are only ever *applied* by the
            // approval path.
            tags: evaluation.tags,
            metadata_partial: evaluation.metadata_partial,
            evaluation_mode: evaluation.evaluation_mode,
            policy_outcome: evaluation.policy_outcome,
            fallback_reason: evaluation.fallback_reason,
        })
    }
}

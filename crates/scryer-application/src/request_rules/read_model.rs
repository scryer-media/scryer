//! Read models for the request-rule GraphQL surface (spec 0003 FR-016, FR-020,
//! FR-044).
//!
//! Nothing in this module decides anything. Every method is a read plus the
//! authorization that read requires, and it exists because the repositories the
//! reads go through are `pub(crate)` — the interface layer cannot reach a
//! decision trace or a lifecycle claim on its own.
//!
//! Two properties are enforced here rather than remembered at each call site:
//!
//! 1. **A requester never sees the vote table.** The same redaction the
//!    pre-flight applies (FR-020) applies to a stored decision read back by the
//!    person it was about: outcome, fallback reason, reasons, and tags, never
//!    the per-rule votes. [`RequestRuleDecisionView::redacted`] carries that
//!    judgement to the projection so the projection cannot forget to ask.
//! 2. **A read model never fails the read it decorates.** A request list that
//!    500s because a trace row could not be loaded is strictly worse than one
//!    that shows the requests without their policy detail, so every failure
//!    inside [`AppUseCase::media_request_policy_facts`] degrades to "no facts"
//!    with a warning.

use std::collections::{HashMap, HashSet};

use scryer_domain::{
    AppPermission, LibraryPermission, LifecycleClaim, LifecycleClaimProducer, MediaRequest,
    RequestDecisionOutcome, RequestRuleDecisionRecord, User,
};

use crate::{AppError, AppResult, AppUseCase};

/// Default and maximum page sizes for the decision browser.
const DEFAULT_DECISION_LIMIT: usize = 50;
const MAX_DECISION_LIMIT: usize = 500;

/// One request's decision trace, together with how much of it the caller may
/// see.
#[derive(Clone, Debug)]
pub struct RequestRuleDecisionView {
    pub record: RequestRuleDecisionRecord,
    /// True when the caller reached the trace as the *requester* rather than as
    /// a manager of the request's library. The projection then drops the votes,
    /// mirroring the pre-flight's redaction (FR-020).
    pub redacted: bool,
}

/// Everything a media-request projection needs beyond the stored row.
#[derive(Clone, Debug, Default)]
pub struct MediaRequestPolicyFacts {
    /// The latest decision recorded for the request, or `None` when it was
    /// never evaluated — which is every request submitted before the gate was
    /// armed.
    pub decision: Option<RequestRuleDecisionRecord>,
    /// See [`RequestRuleDecisionView::redacted`].
    pub decision_redacted: bool,
    /// The live retention claim the request produced, if it has one.
    pub lease_claim: Option<LifecycleClaim>,
}

impl AppUseCase {
    /// Recent decision traces, newest first. Catalog administration, because
    /// the vote table names rules and their verdicts.
    pub async fn list_request_rule_decisions(
        &self,
        actor: &User,
        limit: Option<usize>,
        outcome: Option<RequestDecisionOutcome>,
    ) -> AppResult<Vec<RequestRuleDecisionRecord>> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;
        let limit = limit
            .unwrap_or(DEFAULT_DECISION_LIMIT)
            .clamp(1, MAX_DECISION_LIMIT);
        self.services
            .customization
            .request_rule_decisions
            .list_recent(limit, outcome)
            .await
    }

    /// How many decisions each of `rule_set_ids` took part in.
    ///
    /// One count per rule set rather than a batched query: the authoring list is
    /// tens of rows, and the port's count is a single indexed scan per rule.
    pub async fn request_rule_decision_counts(
        &self,
        actor: &User,
        rule_set_ids: &[String],
    ) -> AppResult<HashMap<String, u64>> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;
        let mut counts = HashMap::with_capacity(rule_set_ids.len());
        for rule_set_id in rule_set_ids {
            let count = self
                .services
                .customization
                .request_rule_decisions
                .count_for_rule_set(rule_set_id)
                .await?;
            counts.insert(rule_set_id.clone(), count);
        }
        Ok(counts)
    }

    /// The latest decision recorded for one request.
    ///
    /// Two audiences reach this: a manager of the request's library, who sees
    /// the whole trace, and the requester themselves, who sees the verdict
    /// without the votes. Anyone else is refused.
    pub async fn request_rule_decision_for_request(
        &self,
        actor: &User,
        request_id: &str,
    ) -> AppResult<Option<RequestRuleDecisionView>> {
        let request_id = request_id.trim();
        if request_id.is_empty() {
            return Err(AppError::Validation("media request id is required".into()));
        }
        let request = self
            .services
            .catalog
            .media_requests
            .get(request_id)
            .await?
            .ok_or_else(|| AppError::NotFound("media request not found".into()))?;

        let manages = self
            .has_library_permission(actor, &request.library_id, LibraryPermission::ManageTitles)
            .await?;
        if !manages
            && !request
                .requesters
                .iter()
                .any(|requester| requester.user_id == actor.id)
        {
            return Err(AppError::Unauthorized(
                "You may not read this request's decision".to_string(),
            ));
        }

        let record = self
            .services
            .customization
            .request_rule_decisions
            .latest_for_request(&request.id)
            .await?;
        Ok(record.map(|record| RequestRuleDecisionView {
            record,
            redacted: !manages,
        }))
    }

    /// Decision traces and lease claims for a batch of already-authorized
    /// requests, keyed by request id.
    ///
    /// The caller has already established that it may see these rows; this only
    /// decides how much *policy* detail each one carries. Two bounded costs:
    /// one batched claim read for the requests that created a title, and one
    /// trace read per request that actually recorded a decision — so a list of
    /// requests submitted before request rules existed costs nothing extra.
    pub async fn media_request_policy_facts(
        &self,
        actor: &User,
        requests: &[MediaRequest],
    ) -> AppResult<HashMap<String, MediaRequestPolicyFacts>> {
        let mut facts: HashMap<String, MediaRequestPolicyFacts> =
            HashMap::with_capacity(requests.len());
        if requests.is_empty() {
            return Ok(facts);
        }

        let managed: HashSet<String> = self
            .authorized_library_ids(actor, None, LibraryPermission::ManageTitles)
            .await
            .unwrap_or_default()
            .into_iter()
            .collect();

        let title_ids: Vec<String> = requests
            .iter()
            .filter_map(|request| request.created_title_id.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        let claims_by_title = if title_ids.is_empty() {
            HashMap::new()
        } else {
            match self
                .services
                .catalog
                .lifecycle_claims
                .list_live_for_titles(&title_ids)
                .await
            {
                Ok(claims) => claims,
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        "could not read lifecycle claims for a media request projection"
                    );
                    HashMap::new()
                }
            }
        };

        for request in requests {
            let mut entry = MediaRequestPolicyFacts {
                decision_redacted: !managed.contains(&request.library_id),
                ..MediaRequestPolicyFacts::default()
            };

            if request.decision_id.is_some() {
                match self
                    .services
                    .customization
                    .request_rule_decisions
                    .latest_for_request(&request.id)
                    .await
                {
                    Ok(record) => entry.decision = record,
                    Err(error) => tracing::warn!(
                        request_id = %request.id,
                        error = %error,
                        "could not read the decision trace for a media request projection"
                    ),
                }
            }

            entry.lease_claim = request
                .created_title_id
                .as_deref()
                .and_then(|title_id| claims_by_title.get(title_id))
                .and_then(|claims| {
                    claims
                        .iter()
                        .find(|claim| {
                            matches!(
                                claim.producer,
                                LifecycleClaimProducer::RequestLease
                                    | LifecycleClaimProducer::RequestPermanent
                            ) && claim.producer_ref.as_deref() == Some(request.id.as_str())
                        })
                        .cloned()
                });

            facts.insert(request.id.clone(), entry);
        }

        Ok(facts)
    }
}

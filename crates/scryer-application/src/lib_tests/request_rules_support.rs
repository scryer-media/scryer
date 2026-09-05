//! In-memory doubles for the request-rules ports (spec 0003 section 6).
//!
//! Each one mirrors the SQL store's *contract*, not just its happy path: the
//! conditional updates are what keep a replayed import from restarting a lease
//! and a release from resurrecting a lapsed hold, so a permissive double here
//! would let the evaluation wave's tests pass against behaviour production does
//! not have.

use super::*;

use crate::lib_tests::support_events_requests::MockMediaRequestRepo;
use scryer_domain::{
    LifecycleClaim, LifecycleClaimKind, LifecycleClaimProducer, LifecycleClaimState,
    RequestDecisionOutcome, RequestRuleDecisionRecord, RequestRuleEvaluationMode,
    RequestRuleRevision, RequestRuleSet,
};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Default)]
pub(super) struct InMemoryRequestRuleRepo {
    rule_sets: Mutex<Vec<RequestRuleSet>>,
    revisions: Mutex<Vec<RequestRuleRevision>>,
}

#[async_trait]
impl crate::ports::RequestRuleSetRepository for InMemoryRequestRuleRepo {
    async fn list_rule_sets(&self) -> AppResult<Vec<RequestRuleSet>> {
        let mut rule_sets = self.rule_sets.lock().await.clone();
        rule_sets.sort_by(|left, right| left.name.cmp(&right.name).then(left.id.cmp(&right.id)));
        Ok(rule_sets)
    }

    async fn get_rule_set(&self, id: &str) -> AppResult<Option<RequestRuleSet>> {
        Ok(self
            .rule_sets
            .lock()
            .await
            .iter()
            .find(|rule_set| rule_set.id == id)
            .cloned())
    }

    async fn create_rule_set(
        &self,
        rule_set: &RequestRuleSet,
        revision: &RequestRuleRevision,
    ) -> AppResult<()> {
        self.rule_sets.lock().await.push(rule_set.clone());
        self.revisions.lock().await.push(revision.clone());
        Ok(())
    }

    async fn add_revision(
        &self,
        revision: &RequestRuleRevision,
        updated_at: DateTime<Utc>,
    ) -> AppResult<()> {
        let mut rule_sets = self.rule_sets.lock().await;
        let rule_set = rule_sets
            .iter_mut()
            .find(|rule_set| rule_set.id == revision.rule_set_id)
            .ok_or_else(|| AppError::NotFound(revision.rule_set_id.clone()))?;
        let mut revisions = self.revisions.lock().await;
        // The SQL store has a UNIQUE (rule_set_id, revision_number); a double
        // that silently duplicated history would hide a replayed revision.
        if revisions.iter().any(|stored| {
            stored.rule_set_id == revision.rule_set_id
                && stored.revision_number == revision.revision_number
        }) {
            return Err(AppError::Repository(format!(
                "revision {} already exists for request rule {}",
                revision.revision_number, revision.rule_set_id
            )));
        }
        rule_set.current_revision_number = revision.revision_number;
        rule_set.updated_at = updated_at;
        revisions.push(revision.clone());
        Ok(())
    }

    async fn get_revision(
        &self,
        rule_set_id: &str,
        revision_number: i64,
    ) -> AppResult<Option<RequestRuleRevision>> {
        Ok(self
            .revisions
            .lock()
            .await
            .iter()
            .find(|revision| {
                revision.rule_set_id == rule_set_id && revision.revision_number == revision_number
            })
            .cloned())
    }

    async fn list_revisions(&self, rule_set_id: &str) -> AppResult<Vec<RequestRuleRevision>> {
        let mut revisions: Vec<RequestRuleRevision> = self
            .revisions
            .lock()
            .await
            .iter()
            .filter(|revision| revision.rule_set_id == rule_set_id)
            .cloned()
            .collect();
        revisions.sort_by_key(|revision| std::cmp::Reverse(revision.revision_number));
        Ok(revisions)
    }

    async fn update_rule_set_metadata(
        &self,
        id: &str,
        name: &str,
        description: &str,
        library_ids: &[String],
        updated_at: DateTime<Utc>,
    ) -> AppResult<()> {
        let mut rule_sets = self.rule_sets.lock().await;
        let rule_set = rule_sets
            .iter_mut()
            .find(|rule_set| rule_set.id == id)
            .ok_or_else(|| AppError::NotFound(id.to_string()))?;
        rule_set.name = name.to_string();
        rule_set.description = description.to_string();
        rule_set.library_ids = library_ids.to_vec();
        rule_set.updated_at = updated_at;
        Ok(())
    }

    async fn update_rule_set_evaluation_mode(
        &self,
        id: &str,
        mode: RequestRuleEvaluationMode,
        enabled: bool,
        updated_at: DateTime<Utc>,
    ) -> AppResult<()> {
        let mut rule_sets = self.rule_sets.lock().await;
        let rule_set = rule_sets
            .iter_mut()
            .find(|rule_set| rule_set.id == id)
            .ok_or_else(|| AppError::NotFound(id.to_string()))?;
        rule_set.evaluation_mode = mode;
        rule_set.enabled = enabled;
        rule_set.updated_at = updated_at;
        Ok(())
    }

    /// Mirrors the FK cascade: the revisions go, the decision traces stay.
    async fn delete_rule_set(&self, id: &str) -> AppResult<()> {
        self.rule_sets
            .lock()
            .await
            .retain(|rule_set| rule_set.id != id);
        self.revisions
            .lock()
            .await
            .retain(|revision| revision.rule_set_id != id);
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct InMemoryRequestRuleDecisionRepo {
    decisions: Mutex<Vec<RequestRuleDecisionRecord>>,
}

impl InMemoryRequestRuleDecisionRepo {
    /// Every trace recorded so far, oldest first — the order the evaluations
    /// happened in. Read by the evaluation wave's tests.
    #[allow(dead_code)]
    pub(super) async fn recorded(&self) -> Vec<RequestRuleDecisionRecord> {
        self.decisions.lock().await.clone()
    }
}

#[async_trait]
impl crate::ports::RequestRuleDecisionRepository for InMemoryRequestRuleDecisionRepo {
    async fn record(&self, decision: &RequestRuleDecisionRecord) -> AppResult<()> {
        self.decisions.lock().await.push(decision.clone());
        Ok(())
    }

    async fn latest_for_request(
        &self,
        request_id: &str,
    ) -> AppResult<Option<RequestRuleDecisionRecord>> {
        let decisions = self.decisions.lock().await;
        Ok(decisions
            .iter()
            .filter(|decision| decision.request_id == request_id)
            .max_by(|left, right| {
                left.evaluated_at
                    .cmp(&right.evaluated_at)
                    .then_with(|| left.id.cmp(&right.id))
            })
            .cloned())
    }

    async fn list_recent(
        &self,
        limit: usize,
        outcome: Option<RequestDecisionOutcome>,
    ) -> AppResult<Vec<RequestRuleDecisionRecord>> {
        let mut decisions: Vec<RequestRuleDecisionRecord> = self
            .decisions
            .lock()
            .await
            .iter()
            .filter(|decision| outcome.is_none_or(|outcome| decision.effective_outcome == outcome))
            .cloned()
            .collect();
        decisions.sort_by(|left, right| {
            right
                .evaluated_at
                .cmp(&left.evaluated_at)
                .then_with(|| right.id.cmp(&left.id))
        });
        decisions.truncate(limit);
        Ok(decisions)
    }

    /// Same substring match the SQL store documents: the rule ids live inside
    /// the serialized votes.
    async fn count_for_rule_set(&self, rule_set_id: &str) -> AppResult<u64> {
        if rule_set_id.trim().is_empty() {
            return Ok(0);
        }
        Ok(self
            .decisions
            .lock()
            .await
            .iter()
            .filter(|decision| decision.votes_json.contains(rule_set_id))
            .count() as u64)
    }
}

#[derive(Default)]
pub(super) struct InMemoryLifecycleClaimRepo {
    claims: Mutex<Vec<LifecycleClaim>>,
    /// Resolves `producer_ref` to the user who submitted that request, standing
    /// in for the SQL store's join to `media_requests`.
    media_requests: Option<Arc<MockMediaRequestRepo>>,
    /// When set, every method fails the way an unreachable datastore does.
    ///
    /// The unreadable-store path is the one the lease facts are *designed*
    /// around — it is what turns four facts unknown and holds the rule reading
    /// them — so a double with no way to reach it would leave that behaviour
    /// untested.
    unreadable: AtomicBool,
}

impl InMemoryLifecycleClaimRepo {
    pub(super) fn with_media_requests(media_requests: Arc<MockMediaRequestRepo>) -> Self {
        Self {
            claims: Mutex::new(Vec::new()),
            media_requests: Some(media_requests),
            unreadable: AtomicBool::new(false),
        }
    }

    /// Read by the lease wave's tests.
    #[allow(dead_code)]
    pub(super) async fn all(&self) -> Vec<LifecycleClaim> {
        self.claims.lock().await.clone()
    }

    /// Seed a claim without going through `create`'s uniqueness check, for the
    /// terminal states a live-claim writer can never produce directly.
    #[allow(dead_code)]
    pub(super) async fn seed(&self, claim: LifecycleClaim) {
        self.claims.lock().await.push(claim);
    }

    /// Make every read and write fail, as an unreachable datastore does.
    #[allow(dead_code)]
    pub(super) fn set_unreadable(&self, unreadable: bool) {
        self.unreadable.store(unreadable, Ordering::SeqCst);
    }

    fn fail_if_armed(&self) -> AppResult<()> {
        if self.unreadable.load(Ordering::SeqCst) {
            return Err(AppError::Repository(
                "lifecycle claim store is unreachable".to_string(),
            ));
        }
        Ok(())
    }
}

#[async_trait]
impl crate::ports::LifecycleClaimRepository for InMemoryLifecycleClaimRepo {
    /// Enforces the partial unique index: one live claim per
    /// (producer, producer_ref).
    async fn create(&self, claim: &LifecycleClaim) -> AppResult<()> {
        let mut claims = self.claims.lock().await;
        if let Some(producer_ref) = claim.producer_ref.as_ref()
            && claim.state.is_live()
            && claims.iter().any(|stored| {
                stored.producer == claim.producer
                    && stored.producer_ref.as_ref() == Some(producer_ref)
                    && stored.state.is_live()
            })
        {
            return Err(AppError::Repository(format!(
                "a live {} claim already exists for {producer_ref}",
                claim.producer.as_storage_str()
            )));
        }
        claims.push(claim.clone());
        Ok(())
    }

    async fn get(&self, id: &str) -> AppResult<Option<LifecycleClaim>> {
        Ok(self
            .claims
            .lock()
            .await
            .iter()
            .find(|claim| claim.id == id)
            .cloned())
    }

    async fn list_for_title(&self, title_id: &str) -> AppResult<Vec<LifecycleClaim>> {
        self.fail_if_armed()?;
        let mut claims: Vec<LifecycleClaim> = self
            .claims
            .lock()
            .await
            .iter()
            .filter(|claim| claim.title_id == title_id)
            .cloned()
            .collect();
        claims.sort_by(|left, right| {
            right
                .created_at
                .cmp(&left.created_at)
                .then_with(|| right.id.cmp(&left.id))
        });
        Ok(claims)
    }

    async fn list_live_for_titles(
        &self,
        title_ids: &[String],
    ) -> AppResult<HashMap<String, Vec<LifecycleClaim>>> {
        self.fail_if_armed()?;
        let wanted: HashSet<&String> = title_ids.iter().collect();
        let mut by_title: HashMap<String, Vec<LifecycleClaim>> = HashMap::new();
        let mut claims: Vec<LifecycleClaim> = self
            .claims
            .lock()
            .await
            .iter()
            .filter(|claim| claim.state.is_live() && wanted.contains(&claim.title_id))
            .cloned()
            .collect();
        claims.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.id.cmp(&right.id))
        });
        for claim in claims {
            by_title
                .entry(claim.title_id.clone())
                .or_default()
                .push(claim);
        }
        Ok(by_title)
    }

    /// Live *and* expired retention claims, mirroring the store's filter: a
    /// released claim is withdrawn rather than spent, so it must not make the
    /// fact builder read `expired`.
    async fn list_retention_history_for_titles(
        &self,
        title_ids: &[String],
    ) -> AppResult<HashMap<String, Vec<LifecycleClaim>>> {
        self.fail_if_armed()?;
        let wanted: HashSet<&String> = title_ids.iter().collect();
        let mut by_title: HashMap<String, Vec<LifecycleClaim>> = HashMap::new();
        let mut claims: Vec<LifecycleClaim> = self
            .claims
            .lock()
            .await
            .iter()
            .filter(|claim| {
                claim.kind == LifecycleClaimKind::RetainUntil
                    && (claim.state.is_live() || claim.state == LifecycleClaimState::Expired)
                    && wanted.contains(&claim.title_id)
            })
            .cloned()
            .collect();
        claims.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.id.cmp(&right.id))
        });
        for claim in claims {
            by_title
                .entry(claim.title_id.clone())
                .or_default()
                .push(claim);
        }
        Ok(by_title)
    }

    async fn list_dormant(&self, limit: usize) -> AppResult<Vec<LifecycleClaim>> {
        self.fail_if_armed()?;
        let mut claims: Vec<LifecycleClaim> = self
            .claims
            .lock()
            .await
            .iter()
            .filter(|claim| claim.state == LifecycleClaimState::Dormant)
            .cloned()
            .collect();
        claims.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.id.cmp(&right.id))
        });
        claims.truncate(limit);
        Ok(claims)
    }

    async fn activate(
        &self,
        id: &str,
        starts_at: DateTime<Utc>,
        expires_at: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        self.fail_if_armed()?;
        let mut claims = self.claims.lock().await;
        // Conditional on dormancy, like the SQL UPDATE: a second import must
        // not restart a window the requester already spent.
        if let Some(claim) = claims
            .iter_mut()
            .find(|claim| claim.id == id && claim.state == LifecycleClaimState::Dormant)
        {
            claim.state = LifecycleClaimState::Active;
            claim.starts_at = Some(starts_at);
            claim.expires_at = expires_at;
            // The write time, not the lease's start: a reconcile pass backdates
            // `starts_at` to the import it missed.
            claim.updated_at = now;
        }
        Ok(())
    }

    async fn expire_due(&self, now: DateTime<Utc>) -> AppResult<u64> {
        self.fail_if_armed()?;
        let mut claims = self.claims.lock().await;
        let mut expired = 0;
        for claim in claims.iter_mut().filter(|claim| {
            claim.state == LifecycleClaimState::Active
                && claim.expires_at.is_some_and(|expires_at| expires_at <= now)
        }) {
            claim.state = LifecycleClaimState::Expired;
            claim.updated_at = now;
            expired += 1;
        }
        Ok(expired)
    }

    async fn release_for_producer_ref(
        &self,
        producer: LifecycleClaimProducer,
        producer_ref: &str,
        reason: &str,
        now: DateTime<Utc>,
    ) -> AppResult<u64> {
        let mut claims = self.claims.lock().await;
        let mut released = 0;
        for claim in claims.iter_mut().filter(|claim| {
            claim.producer == producer
                && claim.producer_ref.as_deref() == Some(producer_ref)
                && claim.state.is_live()
        }) {
            claim.state = LifecycleClaimState::Released;
            claim.released_reason = Some(reason.to_string());
            claim.updated_at = now;
            released += 1;
        }
        Ok(released)
    }

    async fn release_claim(&self, id: &str, reason: &str, now: DateTime<Utc>) -> AppResult<u64> {
        self.fail_if_armed()?;
        let mut claims = self.claims.lock().await;
        let Some(claim) = claims
            .iter_mut()
            .find(|claim| claim.id == id && claim.state.is_live())
        else {
            return Ok(0);
        };
        claim.state = LifecycleClaimState::Released;
        claim.released_reason = Some(reason.to_string());
        claim.updated_at = now;
        Ok(1)
    }

    async fn release_for_title(
        &self,
        title_id: &str,
        reason: &str,
        now: DateTime<Utc>,
    ) -> AppResult<u64> {
        self.fail_if_armed()?;
        let mut claims = self.claims.lock().await;
        let mut released = 0;
        for claim in claims
            .iter_mut()
            .filter(|claim| claim.title_id == title_id && claim.state.is_live())
        {
            claim.state = LifecycleClaimState::Released;
            claim.released_reason = Some(reason.to_string());
            claim.updated_at = now;
            released += 1;
        }
        Ok(released)
    }

    async fn extend(
        &self,
        id: &str,
        expires_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        let mut claims = self.claims.lock().await;
        let Some(claim) = claims
            .iter_mut()
            .find(|claim| claim.id == id && claim.state.is_live())
        else {
            return Err(AppError::Validation(
                "lifecycle claim is no longer live".to_string(),
            ));
        };
        claim.expires_at = Some(expires_at);
        claim.updated_at = now;
        Ok(())
    }

    async fn convert_to_permanent(
        &self,
        id: &str,
        replacement: &LifecycleClaim,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        let mut claims = self.claims.lock().await;
        let Some(claim) = claims
            .iter_mut()
            .find(|claim| claim.id == id && claim.state.is_live())
        else {
            return Err(AppError::Validation(
                "lifecycle claim is no longer live".to_string(),
            ));
        };
        claim.state = LifecycleClaimState::Converted;
        claim.released_reason = Some("converted_to_permanent".to_string());
        claim.updated_at = now;
        claims.push(replacement.clone());
        Ok(())
    }

    async fn count_live_for_user(&self, user_id: &str) -> AppResult<u64> {
        // Guarded like every other read: `set_unreadable` promises an
        // unreachable datastore, and a count that silently answered zero would
        // let the `active_lease_count` fact read as *known* during an outage —
        // which is exactly the failure the fact's unknown state exists to
        // prevent.
        self.fail_if_armed()?;
        let Some(media_requests) = self.media_requests.as_ref() else {
            return Ok(0);
        };
        let claims = self.claims.lock().await.clone();
        let mut live = 0;
        for claim in claims.iter().filter(|claim| claim.state.is_live()) {
            let Some(producer_ref) = claim.producer_ref.as_deref() else {
                continue;
            };
            if crate::ports::MediaRequestRepository::get(media_requests.as_ref(), producer_ref)
                .await?
                .is_some_and(|request| request.created_by_user_id == user_id)
            {
                live += 1;
            }
        }
        Ok(live)
    }
}

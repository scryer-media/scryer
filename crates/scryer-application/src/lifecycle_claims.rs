//! Lifecycle-claim bookkeeping: activation, expiry, and the safety net that
//! catches the imports the hook missed (spec 0003 FR-041, FR-044).
//!
//! A claim written at approval is *dormant*: it names a title that may not even
//! have a file yet, and the requester's window is a promise about how long they
//! keep the media once it arrives, not about how long the grab takes. Starting
//! that clock is what this module does, at the moment the title's first import
//! completes — and, for the imports that happened while the hook was missing or
//! failing, at the start of every lifecycle evaluation pass.
//!
//! Nothing here fails the caller. An import that cannot start a lease is still
//! a completed import, and an unreadable claim store makes lifecycle facts
//! unknown (which holds rules) rather than stopping the pass.

use chrono::{DateTime, Duration, Utc};
use scryer_domain::{LifecycleClaim, LifecycleClaimState};
use tracing::warn;

use crate::lifecycle_rules::facts::first_imported_instant;
use crate::{AppResult, AppUseCase};

/// How many dormant claims one reconcile pass will look at.
///
/// The sweep is a safety net, not the primary path — the import hook is — so it
/// is bounded to keep a backlog from making the lifecycle job's runtime
/// unbounded. Anything past the bound is picked up by the next pass, and the
/// claims are read oldest-first, so the backlog drains in the order it formed.
pub const LIFECYCLE_CLAIM_DORMANT_SWEEP_LIMIT: usize = 500;

/// Maximum orphan claims released per pass; subsequent passes drain the backlog.
pub const LIFECYCLE_CLAIM_ORPHAN_SWEEP_LIMIT: usize = 500;

/// Reason recorded on claims released because their title was deleted.
pub const CLAIM_RELEASE_TITLE_DELETED: &str = "title_deleted";

impl AppUseCase {
    /// Start every dormant claim on a title, at `now`.
    ///
    /// Called from the import path when a title's import completes. A claim
    /// with a positive `duration_days` closes at the later of its stored
    /// expiry and `duration_days` from this instant. A claim without one keeps
    /// its stored expiry as it is, including one already past, which the next
    /// pass then expires. A claim with neither a duration nor a stored expiry
    /// never closes.
    ///
    /// The repository's `activate` is conditional on the claim still being
    /// dormant, so a replayed import cannot restart a window the requester has
    /// already spent.
    pub async fn activate_dormant_claims_for_title(
        &self,
        title_id: &str,
        now: DateTime<Utc>,
    ) -> AppResult<u64> {
        let claims = self
            .services
            .catalog
            .lifecycle_claims
            .list_for_title(title_id)
            .await?;
        let mut activated = 0;
        for claim in claims
            .iter()
            .filter(|claim| claim.state == LifecycleClaimState::Dormant)
        {
            self.services
                .catalog
                .lifecycle_claims
                .activate(&claim.id, now, claim_expiry(claim, now), now)
                .await?;
            activated += 1;
        }
        Ok(activated)
    }

    /// The import hook, wired to the domain event rather than to the five
    /// places that append it: an import that completes is an import that
    /// completes, whether it came from a download, a manual pick, or a
    /// series-movie link.
    pub(crate) async fn maybe_activate_lifecycle_claims_for_import(
        &self,
        event: &scryer_domain::DomainEvent,
    ) {
        if !matches!(
            event.payload,
            scryer_domain::DomainEventPayload::ImportCompleted(_)
        ) {
            return;
        }
        // The subject id lives on the event envelope, not in the payload: the
        // payload's snapshot is display metadata (name, facet, poster).
        let Some(title_id) = event
            .title_id
            .as_deref()
            .map(str::trim)
            .filter(|title_id| !title_id.is_empty())
        else {
            return;
        };
        match self
            .activate_dormant_claims_for_title(title_id, Utc::now())
            .await
        {
            Ok(0) => {}
            Ok(activated) => {
                tracing::debug!(
                    title_id,
                    activated,
                    "started request lease clocks at first import"
                );
            }
            // Never fails the import: the title is on disk either way, and the
            // reconcile sweep in the lifecycle pass picks the claim up.
            Err(error) => warn!(
                title_id,
                error = %error,
                "could not start request lease clocks at import; the lifecycle pass will retry"
            ),
        }
    }

    /// Expire what has run out and start what has already arrived.
    ///
    /// Returns `(expired, activated)`. Both halves are best-effort and logged:
    /// this runs at the head of the lifecycle evaluation job, and a claim
    /// store that is down must not stop rules from evaluating (their claim
    /// facts go unknown on their own, which holds any rule that reads one).
    pub(crate) async fn reconcile_lifecycle_claims(&self, now: DateTime<Utc>) -> (u64, u64) {
        let claims = &self.services.catalog.lifecycle_claims;
        if let Err(error) = claims
            .release_orphaned(
                LIFECYCLE_CLAIM_ORPHAN_SWEEP_LIMIT,
                CLAIM_RELEASE_TITLE_DELETED,
                now,
            )
            .await
        {
            warn!(error = %error, "could not release orphaned lifecycle claims; the next lifecycle pass will retry");
        }
        let expired = match claims.expire_due(now).await {
            Ok(expired) => expired,
            Err(error) => {
                warn!(error = %error, "could not expire due lifecycle claims");
                0
            }
        };

        let dormant = match claims
            .list_dormant(LIFECYCLE_CLAIM_DORMANT_SWEEP_LIMIT)
            .await
        {
            Ok(dormant) => dormant,
            Err(error) => {
                warn!(error = %error, "could not read dormant lifecycle claims");
                return (expired, 0);
            }
        };
        let mut activated = 0;
        for claim in dormant {
            // The lease starts at the import, not at this sweep: the sweep is
            // catching up on a clock that should already have been running, so
            // backdating is the whole point. A title with no file yet is not
            // late — it is waiting, exactly as intended.
            let Some(first_imported_at) = self.title_first_imported_at(&claim.title_id).await
            else {
                continue;
            };
            match claims
                .activate(
                    &claim.id,
                    first_imported_at,
                    claim_expiry(&claim, first_imported_at),
                    now,
                )
                .await
            {
                Ok(()) => activated += 1,
                Err(error) => warn!(
                    claim_id = claim.id.as_str(),
                    title_id = claim.title_id.as_str(),
                    error = %error,
                    "could not activate a dormant lifecycle claim"
                ),
            }
        }
        (expired, activated)
    }

    /// Release every live claim on a deleted title. Best effort: the title row
    /// is already gone; the lifecycle sweep retries any claims left behind.
    pub(crate) async fn release_lifecycle_claims_for_deleted_title(&self, title_id: &str) {
        match self
            .services
            .catalog
            .lifecycle_claims
            .release_for_title(title_id, CLAIM_RELEASE_TITLE_DELETED, Utc::now())
            .await
        {
            Ok(0) => {}
            Ok(released) => tracing::debug!(
                title_id,
                released,
                "released lifecycle claims for a deleted title"
            ),
            Err(error) => warn!(
                title_id,
                error = %error,
                "could not release lifecycle claims for a deleted title"
            ),
        }
    }

    /// The instant of a title's first import, or `None` when it has no files
    /// (or none whose row carries a readable timestamp).
    async fn title_first_imported_at(&self, title_id: &str) -> Option<DateTime<Utc>> {
        let files = self
            .services
            .library
            .media_files
            .list_media_files_for_titles(std::slice::from_ref(&title_id.to_string()))
            .await
            .ok()?;
        first_imported_instant(&files)
    }
}

/// When a claim's window closes, given the instant its clock starts. An
/// operator's explicit expiry, even one granted before the first import
/// starts the clock, is kept: with a positive duration the later of it and
/// the duration's end wins, and without one it stands as stored. A claim
/// with neither never closes.
fn claim_expiry(claim: &LifecycleClaim, starts_at: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let Some(days) = claim.duration_days.filter(|days| *days > 0) else {
        return claim.expires_at;
    };
    let duration_expiry = starts_at + Duration::days(days);
    Some(
        claim
            .expires_at
            .map_or(duration_expiry, |expiry| expiry.max(duration_expiry)),
    )
}

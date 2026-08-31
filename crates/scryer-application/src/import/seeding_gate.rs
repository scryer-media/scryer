//! Seeding-aware removal gate.
//!
//! Removing a torrent's entry from its download client stops it seeding, even
//! with `remove_data: false`. On a private tracker that is an instant hit and
//! run. This module owns the one question the removal and import paths ask
//! before they act: **has this torrent discharged its seeding obligation?**
//!
//! Three operator decisions are baked in and are not configurable:
//!
//! 1. **Universal gate.** Every torrent goes through it, profile or not. An
//!    install with no seeding profiles does not get today's remove-on-import
//!    behaviour back; it gets "remove only when the client itself says the
//!    obligation is discharged".
//! 2. **Hard private rail.** A torrent observed as `is_private = Some(true)`
//!    is never auto-removed on the client's word alone. Only a resolved
//!    profile goal that is provably met (or an operator action) releases it.
//!    `is_private = None` is *unknown*, never "public".
//! 3. **Tri-state `can_remove`.** `None` means "this client cannot answer";
//!    it is never read as `false` and never as `true`. See
//!    `crates/scryer-plugin-sdk/src/lib.rs` for the contract and the P3
//!    plugin audit for what each client can actually observe.
//!
//! The one deliberate opt-out is the profile's `post_import_tracking`. A
//! profile set to `HandOff` settles the download after import without touching
//! the client entry and stops tracking it — Sonarr's post-import category, made
//! explicit. It bypasses the rails above because none of them are about
//! tracking: they exist to prevent a *removal*, and a handoff never removes.
//!
//! ## Why the plugin's `can_remove: Some(true)` is not always enough
//!
//! On several clients `Some(true)` only means "the client stopped the torrent"
//! — uTorrent cannot see its own seed limits from the list API, and
//! Transmission's global idle mode makes "stopped" indistinguishable from
//! "user paused". That is fine as a *baseline* (it is exactly what Sonarr
//! trusts when it has no goal of its own), but it must not override an
//! explicit Scryer goal. So when a resolved profile carries numeric goals,
//! the Scryer-side check is authoritative in both directions:
//! `Some(true)` does not release an unmet goal, and `Some(false)` does not
//! veto a met one (the plugin is asserting an unmet *client* limit, which is
//! not the policy Scryer was told to enforce).
//!
//! ## Observation plumbing
//!
//! `TorrentSeedingObservation` mirrors the fields the plugin SDK carries on
//! `PluginDownloadItem`/`PluginTorrentItem`. The download-client adapter
//! copies them onto `DownloadQueueItem::seeding`, and the tracked-download
//! service keeps the latest one on `TrackedDownload::client_item`. The gate
//! reads it from whichever is freshest:
//!
//! - callers that hold the tracked row (the reconcile tick) pass their own
//!   copy in, so the decision uses the observation from *this* poll rather
//!   than the published snapshot, which is only refreshed after reconcile;
//! - callers that only have a client identity (the manual-import mode check)
//!   fall back to `observe_torrent_seeding`, which reads the published
//!   tracked-download snapshot.
//!
//! When neither answers, the observation is absent and the gate holds — the
//! conservative direction, and the same fail-closed behaviour the universal
//! gate had before the plumbing existed.
//!
//! The other half — the goals a grab resolved to — reads the
//! `PersistedSeedGoals` the grab-time package freezes onto the
//! download-submission row.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use scryer_domain::{
    CompletedDownload, DownloadQueueItem, DownloadSeedingSnapshot, ImportMode, MediaFacet,
    SeedGoalMetAction, download_identity::DownloadId,
};

use crate::{AppResult, AppUseCase, ClientJobLocator, DownloadSourceKind, PersistedSeedGoals};

/// The reasons the gate reports, verbatim, in logs and outcomes.
///
/// Named because the queue projection derives its display state from them: the
/// row the operator sees and the decision the reconciler took must never be
/// able to disagree.
pub(crate) mod reason {
    pub const NON_TORRENT_PROTOCOL: &str = "non_torrent_protocol";
    pub const TORRENT_NO_LONGER_IN_CLIENT: &str = "torrent_no_longer_in_client";
    pub const BLACKHOLE_REMOVAL_IS_DESTRUCTIVE: &str = "torrent_blackhole_removal_is_destructive";
    pub const PROFILE_NEVER_REMOVE: &str = "profile_never_remove";
    pub const PRIVATE_WITHOUT_GOALS: &str = "private_torrent_without_resolved_goals";
    pub const PROFILE_GOAL_MET: &str = "profile_goal_met";
    pub const PROFILE_GOAL_UNMET: &str = "profile_goal_unmet";
    pub const CLIENT_OBLIGATION_MET: &str = "client_reports_seeding_obligation_met";
    pub const CLIENT_LIMIT_UNMET: &str = "client_reports_unmet_seed_limit";
    pub const CLIENT_VERDICT_UNKNOWN: &str = "no_resolved_goals_and_client_verdict_unknown";
    pub const POST_IMPORT_HANDOFF: &str = "post_import_handoff";
}

/// Client type whose "remove" is a filesystem delete, not a session command.
///
/// `torrent-blackhole` has no client session at all: its items are watch-folder
/// entries served by some *external* client, and its `Remove` control does
/// `remove_dir_all` on the path (it refuses `remove_data: false`). Removing is
/// therefore destructive by construction and can never be automatic.
pub(crate) const TORRENT_BLACKHOLE_CLIENT_TYPE: &str = "torrent-blackhole";

/// What to do with the client entry once the obligation is discharged.
///
/// The policy comes from `PersistedSeedGoals`, frozen onto the
/// download-submission row at grab time, so a torrent keeps the goals it was
/// grabbed under even if the profile is later edited or deleted. A grab with
/// no profile falls back to `SeedGoalMetAction::default()` — remove the entry,
/// which is what an install without seeding profiles has always done.
fn goal_met_action_for(goals: &PersistedSeedGoals) -> SeedGoalMetAction {
    goals.goal_met_action.unwrap_or_default()
}

/// Identifies the download whose goals are being looked up.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SeedGoalLookupKey {
    /// Present for tracked downloads whose registry resolution is already in
    /// hand. The queue-projection batch remains identity-keyed.
    pub canonical_download_id: Option<DownloadId>,
    pub client_id: String,
    pub client_type: String,
    pub client_item_id: String,
    /// Normalized info hash when known; torrent identity survives a client
    /// re-add, the client's own item id does not.
    pub info_hash: Option<String>,
}

/// Seed goals prefetched for one reconcile tick in a single query.
///
/// The reconcile tick re-offers *every* settled row to the gate on every poll,
/// and held torrents are the common case once the gate is doing its job — so
/// the shape this replaces is one point query per held torrent per tick.
///
/// The batch only speaks for the **client-identity** lookup. An identity the
/// batch covered but did not answer is a definitive "no identity-keyed
/// resolution": the caller skips its own identity query and goes straight to
/// the info-hash fallback, which is the only lookup a batch keyed on client
/// identity cannot express. An identity the batch does not cover at all (a
/// caller outside the tick, or a failed prefetch) falls back to the full
/// per-row path.
#[derive(Debug, Default)]
pub(crate) struct SeedGoalBatch {
    covered: HashSet<ClientJobLocator>,
    resolved: HashMap<ClientJobLocator, PersistedSeedGoals>,
}

/// What a batch can say about one identity.
enum SeedGoalBatchAnswer {
    /// Not in this batch; do the full per-row lookup.
    Uncovered,
    /// The batch carries this download's goals.
    Resolved(PersistedSeedGoals),
    /// Covered, and there is no identity-keyed resolution. Skip the identity
    /// query; the info-hash fallback still applies.
    NoIdentityResolution,
}

impl SeedGoalBatch {
    /// Read the goals for every supplied identity in one batched query.
    ///
    /// A failed read yields an empty (uncovering) batch rather than an empty
    /// answer: "the prefetch failed" must degrade to per-row reads, never to
    /// "these torrents have no obligation".
    pub(crate) async fn prefetch(app: &AppUseCase, identities: &[ClientJobLocator]) -> Self {
        if identities.is_empty() {
            return Self::default();
        }
        let covered: HashSet<ClientJobLocator> = identities.iter().cloned().collect();
        let unique: Vec<ClientJobLocator> = covered.iter().cloned().collect();
        match app
            .services
            .workflow
            .download_submissions
            .list_seed_goals_for_client_items(&unique)
            .await
        {
            Ok(rows) => Self {
                covered,
                resolved: rows.into_iter().collect(),
            },
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    items = unique.len(),
                    "failed to prefetch seed goals for the reconcile tick; falling back to per-row reads"
                );
                Self::default()
            }
        }
    }

    fn answer(&self, identity: &ClientJobLocator) -> SeedGoalBatchAnswer {
        if let Some(goals) = self.resolved.get(identity) {
            return SeedGoalBatchAnswer::Resolved(goals.clone());
        }
        if self.covered.contains(identity) {
            return SeedGoalBatchAnswer::NoIdentityResolution;
        }
        SeedGoalBatchAnswer::Uncovered
    }
}

/// Read side of the persisted grab-time seed-goal resolution.
///
/// A trait rather than a direct repository call so the gate has one named
/// dependency, and so a failed read is a policy decision made in one place
/// (treated as "no goals", never as "goals met").
#[async_trait::async_trait]
pub(crate) trait SeedGoalsRead: Send + Sync {
    /// The goals this download was grabbed under, or `None` when it was not a
    /// Scryer grab, predates the feature, or no profile applied.
    ///
    /// `batch` is the reconcile tick's prefetch when the caller has one; `None`
    /// (manual import, one-off calls) takes the full per-row path.
    async fn resolved_seed_goals(
        &self,
        key: &SeedGoalLookupKey,
        batch: Option<&SeedGoalBatch>,
    ) -> Option<PersistedSeedGoals>;
}

#[async_trait::async_trait]
impl SeedGoalsRead for AppUseCase {
    async fn resolved_seed_goals(
        &self,
        key: &SeedGoalLookupKey,
        batch: Option<&SeedGoalBatch>,
    ) -> Option<PersistedSeedGoals> {
        let submissions = &self.services.workflow.download_submissions;
        let identity = ClientJobLocator::new(
            Some(key.client_id.as_str()),
            key.client_type.as_str(),
            key.client_item_id.as_str(),
        );
        // Client identity first; the info hash is the fallback because a
        // torrent that was removed and re-added keeps its hash but gets a new
        // client item id on several clients.
        let identity_answer = batch
            .map(|batch| batch.answer(&identity))
            .unwrap_or(SeedGoalBatchAnswer::Uncovered);
        match identity_answer {
            SeedGoalBatchAnswer::Resolved(goals) => return Some(goals),
            // The tick already asked this question for every row it holds.
            SeedGoalBatchAnswer::NoIdentityResolution => {}
            SeedGoalBatchAnswer::Uncovered => match submissions
                .get_seed_goals_for_download(key.canonical_download_id.as_ref(), &identity)
                .await
            {
                Ok(Some(goals)) => return Some(goals),
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        client_id = key.client_id.as_str(),
                        client_type = key.client_type.as_str(),
                        download_client_item_id = key.client_item_id.as_str(),
                        "failed to read persisted seed goals; treating this torrent as having none"
                    );
                }
            },
        }

        let info_hash = key.info_hash.as_deref()?;
        match submissions.find_seed_goals_by_info_hash(info_hash).await {
            Ok(goals) => goals,
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    info_hash,
                    "failed to read persisted seed goals by info hash"
                );
                None
            }
        }
    }
}

/// What a torrent client reports about one torrent's seeding state.
///
/// Every field is optional because the clients differ wildly in what they
/// expose (see the P3 plugin audit): 5 of 13 can report `is_private` at all,
/// several have no seed-time counter, and three keep their goal in a volatile
/// plugin variable.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct TorrentSeedingObservation {
    /// The client's own verdict on its seeding obligation. Tri-state.
    pub can_remove: Option<bool>,
    /// Whether the payload is fully downloaded and stable. **Not** permission
    /// to move it — see the SDK contract.
    pub can_move_files: Option<bool>,
    pub seed_ratio: Option<f64>,
    pub seed_time_seconds: Option<i64>,
    /// `Some(true)` arms the hard private rail. `None` is unknown, not public.
    pub is_private: Option<bool>,
    /// When the payload finished downloading; the wall-clock fallback for the
    /// time axis on clients with no seed-time counter.
    pub completed_at: Option<DateTime<Utc>>,
}

impl From<&DownloadSeedingSnapshot> for TorrentSeedingObservation {
    /// Field-for-field, tri-states intact. The snapshot's goal fields are not
    /// read here: the gate reads goals from the persisted resolution so that a
    /// projection that never ran cannot silently mean "no goals".
    fn from(snapshot: &DownloadSeedingSnapshot) -> Self {
        Self {
            can_remove: snapshot.can_remove,
            can_move_files: snapshot.can_move_files,
            seed_ratio: snapshot.seed_ratio,
            seed_time_seconds: snapshot.seed_time_seconds,
            is_private: snapshot.is_private,
            completed_at: snapshot
                .completed_at
                .as_deref()
                .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
                .map(|value| value.with_timezone(&Utc)),
        }
    }
}

/// The observation carried by one client-item snapshot, if any.
pub(crate) fn observation_from_queue_item(
    item: &DownloadQueueItem,
) -> Option<TorrentSeedingObservation> {
    item.seeding
        .as_ref()
        .map(TorrentSeedingObservation::from)
        .filter(|observation| *observation != TorrentSeedingObservation::default())
}

/// Everything the pure gate needs. Assembled by `evaluate_seeding_gate_for`.
#[derive(Clone, Debug)]
pub(crate) struct SeedingGateInput {
    /// `false` for usenet and anything else that is not a torrent client.
    pub is_torrent: bool,
    pub client_type: String,
    /// `false` once the download has disappeared from the client's listing.
    pub present_in_client: bool,
    pub observation: Option<TorrentSeedingObservation>,
    pub goals: Option<PersistedSeedGoals>,
    pub now: DateTime<Utc>,
}

impl Default for SeedingGateInput {
    fn default() -> Self {
        Self {
            is_torrent: true,
            client_type: String::new(),
            present_in_client: true,
            observation: None,
            goals: None,
            now: Utc::now(),
        }
    }
}

/// What the caller should do with the client entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SeedingGateOutcome {
    /// Not a torrent. The gate does not apply and the legacy path runs.
    NotApplicable,
    /// The entry is already gone from the client. Settle the tracked download
    /// without issuing a removal.
    Vanished,
    /// The obligation is discharged; act on `action`.
    Released { action: SeedGoalMetAction },
    /// The profile's post-import tracking is `HandOff`: settle the download
    /// without touching the client entry and stop managing this torrent.
    ///
    /// Distinct from `Released { Keep }` because the two say different things:
    /// `Keep` is "the obligation is discharged and the profile keeps the
    /// entry", while `HandedOff` is "the operator opted out of management, so
    /// whatever the obligation is, it is no longer Scryer's to track".
    HandedOff,
    /// Still seeding, or the obligation cannot be proven discharged. Hold the
    /// entry and re-evaluate on the next poll.
    Hold,
}

/// The gate's answer plus the reason, which is logged and reported verbatim.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SeedingGateDecision {
    pub outcome: SeedingGateOutcome,
    pub reason: &'static str,
    /// Whether the payload may be moved (as opposed to hardlinked/copied).
    move_allowed: bool,
}

impl SeedingGateDecision {
    fn new(outcome: SeedingGateOutcome, reason: &'static str, move_allowed: bool) -> Self {
        Self {
            outcome,
            reason,
            move_allowed,
        }
    }

    /// The gate does not apply — usenet and other non-torrent protocols.
    pub(crate) fn not_applicable() -> Self {
        Self::new(
            SeedingGateOutcome::NotApplicable,
            reason::NON_TORRENT_PROTOCOL,
            true,
        )
    }

    /// The import mode to actually use, given the configured one.
    ///
    /// `ImportMode::Move` takes the payload out from under the torrent, so it
    /// is only safe when the data is complete **and** the seeding obligation
    /// is discharged. Under the P3 SDK contract `can_move_files` answers only
    /// the first half; this method supplies the second.
    pub(crate) fn import_mode(self, configured: ImportMode) -> ImportMode {
        if configured == ImportMode::Move && !self.move_allowed {
            ImportMode::HardlinkOrCopy
        } else {
            configured
        }
    }
}

/// The pure decision. Every rule the operator locked lives here and nowhere
/// else; `evaluate_seeding_gate_for` only gathers the inputs.
///
/// Two layers: the seeding **obligation** (below) decides what would happen to
/// the client entry, and the profile's **post-import tracking** then decides
/// whether Scryer is still the one deciding.
pub(crate) fn evaluate_seeding_gate(input: &SeedingGateInput) -> SeedingGateDecision {
    let decision = evaluate_seeding_obligation(input);

    // Post-import handoff (Sonarr's post-import category, made explicit).
    //
    // It overrides the *disposition* of any decision that would otherwise hold
    // or release the entry, regardless of goals, `never_remove` or the private
    // rail: those rails exist to prevent a **removal**, and a handoff never
    // removes anything. The operator asked Scryer to stop managing this
    // torrent, so it settles and leaves the queue with the entry untouched.
    //
    // `move_allowed` is deliberately carried over from the obligation decision
    // rather than recomputed. Handing off changes what happens to *tracking*,
    // never what is safe to do to the payload: a torrent that is still seeding
    // is still imported by hardlink-or-copy.
    let hands_off = input
        .goals
        .as_ref()
        .is_some_and(|goals| goals.post_import_tracking.is_hand_off());
    if hands_off
        && matches!(
            decision.outcome,
            SeedingGateOutcome::Hold | SeedingGateOutcome::Released { .. }
        )
    {
        return SeedingGateDecision::new(
            SeedingGateOutcome::HandedOff,
            reason::POST_IMPORT_HANDOFF,
            decision.move_allowed,
        );
    }

    decision
}

/// Whether this torrent's seeding obligation is discharged, and what the
/// profile wants done with the entry when it is.
fn evaluate_seeding_obligation(input: &SeedingGateInput) -> SeedingGateDecision {
    if !input.is_torrent {
        return SeedingGateDecision::not_applicable();
    }

    let observation = input.observation.clone().unwrap_or_default();
    // Data completeness is a hard precondition for a move on its own: a
    // client that says the payload is still being written must never have it
    // moved, whatever the seeding state is.
    let data_stable = observation.can_move_files != Some(false);

    // Nothing left in the client to protect, and nothing to remove. This is
    // the `removes_on_seed_limit` clients (and any torrent the operator pulled
    // by hand); the `AlreadyGone` delete path is the existing precedent.
    if !input.present_in_client {
        return SeedingGateDecision::new(
            SeedingGateOutcome::Vanished,
            reason::TORRENT_NO_LONGER_IN_CLIENT,
            data_stable,
        );
    }

    // Blackhole removal is a filesystem delete against a directory some other
    // client is still serving. Never automatic, regardless of configuration or
    // profile. `Keep` settles the tracked download without touching anything.
    if input
        .client_type
        .trim()
        .eq_ignore_ascii_case(TORRENT_BLACKHOLE_CLIENT_TYPE)
    {
        return SeedingGateDecision::new(
            SeedingGateOutcome::Released {
                action: SeedGoalMetAction::Keep,
            },
            reason::BLACKHOLE_REMOVAL_IS_DESTRUCTIVE,
            data_stable && observation.can_move_files == Some(true),
        );
    }

    let goals = input.goals.clone().unwrap_or_default();

    // Profile-level hard stop: seed forever.
    if goals.never_remove {
        return SeedingGateDecision::new(
            SeedingGateOutcome::Hold,
            reason::PROFILE_NEVER_REMOVE,
            false,
        );
    }

    // Hard private rail. An observed private torrent is never released on the
    // client's word alone — only a profile goal Scryer can prove was met (or
    // an explicit operator action outside this gate) releases it.
    if observation.is_private == Some(true) && !goals.has_goals() {
        return SeedingGateDecision::new(
            SeedingGateOutcome::Hold,
            reason::PRIVATE_WITHOUT_GOALS,
            false,
        );
    }

    if goals.has_goals() {
        // The profile goal is authoritative in both directions: `can_remove`
        // reports a *client* limit, which is a different question.
        return if scryer_goal_is_met(&goals, &observation, input.now) {
            SeedingGateDecision::new(
                SeedingGateOutcome::Released {
                    action: goal_met_action_for(&goals),
                },
                reason::PROFILE_GOAL_MET,
                data_stable,
            )
        } else {
            SeedingGateDecision::new(SeedingGateOutcome::Hold, reason::PROFILE_GOAL_UNMET, false)
        };
    }

    // Universal-gate baseline: no Scryer goal, so the client's own limit
    // regime is the only policy there is (Sonarr's `CanBeRemoved`).
    match observation.can_remove {
        Some(true) => SeedingGateDecision::new(
            SeedingGateOutcome::Released {
                action: goal_met_action_for(&goals),
            },
            reason::CLIENT_OBLIGATION_MET,
            data_stable,
        ),
        Some(false) => {
            SeedingGateDecision::new(SeedingGateOutcome::Hold, reason::CLIENT_LIMIT_UNMET, false)
        }
        None => SeedingGateDecision::new(
            SeedingGateOutcome::Hold,
            reason::CLIENT_VERDICT_UNKNOWN,
            false,
        ),
    }
}

/// Sonarr's OR-semantics: either axis reaching its goal discharges the
/// obligation. The minimal Tier-B primitive; wave 3 extends it to continuous
/// evaluation in the polling loop.
fn scryer_goal_is_met(
    goals: &PersistedSeedGoals,
    observation: &TorrentSeedingObservation,
    now: DateTime<Utc>,
) -> bool {
    if let (Some(goal), Some(observed)) = (goals.seed_goal_ratio, observation.seed_ratio)
        && goal.is_finite()
        && observed.is_finite()
        && observed >= goal
    {
        return true;
    }

    let Some(goal_seconds) = goals.seed_goal_seconds.filter(|value| *value > 0) else {
        return false;
    };

    match observation.seed_time_seconds {
        Some(observed) => observed >= goal_seconds,
        // No seed-time counter on this client. Wall clock since the payload
        // finished is the same fallback Sonarr uses for rTorrent.
        None => observation
            .completed_at
            .map(|completed_at| {
                now.signed_duration_since(completed_at).num_seconds() >= goal_seconds
            })
            .unwrap_or(false),
    }
}

/// Whether a download client speaks the torrent protocol.
///
/// Derived from the client's declared accepted inputs, so plugin-provided
/// clients answer for themselves and the built-in usenet clients are excluded
/// without a hard-coded list.
pub(crate) fn client_type_is_torrent(app: &AppUseCase, client_type: &str) -> bool {
    let plugin_provider = app
        .services
        .integrations
        .download_client_plugin_provider
        .available();
    crate::accepted_inputs_for_client(client_type, plugin_provider)
        .iter()
        .any(|input| {
            matches!(
                input,
                DownloadSourceKind::TorrentFile | DownloadSourceKind::MagnetUri
            )
        })
}

/// The observed torrent state for one client item, from the published
/// tracked-download snapshot.
///
/// The snapshot is the cache the download-client poller republishes at the end
/// of every cycle, so it is one tick behind while a cycle is running. Callers
/// that hold the live tracked row must pass their own observation to
/// `evaluate_seeding_gate_with` instead of relying on this; this is the lookup
/// for callers that only have a client identity (manual import), where being a
/// tick behind changes nothing — the answer only ever downgrades a `Move` to a
/// copy.
async fn observe_torrent_seeding(
    app: &AppUseCase,
    key: &SeedGoalLookupKey,
) -> Option<TorrentSeedingObservation> {
    let snapshot = app
        .runtime
        .acquisition
        .tracked_download_snapshot
        .read()
        .await;
    snapshot
        .values()
        .find(|tracked| tracked_matches_lookup_key(&tracked.client_item, key))
        .and_then(|tracked| observation_from_queue_item(&tracked.client_item))
}

fn tracked_matches_lookup_key(item: &DownloadQueueItem, key: &SeedGoalLookupKey) -> bool {
    if !item
        .client_type
        .trim()
        .eq_ignore_ascii_case(&key.client_type)
    {
        return false;
    }
    let client_id = item.client_id.trim();
    // An empty configured client id on either side means "the only client of
    // this type", which is how the routing key is built elsewhere.
    if !client_id.is_empty() && !key.client_id.is_empty() && client_id != key.client_id {
        return false;
    }
    let item_id = item.download_client_item_id.trim();
    if item_id.eq_ignore_ascii_case(key.client_item_id.trim()) {
        return true;
    }
    match (
        crate::normalize_torrent_info_hash(Some(item_id)),
        key.info_hash.as_deref(),
    ) {
        (Some(observed), Some(expected)) => observed == expected,
        _ => false,
    }
}

/// Assemble the gate input for one client item and decide.
pub(crate) async fn evaluate_seeding_gate_for(
    app: &AppUseCase,
    key: &SeedGoalLookupKey,
    present_in_client: bool,
) -> SeedingGateDecision {
    evaluate_seeding_gate_with(app, key, present_in_client, None, None).await
}

/// As `evaluate_seeding_gate_for`, with an observation the caller already has.
///
/// `observation: None` means "look one up"; it does not mean "there is none". A
/// caller that holds the live tracked row always passes `Some(...)` — even an
/// all-unknown observation — because its copy is the freshest one that exists.
///
/// `goal_batch` is the reconcile tick's one-query prefetch; `None` takes the
/// per-row read path.
pub(crate) async fn evaluate_seeding_gate_with(
    app: &AppUseCase,
    key: &SeedGoalLookupKey,
    present_in_client: bool,
    observation: Option<TorrentSeedingObservation>,
    goal_batch: Option<&SeedGoalBatch>,
) -> SeedingGateDecision {
    if !client_type_is_torrent(app, &key.client_type) {
        return SeedingGateDecision::not_applicable();
    }

    let observation = match observation {
        Some(observation) => Some(observation),
        None => observe_torrent_seeding(app, key).await,
    };
    let input = SeedingGateInput {
        is_torrent: true,
        client_type: key.client_type.clone(),
        present_in_client,
        observation,
        goals: app.resolved_seed_goals(key, goal_batch).await,
        now: Utc::now(),
    };
    evaluate_seeding_gate(&input)
}

/// Gate input key for a completed download.
pub(crate) fn seed_goal_lookup_key_for_completed(
    completed: &CompletedDownload,
) -> SeedGoalLookupKey {
    SeedGoalLookupKey {
        canonical_download_id: None,
        client_id: completed.client_id.trim().to_string(),
        client_type: completed.client_type.trim().to_string(),
        client_item_id: completed.download_client_item_id.trim().to_string(),
        info_hash: crate::normalize_torrent_info_hash(Some(
            completed.download_client_item_id.as_str(),
        )),
    }
}

/// The import mode to use for a completed download, downgrading a configured
/// `Move` to hardlink-or-copy while the torrent is still seeding.
///
/// A torrent Scryer merely observes (`completed` is `None` — a manual import
/// with no client provenance) keeps the configured mode: there is no torrent
/// identity to gate on.
pub(crate) async fn resolve_seeding_safe_import_mode(
    app: &AppUseCase,
    library_id: Option<&str>,
    facet: &MediaFacet,
    completed: Option<&CompletedDownload>,
) -> AppResult<ImportMode> {
    let configured = app.resolve_import_mode(library_id, facet).await?;
    if configured != ImportMode::Move {
        return Ok(configured);
    }
    let Some(completed) = completed else {
        return Ok(configured);
    };

    let key = seed_goal_lookup_key_for_completed(completed);
    let decision = evaluate_seeding_gate_for(app, &key, true).await;
    let effective = decision.import_mode(configured);
    if effective != configured {
        tracing::info!(
            client_id = key.client_id.as_str(),
            client_type = key.client_type.as_str(),
            download_client_item_id = key.client_item_id.as_str(),
            reason = decision.reason,
            "forcing hardlink-or-copy import: torrent has not discharged its seeding obligation"
        );
    }
    Ok(effective)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn goals(ratio: Option<f64>, seconds: Option<i64>) -> PersistedSeedGoals {
        PersistedSeedGoals {
            seeding_profile_id: Some("profile-1".to_string()),
            seed_goal_ratio: ratio,
            seed_goal_seconds: seconds,
            never_remove: false,
            goal_met_action: Some(SeedGoalMetAction::RemoveEntry),
            post_import_tracking: scryer_domain::PostImportTracking::Park,
            resolution_source: crate::SeedGoalResolutionSource::Indexer,
            info_hash: None,
        }
    }

    fn handoff_goals(ratio: Option<f64>, seconds: Option<i64>) -> PersistedSeedGoals {
        PersistedSeedGoals {
            post_import_tracking: scryer_domain::PostImportTracking::HandOff,
            ..goals(ratio, seconds)
        }
    }

    fn input(observation: TorrentSeedingObservation) -> SeedingGateInput {
        SeedingGateInput {
            client_type: "qbittorrent".to_string(),
            observation: Some(observation),
            ..SeedingGateInput::default()
        }
    }

    #[test]
    fn usenet_downloads_are_not_gated() {
        let decision = evaluate_seeding_gate(&SeedingGateInput {
            is_torrent: false,
            client_type: "sabnzbd".to_string(),
            ..SeedingGateInput::default()
        });
        assert_eq!(decision.outcome, SeedingGateOutcome::NotApplicable);
        assert_eq!(decision.import_mode(ImportMode::Move), ImportMode::Move);
    }

    #[test]
    fn a_torrent_with_no_goals_and_no_client_verdict_is_held() {
        let decision = evaluate_seeding_gate(&input(TorrentSeedingObservation::default()));
        assert_eq!(decision.outcome, SeedingGateOutcome::Hold);
        assert_eq!(
            decision.reason,
            "no_resolved_goals_and_client_verdict_unknown"
        );
        assert_eq!(
            decision.import_mode(ImportMode::Move),
            ImportMode::HardlinkOrCopy
        );
    }

    #[test]
    fn a_client_reporting_limits_met_releases_a_torrent_with_no_goals() {
        let decision = evaluate_seeding_gate(&input(TorrentSeedingObservation {
            can_remove: Some(true),
            can_move_files: Some(true),
            ..TorrentSeedingObservation::default()
        }));
        assert_eq!(
            decision.outcome,
            SeedingGateOutcome::Released {
                action: SeedGoalMetAction::RemoveEntry
            }
        );
        assert_eq!(decision.reason, "client_reports_seeding_obligation_met");
        assert_eq!(decision.import_mode(ImportMode::Move), ImportMode::Move);
    }

    #[test]
    fn a_client_reporting_an_unmet_limit_holds_a_torrent_with_no_goals() {
        let decision = evaluate_seeding_gate(&input(TorrentSeedingObservation {
            can_remove: Some(false),
            can_move_files: Some(true),
            ..TorrentSeedingObservation::default()
        }));
        assert_eq!(decision.outcome, SeedingGateOutcome::Hold);
        assert_eq!(decision.reason, "client_reports_unmet_seed_limit");
    }

    #[test]
    fn a_met_profile_goal_overrides_a_client_reporting_an_unmet_limit() {
        let decision = evaluate_seeding_gate(&SeedingGateInput {
            goals: Some(goals(Some(1.0), None)),
            observation: Some(TorrentSeedingObservation {
                can_remove: Some(false),
                can_move_files: Some(true),
                seed_ratio: Some(1.4),
                ..TorrentSeedingObservation::default()
            }),
            ..SeedingGateInput::default()
        });
        assert_eq!(
            decision.outcome,
            SeedingGateOutcome::Released {
                action: SeedGoalMetAction::RemoveEntry
            }
        );
        assert_eq!(decision.reason, "profile_goal_met");
    }

    #[test]
    fn a_client_reporting_limits_met_is_not_enough_when_a_profile_goal_is_unmet() {
        // uTorrent / Transmission global-idle: `Some(true)` only means the
        // client stopped it, which is not the profile's goal.
        let decision = evaluate_seeding_gate(&SeedingGateInput {
            goals: Some(goals(Some(2.0), None)),
            observation: Some(TorrentSeedingObservation {
                can_remove: Some(true),
                can_move_files: Some(true),
                seed_ratio: Some(0.4),
                ..TorrentSeedingObservation::default()
            }),
            ..SeedingGateInput::default()
        });
        assert_eq!(decision.outcome, SeedingGateOutcome::Hold);
        assert_eq!(decision.reason, "profile_goal_unmet");
        assert_eq!(
            decision.import_mode(ImportMode::Move),
            ImportMode::HardlinkOrCopy
        );
    }

    #[test]
    fn either_axis_alone_meets_the_goal() {
        let ratio_only = evaluate_seeding_gate(&SeedingGateInput {
            goals: Some(goals(Some(2.0), Some(86_400))),
            observation: Some(TorrentSeedingObservation {
                seed_ratio: Some(2.0),
                seed_time_seconds: Some(10),
                ..TorrentSeedingObservation::default()
            }),
            ..SeedingGateInput::default()
        });
        assert_eq!(ratio_only.reason, "profile_goal_met");

        let time_only = evaluate_seeding_gate(&SeedingGateInput {
            goals: Some(goals(Some(2.0), Some(3_600))),
            observation: Some(TorrentSeedingObservation {
                seed_ratio: Some(0.1),
                seed_time_seconds: Some(3_601),
                ..TorrentSeedingObservation::default()
            }),
            ..SeedingGateInput::default()
        });
        assert_eq!(time_only.reason, "profile_goal_met");
    }

    #[test]
    fn wall_clock_since_completion_covers_clients_without_a_seed_time_counter() {
        let now = Utc::now();
        let met = evaluate_seeding_gate(&SeedingGateInput {
            goals: Some(goals(None, Some(3_600))),
            observation: Some(TorrentSeedingObservation {
                completed_at: Some(now - chrono::Duration::seconds(3_700)),
                ..TorrentSeedingObservation::default()
            }),
            now,
            ..SeedingGateInput::default()
        });
        assert_eq!(met.reason, "profile_goal_met");

        let unmet = evaluate_seeding_gate(&SeedingGateInput {
            goals: Some(goals(None, Some(3_600))),
            observation: Some(TorrentSeedingObservation {
                completed_at: Some(now - chrono::Duration::seconds(60)),
                ..TorrentSeedingObservation::default()
            }),
            now,
            ..SeedingGateInput::default()
        });
        assert_eq!(unmet.reason, "profile_goal_unmet");
    }

    #[test]
    fn a_private_torrent_without_goals_is_never_released_by_the_client_verdict() {
        let decision = evaluate_seeding_gate(&input(TorrentSeedingObservation {
            can_remove: Some(true),
            can_move_files: Some(true),
            is_private: Some(true),
            ..TorrentSeedingObservation::default()
        }));
        assert_eq!(decision.outcome, SeedingGateOutcome::Hold);
        assert_eq!(decision.reason, "private_torrent_without_resolved_goals");
    }

    #[test]
    fn a_private_torrent_with_a_met_profile_goal_is_released() {
        let decision = evaluate_seeding_gate(&SeedingGateInput {
            goals: Some(goals(Some(1.0), None)),
            observation: Some(TorrentSeedingObservation {
                can_remove: None,
                can_move_files: Some(true),
                is_private: Some(true),
                seed_ratio: Some(1.2),
                ..TorrentSeedingObservation::default()
            }),
            ..SeedingGateInput::default()
        });
        assert_eq!(decision.reason, "profile_goal_met");
    }

    #[test]
    fn an_unknown_private_flag_is_not_read_as_public() {
        // `is_private: None` must not create a removal path that
        // `is_private: Some(false)` would not also create.
        let unknown = evaluate_seeding_gate(&input(TorrentSeedingObservation {
            can_remove: None,
            is_private: None,
            ..TorrentSeedingObservation::default()
        }));
        let public = evaluate_seeding_gate(&input(TorrentSeedingObservation {
            can_remove: None,
            is_private: Some(false),
            ..TorrentSeedingObservation::default()
        }));
        assert_eq!(unknown.outcome, SeedingGateOutcome::Hold);
        assert_eq!(unknown.outcome, public.outcome);
    }

    #[test]
    fn never_remove_holds_even_with_a_met_goal_and_a_willing_client() {
        let decision = evaluate_seeding_gate(&SeedingGateInput {
            goals: Some(PersistedSeedGoals {
                never_remove: true,
                ..goals(Some(1.0), None)
            }),
            observation: Some(TorrentSeedingObservation {
                can_remove: Some(true),
                seed_ratio: Some(9.0),
                ..TorrentSeedingObservation::default()
            }),
            ..SeedingGateInput::default()
        });
        assert_eq!(decision.outcome, SeedingGateOutcome::Hold);
        assert_eq!(decision.reason, "profile_never_remove");
    }

    #[test]
    fn a_vanished_torrent_settles_without_a_removal() {
        let decision = evaluate_seeding_gate(&SeedingGateInput {
            present_in_client: false,
            observation: Some(TorrentSeedingObservation {
                can_remove: None,
                ..TorrentSeedingObservation::default()
            }),
            ..SeedingGateInput::default()
        });
        assert_eq!(decision.outcome, SeedingGateOutcome::Vanished);
        assert_eq!(decision.reason, "torrent_no_longer_in_client");
    }

    #[test]
    fn torrent_blackhole_is_never_auto_removed() {
        let decision = evaluate_seeding_gate(&SeedingGateInput {
            client_type: TORRENT_BLACKHOLE_CLIENT_TYPE.to_string(),
            goals: Some(goals(Some(0.1), None)),
            observation: Some(TorrentSeedingObservation {
                can_remove: None,
                can_move_files: Some(true),
                seed_ratio: Some(99.0),
                ..TorrentSeedingObservation::default()
            }),
            ..SeedingGateInput::default()
        });
        assert_eq!(
            decision.outcome,
            SeedingGateOutcome::Released {
                action: SeedGoalMetAction::Keep
            }
        );
        assert_eq!(decision.reason, "torrent_blackhole_removal_is_destructive");
    }

    #[test]
    fn blackhole_entries_are_not_moved_before_they_settle() {
        let decision = evaluate_seeding_gate(&SeedingGateInput {
            client_type: TORRENT_BLACKHOLE_CLIENT_TYPE.to_string(),
            observation: Some(TorrentSeedingObservation {
                can_move_files: None,
                ..TorrentSeedingObservation::default()
            }),
            ..SeedingGateInput::default()
        });
        assert_eq!(
            decision.import_mode(ImportMode::Move),
            ImportMode::HardlinkOrCopy
        );
    }

    #[test]
    fn goal_met_action_flows_through_to_the_caller() {
        for action in [
            SeedGoalMetAction::RemoveEntry,
            SeedGoalMetAction::StopSeeding,
            SeedGoalMetAction::Keep,
        ] {
            let decision = evaluate_seeding_gate(&SeedingGateInput {
                goals: Some(PersistedSeedGoals {
                    goal_met_action: Some(action),
                    ..goals(Some(1.0), None)
                }),
                observation: Some(TorrentSeedingObservation {
                    seed_ratio: Some(1.0),
                    ..TorrentSeedingObservation::default()
                }),
                ..SeedingGateInput::default()
            });
            assert_eq!(decision.outcome, SeedingGateOutcome::Released { action });
        }
    }

    #[test]
    fn unstable_data_blocks_a_move_even_when_seeding_is_done() {
        let decision = evaluate_seeding_gate(&input(TorrentSeedingObservation {
            can_remove: Some(true),
            can_move_files: Some(false),
            ..TorrentSeedingObservation::default()
        }));
        assert!(matches!(
            decision.outcome,
            SeedingGateOutcome::Released { .. }
        ));
        assert_eq!(
            decision.import_mode(ImportMode::Move),
            ImportMode::HardlinkOrCopy
        );
        assert_eq!(
            decision.import_mode(ImportMode::HardlinkOrCopy),
            ImportMode::HardlinkOrCopy
        );
    }

    // ── post-import handoff ───────────────────────────────────────────────

    #[test]
    fn a_handoff_profile_settles_a_torrent_that_would_otherwise_be_held() {
        // The baseline hold (no client verdict, no met goal) is exactly the
        // state a handoff has to settle: nothing is proven, and the operator
        // said Scryer should stop asking.
        let decision = evaluate_seeding_gate(&SeedingGateInput {
            goals: Some(handoff_goals(Some(2.0), None)),
            observation: Some(TorrentSeedingObservation {
                can_remove: None,
                seed_ratio: Some(0.1),
                ..TorrentSeedingObservation::default()
            }),
            ..SeedingGateInput::default()
        });
        assert_eq!(decision.outcome, SeedingGateOutcome::HandedOff);
        assert_eq!(decision.reason, "post_import_handoff");
    }

    #[test]
    fn a_handoff_never_removes_so_the_removal_rails_do_not_apply() {
        // `never_remove` and the private rail both exist to stop a removal.
        // A handoff removes nothing, so neither of them can block it.
        for goals in [
            PersistedSeedGoals {
                never_remove: true,
                ..handoff_goals(Some(1.0), None)
            },
            handoff_goals(None, None),
        ] {
            let decision = evaluate_seeding_gate(&SeedingGateInput {
                goals: Some(goals),
                observation: Some(TorrentSeedingObservation {
                    can_remove: None,
                    is_private: Some(true),
                    seed_ratio: Some(0.0),
                    ..TorrentSeedingObservation::default()
                }),
                ..SeedingGateInput::default()
            });
            assert_eq!(decision.outcome, SeedingGateOutcome::HandedOff);
        }
    }

    #[test]
    fn a_handoff_makes_the_goal_met_action_moot() {
        // Whatever the profile would have done to the entry once the goal was
        // met, a handoff does nothing to it at all.
        for action in [
            SeedGoalMetAction::RemoveEntry,
            SeedGoalMetAction::StopSeeding,
            SeedGoalMetAction::Keep,
        ] {
            let decision = evaluate_seeding_gate(&SeedingGateInput {
                goals: Some(PersistedSeedGoals {
                    goal_met_action: Some(action),
                    ..handoff_goals(Some(1.0), None)
                }),
                observation: Some(TorrentSeedingObservation {
                    can_remove: Some(true),
                    seed_ratio: Some(5.0),
                    ..TorrentSeedingObservation::default()
                }),
                ..SeedingGateInput::default()
            });
            assert_eq!(decision.outcome, SeedingGateOutcome::HandedOff);
        }
    }

    #[test]
    fn a_handoff_does_not_make_a_seeding_payload_safe_to_move() {
        // The invariant: handoff changes what happens to *tracking*, never the
        // import mode. A torrent whose obligation is unproven is still copied.
        let seeding = evaluate_seeding_gate(&SeedingGateInput {
            goals: Some(handoff_goals(Some(2.0), None)),
            observation: Some(TorrentSeedingObservation {
                can_move_files: Some(true),
                seed_ratio: Some(0.1),
                ..TorrentSeedingObservation::default()
            }),
            ..SeedingGateInput::default()
        });
        assert_eq!(seeding.outcome, SeedingGateOutcome::HandedOff);
        assert_eq!(
            seeding.import_mode(ImportMode::Move),
            ImportMode::HardlinkOrCopy
        );

        // ...and one that is provably discharged still moves, exactly as it
        // would without the handoff.
        let discharged = evaluate_seeding_gate(&SeedingGateInput {
            goals: Some(handoff_goals(Some(2.0), None)),
            observation: Some(TorrentSeedingObservation {
                can_move_files: Some(true),
                seed_ratio: Some(2.5),
                ..TorrentSeedingObservation::default()
            }),
            ..SeedingGateInput::default()
        });
        assert_eq!(discharged.outcome, SeedingGateOutcome::HandedOff);
        assert_eq!(discharged.import_mode(ImportMode::Move), ImportMode::Move);
    }

    #[test]
    fn a_download_with_no_profile_still_parks_under_the_universal_gate() {
        // The fail-closed rail: handoff is opt-in per profile, so an install
        // with no profiles keeps holding.
        let no_profile = evaluate_seeding_gate(&input(TorrentSeedingObservation::default()));
        assert_eq!(no_profile.outcome, SeedingGateOutcome::Hold);

        let parking_profile = evaluate_seeding_gate(&SeedingGateInput {
            goals: Some(goals(Some(2.0), None)),
            observation: Some(TorrentSeedingObservation {
                seed_ratio: Some(0.1),
                ..TorrentSeedingObservation::default()
            }),
            ..SeedingGateInput::default()
        });
        assert_eq!(parking_profile.outcome, SeedingGateOutcome::Hold);
    }

    #[test]
    fn a_vanished_torrent_is_not_reported_as_handed_off() {
        // There is nothing left to hand off; `AlreadyGone` is the honest
        // outcome and the one that settles without a client call.
        let decision = evaluate_seeding_gate(&SeedingGateInput {
            present_in_client: false,
            goals: Some(handoff_goals(None, None)),
            ..SeedingGateInput::default()
        });
        assert_eq!(decision.outcome, SeedingGateOutcome::Vanished);
        assert_eq!(decision.reason, "torrent_no_longer_in_client");
    }

    #[test]
    fn usenet_is_never_handed_off() {
        let decision = evaluate_seeding_gate(&SeedingGateInput {
            is_torrent: false,
            client_type: "sabnzbd".to_string(),
            goals: Some(handoff_goals(None, None)),
            ..SeedingGateInput::default()
        });
        assert_eq!(decision.outcome, SeedingGateOutcome::NotApplicable);
    }

    #[test]
    fn a_resolved_profile_with_no_numeric_goals_still_supplies_its_action() {
        let decision = evaluate_seeding_gate(&SeedingGateInput {
            goals: Some(PersistedSeedGoals {
                goal_met_action: Some(SeedGoalMetAction::StopSeeding),
                ..goals(None, None)
            }),
            observation: Some(TorrentSeedingObservation {
                can_remove: Some(true),
                ..TorrentSeedingObservation::default()
            }),
            ..SeedingGateInput::default()
        });
        assert_eq!(decision.reason, "client_reports_seeding_obligation_met");
        assert_eq!(
            decision.outcome,
            SeedingGateOutcome::Released {
                action: SeedGoalMetAction::StopSeeding
            }
        );
    }
}

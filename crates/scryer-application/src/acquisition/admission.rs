//! The single admission gate.
//!
//! One predicate decides whether a release may replace what is already in the
//! library — and both the grab path and the import path call it. That is the
//! whole point: Scryer must never queue a download it already knows it will
//! refuse, and the only way to guarantee that rather than tune for it is for
//! the two decisions to be the same function over the same facts.
//!
//! ## What belongs here, and what does not
//!
//! Admission owns everything that depends on **what is currently on disk**:
//! the profile's upgrade guard, the required delta, the broader-span guard, and
//! the manual override. None of it belongs in a release's score — a file's
//! persisted bar must not depend on what happened to be beside it the day it
//! landed. See [`crate::canonical_scoring`].
//!
//! **Upgrade cooldown is deliberately absent.** A cooldown rate-limits
//! *starting* work; admission decides whether to *accept* a file that already
//! exists. Time passes between grab and import, so a cooldown evaluated at both
//! ends would let the two gates disagree about an identical release — exactly
//! the divergence this module exists to remove. It stays on the grab path.
//!
//! ## Why the incumbent set is plural
//!
//! Not because several files stack on one episode — only `primary` files are
//! ever upgrade targets, and this module filters to them. The plurality comes
//! from the *candidate's* span: a season pack covering E01–E12 faces up to
//! twelve incumbents, one primary file per episode, and may only land if it
//! beats all of them.
//!
//! ## Where this sits in the loop
//!
//! ```text
//! Missing  ──grab (announced beats nothing)────────▶ Grabbed
//! Occupied ──grab (announced beats bar, policy ok)─▶ Grabbed
//! Grabbed  ──completes──▶ ImportGate ──admit──▶ Occupied | Satisfied
//! ```
//!
//! Both arrows marked "grab" and the admit arm of the import gate are this
//! module. The comparison is lexicographic — **tier, then revision, then score**
//! — and what differs between the lanes is only the [`AdmissionPolicy`]: grab
//! applies churn thresholds, the upgrade guard, the format cutoff and the
//! queue; import accepts ties and refuses only downgrades; manual bypasses the
//! score entirely while the structural span guard still binds.

use std::collections::HashSet;

/// What the candidate is trying to occupy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AdmissionScope {
    /// A title-scoped file (a movie).
    Title,
    /// One series-movie link.
    SeriesMovieLink(String),
    /// The episode span the candidate covers.
    Episodes(Vec<String>),
}

/// A primary file already occupying part of the target scope.
#[derive(Debug, Clone)]
pub(crate) struct Incumbent {
    /// Position of this file's quality in the profile's ordering; lower is
    /// better, `None` when the quality is not one the profile lists.
    pub tier_index: Option<usize>,
    /// PROPER/REPACK rank of the release this file came from, re-derived from
    /// the row exactly like its bar.
    ///
    /// A row with no `grabbed_release_title` and no `scene_name` falls back to
    /// the file *stem*, and a renamed file (`Show - S01E01 - Title.mkv`) carries
    /// no PROPER/REPACK token — so its revision reads `0` even if the release
    /// was a PROPER, and the first PROPER after a rename looks like a revision
    /// upgrade over itself. There is no cheap fix; what stops the loop is the
    /// exact-identity check in `evaluate_auto_candidate`, which still recognises
    /// a re-grab of the identical release.
    pub revision: i32,
    pub file_id: String,
    pub file_path: String,
    /// The release group that produced this file. A REPACK is a group
    /// re-releasing its own encode, so the comparison needs the incumbent's
    /// group and not just its score.
    pub release_group: Option<String>,
    /// The incumbent's canonical landed score — its bar.
    pub score: i32,
    /// Episode ids this file covers. Empty for title and link scopes.
    pub covers: Vec<String>,
    pub created_at: String,
}

/// A release for this scope that is already in the download client's queue.
///
/// Sonarr's `QueueSpecification` treats a queued item exactly as it treats a
/// file on disk, and so do we: it is a pseudo-incumbent on the same
/// tier → revision → score ladder. The alternative — Scryer's old scope-level
/// "something is in flight, skip" — could not tell "the same release is already
/// downloading" from "a 2160p release is available and a 720p one is
/// downloading", so it refused both.
///
/// Its facts are always **re-derived from the submission's release title**, never
/// read out of the `grabbed_release` JSON. That number was computed under
/// whatever profile, persona and rule packs were live at grab time; using it
/// would reintroduce stale decisions instead of applying the current profile,
/// and it does not exist at all for pre-0.18 rows.
#[derive(Debug, Clone)]
pub(crate) struct QueuedRelease {
    /// The release name, for the operator-facing message.
    pub title: String,
    /// Episode members covered by the queued submission. Empty for title and
    /// series-movie-link scopes, which are evaluated as single-file subjects.
    pub covers: Vec<String>,
    pub tier_index: Option<usize>,
    pub revision: i32,
    pub score: i32,
    /// [`release_key`] of `title`, so the gate can tell "the very release that
    /// is already in flight" from "an equal one". `None` when the queued
    /// title is unknown.
    pub release_key: Option<u64>,
}

/// The identity a release name carries for the "already grabbed" refusal:
/// case-folded, with every separator dropped, so `Desert.Warrior.2025.1080p`
/// and `desert warrior 2025 1080p` name the same download. `None` for an empty
/// name, which must never match anything.
pub(crate) fn release_key(title: &str) -> Option<u64> {
    use std::hash::{Hash, Hasher};
    let normalized = title
        .chars()
        .filter(|ch| ch.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>();
    if normalized.is_empty() {
        return None;
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    normalized.hash(&mut hasher);
    Some(hasher.finish())
}

/// Incumbent-aware policy. Both callers build one; only the numbers differ.
///
/// Grab is deliberately the stricter of the two: it applies the persona's churn
/// thresholds, while import asks only that the file be better than what it
/// replaces. Strictness in that direction is safe — anything grab declines was
/// never downloaded — and it is what lets the two share a gate without import
/// second-guessing a download it already authorised.
///
/// There is no "cross-tier" relaxation any more. It existed because the quality
/// tier used to be worth 3200/900/300 *inside* the score, so a whole-tier
/// upgrade showed up as a delta above 1000 and the churn threshold had to be
/// relaxed for it. Tier is now compared before score and a better tier admits
/// outright above this code, so the only thing a delta threshold ever sees is a
/// same-tier comparison — and there is exactly one number for that.
#[derive(Debug, Clone, Copy)]
pub(crate) struct AdmissionPolicy {
    pub allow_upgrades: bool,
    /// Minimum improvement required to displace a same-tier incumbent.
    pub min_delta: i32,
    /// Once an incumbent has reached this score, same-tier improvements stop
    /// being worth the bandwidth. A better *tier* still admits, because it never
    /// reaches this comparison.
    ///
    /// Sonarr's `CutoffFormatScore`, and it is a **ceiling on the incumbent**,
    /// not a floor on the candidate. The two used to be the same profile field
    /// (`min_score_to_grab`), which meant "never grab anything under 100" also
    /// said "stop upgrading once you have 100". The floor is now a veto in
    /// the scorer (`apply_min_score_gate`) and never reaches this module; the
    /// grab lanes fill this from `criteria.cutoff_score`, falling back to
    /// `min_score_to_grab` when the profile forbids upgrades — which is exactly
    /// Sonarr's `profile.UpgradeAllowed ? CutoffFormatScore : MinFormatScore`.
    pub cutoff_score: Option<i32>,
    /// An operator asked for this explicitly. Bypasses every score comparison;
    /// the structural span guard still applies.
    pub manual_override: bool,
    /// Whether the scope's in-flight submissions count as pseudo-incumbents.
    ///
    /// **Grab only.** Import must never re-litigate the queue: the bytes
    /// are already on disk, and refusing them because something else is still
    /// downloading would discard a finished download in favour of one that may
    /// never finish. Queued downloads coexist rather than being canceled;
    /// whichever lands worse is skipped at its own import.
    pub applies_to_queue: bool,
}

impl AdmissionPolicy {
    pub(crate) fn manual() -> Self {
        Self {
            allow_upgrades: true,
            min_delta: 0,
            cutoff_score: None,
            manual_override: true,
            applies_to_queue: false,
        }
    }

    /// Import's policy: refuse a downgrade, accept anything else.
    ///
    /// Deliberately more permissive than grab, and deliberately *not* strictly
    /// better. The bytes are already on disk; discarding a file that merely ties
    /// the incumbent wastes the download and produces exactly the "existing file
    /// is equal or better" refusal that this change set exists to remove. The
    /// churn thresholds, the profile's upgrade guard and the score floor are
    /// acquisition policy and already ran at grab — re-applying them here would
    /// have import second-guess a download Scryer itself authorised.
    ///
    /// This is how Sonarr splits it: `UpgradableSpecification` (grab) rejects on
    /// `newFormatScore <= currentFormatScore` and applies `UpgradeAllowed`,
    /// `CutoffFormatScore` and `MinUpgradeFormatScore`; the import spec rejects
    /// only `newFormatScore < currentFormatScore` and applies none of them.
    pub(crate) fn not_a_downgrade() -> Self {
        Self {
            allow_upgrades: true,
            min_delta: 0,
            cutoff_score: None,
            manual_override: false,
            applies_to_queue: false,
        }
    }

    /// The improvement this candidate must show over a same-tier incumbent.
    ///
    /// A `min_delta` of `0` means "a tie is good enough" — import's stance. No
    /// floor is imposed here: grab's personas all set a positive same-tier
    /// delta, so the only policy that reaches zero is the one that means to.
    fn required_delta(&self) -> i32 {
        self.min_delta
    }

    /// Whether an incumbent has already reached the profile's format cutoff,
    /// past which same-tier trimmings stop earning bandwidth.
    ///
    /// This is only ever consulted for a same-tier comparison: a better tier has
    /// already admitted by the time control reaches here.
    fn incumbent_at_format_cutoff(&self, incumbent_score: i32) -> bool {
        self.cutoff_score
            .is_some_and(|cutoff| incumbent_score >= cutoff)
    }
}

/// What a candidate is, for admission purposes: where it sits in the profile's
/// quality ordering, whether it is a re-release of that quality, and what it
/// scores within that tier — compared in exactly that order.
///
/// Three separate values rather than one number. Sonarr compares
/// `QualityModelComparer` (quality, then revision) first and only consults the
/// custom-format score when both are equal; folding any of it into the score is
/// what let a size cliff outweigh a resolution step here.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CandidateFacts {
    pub tier_index: Option<usize>,
    /// PROPER/REPACK rank; [`crate::acquisition::scoring::revision_rank`].
    pub revision: i32,
    pub score: i32,
    /// [`release_key`] of the candidate's release name, when the caller knows
    /// it. Only the queue comparison reads it: a candidate that *is* the
    /// queued release is refused before any ladder runs.
    pub release_key: Option<u64>,
}

impl CandidateFacts {
    pub(crate) fn new(tier_index: Option<usize>, revision: i32, score: i32) -> Self {
        Self {
            tier_index,
            revision,
            score,
            release_key: None,
        }
    }

    /// Attach the candidate's release identity; see [`release_key`].
    pub(crate) fn with_release_title(mut self, title: &str) -> Self {
        self.release_key = release_key(title);
        self
    }
}

/// Compare two positions in the profile's quality ordering.
///
/// Lower index is better. A quality the profile does not list (`None`) ranks
/// below every listed one — it is only here at all because some other gate let
/// it through.
fn tier_cmp(left: Option<usize>, right: Option<usize>) -> std::cmp::Ordering {
    match (left, right) {
        (Some(l), Some(r)) => r.cmp(&l),
        (Some(_), None) => std::cmp::Ordering::Greater,
        (None, Some(_)) => std::cmp::Ordering::Less,
        (None, None) => std::cmp::Ordering::Equal,
    }
}

/// [`tier_cmp`] as an ascending sort key: best tier first, unlisted qualities
/// last.
///
/// Anywhere that has to *order* files or releases by tier rather than compare
/// two of them — the search rank head, the scan's primary-role election —
/// reaches for this, so no sort can quietly disagree with the gate about which
/// of two qualities is better.
pub(crate) fn tier_sort_key(tier_index: Option<usize>) -> usize {
    tier_index.unwrap_or(usize::MAX)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AdmissionRejectionReason {
    /// The candidate sits in a lower quality tier than the incumbent. No score
    /// rescues this: a 720p file does not replace a 1080p one because it has a
    /// better release group.
    LowerQualityTier,
    /// An incumbent covers episodes the candidate does not, so replacing it
    /// would silently drop coverage.
    BroaderIncumbentSpan,
    /// Same tier, but the incumbent is the later revision — the candidate is a
    /// plain release facing a PROPER or a REPACK. Sonarr's `BetterRevision`.
    LowerRevision {
        incumbent_revision: i32,
        candidate_revision: i32,
    },
    /// The profile forbids upgrades and something is already there.
    UpgradesDisabled,
    /// A monitored member of this collection has not aired yet, so no pack can
    /// contain it. Sonarr's `FullSeasonSpecification`: fetching a "season pack"
    /// mid-season buys a partial season at pack bandwidth and then blocks the
    /// per-episode searches that would actually fill it.
    SeasonIncomplete,
    NotAnUpgrade {
        incumbent_score: i32,
        candidate_score: i32,
        required_delta: i32,
    },
    /// A release already in the download queue for this scope is equal or
    /// better, so fetching this one buys nothing. Sonarr's `QueueSpecification`.
    QueuedEqualOrBetter {
        queued_title: String,
        queued_score: i32,
        candidate_score: i32,
    },
    /// The candidate *is* the release already grabbed for this scope — same
    /// name, still in flight or still unresolved. Never fetched twice, whatever
    /// the scores say: a second copy of the same bytes is never an upgrade.
    QueuedSameRelease { queued_title: String },
    /// The incumbent has already reached the profile's `cutoff_score`, so no
    /// same-tier candidate is worth the bandwidth however far ahead it scores.
    /// Sonarr's `CustomFormatCutoff`, kept distinct from `NotAnUpgrade` because
    /// reporting a `required_delta` the candidate could never have cleared reads
    /// as a threshold an operator can tune, and it is not one.
    FormatCutoffReached {
        incumbent_score: i32,
        cutoff_score: i32,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct AdmissionRejection {
    pub reason: AdmissionRejectionReason,
    pub message: String,
    pub incumbent_file_id: String,
    pub incumbent_file_path: String,
}

#[derive(Debug, Clone)]
pub(crate) enum AdmissionVerdict {
    Admit {
        /// Incumbent file ids the candidate displaces, best-first. Empty when
        /// the scope was unoccupied.
        ranked_superseded: Vec<String>,
        previous_best_score: i32,
    },
    Reject(AdmissionRejection),
}

impl AdmissionVerdict {
    pub(crate) fn is_admitted(&self) -> bool {
        matches!(self, Self::Admit { .. })
    }

    /// Incumbent file ids this candidate displaces, best-first. Empty on a
    /// rejection and on an unoccupied scope.
    ///
    /// **Assertions only.** Production destructures the verdict — `decide_import`
    /// needs `previous_best_score` off the same arm — so this exists to keep the
    /// tests reading like the contract rather than like a match.
    #[cfg(test)]
    pub(crate) fn superseded(&self) -> &[String] {
        match self {
            Self::Admit {
                ranked_superseded, ..
            } => ranked_superseded,
            Self::Reject(_) => &[],
        }
    }

    pub(crate) fn rejection(&self) -> Option<&AdmissionRejection> {
        match self {
            Self::Admit { .. } => None,
            Self::Reject(rejection) => Some(rejection),
        }
    }
}

/// The target scope together with the primary files currently occupying it.
///
/// Holding both means callers cannot pair a scope with the wrong incumbent list,
/// and the primary-only filter happens once here instead of in each gate.
#[derive(Debug, Clone)]
pub(crate) struct AdmissionSubject {
    /// Whether this scope is filled by one file per member rather than a single
    /// file spanning all of them.
    ///
    /// This is the difference between grabbing a season pack and importing one
    /// file that covers a season. A pack arrives as one file per episode, each
    /// gated on its own at import, so one improvable episode is reason enough to
    /// fetch it — Sonarr's `SeasonPackUpgrade::Any`. A single file replacing a
    /// whole span must beat everything it displaces, or the episodes it does not
    /// improve are silently downgraded.
    per_member: bool,
    /// Monitored members of this scope whose air date is still far enough in the
    /// future that no release can contain them. Only meaningful for a
    /// per-member (season pack) subject; see [`AdmissionSubject::with_unaired_members`].
    unaired_members: usize,
    scope: AdmissionScope,
    incumbents: Vec<Incumbent>,
    /// In-flight submissions covering this scope; see [`QueuedRelease`]. Only
    /// the grab policy consults them.
    queued: Vec<QueuedRelease>,
}

impl AdmissionSubject {
    /// Build a subject, keeping only files that are genuinely upgrade targets.
    ///
    /// `is_primary` is supplied by the caller because the two sides derive the
    /// role differently in the store — the point is that both end up filtering.
    pub(crate) fn new(
        scope: AdmissionScope,
        incumbents: impl IntoIterator<Item = (Incumbent, bool)>,
    ) -> Self {
        let mut incumbents: Vec<Incumbent> = incumbents
            .into_iter()
            .filter_map(|(incumbent, is_primary)| is_primary.then_some(incumbent))
            .collect();

        // Best-first: highest bar, then newest, then id — a total order, so the
        // verdict never depends on the order rows came back from the store.
        incumbents.sort_by(|left, right| {
            right
                .score
                .cmp(&left.score)
                .then_with(|| right.created_at.cmp(&left.created_at))
                .then_with(|| right.file_id.cmp(&left.file_id))
        });

        Self {
            per_member: false,
            unaired_members: 0,
            scope,
            incumbents,
            queued: Vec::new(),
        }
    }

    /// Attach the scope's in-flight submissions as pseudo-incumbents.
    pub(crate) fn with_queued(mut self, queued: Vec<QueuedRelease>) -> Self {
        self.queued = queued;
        self
    }

    /// Mark this scope as filled by one file per member — a season pack grab.
    /// See [`AdmissionSubject::per_member`].
    pub(crate) fn per_member(mut self) -> Self {
        self.per_member = true;
        self
    }

    /// How many monitored members have not aired yet.
    ///
    /// A pack cannot contain an episode that does not exist, so any positive
    /// count makes a pack grab a guaranteed partial fetch —
    /// [`AdmissionRejectionReason::SeasonIncomplete`]. Counted by the caller,
    /// which is where the clock and the catalog live.
    pub(crate) fn with_unaired_members(mut self, unaired_members: usize) -> Self {
        self.unaired_members = unaired_members;
        self
    }

    pub(crate) fn is_unoccupied(&self) -> bool {
        self.incumbents.is_empty()
    }

    pub(crate) fn incumbents(&self) -> &[Incumbent] {
        &self.incumbents
    }

    pub(crate) fn best_score(&self) -> Option<i32> {
        self.incumbents
            .iter()
            .map(|incumbent| incumbent.score)
            .max()
    }

    /// The tier and score of the best file in scope — the bar a candidate has to
    /// clear, in the order admission compares them: **tier first, then score**.
    /// `None` when nothing occupies the scope.
    ///
    /// Deliberately not [`AdmissionSubject::best_score`] plus that file's tier.
    /// `best_score` sorts on the number alone, so in a scope holding a 2160p file
    /// scoring 100 and a 1080p one scoring 900 it names the 1080p file — and a
    /// 1080p candidate compared against it would look like a tier upgrade. The
    /// bar is the best *file*, judged the way the gate judges files.
    pub(crate) fn best_incumbent(&self) -> Option<(Option<usize>, i32)> {
        self.best_incumbent_record()
            .map(|incumbent| (incumbent.tier_index, incumbent.score))
    }

    /// The whole record behind [`AdmissionSubject::best_incumbent`].
    ///
    /// The cutoff gate needs more than the pair: the incumbent's revision (to
    /// decide whether a candidate is a revision upgrade over it) and its
    /// `created_at` (for the old-file guard). Reading them off the same record
    /// the bar came from is what stops the two answers describing different
    /// files.
    pub(crate) fn best_incumbent_record(&self) -> Option<&Incumbent> {
        self.incumbents.iter().min_by(|left, right| {
            tier_cmp(right.tier_index, left.tier_index).then_with(|| right.score.cmp(&left.score))
        })
    }
}

/// How a candidate beats a queued release, when it does — the rung of the ladder
/// it won on. Only the revision rung is exempt from `allow_upgrades`.
enum QueueWin {
    Tier,
    Revision,
    Score,
}

/// Refuse when a release already in the queue for this scope is equal or better.
///
/// The same tier → revision → score ladder an incumbent gets, with two
/// deliberate omissions: no format cutoff (nothing has landed, so "this scope
/// is already good enough" is not the question being asked). The churn delta
/// *does* apply, matching Sonarr's `MinUpgradeFormatScore` inside
/// `QueueSpecification`: two near-identical releases downloading in parallel is
/// the waste this rule exists to prevent.
fn queued_candidate_win(
    candidate: CandidateFacts,
    queued: &QueuedRelease,
    policy: &AdmissionPolicy,
) -> Option<QueueWin> {
    match tier_cmp(candidate.tier_index, queued.tier_index) {
        std::cmp::Ordering::Greater => Some(QueueWin::Tier),
        std::cmp::Ordering::Less => None,
        std::cmp::Ordering::Equal => match candidate.revision.cmp(&queued.revision) {
            std::cmp::Ordering::Greater => Some(QueueWin::Revision),
            std::cmp::Ordering::Less => None,
            // A tie is never a win over a download already in flight: with a
            // zero churn threshold the old `>=` admitted an identical score,
            // and the scope fetched the same release again every pass.
            std::cmp::Ordering::Equal => (candidate.score.saturating_sub(queued.score)
                >= policy.required_delta().max(1))
            .then_some(QueueWin::Score),
        },
    }
}

fn queued_blocks_candidate(
    queued: &QueuedRelease,
    candidate: CandidateFacts,
    policy: &AdmissionPolicy,
) -> bool {
    policy.applies_to_queue
        && !policy.manual_override
        && queued_candidate_win(candidate, queued, policy).is_none()
}

fn queued_equal_or_better_rejection(
    queued: &QueuedRelease,
    candidate: CandidateFacts,
) -> AdmissionRejection {
    AdmissionRejection {
        reason: AdmissionRejectionReason::QueuedEqualOrBetter {
            queued_title: queued.title.clone(),
            queued_score: queued.score,
            candidate_score: candidate.score,
        },
        message: format!(
            "{} is already downloading for this scope and is equal or better (score {} >= {})",
            queued.title, queued.score, candidate.score
        ),
        incumbent_file_id: String::new(),
        incumbent_file_path: String::new(),
    }
}

fn queued_rejection(
    subject: &AdmissionSubject,
    candidate: CandidateFacts,
    policy: &AdmissionPolicy,
) -> Option<AdmissionRejection> {
    if !policy.applies_to_queue || policy.manual_override {
        return None;
    }
    subject.queued.iter().find_map(|queued| {
        // The very release that is already grabbed is refused before any
        // comparison: no score, size or profile change makes a second copy
        // of the same download worth fetching. This is the guard that stops
        // a scope re-grabbing one release every sync while the first copy
        // is still working its way through the client.
        if candidate.release_key.is_some() && candidate.release_key == queued.release_key {
            return Some(AdmissionRejection {
                reason: AdmissionRejectionReason::QueuedSameRelease {
                    queued_title: queued.title.clone(),
                },
                message: format!(
                    "{} is already grabbed for this scope; the same release is not fetched twice",
                    queued.title
                ),
                incumbent_file_id: String::new(),
                incumbent_file_path: String::new(),
            });
        }
        // The same tier → revision → score ladder an incumbent gets.
        let win = queued_candidate_win(candidate, queued, policy);
        match win {
            // A revision upgrade admits regardless of `allow_upgrades`, exactly
            // as it does over a file on disk.
            Some(QueueWin::Revision) => None,
            // Sonarr's `QueueUpgradesNotAllowed`: the candidate would only be
            // an *upgrade* over what is already downloading, and the profile
            // forbids upgrades — so it is not fetched beside it either.
            Some(QueueWin::Tier | QueueWin::Score) if !policy.allow_upgrades => {
                Some(AdmissionRejection {
                    reason: AdmissionRejectionReason::UpgradesDisabled,
                    message: format!(
                        "{} is already downloading for this scope and the quality profile disallows upgrades",
                        queued.title
                    ),
                    incumbent_file_id: String::new(),
                    incumbent_file_path: String::new(),
                })
            }
            Some(QueueWin::Tier | QueueWin::Score) => None,
            None => Some(queued_equal_or_better_rejection(queued, candidate)),
        }
    })
}

/// "The profile forbids upgrades and this file is in the way." One
/// construction because three branches of the ladder reach it and their wording
/// must not drift.
fn upgrades_disabled(incumbent: &Incumbent) -> AdmissionRejection {
    AdmissionRejection {
        reason: AdmissionRejectionReason::UpgradesDisabled,
        message: format!(
            "existing file {} cannot be replaced because the quality profile disallows upgrades",
            incumbent.file_path
        ),
        incumbent_file_id: incumbent.file_id.clone(),
        incumbent_file_path: incumbent.file_path.clone(),
    }
}

/// Admit when at least one member of the scope would be improved.
///
/// An unoccupied member counts as an improvement — it is missing, and fetching
/// it is the whole point. A member held by a file the candidate cannot beat is
/// simply not a reason to fetch; it is not a reason to refuse either, because
/// the members that *would* improve still justify the download and each member
/// is gated again on its own at import.
///
/// The one refusal that overrides all of that is [`AdmissionRejectionReason::SeasonIncomplete`]:
/// a pack cannot contain an episode that has not aired, so a pack fetched while
/// the season is still airing is guaranteed to be partial. Sonarr's
/// `FullSeasonSpecification`.
fn evaluate_any_member(
    subject: &AdmissionSubject,
    candidate: CandidateFacts,
    policy: &AdmissionPolicy,
) -> AdmissionVerdict {
    let candidate_score = candidate.score;
    let occupied: usize = subject.incumbents.len();

    // A pack scope with no members is a season nobody monitors. There is
    // nothing to fill and nothing to improve, so falling through to the
    // unoccupied "admit" below would fetch a whole season on the strength of
    // having no reason not to. Refused, with its own message so the decision log
    // does not read as an airing-season refusal.
    if matches!(&subject.scope, AdmissionScope::Episodes(ids) if ids.is_empty()) {
        return AdmissionVerdict::Reject(AdmissionRejection {
            reason: AdmissionRejectionReason::SeasonIncomplete,
            message: "no monitored episodes in this season, so there is nothing a pack could fill"
                .to_string(),
            incumbent_file_id: String::new(),
            incumbent_file_path: String::new(),
        });
    }

    if subject.unaired_members > 0 {
        let blocker = subject.incumbents.first();
        return AdmissionVerdict::Reject(AdmissionRejection {
            reason: AdmissionRejectionReason::SeasonIncomplete,
            message: format!(
                "{} monitored episode(s) in this season have not aired yet, so no pack can be complete",
                subject.unaired_members
            ),
            incumbent_file_id: blocker.map(|i| i.file_id.clone()).unwrap_or_default(),
            incumbent_file_path: blocker.map(|i| i.file_path.clone()).unwrap_or_default(),
        });
    }

    // An operator asked for this pack by hand, so no score is consulted (m11) —
    // the same exemption `evaluate_admission` gives a manual replacement. The
    // structural refusals above stay binding: `SeasonIncomplete` is not a
    // preference, and a pack that cannot contain half its episodes is still a
    // partial fetch however it was requested.
    if policy.manual_override {
        return AdmissionVerdict::Admit {
            ranked_superseded: subject
                .incumbents
                .iter()
                .map(|incumbent| incumbent.file_id.clone())
                .collect(),
            previous_best_score: subject.best_score().unwrap_or(0),
        };
    }

    let (members, incumbent_covered_members, queued_covered_members) = match &subject.scope {
        AdmissionScope::Episodes(ids) => {
            let members: HashSet<&str> = ids.iter().map(String::as_str).collect();
            let incumbent_covered_members: HashSet<&str> = subject
                .incumbents
                .iter()
                .flat_map(|incumbent| incumbent.covers.iter())
                .map(String::as_str)
                .filter(|episode_id| members.contains(episode_id))
                .collect();
            let queued_covered_members: HashSet<&str> = subject
                .queued
                .iter()
                .filter(|queued| queued_blocks_candidate(queued, candidate, policy))
                .flat_map(|queued| queued.covers.iter())
                .map(String::as_str)
                .filter(|episode_id| members.contains(episode_id))
                .collect();
            (ids.len(), incumbent_covered_members, queued_covered_members)
        }
        AdmissionScope::Title | AdmissionScope::SeriesMovieLink(_) => {
            (occupied, HashSet::new(), HashSet::new())
        }
    };

    let improvable: Vec<String> = subject
        .incumbents
        .iter()
        .filter(|incumbent| {
            let covered_members: Vec<&str> = incumbent
                .covers
                .iter()
                .map(String::as_str)
                .filter(|episode_id| incumbent_covered_members.contains(episode_id))
                .collect();
            if !covered_members.is_empty()
                && covered_members
                    .iter()
                    .all(|episode_id| queued_covered_members.contains(episode_id))
            {
                return false;
            }
            // Same ladder as `evaluate_admission`, per member: tier, then
            // revision, then score. A higher revision improves the member
            // whatever the profile says about upgrades — see the long note at
            // the single-file ladder.
            match tier_cmp(candidate.tier_index, incumbent.tier_index) {
                std::cmp::Ordering::Less => return false,
                std::cmp::Ordering::Greater => return policy.allow_upgrades,
                std::cmp::Ordering::Equal => {}
            }
            match candidate.revision.cmp(&incumbent.revision) {
                std::cmp::Ordering::Greater => return true,
                std::cmp::Ordering::Less => return false,
                std::cmp::Ordering::Equal => {}
            }
            if !policy.allow_upgrades {
                return false;
            }
            let delta = candidate_score.saturating_sub(incumbent.score);
            !policy.incumbent_at_format_cutoff(incumbent.score) && delta >= policy.required_delta()
        })
        .map(|incumbent| incumbent.file_id.clone())
        .collect();

    let covered_members = incumbent_covered_members
        .union(&queued_covered_members)
        .count();
    let has_missing_member = members > covered_members;

    if has_missing_member || !improvable.is_empty() {
        return AdmissionVerdict::Admit {
            ranked_superseded: improvable,
            previous_best_score: subject.best_score().unwrap_or(0),
        };
    }

    if members > 0
        && queued_covered_members.len() == members
        && let Some(queued) = subject
            .queued
            .iter()
            .find(|queued| queued_blocks_candidate(queued, candidate, policy))
    {
        return AdmissionVerdict::Reject(queued_equal_or_better_rejection(queued, candidate));
    }

    // No missing member, nothing improvable — and, when the member set is empty
    // (every episode unmonitored), nothing to refuse *for* either. An empty
    // subject is unoccupied, not a panic: the monitored/unaired filtering above
    // this function can legitimately produce one.
    let Some(blocker) = subject
        .incumbents
        .iter()
        .max_by_key(|incumbent| incumbent.score)
    else {
        return AdmissionVerdict::Admit {
            ranked_superseded: Vec::new(),
            previous_best_score: 0,
        };
    };

    // Tier first here too. A pack every member of which is held by a *better
    // quality* file was refused as `NotAnUpgrade` with a `required_delta` the
    // candidate could never have cleared — a threshold an operator might go and
    // tune, when the honest answer is that no score crosses a tier.
    let lower_tier_than_every_member = subject.incumbents.iter().all(|incumbent| {
        tier_cmp(candidate.tier_index, incumbent.tier_index) == std::cmp::Ordering::Less
    });

    let (reason, message) = if lower_tier_than_every_member {
        (
            AdmissionRejectionReason::LowerQualityTier,
            "every episode in this pack is already held by a better quality file".to_string(),
        )
    } else if !policy.allow_upgrades {
        (
            AdmissionRejectionReason::UpgradesDisabled,
            "this pack cannot replace anything because the quality profile disallows upgrades"
                .to_string(),
        )
    } else if let Some(cutoff_score) = policy
        .cutoff_score
        .filter(|cutoff| blocker.score >= *cutoff)
    {
        (
            AdmissionRejectionReason::FormatCutoffReached {
                incumbent_score: blocker.score,
                cutoff_score,
            },
            format!(
                "every episode in this pack has reached the profile's score cutoff ({} >= {cutoff_score})",
                blocker.score
            ),
        )
    } else {
        (
            AdmissionRejectionReason::NotAnUpgrade {
                incumbent_score: blocker.score,
                candidate_score,
                required_delta: policy.required_delta(),
            },
            format!(
                "every episode in this pack is already held by an equal or better file (best score {} >= {candidate_score})",
                blocker.score
            ),
        )
    };

    AdmissionVerdict::Reject(AdmissionRejection {
        reason,
        message,
        incumbent_file_id: blocker.file_id.clone(),
        incumbent_file_path: blocker.file_path.clone(),
    })
}

/// Decide whether a candidate may take the scope.
///
/// `candidate.score` is the canonical **release score** — announced evidence
/// only — on both sides. An incumbent's `score` is its landed total. That
/// asymmetry is intended: a candidate that merely *claims* to match a file which
/// measured well must clear what that file actually turned out to be.
pub(crate) fn evaluate_admission(
    subject: &AdmissionSubject,
    candidate: CandidateFacts,
    policy: &AdmissionPolicy,
) -> AdmissionVerdict {
    // Before anything about the library: a scope with a better release already
    // downloading has nothing to gain from this one, whatever is on disk.
    if !subject.per_member
        && let Some(rejection) = queued_rejection(subject, candidate, policy)
    {
        return AdmissionVerdict::Reject(rejection);
    }

    let candidate_score = candidate.score;
    let target_span: HashSet<&str> = match &subject.scope {
        AdmissionScope::Episodes(ids) => ids.iter().map(String::as_str).collect(),
        AdmissionScope::Title | AdmissionScope::SeriesMovieLink(_) => HashSet::new(),
    };
    let span_scoped = matches!(subject.scope, AdmissionScope::Episodes(_));

    if subject.per_member {
        return evaluate_any_member(subject, candidate, policy);
    }

    for incumbent in &subject.incumbents {
        // Structural guard first, and it binds even a manual replacement:
        // dropping coverage is data loss, not a preference.
        if span_scoped {
            let covered: HashSet<&str> = incumbent.covers.iter().map(String::as_str).collect();
            if !covered.is_subset(&target_span) {
                return AdmissionVerdict::Reject(AdmissionRejection {
                    reason: AdmissionRejectionReason::BroaderIncumbentSpan,
                    message: format!(
                        "existing file {} spans a broader episode set and cannot be replaced by this import",
                        incumbent.file_path
                    ),
                    incumbent_file_id: incumbent.file_id.clone(),
                    incumbent_file_path: incumbent.file_path.clone(),
                });
            }
        }

        // An operator asked for this by hand. No score is consulted.
        if policy.manual_override {
            continue;
        }

        // Tier before score, always. A candidate in a lower tier than what it
        // would replace is refused outright — this is Sonarr's
        // `qualityCompare < 0 => BetterQuality`, and it is what stops a
        // custom-format bonus or a size term buying its way across a resolution
        // step. Only when the tiers match does the score get a say.
        match tier_cmp(candidate.tier_index, incumbent.tier_index) {
            std::cmp::Ordering::Less => {
                return AdmissionVerdict::Reject(AdmissionRejection {
                    reason: AdmissionRejectionReason::LowerQualityTier,
                    message: format!(
                        "existing file {} is a better quality than this candidate",
                        incumbent.file_path
                    ),
                    incumbent_file_id: incumbent.file_id.clone(),
                    incumbent_file_path: incumbent.file_path.clone(),
                });
            }
            // A whole tier better clears the score comparison outright; the
            // churn thresholds exist to stop trimmings, not genuine upgrades.
            std::cmp::Ordering::Greater => {
                if !policy.allow_upgrades {
                    return AdmissionVerdict::Reject(upgrades_disabled(incumbent));
                }
                continue;
            }
            std::cmp::Ordering::Equal => {}
        }

        // Revision, between tier and score. A PROPER or a REPACK of the
        // quality already on disk is a re-release of the *same* thing, so it is
        // not a matter of degree: it wins outright and the score never gets a
        // say — a re-encode fixing a sync fault can easily score a few points
        // worse than the broken original, and a churn threshold would refuse
        // exactly the fix the release exists to deliver.
        //
        // It admits **regardless of `allow_upgrades`**, which is Sonarr's order
        // (`UpgradableSpecification` returns on the revision before it consults
        // `UpgradeAllowed`). "Do not upgrade" means "do not chase better
        // quality"; it does not mean "keep the broken copy". A dedicated
        // `DownloadPropersAndRepacks` setting is the follow-on if an operator
        // ever wants to opt out.
        //
        // Coarser than Sonarr's comparison: tiers are resolution-only until
        // Part 5, so "same tier" here is "same resolution" rather than Sonarr's
        // exact `Quality`. A WEB-DL PROPER therefore counts as a revision of a
        // Bluray release of the same resolution. That is the same coarseness the
        // rest of the ladder already has.
        match candidate.revision.cmp(&incumbent.revision) {
            std::cmp::Ordering::Greater => continue,
            std::cmp::Ordering::Less => {
                if !policy.allow_upgrades {
                    return AdmissionVerdict::Reject(upgrades_disabled(incumbent));
                }
                return AdmissionVerdict::Reject(AdmissionRejection {
                    reason: AdmissionRejectionReason::LowerRevision {
                        incumbent_revision: incumbent.revision,
                        candidate_revision: candidate.revision,
                    },
                    message: format!(
                        "existing file {} is a later revision (PROPER/REPACK {} > {})",
                        incumbent.file_path, incumbent.revision, candidate.revision
                    ),
                    incumbent_file_id: incumbent.file_id.clone(),
                    incumbent_file_path: incumbent.file_path.clone(),
                });
            }
            std::cmp::Ordering::Equal => {}
        }

        if !policy.allow_upgrades {
            return AdmissionVerdict::Reject(upgrades_disabled(incumbent));
        }

        // An incumbent already past the profile's format cutoff is only worth
        // displacing by a whole tier, not by trimmings — and a whole tier
        // admitted above, without reaching this line. Reported on its own
        // because a `NotAnUpgrade` naming a `required_delta` the candidate could
        // never have cleared reads as a tunable threshold, and it is not one.
        if let Some(cutoff_score) = policy
            .cutoff_score
            .filter(|cutoff| incumbent.score >= *cutoff)
        {
            return AdmissionVerdict::Reject(AdmissionRejection {
                reason: AdmissionRejectionReason::FormatCutoffReached {
                    incumbent_score: incumbent.score,
                    cutoff_score,
                },
                message: format!(
                    "existing file {} has reached the profile's score cutoff ({} >= {})",
                    incumbent.file_path, incumbent.score, cutoff_score
                ),
                incumbent_file_id: incumbent.file_id.clone(),
                incumbent_file_path: incumbent.file_path.clone(),
            });
        }

        let delta = candidate_score.saturating_sub(incumbent.score);
        let required_delta = policy.required_delta();
        if delta < required_delta {
            return AdmissionVerdict::Reject(AdmissionRejection {
                reason: AdmissionRejectionReason::NotAnUpgrade {
                    incumbent_score: incumbent.score,
                    candidate_score,
                    required_delta,
                },
                message: format!(
                    "existing file {} is equal or better (score {} >= {})",
                    incumbent.file_path, incumbent.score, candidate_score
                ),
                incumbent_file_id: incumbent.file_id.clone(),
                incumbent_file_path: incumbent.file_path.clone(),
            });
        }
    }

    AdmissionVerdict::Admit {
        ranked_superseded: subject
            .incumbents
            .iter()
            .map(|incumbent| incumbent.file_id.clone())
            .collect(),
        previous_best_score: subject.best_score().unwrap_or(0),
    }
}

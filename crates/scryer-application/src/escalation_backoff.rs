//! The escalating provider backoff shared by indexers and download clients.
//!
//! Sonarr keeps one `EscalationBackOff` ladder and one `ProviderStatusServiceBase`
//! and points both provider families at it. Scryer had the ladder inlined in the
//! indexer search client and nothing at all on the download-client side, so this
//! module is that ladder lifted out: the period table, the clamping, and the
//! `Retry-After` climb are defined once here and both families supply only their
//! own table and their own idea of what a level means.

use chrono::{DateTime, Duration, Utc};

/// An escalating ladder of disable periods indexed by escalation level.
///
/// The ladder owns only the arithmetic — how long level *n* disables a provider
/// and how a level climbs. When a level escalates is the caller's policy, because
/// the two families disagree about it: indexers escalate on every failed dispatch,
/// download clients only once the failure has persisted for
/// [`DOWNLOAD_CLIENT_ESCALATION_GRACE`].
#[derive(Clone, Copy, Debug)]
pub struct EscalationLadder {
    periods_secs: &'static [u64],
}

impl EscalationLadder {
    pub const fn new(periods_secs: &'static [u64]) -> Self {
        Self { periods_secs }
    }

    /// The highest level the ladder defines. Levels above it clamp to it.
    pub const fn max_level(&self) -> usize {
        self.periods_secs.len() - 1
    }

    /// How long level `level` disables a provider, clamped to the top rung.
    pub fn period(&self, level: usize) -> Duration {
        let secs = self.periods_secs[level.min(self.max_level())];
        Duration::seconds(secs as i64)
    }

    /// When a provider entering `level` at `now` may be used again.
    pub fn disabled_until(&self, level: usize, now: DateTime<Utc>) -> DateTime<Utc> {
        now + self.period(level)
    }

    /// Climb from `level` until the rung covers `at_least`, capped at the top.
    ///
    /// Sonarr's rule (`ProviderStatusServiceBase.RecordFailure`): a provider's
    /// `Retry-After` never becomes the backoff itself. It escalates the ladder
    /// until a rung covers the delay, and that rung is the backoff. Quantizing
    /// keeps every backoff on a ladder an operator can reason about, and no
    /// delay, however long or unrepresentable, is honored literally.
    pub fn level_covering(&self, level: usize, at_least: std::time::Duration) -> usize {
        let mut level = level.min(self.max_level());
        while level < self.max_level()
            && std::time::Duration::from_secs(self.periods_secs[level]) < at_least
        {
            level += 1;
        }
        level
    }
}

/// Short escalating indexer backoff periods. Provider `Retry-After` handling can
/// choose longer when explicitly supplied, but generic storm containment caps at
/// one hour to avoid stranding every indexer after one transient burst.
///
/// Indexer levels are one-based against this table: a provider at level `n` took
/// its last backoff from rung `n - 1`, and the next failure takes rung `n`.
pub const INDEXER_BACKOFF_LADDER: EscalationLadder = EscalationLadder::new(&[
    5 * 60,  // 5 minutes
    10 * 60, // 10 minutes
    15 * 60, // 15 minutes
    30 * 60, // 30 minutes
    60 * 60, // 1 hour
]);

/// Replaces the rungs of [`INDEXER_BACKOFF_LADDER`] with a comma-separated list
/// of seconds, ascending (for example `15,30,45,90,180`). The rungs are the
/// product's own operational backoff, so a test harness that has to watch an
/// indexer fail, be disabled, and then recover has no other way to do it than to
/// wait out five real minutes.
const INDEXER_BACKOFF_LADDER_ENV: &str = "SCRYER_INDEXER_BACKOFF_LADDER_SECS";

/// A floor, not a suggestion: a typo like `0` would turn the backoff off
/// entirely and let a failing indexer be retried in a hot loop.
const MINIMUM_INDEXER_BACKOFF_RUNG: u64 = 5;

/// The indexer ladder in force, the shipped table unless overridden.
///
/// Read once: the backoff tracker consults it on every failed dispatch, and a
/// ladder that could change under a running install would make an indexer's
/// disabled window unreproducible.
static INDEXER_BACKOFF_LADDER_IN_FORCE: std::sync::LazyLock<EscalationLadder> =
    std::sync::LazyLock::new(|| {
        parse_indexer_backoff_ladder(std::env::var(INDEXER_BACKOFF_LADDER_ENV).ok().as_deref())
            .unwrap_or(INDEXER_BACKOFF_LADDER)
    });

/// The configured indexer backoff ladder. Every indexer backoff decision must
/// go through this rather than [`INDEXER_BACKOFF_LADDER`] directly, or the
/// override would not take.
pub fn indexer_backoff_ladder() -> EscalationLadder {
    *INDEXER_BACKOFF_LADDER_IN_FORCE
}

/// Absent, blank, and malformed all fall back to the shipped ladder; a rung
/// shorter than [`MINIMUM_INDEXER_BACKOFF_RUNG`] is clamped up to it.
///
/// The rungs must ascend, because [`EscalationLadder::level_covering`] climbs
/// until a rung covers a `Retry-After` and stops at the first one that does: a
/// ladder that dips would quantize a delay onto a shorter window than the rung
/// below it. A list that does not ascend is a typo, so it is refused whole
/// rather than sorted into something the operator did not write.
fn parse_indexer_backoff_ladder(raw: Option<&str>) -> Option<EscalationLadder> {
    let raw = raw.map(str::trim).filter(|value| !value.is_empty())?;

    let mut periods_secs: Vec<u64> = Vec::new();
    for field in raw.split(',') {
        let Ok(seconds) = field.trim().parse::<u64>() else {
            tracing::warn!(
                env = INDEXER_BACKOFF_LADDER_ENV,
                value = raw,
                "indexer backoff ladder override is not a comma-separated list of seconds; \
                 keeping the shipped ladder"
            );
            return None;
        };
        periods_secs.push(seconds.max(MINIMUM_INDEXER_BACKOFF_RUNG));
    }

    if periods_secs.windows(2).any(|pair| pair[0] >= pair[1]) {
        tracing::warn!(
            env = INDEXER_BACKOFF_LADDER_ENV,
            value = raw,
            "indexer backoff ladder override does not ascend; keeping the shipped ladder"
        );
        return None;
    }

    tracing::info!(
        env = INDEXER_BACKOFF_LADDER_ENV,
        rungs_secs = ?periods_secs,
        "indexer backoff ladder overridden"
    );
    Some(EscalationLadder::new(Box::leak(
        periods_secs.into_boxed_slice(),
    )))
}

/// Sonarr's download-client ladder (`DownloadClientStatusService`), indexed by
/// the level the client is *on*: level 0 is healthy and disables nothing, so the
/// first failure disables for a minute and only a persistent outage climbs.
pub const DOWNLOAD_CLIENT_BACKOFF_LADDER: EscalationLadder = EscalationLadder::new(&[
    0,       // healthy
    60,      // 1 minute
    5 * 60,  // 5 minutes
    15 * 60, // 15 minutes
    30 * 60, // 30 minutes
    60 * 60, // 1 hour
]);

/// How long a download client must have been failing before a further failure
/// escalates it. Sonarr's `ProviderStatusServiceBase` uses the same 5 minutes:
/// a burst of failures inside one outage is one outage, not five.
pub const DOWNLOAD_CLIENT_ESCALATION_GRACE: Duration = Duration::minutes(5);

/// One download client's current failure run, or the healthy zero value.
///
/// Persisted in `download_client_status` (migration 0241); an absent row is this
/// type's [`Default`], which is why success clears the row rather than writing a
/// level-zero one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DownloadClientStatus {
    /// When the current failure run started. Cleared by success.
    pub initial_failure_at: Option<DateTime<Utc>>,
    /// The most recent failure in that run.
    pub most_recent_failure_at: Option<DateTime<Utc>>,
    /// Which rung of [`DOWNLOAD_CLIENT_BACKOFF_LADDER`] the client is on.
    pub escalation_level: usize,
    /// When the client may be used again. Past or absent means usable.
    pub disabled_until: Option<DateTime<Utc>>,
}

impl DownloadClientStatus {
    /// Whether the client is inside its backoff window and must be skipped.
    pub fn is_blocked(&self, now: DateTime<Utc>) -> bool {
        self.disabled_until.is_some_and(|until| until > now)
    }

    /// Apply one failure at `now` and return the status to persist.
    ///
    /// Pure: the caller decides when to call it and what to do with the result.
    /// The first failure of a run starts the clock and takes rung 1; further
    /// failures hold that rung until the run has lasted
    /// [`DOWNLOAD_CLIENT_ESCALATION_GRACE`], then climb one rung each, capped at
    /// the ladder's top.
    pub fn after_failure(&self, now: DateTime<Utc>) -> Self {
        let ladder = DOWNLOAD_CLIENT_BACKOFF_LADDER;
        let (initial_failure_at, escalation_level) = if self.escalation_level == 0 {
            (now, 1)
        } else {
            let initial_failure_at = self.initial_failure_at.unwrap_or(now);
            let escalated = initial_failure_at + DOWNLOAD_CLIENT_ESCALATION_GRACE < now;
            let level = if escalated {
                (self.escalation_level + 1).min(ladder.max_level())
            } else {
                self.escalation_level
            };
            (initial_failure_at, level)
        };

        Self {
            initial_failure_at: Some(initial_failure_at),
            most_recent_failure_at: Some(now),
            escalation_level,
            disabled_until: Some(ladder.disabled_until(escalation_level, now)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(minutes: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000 + minutes * 60, 0).expect("fixture instant is valid")
    }

    #[test]
    fn the_first_failure_disables_the_client_for_a_minute() {
        let status = DownloadClientStatus::default().after_failure(at(0));

        assert_eq!(status.escalation_level, 1);
        assert_eq!(status.initial_failure_at, Some(at(0)));
        assert_eq!(status.most_recent_failure_at, Some(at(0)));
        assert_eq!(status.disabled_until, Some(at(0) + Duration::seconds(60)));
        assert!(status.is_blocked(at(0)));
        assert!(!status.is_blocked(at(0) + Duration::seconds(61)));
    }

    #[test]
    fn failures_inside_the_grace_hold_the_level_and_only_extend_the_window() {
        let first = DownloadClientStatus::default().after_failure(at(0));
        let second = first.after_failure(at(2));

        assert_eq!(
            second.escalation_level, 1,
            "the outage is still the same one"
        );
        assert_eq!(second.initial_failure_at, Some(at(0)));
        assert_eq!(second.most_recent_failure_at, Some(at(2)));
        assert_eq!(second.disabled_until, Some(at(2) + Duration::seconds(60)));
    }

    #[test]
    fn a_failure_after_the_grace_climbs_one_rung() {
        let first = DownloadClientStatus::default().after_failure(at(0));
        let second = first.after_failure(at(6));

        assert_eq!(second.escalation_level, 2);
        assert_eq!(
            second.initial_failure_at,
            Some(at(0)),
            "the run is unbroken"
        );
        assert_eq!(second.disabled_until, Some(at(6) + Duration::minutes(5)));
    }

    #[test]
    fn the_ladder_climbs_one_rung_per_post_grace_failure_and_stops_at_an_hour() {
        let mut status = DownloadClientStatus::default().after_failure(at(0));
        let expected = [
            (2, Duration::minutes(5)),
            (3, Duration::minutes(15)),
            (4, Duration::minutes(30)),
            (5, Duration::minutes(60)),
            (5, Duration::minutes(60)),
            (5, Duration::minutes(60)),
        ];

        for (step, (level, period)) in expected.into_iter().enumerate() {
            let now = at(10 + step as i64 * 10);
            status = status.after_failure(now);
            assert_eq!(status.escalation_level, level, "step {step}");
            assert_eq!(status.disabled_until, Some(now + period), "step {step}");
        }

        assert_eq!(
            status.escalation_level,
            DOWNLOAD_CLIENT_BACKOFF_LADDER.max_level(),
            "5 is the last rung Sonarr defines"
        );
    }

    #[test]
    fn a_healthy_status_blocks_nothing() {
        let healthy = DownloadClientStatus::default();

        assert!(!healthy.is_blocked(at(0)));
        assert_eq!(healthy.escalation_level, 0);
    }

    #[test]
    fn a_lapsed_window_no_longer_blocks_but_keeps_the_level() {
        let status = DownloadClientStatus::default().after_failure(at(0));

        assert!(!status.is_blocked(at(5)));
        assert_eq!(
            status.escalation_level, 1,
            "the level survives so the next failure escalates instead of restarting"
        );
    }

    #[test]
    fn the_indexer_ladder_keeps_its_five_minute_floor_and_one_hour_cap() {
        assert_eq!(INDEXER_BACKOFF_LADDER.period(0), Duration::minutes(5));
        assert_eq!(INDEXER_BACKOFF_LADDER.period(4), Duration::minutes(60));
        assert_eq!(
            INDEXER_BACKOFF_LADDER.period(99),
            Duration::minutes(60),
            "levels above the table clamp to the top rung"
        );
    }

    #[test]
    fn an_unset_or_blank_override_leaves_the_shipped_ladder_alone() {
        assert!(parse_indexer_backoff_ladder(None).is_none());
        assert!(parse_indexer_backoff_ladder(Some("   ")).is_none());
        assert_eq!(
            std::env::var(INDEXER_BACKOFF_LADDER_ENV).ok(),
            None,
            "this test asserts the unset default; the suite must not set the override"
        );
        assert_eq!(
            indexer_backoff_ladder().period(0),
            INDEXER_BACKOFF_LADDER.period(0)
        );
    }

    #[test]
    fn a_valid_override_replaces_every_rung() {
        let ladder =
            parse_indexer_backoff_ladder(Some(" 15, 30 ,45,90,180 ")).expect("the list is valid");

        assert_eq!(ladder.max_level(), 4);
        assert_eq!(ladder.period(0), Duration::seconds(15));
        assert_eq!(ladder.period(3), Duration::seconds(90));
        assert_eq!(
            ladder.period(99),
            Duration::seconds(180),
            "levels above the override clamp to its top rung"
        );
    }

    #[test]
    fn a_malformed_override_falls_back_to_the_shipped_ladder() {
        for raw in ["15,thirty,45", "15;30;45", "15,-30", "15,30,"] {
            assert!(
                parse_indexer_backoff_ladder(Some(raw)).is_none(),
                "{raw:?} must be refused"
            );
        }
    }

    #[test]
    fn an_override_that_does_not_ascend_is_refused_whole() {
        assert!(parse_indexer_backoff_ladder(Some("30,15")).is_none());
        assert!(parse_indexer_backoff_ladder(Some("15,15")).is_none());
    }

    #[test]
    fn an_override_rung_below_the_floor_is_clamped_up_to_it() {
        let ladder = parse_indexer_backoff_ladder(Some("0,10")).expect("the list is valid");

        assert_eq!(
            ladder.period(0),
            Duration::seconds(MINIMUM_INDEXER_BACKOFF_RUNG as i64),
            "a zero rung would let a failing indexer be retried in a hot loop"
        );
        assert_eq!(ladder.period(1), Duration::seconds(10));
    }

    #[test]
    fn a_retry_after_rounds_up_onto_the_overridden_ladder() {
        let ladder =
            parse_indexer_backoff_ladder(Some("15,30,45,90,180")).expect("the list is valid");

        let covering = ladder.level_covering(0, std::time::Duration::from_secs(25));

        assert_eq!(covering, 1, "30 s is the first overridden rung over 25 s");
        assert_eq!(
            ladder.disabled_until(covering, at(0)),
            at(0) + Duration::seconds(30),
            "the overridden rung, not the raw header and not the shipped 10 minutes"
        );
    }

    #[test]
    fn a_retry_after_climbs_to_the_rung_that_covers_it() {
        let covering =
            INDEXER_BACKOFF_LADDER.level_covering(0, std::time::Duration::from_secs(12 * 60));

        assert_eq!(covering, 2, "15 minutes is the first rung over 12");
        assert_eq!(
            INDEXER_BACKOFF_LADDER
                .level_covering(0, std::time::Duration::from_secs(365 * 24 * 60 * 60)),
            INDEXER_BACKOFF_LADDER.max_level(),
            "an absurd delay is capped, never honored literally"
        );
        assert_eq!(
            INDEXER_BACKOFF_LADDER.level_covering(3, std::time::Duration::from_secs(60)),
            3,
            "a short delay never walks the level back down"
        );
    }
}

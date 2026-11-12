//! The hourly create allowances TorBox enforces on top of its per-minute
//! limit: 60 uncached torrents and 60 usenet downloads per hour per token.
//!
//! The tracker lives in memory only. It does no work of its own: a create
//! reserves its slot before it is sent and gives it back when it did not
//! count, and the tracker forgets sends older than the window as it is asked.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use scryer_application::{AppError, torbox_refusal};

/// Creates TorBox allows per lane in one window.
pub const TORBOX_HOURLY_CREATE_LIMIT: usize = 60;
/// The window TorBox counts creates over.
pub const TORBOX_CREATE_WINDOW: Duration = Duration::from_secs(60 * 60);

/// Which hourly allowance a create draws on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TorboxBudgetLane {
    /// `createtorrent` for a torrent TorBox did not already have cached.
    UncachedTorrent,
    /// `createusenetdownload`, cached or not.
    UsenetCreate,
}

impl TorboxBudgetLane {
    fn label(self) -> &'static str {
        match self {
            Self::UncachedTorrent => "uncached torrent",
            Self::UsenetCreate => "usenet download",
        }
    }
}

/// The time source the tracker reads, so tests can move time by hand.
pub trait TorboxClock: Send + Sync {
    fn now(&self) -> Instant;
}

/// The monotonic system clock.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemTorboxClock;

impl TorboxClock for SystemTorboxClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

#[derive(Default)]
struct Lanes {
    uncached_torrent: VecDeque<Instant>,
    usenet_create: VecDeque<Instant>,
}

impl Lanes {
    fn lane_mut(&mut self, lane: TorboxBudgetLane) -> &mut VecDeque<Instant> {
        match lane {
            TorboxBudgetLane::UncachedTorrent => &mut self.uncached_torrent,
            TorboxBudgetLane::UsenetCreate => &mut self.usenet_create,
        }
    }
}

/// Sliding-window counters for the two hourly create allowances of one token.
pub struct TorboxHourlyBudget {
    clock: Arc<dyn TorboxClock>,
    limit: usize,
    window: Duration,
    lanes: Mutex<Lanes>,
}

impl std::fmt::Debug for TorboxHourlyBudget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TorboxHourlyBudget")
            .field("limit", &self.limit)
            .field("window", &self.window)
            .finish_non_exhaustive()
    }
}

impl Default for TorboxHourlyBudget {
    fn default() -> Self {
        Self::new(Arc::new(SystemTorboxClock))
    }
}

impl TorboxHourlyBudget {
    /// TorBox's documented allowance, read against `clock`.
    pub fn new(clock: Arc<dyn TorboxClock>) -> Self {
        Self::with_limit(clock, TORBOX_HOURLY_CREATE_LIMIT, TORBOX_CREATE_WINDOW)
    }

    pub fn with_limit(clock: Arc<dyn TorboxClock>, limit: usize, window: Duration) -> Self {
        Self {
            clock,
            limit: limit.max(1),
            window,
            lanes: Mutex::new(Lanes::default()),
        }
    }

    /// `None` when a create on `lane` may be sent now, otherwise how long
    /// until the oldest counted create leaves the window.
    #[cfg(test)]
    pub fn wait_before_send(&self, lane: TorboxBudgetLane) -> Option<Duration> {
        let now = self.clock.now();
        let mut lanes = self.lanes.lock().unwrap_or_else(|p| p.into_inner());
        let sends = lanes.lane_mut(lane);
        prune(sends, now, self.window);
        if sends.len() < self.limit {
            return None;
        }
        let oldest = *sends.front()?;
        Some((oldest + self.window).saturating_duration_since(now))
    }

    /// Take one create on `lane` now, or refuse with how long until one is
    /// free. The check and the count happen under one lock, so two creates
    /// racing for the last slot cannot both take it.
    pub fn reserve(&self, lane: TorboxBudgetLane) -> Result<TorboxReservation, AppError> {
        let now = self.clock.now();
        let mut lanes = self.lanes.lock().unwrap_or_else(|p| p.into_inner());
        let sends = lanes.lane_mut(lane);
        prune(sends, now, self.window);
        if sends.len() < self.limit {
            sends.push_back(now);
            return Ok(TorboxReservation { lane, at: now });
        }
        let wait = sends
            .front()
            .map(|oldest| (*oldest + self.window).saturating_duration_since(now))
            .unwrap_or_default();
        Err(AppError::temporary_unavailable_reason(
            torbox_refusal::HOURLY_LIMIT,
            format!(
                "TorBox allows {} {} creates an hour; the next may be sent in {}s",
                self.limit,
                lane.label(),
                wait.as_secs().max(1)
            ),
            Some(wait),
        ))
    }

    /// Give back a reservation for a create that did not count: one TorBox
    /// refused before accepting anything, or a torrent it already had cached.
    pub fn release(&self, reservation: TorboxReservation) {
        let mut lanes = self.lanes.lock().unwrap_or_else(|p| p.into_inner());
        let sends = lanes.lane_mut(reservation.lane);
        if let Some(index) = sends.iter().position(|at| *at == reservation.at) {
            sends.remove(index);
        }
    }
}

/// One create taken from a lane, to be given back if it did not count.
#[derive(Debug)]
#[must_use = "a reservation that did not count is released"]
pub struct TorboxReservation {
    lane: TorboxBudgetLane,
    at: Instant,
}

fn prune(sends: &mut VecDeque<Instant>, now: Instant, window: Duration) {
    while let Some(oldest) = sends.front() {
        if now.saturating_duration_since(*oldest) >= window {
            sends.pop_front();
        } else {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ManualClock {
        base: Instant,
        offset: Mutex<Duration>,
    }

    impl ManualClock {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                base: Instant::now(),
                offset: Mutex::new(Duration::ZERO),
            })
        }

        fn advance(&self, by: Duration) {
            *self.offset.lock().unwrap() += by;
        }
    }

    impl TorboxClock for ManualClock {
        fn now(&self) -> Instant {
            self.base + *self.offset.lock().unwrap()
        }
    }

    #[test]
    fn sends_are_allowed_until_the_window_is_full() {
        let clock = ManualClock::new();
        let budget = TorboxHourlyBudget::new(clock.clone());
        for _ in 0..TORBOX_HOURLY_CREATE_LIMIT {
            assert_eq!(
                budget.wait_before_send(TorboxBudgetLane::UsenetCreate),
                None
            );
            let _counted = budget.reserve(TorboxBudgetLane::UsenetCreate).unwrap();
            clock.advance(Duration::from_secs(10));
        }
        // Sixty sends ten seconds apart: the first left 600s ago, so it leaves
        // the window in 3000s.
        assert_eq!(
            budget.wait_before_send(TorboxBudgetLane::UsenetCreate),
            Some(Duration::from_secs(3000))
        );
        clock.advance(Duration::from_secs(2999));
        assert_eq!(
            budget.wait_before_send(TorboxBudgetLane::UsenetCreate),
            Some(Duration::from_secs(1))
        );
        clock.advance(Duration::from_secs(1));
        assert_eq!(
            budget.wait_before_send(TorboxBudgetLane::UsenetCreate),
            None
        );
    }

    #[test]
    fn lanes_are_counted_apart() {
        let clock = ManualClock::new();
        let budget = TorboxHourlyBudget::with_limit(clock.clone(), 2, TORBOX_CREATE_WINDOW);
        let _first = budget.reserve(TorboxBudgetLane::UsenetCreate).unwrap();
        let _second = budget.reserve(TorboxBudgetLane::UsenetCreate).unwrap();
        assert!(
            budget
                .wait_before_send(TorboxBudgetLane::UsenetCreate)
                .is_some()
        );
        assert_eq!(
            budget.wait_before_send(TorboxBudgetLane::UncachedTorrent),
            None
        );
    }

    #[test]
    fn a_released_reservation_frees_its_slot_and_only_its_own() {
        let clock = ManualClock::new();
        let budget = TorboxHourlyBudget::with_limit(clock.clone(), 2, TORBOX_CREATE_WINDOW);
        let kept = budget.reserve(TorboxBudgetLane::UncachedTorrent).unwrap();
        clock.advance(Duration::from_secs(5));
        let refused = budget.reserve(TorboxBudgetLane::UncachedTorrent).unwrap();
        assert!(budget.reserve(TorboxBudgetLane::UncachedTorrent).is_err());
        budget.release(refused);
        assert_eq!(
            budget.wait_before_send(TorboxBudgetLane::UncachedTorrent),
            None
        );
        let _again = budget.reserve(TorboxBudgetLane::UncachedTorrent).unwrap();
        // The kept reservation still counts: it is the oldest send.
        assert_eq!(
            budget.wait_before_send(TorboxBudgetLane::UncachedTorrent),
            Some(TORBOX_CREATE_WINDOW - Duration::from_secs(5))
        );
        drop(kept);
    }

    #[test]
    fn concurrent_reservations_never_exceed_the_limit() {
        let budget = Arc::new(TorboxHourlyBudget::with_limit(
            Arc::new(SystemTorboxClock),
            3,
            TORBOX_CREATE_WINDOW,
        ));
        let threads: Vec<_> = (0..16)
            .map(|_| {
                let budget = Arc::clone(&budget);
                std::thread::spawn(move || budget.reserve(TorboxBudgetLane::UsenetCreate).is_ok())
            })
            .collect();
        let granted = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .filter(|granted| *granted)
            .count();
        assert_eq!(granted, 3);
    }

    #[test]
    fn an_exhausted_lane_refuses_with_the_hourly_reason_and_delay() {
        let clock = ManualClock::new();
        let budget = TorboxHourlyBudget::with_limit(clock.clone(), 1, TORBOX_CREATE_WINDOW);
        let _counted = budget.reserve(TorboxBudgetLane::UsenetCreate).unwrap();
        clock.advance(Duration::from_secs(600));
        let error = budget
            .reserve(TorboxBudgetLane::UsenetCreate)
            .expect_err("lane is full");
        assert_eq!(error.reason(), Some(torbox_refusal::HOURLY_LIMIT));
        match error {
            AppError::TemporaryUnavailable { retry_after, .. } => {
                assert_eq!(retry_after, Some(Duration::from_secs(3000)));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }
}

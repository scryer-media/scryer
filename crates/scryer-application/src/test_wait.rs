//! Condition waits for tests.
//!
//! A test waits for the state its assertion depends on, never for elapsed
//! time. The deadline here is only a hang guard: generous enough that a
//! correct run on a slow, loaded runner never reaches it, so a missed
//! condition fails the test with a clear message instead of hanging it.

use std::future::Future;
use std::time::Duration;

/// Hang guard for every condition wait in this crate's tests.
pub(crate) const TEST_WAIT_DEADLINE: Duration = Duration::from_secs(30);

/// How often a polled condition is re-checked. Only affects latency.
const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Polls `probe` until it yields a value, panicking with `what` if the hang
/// guard expires first.
pub(crate) async fn wait_for<T, F, Fut>(what: &str, mut probe: F) -> T
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Option<T>>,
{
    let deadline = tokio::time::Instant::now() + TEST_WAIT_DEADLINE;
    loop {
        if let Some(value) = probe().await {
            return value;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out after {TEST_WAIT_DEADLINE:?} waiting for {what}"
        );
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

/// Polls `predicate` until it holds, panicking with `what` if the hang guard
/// expires first.
pub(crate) async fn wait_until<F, Fut>(what: &str, mut predicate: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    wait_for(what, || {
        let check = predicate();
        async move { check.await.then_some(()) }
    })
    .await
}

/// Awaits `future` under the hang guard, panicking with `what` if it expires.
pub(crate) async fn within_deadline<F: Future>(what: &str, future: F) -> F::Output {
    tokio::time::timeout(TEST_WAIT_DEADLINE, future)
        .await
        .unwrap_or_else(|_| panic!("timed out after {TEST_WAIT_DEADLINE:?} waiting for {what}"))
}

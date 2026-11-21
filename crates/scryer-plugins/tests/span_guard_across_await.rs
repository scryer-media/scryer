//! A span guard held across an await poisons the worker thread that entered it.
//!
//! Gate 7 aborted one Scryer process with
//!
//! ```text
//! thread 'tokio-rt-worker' panicked at tracing-subscriber-0.3.23/src/registry/sharded.rs:317:9:
//! assertion `left != right` failed: tried to clone a span (Id(1924145348616)) that already closed
//! ```
//!
//! six seconds after the download-client component ran, on unrelated code. The
//! mechanism is the `Entered` guard the component hosts held across their
//! awaits:
//!
//! 1. `Span::enter()` pushes the raw `Id` onto the *entering* thread's
//!    thread-local `SpanStack` (`sharded.rs`, `Registry::enter`).
//! 2. The task awaits. The guard lives in the future's state, so nothing exits.
//! 3. Tokio's work-stealing resumes the task on another worker. The guard drops
//!    there, and `exit` pops a stack the id was never pushed onto —
//!    `SpanStack::pop` returns `false` *silently* (`registry/stack.rs`).
//!    The first thread keeps the id forever.
//! 4. The span closes; its refcount reaches zero, but the slab slot is not yet
//!    reused.
//! 5. Any later *contextual* span created on the first thread resolves its
//!    parent through `current_span()`, finds the stale id, and calls
//!    `clone_span` on a span whose refcount is already zero. That is the
//!    assertion above.
//!
//! In tracing 0.1 only `EnteredSpan` carries `PhantomNotSend`; the `Entered`
//! returned by `Span::enter()` is `Send`, so nothing stops the future from
//! crossing threads and nothing warns at compile time.
//!
//! These tests drive the sequence deterministically — two real threads, polled
//! by hand, no scheduler luck — and then hold the component hosts to the
//! construction that cannot exhibit it.

// disk-guard: test, example and build-script code may call the filesystem directly.
#![allow(clippy::disallowed_methods)]

use std::any::Any;
use std::future::Future;
use std::panic::{self, AssertUnwindSafe};
use std::pin::Pin;
use std::sync::mpsc::{self, Sender};
use std::task::{Context, Poll, Waker};
use std::thread;

use tracing::Instrument;
use tracing_subscriber::registry::Registry;

/// One OS thread that runs whatever closure it is handed, so a test can say
/// "this ran on the same thread as that" and mean it.
struct PinnedThread {
    work: Sender<Box<dyn FnOnce() + Send>>,
    handle: Option<thread::JoinHandle<()>>,
}

impl PinnedThread {
    fn new() -> Self {
        let (work, jobs) = mpsc::channel::<Box<dyn FnOnce() + Send>>();
        let handle = thread::spawn(move || {
            while let Ok(job) = jobs.recv() {
                job();
            }
        });
        Self {
            work,
            handle: Some(handle),
        }
    }

    /// Run `job` on this thread and wait for it.
    fn run<T: Send + 'static>(&self, job: impl FnOnce() -> T + Send + 'static) -> T {
        let (done, wait) = mpsc::channel();
        self.work
            .send(Box::new(move || {
                let _ = done.send(job());
            }))
            .expect("pinned thread accepts work");
        wait.recv().expect("pinned thread answers")
    }
}

impl Drop for PinnedThread {
    fn drop(&mut self) {
        let (work, _) = mpsc::channel();
        let closed = std::mem::replace(&mut self.work, work);
        drop(closed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// A future that is `Pending` exactly once, so a caller can choose which thread
/// polls it the first time and which thread polls it the second.
#[derive(Default)]
struct YieldOnce {
    yielded: bool,
}

impl Future for YieldOnce {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.yielded {
            return Poll::Ready(());
        }
        self.yielded = true;
        cx.waker().wake_by_ref();
        Poll::Pending
    }
}

type BoxedUnitFuture = Pin<Box<dyn Future<Output = ()> + Send>>;

fn poll_once(future: &mut BoxedUnitFuture) -> Poll<()> {
    let mut cx = Context::from_waker(Waker::noop());
    future.as_mut().poll(&mut cx)
}

/// The shape the hosts used: enter the span, then await while still entered.
fn guard_held_across_await() -> BoxedUnitFuture {
    Box::pin(async {
        let span = tracing::info_span!("component_invoke_guarded");
        let _enter = span.enter();
        YieldOnce::default().await;
    })
}

/// The sanctioned shape: the span instruments the future, so `Instrumented`
/// enters and exits inside each poll and can never straddle a thread hop.
fn span_instruments_the_future() -> BoxedUnitFuture {
    Box::pin(
        async {
            YieldOnce::default().await;
        }
        .instrument(tracing::info_span!("component_invoke_instrumented")),
    )
}

/// Poll `future` to completion across two threads, then ask the first thread to
/// open a new contextual span. Returns whatever that last step panicked with.
fn open_a_span_after_a_thread_hop(
    build: fn() -> BoxedUnitFuture,
) -> Result<(), Box<dyn Any + Send>> {
    let first = PinnedThread::new();
    let second = PinnedThread::new();

    let future = first.run(move || {
        let mut future = build();
        assert!(
            poll_once(&mut future).is_pending(),
            "the future must yield on its first poll so the two halves land on different threads",
        );
        future
    });

    second.run(move || {
        let mut future = future;
        assert!(
            poll_once(&mut future).is_ready(),
            "the future must finish on its second poll",
        );
        // Dropping it here drops the guard and the span on this thread, which
        // is the whole point: the exit and the close happen away from the
        // thread that entered.
        drop(future);
    });

    first.run(|| {
        panic::catch_unwind(AssertUnwindSafe(|| {
            let _later = tracing::info_span!("unrelated_work_on_the_same_worker");
        }))
    })
}

fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        return (*message).to_string();
    }
    if let Some(message) = payload.downcast_ref::<String>() {
        return message.clone();
    }
    "<non-string panic payload>".to_string()
}

/// Both halves of the mechanism in one process: the guard poisons the entering
/// thread, and instrumenting the future does not.
///
/// One test rather than two because the subscriber is global — installing it
/// twice would make the second case depend on which test ran first.
#[test]
fn a_span_guard_held_across_an_await_poisons_the_thread_that_entered_it() {
    tracing::subscriber::set_global_default(Registry::default())
        .expect("this test owns the global subscriber");

    let instrumented = open_a_span_after_a_thread_hop(span_instruments_the_future);
    assert!(
        instrumented.is_ok(),
        "instrumenting the future must leave the entering thread usable, but opening a \
         later span on it panicked with: {}",
        instrumented
            .as_ref()
            .err()
            .map(|payload| panic_message(payload.as_ref()))
            .unwrap_or_default(),
    );

    let guarded = open_a_span_after_a_thread_hop(guard_held_across_await);
    let payload =
        guarded.expect_err("holding the guard across the await must poison the entering thread");
    let message = panic_message(payload.as_ref());
    assert!(
        message.contains("tried to clone a span") && message.contains("already closed"),
        "the poisoned thread must fail the way gate 7 did, but it panicked with: {message}",
    );
}

/// The four component hosts must not reintroduce the guard.
///
/// This is the regression guard: the mechanism test above proves what the
/// pattern does, and this proves no host still uses it. `Entered` is `Send` in
/// tracing 0.1 and clippy dropped `await_holding_span_guard`, so nothing else
/// in the toolchain will catch a relapse.
#[test]
fn the_component_hosts_instrument_their_futures_instead_of_entering_a_guard() {
    const HOSTS: [&str; 4] = [
        "archive_component_host.rs",
        "download_client_component_host.rs",
        "notification_component_host.rs",
        "subtitle_component_host.rs",
    ];

    let host_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("wasmtime_host");

    let mut offenders = Vec::new();
    for host in HOSTS {
        let path = host_dir.join(host);
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("{} must be readable: {error}", path.display()));
        for (index, line) in source.lines().enumerate() {
            let line = line.trim();
            if line.contains(".enter()") && !line.starts_with("//") && !line.starts_with("///") {
                offenders.push(format!("{host}:{}: {line}", index + 1));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "a component host entered a span guard instead of instrumenting its future, which \
         poisons the polling thread when the task migrates:\n  {}",
        offenders.join("\n  "),
    );
}

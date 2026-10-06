//! Bounded retry for the first PostgreSQL connection at startup.
//!
//! A container started next to PostgreSQL can come up before the server
//! accepts connections. Startup keeps retrying errors that mean "not ready
//! yet" until a bounded window runs out, then fails so the service manager
//! can restart the process. Configuration errors (bad credentials, unknown
//! database, invalid options) fail on the first attempt.

use std::fmt;
use std::future::Future;
use std::io;
use std::time::Duration;

use scryer_application::AppError;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{ConnectOptions, Connection};
use tokio::time::Instant;

pub(super) const STARTUP_TIMEOUT_ENV: &str = "SCRYER_POSTGRES_STARTUP_TIMEOUT_SECS";
const DEFAULT_STARTUP_WINDOW: Duration = Duration::from_secs(120);
const MAX_STARTUP_WINDOW_SECS: u64 = 86_400;
const INITIAL_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(10);
const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(10);
const MIN_ATTEMPT_BUDGET: Duration = Duration::from_secs(1);

/// SQLSTATE `cannot_connect_now`: the server is starting up or shutting down.
const SQLSTATE_CANNOT_CONNECT_NOW: &str = "57P03";
/// SQLSTATE `too_many_connections`: every connection slot is taken right now.
const SQLSTATE_TOO_MANY_CONNECTIONS: &str = "53300";

const WINDOW_EXHAUSTED_PREFIX: &str = "cannot open PostgreSQL database after retrying for";

/// True when `error` reports that PostgreSQL never accepted a connection
/// within the startup window, as opposed to a configuration or migration
/// failure that a restart would not fix.
pub fn is_startup_window_exhausted(error: &AppError) -> bool {
    matches!(error, AppError::Repository(message) if message.starts_with(WINDOW_EXHAUSTED_PREFIX))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct StartupRetryPolicy {
    /// Total time to keep retrying. Zero means a single attempt.
    pub window: Duration,
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
    /// Upper bound on one attempt, so a stalled handshake cannot eat the window.
    pub attempt_timeout: Duration,
}

impl StartupRetryPolicy {
    pub(super) fn from_env() -> Self {
        Self::with_window(startup_window_from_env_value(
            std::env::var(STARTUP_TIMEOUT_ENV).ok().as_deref(),
        ))
    }

    fn with_window(window: Duration) -> Self {
        Self {
            window,
            initial_backoff: INITIAL_BACKOFF,
            max_backoff: MAX_BACKOFF,
            attempt_timeout: ATTEMPT_TIMEOUT,
        }
    }
}

fn startup_window_from_env_value(value: Option<&str>) -> Duration {
    let Some(raw) = value.map(str::trim).filter(|raw| !raw.is_empty()) else {
        return DEFAULT_STARTUP_WINDOW;
    };
    match raw.parse::<u64>() {
        Ok(seconds) => Duration::from_secs(seconds.min(MAX_STARTUP_WINDOW_SECS)),
        Err(_) => {
            tracing::warn!(
                env = STARTUP_TIMEOUT_ENV,
                value = raw,
                default_secs = DEFAULT_STARTUP_WINDOW.as_secs(),
                "ignoring invalid PostgreSQL startup timeout; using the default"
            );
            DEFAULT_STARTUP_WINDOW
        }
    }
}

/// Whether a connection error plausibly means the server is not ready yet.
pub(super) fn is_transient_startup_error(error: &sqlx::Error) -> bool {
    match error {
        // Refused, reset, unreachable, DNS lookup failure, missing unix
        // socket, and similar are all expected while the server boots.
        sqlx::Error::Io(io_error) => !matches!(
            io_error.kind(),
            io::ErrorKind::PermissionDenied
                | io::ErrorKind::InvalidInput
                | io::ErrorKind::Unsupported
        ),
        sqlx::Error::PoolTimedOut | sqlx::Error::Tls(_) => true,
        sqlx::Error::Database(database_error) => matches!(
            database_error.code().as_deref(),
            Some(SQLSTATE_CANNOT_CONNECT_NOW | SQLSTATE_TOO_MANY_CONNECTIONS)
        ),
        _ => false,
    }
}

#[derive(Debug)]
pub(super) struct RetryNotice<'a, E> {
    pub attempt: u32,
    pub elapsed: Duration,
    pub remaining: Duration,
    pub next_delay: Duration,
    pub error: &'a E,
}

#[derive(Debug)]
pub(super) struct RetrySuccess<T> {
    pub value: T,
    pub attempts: u32,
    pub elapsed: Duration,
}

#[derive(Debug)]
pub(super) struct RetryFailure<E> {
    pub error: E,
    pub attempts: u32,
    pub elapsed: Duration,
    /// The last error was retryable but the window ran out. False for a
    /// non-retryable error and for a zero window.
    pub window_exhausted: bool,
}

/// Runs `attempt` until it succeeds, returns a non-retryable error, or the
/// policy window runs out.
///
/// `attempt` receives the time budget for that try: `None` for a zero window
/// (one unbounded attempt, the historical behavior), otherwise the smaller
/// of the per-attempt timeout and the remaining window, floored at one second
/// so the last try is never starved. Total time is bounded by the window
/// plus that floor.
pub(super) async fn retry_until_deadline<T, E, F, Fut>(
    policy: StartupRetryPolicy,
    mut attempt: F,
    is_retryable: impl Fn(&E) -> bool,
    mut on_retry: impl FnMut(&RetryNotice<'_, E>),
) -> Result<RetrySuccess<T>, RetryFailure<E>>
where
    F: FnMut(Option<Duration>) -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    let started = Instant::now();
    let deadline = started + policy.window;
    let mut backoff = policy.initial_backoff;
    let mut attempts: u32 = 0;
    loop {
        attempts += 1;
        let budget = (!policy.window.is_zero()).then(|| {
            let remaining = deadline.saturating_duration_since(Instant::now());
            policy
                .attempt_timeout
                .min(remaining.max(MIN_ATTEMPT_BUDGET))
        });
        let error = match attempt(budget).await {
            Ok(value) => {
                return Ok(RetrySuccess {
                    value,
                    attempts,
                    elapsed: started.elapsed(),
                });
            }
            Err(error) => error,
        };
        let retryable = is_retryable(&error);
        let remaining = deadline.saturating_duration_since(Instant::now());
        if !retryable || remaining.is_zero() {
            return Err(RetryFailure {
                error,
                attempts,
                elapsed: started.elapsed(),
                window_exhausted: retryable && !policy.window.is_zero(),
            });
        }
        let delay = backoff.min(remaining);
        on_retry(&RetryNotice {
            attempt: attempts,
            elapsed: started.elapsed(),
            remaining,
            next_delay: delay,
            error: &error,
        });
        tokio::time::sleep(delay).await;
        backoff = backoff.saturating_mul(2).min(policy.max_backoff);
    }
}

#[derive(Debug)]
enum StartupConnectError {
    Sqlx(sqlx::Error),
    AttemptTimedOut(Duration),
}

impl fmt::Display for StartupConnectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlx(error) => error.fmt(f),
            Self::AttemptTimedOut(budget) => write!(
                f,
                "connection attempt did not complete within {}s",
                budget.as_secs()
            ),
        }
    }
}

fn is_transient_connect_error(error: &StartupConnectError) -> bool {
    match error {
        StartupConnectError::Sqlx(error) => is_transient_startup_error(error),
        StartupConnectError::AttemptTimedOut(_) => true,
    }
}

fn startup_failure_message(failure: &RetryFailure<StartupConnectError>) -> String {
    if failure.window_exhausted {
        format!(
            "{WINDOW_EXHAUSTED_PREFIX} {}s ({} attempts): {}",
            failure.elapsed.as_secs(),
            failure.attempts,
            failure.error
        )
    } else {
        format!("cannot open PostgreSQL database: {}", failure.error)
    }
}

/// Opens the pool, retrying "not ready yet" failures within `policy`.
pub(super) async fn connect_pool_with_startup_retry(
    connect_options: PgConnectOptions,
    max_connections: u32,
    policy: StartupRetryPolicy,
) -> Result<sqlx::PgPool, AppError> {
    let outcome = retry_until_deadline(
        policy,
        |budget| {
            let connect_options = connect_options.clone();
            async move {
                let pool_options = PgPoolOptions::new().max_connections(max_connections);
                let Some(budget) = budget else {
                    return pool_options
                        .connect_with(connect_options)
                        .await
                        .map_err(StartupConnectError::Sqlx);
                };
                // The pool silently retries a refused connection until its own
                // acquire timeout and then reports only that it timed out, so
                // probe with one plain connection first to surface the real
                // error and keep the backoff schedule ours.
                let attempt = async {
                    let probe = connect_options.connect().await?;
                    let _ = probe.close().await;
                    pool_options.connect_with(connect_options).await
                };
                match tokio::time::timeout(budget, attempt).await {
                    Ok(result) => result.map_err(StartupConnectError::Sqlx),
                    Err(_) => Err(StartupConnectError::AttemptTimedOut(budget)),
                }
            }
        },
        is_transient_connect_error,
        |notice| {
            tracing::warn!(
                attempt = notice.attempt,
                elapsed_secs = notice.elapsed.as_secs(),
                remaining_secs = notice.remaining.as_secs(),
                retry_in_ms = notice.next_delay.as_millis() as u64,
                error = %notice.error,
                "PostgreSQL is not accepting connections yet; retrying"
            );
        },
    )
    .await;

    match outcome {
        Ok(success) => {
            if success.attempts > 1 {
                tracing::info!(
                    attempts = success.attempts,
                    waited_ms = success.elapsed.as_millis() as u64,
                    "connected to PostgreSQL after waiting for it to accept connections"
                );
            }
            Ok(success.value)
        }
        Err(failure) => {
            if failure.window_exhausted {
                tracing::error!(
                    attempts = failure.attempts,
                    elapsed_secs = failure.elapsed.as_secs(),
                    window_secs = policy.window.as_secs(),
                    error = %failure.error,
                    "PostgreSQL did not accept connections within the startup window; set {} to wait longer",
                    STARTUP_TIMEOUT_ENV
                );
            }
            Err(AppError::Repository(startup_failure_message(&failure)))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;
    use std::cell::RefCell;
    use std::error::Error as StdError;

    use super::*;

    #[derive(Debug)]
    struct SyntheticPgError {
        code: &'static str,
    }

    impl fmt::Display for SyntheticPgError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "synthetic server error {}", self.code)
        }
    }

    impl StdError for SyntheticPgError {}

    impl sqlx::error::DatabaseError for SyntheticPgError {
        fn message(&self) -> &str {
            "synthetic server error"
        }

        fn code(&self) -> Option<Cow<'_, str>> {
            Some(Cow::Borrowed(self.code))
        }

        fn as_error(&self) -> &(dyn StdError + Send + Sync + 'static) {
            self
        }

        fn as_error_mut(&mut self) -> &mut (dyn StdError + Send + Sync + 'static) {
            self
        }

        fn into_error(self: Box<Self>) -> Box<dyn StdError + Send + Sync + 'static> {
            self
        }

        fn kind(&self) -> sqlx::error::ErrorKind {
            sqlx::error::ErrorKind::Other
        }
    }

    fn server_error(code: &'static str) -> sqlx::Error {
        sqlx::Error::Database(Box::new(SyntheticPgError { code }))
    }

    fn io_error(kind: io::ErrorKind) -> sqlx::Error {
        sqlx::Error::Io(io::Error::new(kind, "synthetic io failure"))
    }

    #[derive(Debug, PartialEq, Eq)]
    enum FakeError {
        Transient,
        Fatal,
    }

    fn fake_retryable(error: &FakeError) -> bool {
        matches!(error, FakeError::Transient)
    }

    fn policy(window_secs: u64) -> StartupRetryPolicy {
        StartupRetryPolicy::with_window(Duration::from_secs(window_secs))
    }

    #[test]
    fn startup_errors_that_mean_not_ready_are_retryable() {
        for kind in [
            io::ErrorKind::ConnectionRefused,
            io::ErrorKind::ConnectionReset,
            io::ErrorKind::ConnectionAborted,
            io::ErrorKind::HostUnreachable,
            io::ErrorKind::NetworkUnreachable,
            io::ErrorKind::TimedOut,
            io::ErrorKind::NotFound,
            io::ErrorKind::UnexpectedEof,
            io::ErrorKind::Other,
        ] {
            assert!(is_transient_startup_error(&io_error(kind)), "{kind:?}");
        }
        assert!(is_transient_startup_error(&sqlx::Error::PoolTimedOut));
        assert!(is_transient_startup_error(&sqlx::Error::Tls(
            "synthetic handshake aborted".into()
        )));
        assert!(is_transient_startup_error(&server_error("57P03")));
        assert!(is_transient_startup_error(&server_error("53300")));
    }

    #[test]
    fn startup_configuration_errors_are_not_retryable() {
        for kind in [
            io::ErrorKind::PermissionDenied,
            io::ErrorKind::InvalidInput,
            io::ErrorKind::Unsupported,
        ] {
            assert!(!is_transient_startup_error(&io_error(kind)), "{kind:?}");
        }
        for code in ["28P01", "28000", "3D000", "42501", "08P01"] {
            assert!(!is_transient_startup_error(&server_error(code)), "{code}");
        }
        assert!(!is_transient_startup_error(&sqlx::Error::Configuration(
            "synthetic bad option".into()
        )));
        assert!(!is_transient_startup_error(&sqlx::Error::PoolClosed));
    }

    #[test]
    fn startup_window_env_value_parses_with_default_and_cap() {
        assert_eq!(startup_window_from_env_value(None), DEFAULT_STARTUP_WINDOW);
        assert_eq!(
            startup_window_from_env_value(Some("  ")),
            DEFAULT_STARTUP_WINDOW
        );
        assert_eq!(
            startup_window_from_env_value(Some("not-a-number")),
            DEFAULT_STARTUP_WINDOW
        );
        assert_eq!(startup_window_from_env_value(Some("0")), Duration::ZERO);
        assert_eq!(
            startup_window_from_env_value(Some(" 45 ")),
            Duration::from_secs(45)
        );
        assert_eq!(
            startup_window_from_env_value(Some("99999999999")),
            Duration::from_secs(MAX_STARTUP_WINDOW_SECS)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn transient_errors_retry_with_backoff_then_succeed() {
        let budgets = RefCell::new(Vec::new());
        let delays = RefCell::new(Vec::new());
        let outcome = retry_until_deadline(
            policy(120),
            |budget| {
                budgets.borrow_mut().push(budget);
                let attempt = budgets.borrow().len();
                async move {
                    if attempt < 4 {
                        Err(FakeError::Transient)
                    } else {
                        Ok("connected")
                    }
                }
            },
            fake_retryable,
            |notice| {
                delays
                    .borrow_mut()
                    .push((notice.attempt, notice.next_delay))
            },
        )
        .await
        .expect("fourth attempt succeeds");

        assert_eq!(outcome.value, "connected");
        assert_eq!(outcome.attempts, 4);
        assert_eq!(outcome.elapsed, Duration::from_secs(1 + 2 + 4));
        assert_eq!(
            delays.into_inner(),
            vec![
                (1, Duration::from_secs(1)),
                (2, Duration::from_secs(2)),
                (3, Duration::from_secs(4)),
            ]
        );
        assert!(
            budgets
                .into_inner()
                .iter()
                .all(|budget| *budget == Some(ATTEMPT_TIMEOUT))
        );
    }

    #[tokio::test(start_paused = true)]
    async fn non_retryable_error_returns_after_one_attempt() {
        let mut attempts = 0;
        let mut notices = 0;
        let failure = retry_until_deadline(
            policy(120),
            |_| {
                attempts += 1;
                async { Err::<(), _>(FakeError::Fatal) }
            },
            fake_retryable,
            |_| notices += 1,
        )
        .await
        .expect_err("fatal error is not retried");

        assert_eq!(failure.error, FakeError::Fatal);
        assert_eq!(failure.attempts, 1);
        assert_eq!(failure.elapsed, Duration::ZERO);
        assert!(!failure.window_exhausted);
        assert_eq!(attempts, 1);
        assert_eq!(notices, 0);
    }

    #[tokio::test(start_paused = true)]
    async fn exhausted_window_returns_last_error_with_attempt_count() {
        let delays = RefCell::new(Vec::new());
        let budgets = RefCell::new(Vec::new());
        let failure = retry_until_deadline(
            policy(30),
            |budget| {
                budgets.borrow_mut().push(budget);
                async { Err::<(), _>(FakeError::Transient) }
            },
            fake_retryable,
            |notice| delays.borrow_mut().push(notice.next_delay.as_secs()),
        )
        .await
        .expect_err("server never becomes ready");

        // 1 + 2 + 4 + 8 + 10 = 25s, then the last 5s of the window, then a
        // final attempt at the deadline.
        assert_eq!(delays.into_inner(), vec![1, 2, 4, 8, 10, 5]);
        assert_eq!(failure.attempts, 7);
        assert_eq!(failure.elapsed, Duration::from_secs(30));
        assert!(failure.window_exhausted);
        assert_eq!(failure.error, FakeError::Transient);
        let budgets = budgets.into_inner();
        assert_eq!(budgets[0], Some(ATTEMPT_TIMEOUT));
        assert_eq!(budgets[5], Some(Duration::from_secs(5)));
        assert_eq!(budgets[6], Some(MIN_ATTEMPT_BUDGET));
    }

    #[tokio::test(start_paused = true)]
    async fn zero_window_makes_a_single_unbounded_attempt() {
        let budgets = RefCell::new(Vec::new());
        let failure = retry_until_deadline(
            policy(0),
            |budget| {
                budgets.borrow_mut().push(budget);
                async { Err::<(), _>(FakeError::Transient) }
            },
            fake_retryable,
            |_| panic!("a zero window never retries"),
        )
        .await
        .expect_err("single attempt fails");

        assert_eq!(budgets.into_inner(), vec![None]);
        assert_eq!(failure.attempts, 1);
        assert!(!failure.window_exhausted);
    }

    #[test]
    fn failure_message_marks_only_an_exhausted_window() {
        let exhausted = RetryFailure {
            error: StartupConnectError::Sqlx(io_error(io::ErrorKind::ConnectionRefused)),
            attempts: 7,
            elapsed: Duration::from_secs(120),
            window_exhausted: true,
        };
        let message = startup_failure_message(&exhausted);
        assert!(
            message.starts_with(
                "cannot open PostgreSQL database after retrying for 120s (7 attempts): "
            ),
            "{message}"
        );
        assert!(is_startup_window_exhausted(&AppError::Repository(message)));

        let rejected = RetryFailure {
            error: StartupConnectError::Sqlx(server_error("28P01")),
            attempts: 1,
            elapsed: Duration::ZERO,
            window_exhausted: false,
        };
        let message = startup_failure_message(&rejected);
        assert!(
            message.starts_with("cannot open PostgreSQL database: "),
            "{message}"
        );
        assert!(!is_startup_window_exhausted(&AppError::Repository(message)));
    }
}

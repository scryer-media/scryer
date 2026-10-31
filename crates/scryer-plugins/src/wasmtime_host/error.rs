//! Trap / exit / protocol error mapping for the native wasmtime plugin hosts.
//!
//! `AppError` has no dedicated timeout/resource-limit/protocol variant, so every
//! failure category maps to `AppError::Repository` with a distinct, categorized
//! message — matching the existing archive path, which also used `Repository`,
//! and keeping the change out of the middleware/GraphQL error surface. The
//! category is captured in the testable `FailureKind` enum before it is
//! flattened into the message.

use scryer_application::AppError;
use wasmtime::Trap;
use wasmtime_wasi::I32Exit;

/// The §7.2.8 error categories, kept as a discriminated value so the
/// classification logic can be unit-tested without a running guest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FailureKind {
    /// Epoch deadline fired — the guest exceeded its wall-clock budget.
    Timeout,
    /// The store limiter denied a memory allocation (cap exceeded / OOM).
    ResourceLimit,
    /// Non-zero `proc_exit`, or any other trap: a guest-side fault.
    PluginFailure,
    /// The guest exited cleanly but produced malformed / absent stdout JSON.
    Protocol,
}

/// A classified invocation failure, before message formatting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RunFailure {
    pub(crate) kind: FailureKind,
    /// Category-specific detail (exit code, trap text, parse error, …).
    pub(crate) detail: String,
}

impl RunFailure {
    fn new(kind: FailureKind, detail: impl Into<String>) -> Self {
        Self {
            kind,
            detail: detail.into(),
        }
    }
}

/// The kind of plugin a failed invocation ran, which names it in the message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PluginHostKind {
    ArchiveExtractor,
    DownloadClient,
    List,
    Notification,
    Subtitle,
}

impl PluginHostKind {
    const fn label(self) -> &'static str {
        match self {
            Self::ArchiveExtractor => "archive extractor plugin",
            Self::DownloadClient => "download client plugin",
            Self::List => "list plugin",
            Self::Notification => "notification plugin",
            Self::Subtitle => "subtitle provider plugin",
        }
    }
}

/// Context threaded into the operator-facing message.
pub(crate) struct InvocationContext<'a> {
    pub(crate) plugin_kind: PluginHostKind,
    pub(crate) plugin_id: &'a str,
    pub(crate) plugin_version: &'a str,
    pub(crate) operation: &'a str,
    /// Wall-clock budget for the run (names the timeout in the message).
    pub(crate) budget: std::time::Duration,
    /// Size-capped tail of guest stderr, if any.
    pub(crate) stderr_tail: &'a str,
}

/// Classify a wasmtime error raised during instantiation or the `_start` call.
///
/// `memory_denied` is checked first: a limiter denial surfaces downstream as a
/// trap (or as a non-zero exit once the guest aborts), but the resource limit is
/// the root cause we want to report.
pub(crate) fn classify_error(error: &wasmtime::Error, memory_denied: bool) -> RunFailure {
    if memory_denied {
        return RunFailure::new(
            FailureKind::ResourceLimit,
            "guest exceeded the configured memory cap",
        );
    }
    if let Some(exit) = error.downcast_ref::<I32Exit>() {
        return RunFailure::new(
            FailureKind::PluginFailure,
            format!("guest exited with status {}", exit.0),
        );
    }
    if let Some(trap) = error.downcast_ref::<Trap>() {
        if *trap == Trap::Interrupt {
            return RunFailure::new(FailureKind::Timeout, "epoch deadline exceeded");
        }
        return RunFailure::new(FailureKind::PluginFailure, format!("guest trapped: {trap}"));
    }
    RunFailure::new(FailureKind::PluginFailure, format!("{error:#}"))
}

/// A malformed / absent stdout response after an otherwise clean run.
pub(crate) fn protocol_failure(detail: impl Into<String>) -> RunFailure {
    RunFailure::new(FailureKind::Protocol, detail)
}

/// Flatten a classified failure into the operator-facing `AppError`.
pub(crate) fn to_app_error(failure: &RunFailure, ctx: &InvocationContext<'_>) -> AppError {
    let plugin = format!(
        "{} {}@{}",
        ctx.plugin_kind.label(),
        ctx.plugin_id,
        ctx.plugin_version
    );
    let stderr = if ctx.stderr_tail.trim().is_empty() {
        String::new()
    } else {
        format!(" (stderr: {})", ctx.stderr_tail.trim())
    };
    let message = match failure.kind {
        FailureKind::Timeout => format!(
            "{plugin} timed out during {} after {:?}: {}",
            ctx.operation, ctx.budget, failure.detail
        ),
        FailureKind::ResourceLimit => format!(
            "{plugin} exceeded its memory limit during {}: {}{stderr}",
            ctx.operation, failure.detail
        ),
        FailureKind::PluginFailure => format!(
            "{plugin} failed during {}: {}{stderr}",
            ctx.operation, failure.detail
        ),
        FailureKind::Protocol => format!(
            "{plugin} returned a malformed response during {}: {}{stderr}",
            ctx.operation, failure.detail
        ),
    };
    AppError::Repository(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonzero_exit_is_plugin_failure_with_code() {
        let err = wasmtime::Error::from(I32Exit(3));
        let failure = classify_error(&err, false);
        assert_eq!(failure.kind, FailureKind::PluginFailure);
        assert!(
            failure.detail.contains('3'),
            "detail should name the exit code: {}",
            failure.detail
        );
    }

    #[test]
    fn epoch_interrupt_is_timeout() {
        let err = wasmtime::Error::from(Trap::Interrupt);
        let failure = classify_error(&err, false);
        assert_eq!(failure.kind, FailureKind::Timeout);
    }

    #[test]
    fn memory_denied_wins_over_symptom() {
        // Even if the symptom is a generic trap, a limiter denial reports the
        // resource limit as the root cause.
        let err = wasmtime::Error::from(Trap::MemoryOutOfBounds);
        let failure = classify_error(&err, true);
        assert_eq!(failure.kind, FailureKind::ResourceLimit);

        // ...and a denial supersedes even a zero exit, which would otherwise
        // read as an ordinary guest completion.
        let clean_exit = wasmtime::Error::from(I32Exit(0));
        assert_eq!(
            classify_error(&clean_exit, true).kind,
            FailureKind::ResourceLimit
        );
    }

    #[test]
    fn other_trap_is_plugin_failure() {
        let err = wasmtime::Error::from(Trap::UnreachableCodeReached);
        let failure = classify_error(&err, false);
        assert_eq!(failure.kind, FailureKind::PluginFailure);
    }

    #[test]
    fn generic_error_is_plugin_failure() {
        let err = wasmtime::Error::msg("module has no start function");
        let failure = classify_error(&err, false);
        assert_eq!(failure.kind, FailureKind::PluginFailure);
    }

    #[test]
    fn app_error_messages_are_categorized() {
        let ctx = InvocationContext {
            plugin_kind: PluginHostKind::ArchiveExtractor,
            plugin_id: "com.scryer.archive",
            plugin_version: "1.2.3",
            operation: "ExtractArchive",
            budget: std::time::Duration::from_secs(3600),
            stderr_tail: "boom",
        };
        let timeout = to_app_error(
            &RunFailure::new(FailureKind::Timeout, "epoch deadline exceeded"),
            &ctx,
        );
        let AppError::Repository(message) = timeout else {
            panic!("expected Repository");
        };
        assert!(message.contains("timed out"));
        assert!(message.contains("com.scryer.archive@1.2.3"));

        let protocol = to_app_error(&protocol_failure("expected value at line 1"), &ctx);
        let AppError::Repository(message) = protocol else {
            panic!("expected Repository");
        };
        assert!(message.contains("malformed response"));
        assert!(message.contains("stderr: boom"));
    }

    #[test]
    fn the_message_names_the_kind_of_plugin_that_failed() {
        let failure = RunFailure::new(FailureKind::Timeout, "epoch deadline exceeded");
        for (plugin_kind, plugin_id, label) in [
            (
                PluginHostKind::Subtitle,
                "com.example.subtitles",
                "subtitle provider plugin com.example.subtitles@2.0.0 timed out",
            ),
            (
                PluginHostKind::ArchiveExtractor,
                "com.example.archive",
                "archive extractor plugin com.example.archive@2.0.0 timed out",
            ),
            (
                PluginHostKind::Notification,
                "com.example.notify",
                "notification plugin com.example.notify@2.0.0 timed out",
            ),
        ] {
            let ctx = InvocationContext {
                plugin_kind,
                plugin_id,
                plugin_version: "2.0.0",
                operation: "Search",
                budget: std::time::Duration::from_secs(30),
                stderr_tail: "",
            };
            let AppError::Repository(message) = to_app_error(&failure, &ctx) else {
                panic!("expected Repository");
            };
            assert!(message.starts_with(label), "{message}");
        }
    }
}

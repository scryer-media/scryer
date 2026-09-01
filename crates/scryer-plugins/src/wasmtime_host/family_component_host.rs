//! The parts of a family component host that are genuinely family-neutral.
//!
//! Three worlds now share the same shape — `scryer:subtitle`,
//! `scryer:download-client` and `scryer:notification` all import
//! `scryer:host/services@1.0.0` and export the command ABI's `describe` /
//! `process` — and each host file is otherwise a near-copy of the last. This
//! module holds the pieces where copying would be a hazard rather than merely
//! repetition:
//!
//! * [`dispatch_host_call`], which decides where a host call runs and how its
//!   failure is classified. That is the authority-bearing half of the shared
//!   import, and three independently drifting copies of it is exactly the bug
//!   this factoring prevents.
//! * the describe budget and the guest-stderr tail, whose *values* are the
//!   contract ("every backing gets the same 10s describe budget").
//!
//! Deliberately NOT factored: the `bindgen!` block, the `Ctx`, the `Runtime`,
//! the `describe`/`process` drivers. Those differ only in generated type names
//! and log targets, so a macro could absorb them — but they are also where each
//! family documents *why* it has the authority it has (a download client's
//! session cookie outliving its instance, a notification channel's socket
//! grant), and a macro would replace that prose with parameters. The
//! duplication there is legible and per-family; the duplication here was not.
//!
//! The archive world is not a client of this module: it imports family-specific
//! crypto rather than the shared services interface, so it has no host call to
//! dispatch.

use std::time::Duration;

use wasmtime_wasi::p2::pipe::MemoryOutputPipe;

use crate::wasmtime_host::command_host::CommandHost;

/// Describe runs reuse the 10s describe budget of every other backing.
pub(crate) const DESCRIBE_TIMEOUT: Duration = Duration::from_secs(10);

/// Amount of guest stderr forwarded to tracing / attached to error messages.
const STDERR_TAIL_BYTES: usize = 8 * 1024;

/// Why a `scryer:host/services@1.0.0` host call could not produce a response.
///
/// Both variants mean the guest never reached the service layer, which is what
/// the world's `host-error` enum is for; an unconfigured *service* is not one of
/// these, because that answers in-band with a typed `Unsupported`.
pub(crate) enum HostCallFailure {
    /// The guest sent something that is not a decodable `PluginHostRequest`, or
    /// exceeded the encoded-request cap. Recoverable: a different request may
    /// well work.
    InvalidRequest,
    /// The host could not run the service or encode its response.
    Failed,
}

/// Run one encoded host request against a family host's [`CommandHost`].
///
/// The service layer is synchronous and can block — `ArchiveExtract` drives a
/// nested extractor invocation through a runtime handle, and a socket read
/// blocks on the wire — so it runs on the blocking pool rather than on the async
/// worker that is servicing the guest, exactly as the core-module
/// `scryer_host_call` shim does. A `CommandHost` is a cheap `Arc` clone, so the
/// move costs nothing and every piece of shared state it holds — the plugin
/// state map, the socket handle table — stays shared.
///
/// The failure carries its own diagnostic rather than logging it: a `tracing`
/// target has to be a literal at the callsite, so each family keeps its own
/// (`scryer_plugins::subtitle`, `scryer_plugins::notification`, …) and logs the
/// string this returns.
pub(crate) async fn dispatch_host_call(
    command_host: &CommandHost,
    request: Vec<u8>,
) -> Result<Vec<u8>, HostCallError> {
    let command_host = command_host.clone();
    match tokio::task::spawn_blocking(move || command_host.call_bytes(&request)).await {
        Ok(Ok(response)) => Ok(response),
        Ok(Err(error)) => {
            // A rejected or undecodable request is the guest's fault and is
            // recoverable by sending a different one; everything else is the
            // transport failing.
            let failure = if error.contains("encoded host request exceeds")
                || error.contains("invalid postcard host request")
            {
                HostCallFailure::InvalidRequest
            } else {
                HostCallFailure::Failed
            };
            Err(HostCallError::Service { failure, error })
        }
        Err(error) => Err(HostCallError::Task(error.to_string())),
    }
}

/// A host call that did not produce response bytes, and where it went wrong.
pub(crate) enum HostCallError {
    /// The service layer ran and refused, or could not encode its answer.
    Service {
        failure: HostCallFailure,
        error: String,
    },
    /// The blocking task carrying the call never completed.
    Task(String),
}

/// Size-capped, lossy tail of a captured output pipe.
pub(crate) fn tail_of(pipe: &MemoryOutputPipe) -> String {
    let bytes = pipe.contents();
    let start = bytes.len().saturating_sub(STDERR_TAIL_BYTES);
    String::from_utf8_lossy(&bytes[start..]).into_owned()
}

pub(crate) fn stderr_suffix(stderr_tail: &str) -> String {
    if stderr_tail.is_empty() {
        String::new()
    } else {
        format!("; guest stderr: {stderr_tail}")
    }
}

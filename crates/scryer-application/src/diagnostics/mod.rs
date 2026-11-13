//! The diagnostics bundle: one compressed archive of logs, metrics and a
//! settings and environment summary that an operator can hand to the
//! maintainer.
//!
//! Every byte written into the archive passes through [`sanitize`]. Values
//! the instance knows to be identifying (names, hosts, usernames, labels,
//! secrets, paths) are replaced by stable pseudonyms, and pattern layers catch
//! URLs, credentials, addresses and paths the instance does not know about.
//! Sanitization is best effort; the web page asks the operator to review the
//! bundle before sharing it.
//!
//! The bundle is built in memory and returned as bytes. Nothing is written to
//! disk and nothing is removed.

mod archive;
mod environment;
mod gather;
mod logs;
pub mod sanitize;

#[cfg(test)]
mod tests;

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde_json::{Map, Value};

pub use sanitize::{KnownValues, PATH_POLICY, PathPolicy, Sanitizer};

/// The largest compressed bundle produced. The oldest log content is dropped
/// until the archive fits; the manifest records what was dropped.
pub const DIAGNOSTICS_BUNDLE_MAX_COMPRESSED_BYTES: usize = 8 * 1024 * 1024;

/// Version of the bundle layout, bumped when a member changes shape.
pub const DIAGNOSTICS_BUNDLE_FORMAT_VERSION: u32 = 1;

/// The Prometheus exposition, when the binary has a recorder installed.
#[derive(Clone, Debug)]
pub enum DiagnosticsMetrics {
    /// `SCRYER_METRICS` is off: no recorder is installed and nothing was
    /// collected, so there is nothing to render.
    Disabled,
    Rendered(String),
}

/// What the binary knows and the application layer does not: its log
/// buffer and log file, its metrics recorder, its process and environment
/// facts, and the values that identify the host it runs on.
#[derive(Clone, Debug)]
pub struct DiagnosticsHostInputs {
    /// The in-memory log buffer, oldest line first.
    pub buffered_log_lines: Vec<String>,
    /// The active log file, when file logging is on.
    pub log_file: Option<PathBuf>,
    pub metrics: DiagnosticsMetrics,
    /// Environment facts gathered by the binary. Sanitized like every other
    /// member before it is written.
    pub environment: Map<String, Value>,
    /// Facts the binary could not gather, as `(item, reason)`.
    pub environment_omissions: Vec<(String, String)>,
    /// Identifying values only the binary knows: host name, OS user, home and
    /// data directories, log file path, bind address, TLS file paths, base
    /// path, and the value of every `SCRYER_*` variable.
    pub sensitive_values: Vec<String>,
    /// Identifying paths only the binary knows, replaced by the pseudonym the
    /// pair names (`data-dir`, `log-dir`, ...).
    pub sensitive_paths: Vec<(String, String)>,
    /// The running binary's version.
    pub version: String,
    pub generated_at: DateTime<Utc>,
}

/// A finished bundle.
#[derive(Clone, Debug)]
pub struct DiagnosticsBundle {
    pub file_name: String,
    pub bytes: Vec<u8>,
}

/// One item left out of the bundle and why. `reason` is a stable code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Omission {
    pub item: String,
    pub reason: String,
}

impl Omission {
    pub(crate) fn new(item: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            item: item.into(),
            reason: reason.into(),
        }
    }
}

/// The file name the browser saves the bundle under.
pub fn diagnostics_bundle_file_name(version: &str, generated_at: DateTime<Utc>) -> String {
    let version = version
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+'))
        .collect::<String>();
    format!(
        "scryer-diagnostics-{version}-{}.tar.zst",
        generated_at.format("%Y%m%dT%H%M%SZ")
    )
}

impl crate::AppUseCase {
    /// Builds the diagnostics bundle. Requires the system-settings permission,
    /// the same one the service-log page needs.
    pub async fn build_diagnostics_bundle(
        &self,
        actor: &crate::User,
        host: DiagnosticsHostInputs,
    ) -> crate::AppResult<DiagnosticsBundle> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        let mut omissions = Vec::new();
        let mut known = KnownValues::new();
        for word in ["movie", "series", "anime"] {
            known.protect(word);
        }
        let incomplete = self.harvest_known_values(actor, &mut known).await;
        for value in &host.sensitive_values {
            known.secret(value);
        }
        for (pseudonym, path) in &host.sensitive_paths {
            known.path(pseudonym, path);
        }
        let summary = self.diagnostics_summary(actor, &mut omissions).await;
        let mut environment = host.environment.clone();
        self.diagnostics_environment(actor, &mut environment, &mut omissions)
            .await;
        for (item, reason) in &host.environment_omissions {
            omissions.push(Omission::new(format!("environment.{item}"), reason.clone()));
        }

        let log_file = host.log_file.clone();
        let file_logs = match log_file {
            Some(path) => tokio::task::spawn_blocking(move || logs::read_log_files(&path))
                .await
                .unwrap_or_else(|_| logs::LogFileRead::failed("read_task_failed")),
            None => logs::LogFileRead::absent(),
        };

        let version = host.version.clone();
        let generated_at = host.generated_at;
        let inputs = archive::ArchiveInputs {
            version: version.clone(),
            generated_at,
            summary,
            environment,
            buffered_log_lines: host.buffered_log_lines,
            file_logs,
            metrics: host.metrics,
            omissions,
            known_values_incomplete: incomplete,
        };
        let bytes = tokio::task::spawn_blocking(move || {
            let sanitizer = known.into_sanitizer();
            archive::build(&sanitizer, inputs)
        })
        .await
        .map_err(|error| {
            crate::AppError::Repository(format!("diagnostics bundle task: {error}"))
        })??;

        Ok(DiagnosticsBundle {
            file_name: diagnostics_bundle_file_name(&version, generated_at),
            bytes,
        })
    }
}

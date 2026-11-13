//! Sanitizes every member and packs the bundle as a zstd tar archive within
//! [`DIAGNOSTICS_BUNDLE_MAX_COMPRESSED_BYTES`].

use chrono::{DateTime, Utc};
use serde_json::{Map, Value, json};

use super::logs::LogFileRead;
use super::sanitize::{PATH_POLICY, PathPolicy, Sanitizer};
use super::{
    DIAGNOSTICS_BUNDLE_FORMAT_VERSION, DIAGNOSTICS_BUNDLE_MAX_COMPRESSED_BYTES, DiagnosticsMetrics,
    Omission,
};
use crate::{AppError, AppResult};

const ZSTD_LEVEL: i32 = 6;
/// Share of the oldest remaining log lines dropped per trimming round.
const TRIM_FRACTION: usize = 4;

pub(crate) const MEMBER_MANIFEST: &str = "manifest.json";
pub(crate) const MEMBER_SUMMARY: &str = "summary.json";
pub(crate) const MEMBER_ENVIRONMENT: &str = "environment.json";
pub(crate) const MEMBER_BUFFER_LOG: &str = "logs/buffer.log";
pub(crate) const MEMBER_FILE_LOG: &str = "logs/file.log";
pub(crate) const MEMBER_METRICS: &str = "metrics.prom";

/// The sanitizer layers, in the order they run, as the manifest lists them.
fn sanitization_passes() -> Vec<&'static str> {
    vec![
        "structured_field_names",
        "secret_patterns",
        "urls",
        "emails",
        match PATH_POLICY {
            PathPolicy::KeepFileName => "paths_keep_file_name",
            PathPolicy::RedactAll => "paths_redact_all",
        },
        "known_values",
        "host_port",
        "bare_hosts",
        "ip_addresses",
    ]
}

pub(crate) struct ArchiveInputs {
    pub version: String,
    pub generated_at: DateTime<Utc>,
    pub summary: Value,
    pub environment: Map<String, Value>,
    pub buffered_log_lines: Vec<String>,
    pub file_logs: LogFileRead,
    pub metrics: DiagnosticsMetrics,
    pub omissions: Vec<Omission>,
    pub known_values_incomplete: Vec<String>,
}

/// Runs `work` and turns a panic into `None`, so one item that trips the
/// sanitizer is left out instead of failing, or leaking into, the bundle.
fn guarded<T>(work: impl FnOnce() -> T) -> Option<T> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)).ok()
}

struct Truncation {
    item: &'static str,
    reason: &'static str,
    dropped_lines: usize,
}

pub(crate) fn build(sanitizer: &Sanitizer, inputs: ArchiveInputs) -> AppResult<Vec<u8>> {
    let ArchiveInputs {
        version,
        generated_at,
        mut summary,
        environment,
        buffered_log_lines,
        file_logs,
        metrics,
        mut omissions,
        known_values_incomplete,
    } = inputs;
    let mut truncations: Vec<Truncation> = Vec::new();

    let mut buffer_lines: Option<Vec<String>> = None;
    let mut file_lines: Option<Vec<String>> = None;
    let mut metrics_text: Option<String> = None;

    if !known_values_incomplete.is_empty() {
        let reason = format!(
            "known_values_incomplete:{}",
            known_values_incomplete.join(",")
        );
        omissions.push(Omission::new(MEMBER_BUFFER_LOG, reason.clone()));
        omissions.push(Omission::new(MEMBER_FILE_LOG, reason.clone()));
        omissions.push(Omission::new(MEMBER_METRICS, reason));
    } else {
        match guarded(|| sanitize_lines(sanitizer, &buffered_log_lines)) {
            Some(lines) => buffer_lines = Some(lines),
            None => omissions.push(Omission::new(MEMBER_BUFFER_LOG, "sanitizer_failed")),
        }

        if !file_logs.configured {
            omissions.push(Omission::new(
                MEMBER_FILE_LOG,
                "file_logging_not_configured",
            ));
        } else {
            for (source, reason) in &file_logs.failures {
                omissions.push(Omission::new(format!("{MEMBER_FILE_LOG}:{source}"), reason));
            }
            if file_logs.read_capped {
                truncations.push(Truncation {
                    item: MEMBER_FILE_LOG,
                    reason: "read_cap",
                    dropped_lines: 0,
                });
            }
            if !file_logs.sources.is_empty() {
                match guarded(|| sanitize_lines(sanitizer, &file_logs.lines)) {
                    Some(lines) => file_lines = Some(lines),
                    None => omissions.push(Omission::new(MEMBER_FILE_LOG, "sanitizer_failed")),
                }
            }
        }

        match metrics {
            DiagnosticsMetrics::Disabled => {
                omissions.push(Omission::new(MEMBER_METRICS, "metrics_disabled"));
            }
            DiagnosticsMetrics::Rendered(exposition) => {
                match guarded(|| sanitizer.metrics(&exposition)) {
                    Some(text) => metrics_text = Some(text),
                    None => omissions.push(Omission::new(MEMBER_METRICS, "sanitizer_failed")),
                }
            }
        }
    }

    let summary_bytes = match guarded(|| {
        sanitizer.json_values(&mut summary);
        serde_json::to_vec_pretty(&summary)
    }) {
        Some(Ok(bytes)) => Some(bytes),
        _ => {
            omissions.push(Omission::new(MEMBER_SUMMARY, "sanitizer_failed"));
            None
        }
    };
    let mut environment = Value::Object(environment);
    let environment_bytes = match guarded(|| {
        sanitizer.json_values(&mut environment);
        serde_json::to_vec_pretty(&environment)
    }) {
        Some(Ok(bytes)) => Some(bytes),
        _ => {
            omissions.push(Omission::new(MEMBER_ENVIRONMENT, "sanitizer_failed"));
            None
        }
    };

    loop {
        let mut members: Vec<(&'static str, Vec<u8>)> = Vec::new();
        if let Some(bytes) = &summary_bytes {
            members.push((MEMBER_SUMMARY, bytes.clone()));
        }
        if let Some(bytes) = &environment_bytes {
            members.push((MEMBER_ENVIRONMENT, bytes.clone()));
        }
        if let Some(lines) = &buffer_lines {
            members.push((MEMBER_BUFFER_LOG, join_lines(lines)));
        }
        if let Some(lines) = &file_lines {
            members.push((MEMBER_FILE_LOG, join_lines(lines)));
        }
        if let Some(text) = &metrics_text {
            members.push((MEMBER_METRICS, text.clone().into_bytes()));
        }

        let mut manifest = manifest(
            &version,
            generated_at,
            &members,
            &omissions,
            &truncations,
            sanitizer,
        );
        sanitizer.json_values(&mut manifest);
        let manifest_bytes = serde_json::to_vec_pretty(&manifest)
            .map_err(|error| AppError::Repository(format!("diagnostics manifest: {error}")))?;
        members.insert(0, (MEMBER_MANIFEST, manifest_bytes));

        let compressed = pack(&members, generated_at)?;
        if compressed.len() <= DIAGNOSTICS_BUNDLE_MAX_COMPRESSED_BYTES {
            return Ok(compressed);
        }

        // Oldest content first: the file log holds the older history, the
        // buffer only the most recent lines.
        let trimmed = trim_oldest(&mut file_lines, MEMBER_FILE_LOG, &mut truncations)
            || trim_oldest(&mut buffer_lines, MEMBER_BUFFER_LOG, &mut truncations);
        if !trimmed {
            if metrics_text.take().is_some() {
                omissions.push(Omission::new(MEMBER_METRICS, "size_cap"));
                continue;
            }
            return Err(AppError::Repository(
                "diagnostics bundle exceeds its size cap without logs".to_string(),
            ));
        }
    }
}

fn sanitize_lines(sanitizer: &Sanitizer, lines: &[String]) -> Vec<String> {
    lines.iter().map(|line| sanitizer.log_line(line)).collect()
}

fn join_lines(lines: &[String]) -> Vec<u8> {
    let mut out = Vec::with_capacity(lines.iter().map(|line| line.len() + 1).sum());
    for line in lines {
        out.extend_from_slice(line.as_bytes());
        out.push(b'\n');
    }
    out
}

/// Drops the oldest share of `lines`; false when there is nothing to drop.
fn trim_oldest(
    lines: &mut Option<Vec<String>>,
    item: &'static str,
    truncations: &mut Vec<Truncation>,
) -> bool {
    let Some(current) = lines.as_mut() else {
        return false;
    };
    if current.is_empty() {
        return false;
    }
    let drop = (current.len() / TRIM_FRACTION).max(1);
    current.drain(..drop);
    match truncations
        .iter_mut()
        .find(|truncation| truncation.item == item && truncation.reason == "size_cap")
    {
        Some(truncation) => truncation.dropped_lines += drop,
        None => truncations.push(Truncation {
            item,
            reason: "size_cap",
            dropped_lines: drop,
        }),
    }
    true
}

fn manifest(
    version: &str,
    generated_at: DateTime<Utc>,
    members: &[(&'static str, Vec<u8>)],
    omissions: &[Omission],
    truncations: &[Truncation],
    sanitizer: &Sanitizer,
) -> Value {
    json!({
        "format_version": DIAGNOSTICS_BUNDLE_FORMAT_VERSION,
        "generated_at": generated_at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        "scryer_version": version,
        "compressed_size_cap_bytes": DIAGNOSTICS_BUNDLE_MAX_COMPRESSED_BYTES,
        "files": members.iter().map(|(name, bytes)| json!({
            "name": name,
            "bytes": bytes.len(),
        })).collect::<Vec<_>>(),
        "sanitization": {
            "best_effort": true,
            "passes": sanitization_passes(),
            "path_policy": match PATH_POLICY {
                PathPolicy::KeepFileName => "keep_file_name",
                PathPolicy::RedactAll => "redact_all",
            },
            "known_values": sanitizer.known_value_count(),
            "short_values_not_replaced": sanitizer.skipped_short_values(),
            "vocabulary_values_not_replaced": sanitizer.skipped_protected_values(),
        },
        "omissions": omissions.iter().map(|omission| json!({
            "item": omission.item,
            "reason": omission.reason,
        })).collect::<Vec<_>>(),
        "truncations": truncations.iter().map(|truncation| json!({
            "item": truncation.item,
            "reason": truncation.reason,
            "dropped_lines": truncation.dropped_lines,
        })).collect::<Vec<_>>(),
    })
}

fn pack(members: &[(&'static str, Vec<u8>)], generated_at: DateTime<Utc>) -> AppResult<Vec<u8>> {
    let mut builder = tar::Builder::new(Vec::new());
    let mtime = u64::try_from(generated_at.timestamp()).unwrap_or(0);
    for (name, bytes) in members {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_mtime(mtime);
        header.set_entry_type(tar::EntryType::Regular);
        header.set_cksum();
        builder
            .append_data(&mut header, name, bytes.as_slice())
            .map_err(|error| AppError::Repository(format!("diagnostics archive: {error}")))?;
    }
    let tar = builder
        .into_inner()
        .map_err(|error| AppError::Repository(format!("diagnostics archive: {error}")))?;
    zstd::encode_all(tar.as_slice(), ZSTD_LEVEL)
        .map_err(|error| AppError::Repository(format!("diagnostics compression: {error}")))
}

//! Reads the diagnostics material one piece at a time, for callers that ask a
//! narrow question instead of downloading the whole bundle: one summary or
//! environment section, the known-value harvest and a log search.
//!
//! Each piece is gathered on its own, with the same functions and the same
//! sanitizer the bundle uses, so a pseudonym means the same thing in both.

use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::Value;

use super::Omission;
use super::sanitize::{KnownValues, Sanitizer};
use crate::{AppError, AppResult, AppUseCase, User};

/// The summary member's sections, in the order the bundle writes them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SummarySection {
    QualityProfiles,
    Indexers,
    DownloadClients,
    Libraries,
    RuleSets,
    DelayProfiles,
    SeedingProfiles,
    NotificationChannels,
    MediaServers,
    SubtitleProviders,
    Proxies,
    Lists,
    Plugins,
    HealthChecks,
    JobRuns,
    Titles,
    DownloadQueue,
    ReleaseDecisions,
    IndexerErrors,
    Users,
}

impl SummarySection {
    pub const ALL: &'static [Self] = &[
        Self::QualityProfiles,
        Self::Indexers,
        Self::DownloadClients,
        Self::Libraries,
        Self::RuleSets,
        Self::DelayProfiles,
        Self::SeedingProfiles,
        Self::NotificationChannels,
        Self::MediaServers,
        Self::SubtitleProviders,
        Self::Proxies,
        Self::Lists,
        Self::Plugins,
        Self::HealthChecks,
        Self::JobRuns,
        Self::Titles,
        Self::DownloadQueue,
        Self::ReleaseDecisions,
        Self::IndexerErrors,
        Self::Users,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::QualityProfiles => "quality_profiles",
            Self::Indexers => "indexers",
            Self::DownloadClients => "download_clients",
            Self::Libraries => "libraries",
            Self::RuleSets => "rule_sets",
            Self::DelayProfiles => "delay_profiles",
            Self::SeedingProfiles => "seeding_profiles",
            Self::NotificationChannels => "notification_channels",
            Self::MediaServers => "media_servers",
            Self::SubtitleProviders => "subtitle_providers",
            Self::Proxies => "proxies",
            Self::Lists => "lists",
            Self::Plugins => "plugins",
            Self::HealthChecks => "health_checks",
            Self::JobRuns => "job_runs",
            Self::Titles => "titles",
            Self::DownloadQueue => "download_queue",
            Self::ReleaseDecisions => "release_decisions",
            Self::IndexerErrors => "indexer_errors",
            Self::Users => "users",
        }
    }
}

/// The environment sections the application layer owns, in the order the
/// bundle writes them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvironmentSection {
    Datastore,
    SetupComplete,
    SmgCompatibilityNotice,
    Settings,
    NetworkConfiguration,
    UserCount,
    /// Probes every library root and download root; nothing else does.
    Filesystems,
}

impl EnvironmentSection {
    pub const ALL: &'static [Self] = &[
        Self::Datastore,
        Self::SetupComplete,
        Self::SmgCompatibilityNotice,
        Self::Settings,
        Self::NetworkConfiguration,
        Self::UserCount,
        Self::Filesystems,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Datastore => "datastore",
            Self::SetupComplete => "setup_complete",
            Self::SmgCompatibilityNotice => "smg_compatibility_notice",
            Self::Settings => "settings",
            Self::NetworkConfiguration => "network_configuration",
            Self::UserCount => "user_count",
            Self::Filesystems => "filesystems",
        }
    }
}

/// A section the application layer reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagnosticsSection {
    Summary(SummarySection),
    Environment(EnvironmentSection),
}

/// One section as read, before sanitization. `value` is `None` when it could
/// not be read; `omissions` say why, as `(item, reason code)`.
#[derive(Clone, Debug)]
pub struct DiagnosticsSectionRead {
    pub value: Option<Value>,
    pub omissions: Vec<Omission>,
}

/// The sanitizer built from the known-value harvest.
pub struct DiagnosticsHarvest {
    pub sanitizer: Sanitizer,
    /// Sources the harvest could not read. Logs and metrics must not be shown
    /// while any are missing, since their values would go unreplaced.
    pub known_values_incomplete: Vec<String>,
}

impl DiagnosticsHarvest {
    /// The omission reason the bundle records for logs and metrics when the
    /// harvest is incomplete, or `None` when it is complete.
    pub fn incomplete_reason(&self) -> Option<String> {
        (!self.known_values_incomplete.is_empty()).then(|| {
            format!(
                "known_values_incomplete:{}",
                self.known_values_incomplete.join(",")
            )
        })
    }
}

impl AppUseCase {
    /// Every identifying value the instance and the host know, with the
    /// sources that could not be read. Shared by the bundle and every
    /// narrower read, so pseudonyms agree.
    pub(super) async fn diagnostics_known_values(
        &self,
        actor: &User,
        sensitive_values: &[String],
        sensitive_paths: &[(String, String)],
    ) -> (KnownValues, Vec<String>) {
        let mut known = KnownValues::new();
        for word in ["movie", "series", "anime"] {
            known.protect(word);
        }
        let incomplete = self.harvest_known_values(actor, &mut known).await;
        for value in sensitive_values {
            known.secret(value);
        }
        for (pseudonym, path) in sensitive_paths {
            known.path(pseudonym, path);
        }
        (known, incomplete)
    }

    /// The bundle's sanitizer, built once for one caller's request. Requires
    /// the system-settings permission.
    pub async fn diagnostics_harvest(
        &self,
        actor: &User,
        sensitive_values: &[String],
        sensitive_paths: &[(String, String)],
    ) -> AppResult<DiagnosticsHarvest> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let (known, known_values_incomplete) = self
            .diagnostics_known_values(actor, sensitive_values, sensitive_paths)
            .await;
        let sanitizer = tokio::task::spawn_blocking(move || known.into_sanitizer())
            .await
            .map_err(|error| AppError::Repository(format!("diagnostics harvest task: {error}")))?;
        Ok(DiagnosticsHarvest {
            sanitizer,
            known_values_incomplete,
        })
    }

    /// One summary or environment section, gathered on its own and not yet
    /// sanitized. Requires the system-settings permission.
    pub async fn diagnostics_section(
        &self,
        actor: &User,
        section: DiagnosticsSection,
    ) -> AppResult<DiagnosticsSectionRead> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let mut omissions = Vec::new();
        let value = match section {
            DiagnosticsSection::Summary(name) => match self.summary_section(actor, name).await {
                Ok(value) => Some(value),
                Err(error) => {
                    omissions.push(Omission::new(
                        format!("summary.{}", name.as_str()),
                        format!("read_failed:{}", super::gather::error_kind(&error)),
                    ));
                    None
                }
            },
            DiagnosticsSection::Environment(name) => {
                self.environment_section(actor, name, &mut omissions).await
            }
        };
        Ok(DiagnosticsSectionRead { value, omissions })
    }
}

/// A log level, lowest first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl LogLevel {
    pub const ALL: [Self; 5] = [
        Self::Trace,
        Self::Debug,
        Self::Info,
        Self::Warn,
        Self::Error,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Trace => "TRACE",
            Self::Debug => "DEBUG",
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|level| level.as_str().eq_ignore_ascii_case(value))
    }
}

fn json_record(line: &str) -> Option<serde_json::Map<String, Value>> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('{') {
        return None;
    }
    match serde_json::from_str::<Value>(trimmed) {
        Ok(Value::Object(map)) => Some(map),
        _ => None,
    }
}

/// The longest log search pattern accepted.
pub const LOG_PATTERN_MAX_CHARS: usize = 256;
/// Compiled program and lazy DFA size limits for a log search pattern, so a
/// caller-supplied pattern cannot claim unbounded memory.
const LOG_PATTERN_SIZE_LIMIT: usize = 1 << 20;
const LOG_PATTERN_DFA_SIZE_LIMIT: usize = 1 << 20;
/// Characters of a structured line's message kept in a search result.
pub const LOG_MESSAGE_MAX_CHARS: usize = 400;
/// Characters of a plain line kept in a search result.
pub const LOG_PLAIN_LINE_MAX_CHARS: usize = 500;

/// A case-insensitive log search pattern.
#[derive(Clone, Debug)]
pub struct LogPattern(regex::Regex);

/// Why a log search pattern was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogPatternRefusal {
    TooLong,
    Invalid,
}

impl LogPattern {
    pub fn new(pattern: &str) -> Result<Self, LogPatternRefusal> {
        if pattern.chars().count() > LOG_PATTERN_MAX_CHARS {
            return Err(LogPatternRefusal::TooLong);
        }
        regex::RegexBuilder::new(pattern)
            .case_insensitive(true)
            .size_limit(LOG_PATTERN_SIZE_LIMIT)
            .dfa_size_limit(LOG_PATTERN_DFA_SIZE_LIMIT)
            .build()
            .map(Self)
            .map_err(|_| LogPatternRefusal::Invalid)
    }
}

/// Which log lines a search reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogSource {
    Buffer,
    File,
    Both,
}

/// Where a search continues: the number of lines already passed, counted
/// from the newest end of one source. With [`LogSource::Both`] the buffer is
/// read first, then the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogCursor {
    Buffer(usize),
    File(usize),
}

impl LogCursor {
    pub fn parse(value: &str) -> Option<Self> {
        let (source, offset) = value.split_once(':')?;
        let offset = offset.parse().ok()?;
        match source {
            "buffer" => Some(Self::Buffer(offset)),
            "file" => Some(Self::File(offset)),
            _ => None,
        }
    }

    pub fn render(self) -> String {
        match self {
            Self::Buffer(offset) => format!("buffer:{offset}"),
            Self::File(offset) => format!("file:{offset}"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct LogSearch {
    pub pattern: Option<LogPattern>,
    /// Lines at this level or above.
    pub min_level: Option<LogLevel>,
    pub source: LogSource,
    pub max_lines: usize,
    /// Characters of matching lines a result holds at most. The line that
    /// would pass it is left for the next page; the first line always fits.
    pub max_chars: usize,
    /// Lines read and sanitized at most. A search that reaches it before
    /// the page is full stops with a cursor at the next unread line.
    pub max_scanned: usize,
    pub before: Option<LogCursor>,
}

/// One matching line, sanitized and projected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogHit {
    /// `buffer` or `file`.
    pub source: &'static str,
    pub line: String,
}

#[derive(Clone, Debug, Default)]
pub struct LogSearchResult {
    /// Newest first.
    pub lines: Vec<LogHit>,
    /// Lines read and sanitized to produce the result.
    pub scanned: usize,
    /// Where the next older page starts; `None` when nothing older is left.
    pub next_cursor: Option<LogCursor>,
    /// Whether the search stopped at [`LogSearch::max_scanned`] with older
    /// lines left unread.
    pub scan_capped: bool,
    /// Whether the search stopped because `cancelled` was raised.
    pub cancelled: bool,
}

/// Searches log lines newest first. Every line is sanitized before it is
/// matched, so a pattern can only find what the bundle would show. The
/// search stops early when `cancelled` is raised.
pub fn search_log_lines(
    sanitizer: &Sanitizer,
    buffer: &[String],
    file: &[String],
    search: &LogSearch,
    cancelled: &AtomicBool,
) -> LogSearchResult {
    let mut result = LogSearchResult::default();
    let max_lines = search.max_lines.max(1);
    let reads_buffer = matches!(search.source, LogSource::Buffer | LogSource::Both);
    let reads_file = matches!(search.source, LogSource::File | LogSource::Both);
    let (buffer_start, file_start) = match search.before {
        None => (0, 0),
        Some(LogCursor::Buffer(offset)) => (offset, 0),
        // The buffer is read first, so a file cursor means it is done.
        Some(LogCursor::File(offset)) => (buffer.len(), offset),
    };
    let mut sources: Vec<(&'static str, &[String], usize)> = Vec::new();
    if reads_buffer {
        sources.push(("buffer", buffer, buffer_start));
    }
    if reads_file {
        sources.push(("file", file, file_start));
    }
    let mut chars = 0;
    for (position, (source, lines, start)) in sources.iter().enumerate() {
        let mut passed = *start;
        for line in lines.iter().rev().skip(*start) {
            if cancelled.load(Ordering::Acquire) {
                result.cancelled = true;
                return result;
            }
            if result.lines.len() >= max_lines {
                result.next_cursor = Some(cursor(source, passed));
                return result;
            }
            if result.scanned >= search.max_scanned {
                result.next_cursor = Some(cursor(source, passed));
                result.scan_capped = true;
                return result;
            }
            let sanitized = sanitizer.log_line(line);
            let hit = matching_line(&sanitized, search);
            if let Some(projected) = &hit {
                let size = projected.chars().count();
                if !result.lines.is_empty() && chars + size > search.max_chars {
                    // This line starts the next page.
                    result.next_cursor = Some(cursor(source, passed));
                    return result;
                }
                chars += size;
            }
            passed += 1;
            result.scanned += 1;
            if let Some(projected) = hit {
                result.lines.push(LogHit {
                    source,
                    line: projected,
                });
            }
        }
        if result.lines.len() >= max_lines {
            // Exactly full at the end of this source: continue from the next
            // one, if there is anything left in it.
            let next = sources[position + 1..]
                .iter()
                .find(|(_, lines, start)| lines.len() > *start);
            result.next_cursor = next.map(|(source, _, start)| cursor(source, *start));
            return result;
        }
    }
    result
}

fn cursor(source: &str, passed: usize) -> LogCursor {
    if source == "buffer" {
        LogCursor::Buffer(passed)
    } else {
        LogCursor::File(passed)
    }
}

/// The projected line when `line` (already sanitized) matches the search.
fn matching_line(line: &str, search: &LogSearch) -> Option<String> {
    if let Some(record) = json_record(line) {
        let level = record.get("level").and_then(Value::as_str).unwrap_or("");
        if let Some(minimum) = search.min_level
            && LogLevel::parse(level).is_none_or(|level| level < minimum)
        {
            return None;
        }
        let message = record
            .get("fields")
            .and_then(|fields| fields.get("message"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let target = record.get("target").and_then(Value::as_str).unwrap_or("");
        if let Some(pattern) = &search.pattern
            && !pattern.0.is_match(&format!("{message} {target}"))
        {
            return None;
        }
        let timestamp = record
            .get("timestamp")
            .and_then(Value::as_str)
            .unwrap_or("");
        return Some(format!(
            "{timestamp} {level} {target} {}",
            cut_chars(message, LOG_MESSAGE_MAX_CHARS)
        ));
    }
    if let Some(minimum) = search.min_level
        && !LogLevel::ALL
            .into_iter()
            .filter(|level| *level >= minimum)
            .any(|level| line.contains(level.as_str()))
    {
        return None;
    }
    if let Some(pattern) = &search.pattern
        && !pattern.0.is_match(line)
    {
        return None;
    }
    Some(cut_chars(line, LOG_PLAIN_LINE_MAX_CHARS).to_string())
}

/// `text` cut to at most `max` characters, on a character boundary.
pub fn cut_chars(text: &str, max: usize) -> &str {
    match text.char_indices().nth(max) {
        Some((index, _)) => &text[..index],
        None => text,
    }
}

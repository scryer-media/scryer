use super::{ExecutionModeValue, MediaFacetValue};
use async_graphql::{Enum, ID, InputObject, MaybeUndefined, SimpleObject};
use chrono::{DateTime, Utc};
use scryer_domain::{ScheduleWeekday, ScriptLanguage, ScriptSchedule, ScriptTrigger};

// ── Post-Processing Scripts ────────────────────────────────────────────────

/// Language an inline script is written in; selects its interpreter.
#[derive(Enum, Copy, Clone, Eq, PartialEq, Debug)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ScriptLanguageValue {
    /// Shell script.
    Shell,
    /// Python 3.
    Python,
    /// PowerShell.
    #[graphql(name = "POWERSHELL")]
    PowerShell,
    /// Windows batch file.
    Batch,
    /// Go source file.
    Go,
}

impl From<ScriptLanguage> for ScriptLanguageValue {
    fn from(value: ScriptLanguage) -> Self {
        match value {
            ScriptLanguage::Shell => Self::Shell,
            ScriptLanguage::Python => Self::Python,
            ScriptLanguage::PowerShell => Self::PowerShell,
            ScriptLanguage::Batch => Self::Batch,
            ScriptLanguage::Go => Self::Go,
        }
    }
}

impl From<ScriptLanguageValue> for ScriptLanguage {
    fn from(value: ScriptLanguageValue) -> Self {
        match value {
            ScriptLanguageValue::Shell => Self::Shell,
            ScriptLanguageValue::Python => Self::Python,
            ScriptLanguageValue::PowerShell => Self::PowerShell,
            ScriptLanguageValue::Batch => Self::Batch,
            ScriptLanguageValue::Go => Self::Go,
        }
    }
}

/// What starts a script.
#[derive(Enum, Copy, Clone, Eq, PartialEq, Debug)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ScriptTriggerValue {
    /// The import pipeline, after a file lands in the library.
    PostImport,
    /// The job scheduler, on the script's own schedule.
    Schedule,
}

impl From<ScriptTrigger> for ScriptTriggerValue {
    fn from(value: ScriptTrigger) -> Self {
        match value {
            ScriptTrigger::PostImport => Self::PostImport,
            ScriptTrigger::Schedule => Self::Schedule,
        }
    }
}

impl From<ScriptTriggerValue> for ScriptTrigger {
    fn from(value: ScriptTriggerValue) -> Self {
        match value {
            ScriptTriggerValue::PostImport => Self::PostImport,
            ScriptTriggerValue::Schedule => Self::Schedule,
        }
    }
}

/// Shape of a script schedule.
#[derive(Enum, Copy, Clone, Eq, PartialEq, Debug)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ScriptScheduleKindValue {
    /// Runs only when started by hand.
    Manual,
    /// Repeats every `everySeconds` seconds.
    Interval,
    /// Runs once a day at `timeLocal`.
    Daily,
    /// Runs on each of `days` at `timeLocal`.
    Weekly,
    /// Runs on a five-field crontab `expression`.
    Cron,
}

/// Day of the week for weekly schedules.
#[derive(Enum, Copy, Clone, Eq, PartialEq, Debug)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ScheduleWeekdayValue {
    /// Monday.
    Monday,
    /// Tuesday.
    Tuesday,
    /// Wednesday.
    Wednesday,
    /// Thursday.
    Thursday,
    /// Friday.
    Friday,
    /// Saturday.
    Saturday,
    /// Sunday.
    Sunday,
}

impl From<ScheduleWeekday> for ScheduleWeekdayValue {
    fn from(value: ScheduleWeekday) -> Self {
        match value {
            ScheduleWeekday::Monday => Self::Monday,
            ScheduleWeekday::Tuesday => Self::Tuesday,
            ScheduleWeekday::Wednesday => Self::Wednesday,
            ScheduleWeekday::Thursday => Self::Thursday,
            ScheduleWeekday::Friday => Self::Friday,
            ScheduleWeekday::Saturday => Self::Saturday,
            ScheduleWeekday::Sunday => Self::Sunday,
        }
    }
}

impl From<ScheduleWeekdayValue> for ScheduleWeekday {
    fn from(value: ScheduleWeekdayValue) -> Self {
        match value {
            ScheduleWeekdayValue::Monday => Self::Monday,
            ScheduleWeekdayValue::Tuesday => Self::Tuesday,
            ScheduleWeekdayValue::Wednesday => Self::Wednesday,
            ScheduleWeekdayValue::Thursday => Self::Thursday,
            ScheduleWeekdayValue::Friday => Self::Friday,
            ScheduleWeekdayValue::Saturday => Self::Saturday,
            ScheduleWeekdayValue::Sunday => Self::Sunday,
        }
    }
}

#[derive(InputObject, Clone, Debug)]
/// Schedule for a scheduled script. Only the fields of the chosen kind are read.
pub struct ScriptScheduleInput {
    /// Schedule shape.
    pub kind: ScriptScheduleKindValue,
    /// Interval length in seconds, at least 60. Required for `INTERVAL`.
    pub every_seconds: Option<i32>,
    /// Host-local time of day as 24-hour `HH:MM`. Required for `DAILY` and `WEEKLY`.
    pub time_local: Option<String>,
    /// Days to run on. Required for `WEEKLY`.
    pub days: Option<Vec<ScheduleWeekdayValue>>,
    /// Five-field crontab expression in host-local time. Required for `CRON`.
    pub expression: Option<String>,
}

impl ScriptScheduleInput {
    /// The domain schedule, or the name of the field the chosen kind is missing.
    pub fn into_domain(self) -> Result<ScriptSchedule, &'static str> {
        Ok(match self.kind {
            ScriptScheduleKindValue::Manual => ScriptSchedule::Manual,
            ScriptScheduleKindValue::Interval => ScriptSchedule::Interval {
                every_seconds: i64::from(self.every_seconds.ok_or("everySeconds")?),
            },
            ScriptScheduleKindValue::Daily => ScriptSchedule::Daily {
                time_local: self.time_local.ok_or("timeLocal")?,
            },
            ScriptScheduleKindValue::Weekly => ScriptSchedule::Weekly {
                days: self
                    .days
                    .ok_or("days")?
                    .into_iter()
                    .map(ScheduleWeekday::from)
                    .collect(),
                time_local: self.time_local.ok_or("timeLocal")?,
            },
            ScriptScheduleKindValue::Cron => ScriptSchedule::Cron {
                expression: self.expression.ok_or("expression")?,
            },
        })
    }
}

#[derive(SimpleObject, Clone, Debug)]
/// Stored schedule of a scheduled script.
pub struct ScriptSchedulePayload {
    /// Schedule shape.
    pub kind: ScriptScheduleKindValue,
    /// Interval length in seconds, for `INTERVAL`.
    pub every_seconds: Option<i32>,
    /// Host-local time of day as `HH:MM`, for `DAILY` and `WEEKLY`.
    pub time_local: Option<String>,
    /// Days to run on, for `WEEKLY`.
    pub days: Option<Vec<ScheduleWeekdayValue>>,
    /// Crontab expression, for `CRON`.
    pub expression: Option<String>,
}

impl From<ScriptSchedule> for ScriptSchedulePayload {
    fn from(value: ScriptSchedule) -> Self {
        let empty = Self {
            kind: ScriptScheduleKindValue::Manual,
            every_seconds: None,
            time_local: None,
            days: None,
            expression: None,
        };
        match value {
            ScriptSchedule::Manual => empty,
            ScriptSchedule::Interval { every_seconds } => Self {
                kind: ScriptScheduleKindValue::Interval,
                every_seconds: Some(i32::try_from(every_seconds).unwrap_or(i32::MAX)),
                ..empty
            },
            ScriptSchedule::Daily { time_local } => Self {
                kind: ScriptScheduleKindValue::Daily,
                time_local: Some(time_local),
                ..empty
            },
            ScriptSchedule::Weekly { days, time_local } => Self {
                kind: ScriptScheduleKindValue::Weekly,
                time_local: Some(time_local),
                days: Some(days.into_iter().map(ScheduleWeekdayValue::from).collect()),
                ..empty
            },
            ScriptSchedule::Cron { expression } => Self {
                kind: ScriptScheduleKindValue::Cron,
                expression: Some(expression),
                ..empty
            },
        }
    }
}

#[derive(SimpleObject, Clone, Debug)]
/// Result of checking a schedule before saving it.
pub struct ScriptScheduleValidationPayload {
    /// Whether the schedule can be saved.
    pub valid: bool,
    /// Why the schedule is invalid, or null when it is valid.
    pub error: Option<String>,
    /// Short description of the schedule; the raw expression for cron schedules.
    pub description: Option<String>,
    /// The next three fire times, empty for manual or invalid schedules.
    pub next_runs: Vec<DateTime<Utc>>,
}

#[derive(SimpleObject, Clone)]
/// Configured post-processing script and execution policy.
pub struct PostProcessingScriptPayload {
    /// Post-processing script ID.
    pub id: ID,
    /// Script display name.
    pub name: String,
    /// Script description.
    pub description: String,
    /// Script source type.
    pub script_type: String,
    /// Source text executed by the post-processing runtime.
    pub script_content: String,
    /// Media facets to which the script applies.
    pub applied_facets: Vec<String>,
    /// Workflow phase in which the script runs.
    pub execution_mode: ExecutionModeValue,
    /// Maximum runtime in seconds.
    pub timeout_secs: i32,
    /// Relative execution priority.
    pub priority: i32,
    /// Whether execution is enabled.
    pub enabled: bool,
    /// Whether debug output is enabled.
    pub debug: bool,
    /// Language of inline script content.
    pub language: ScriptLanguageValue,
    /// What starts the script.
    pub trigger: ScriptTriggerValue,
    /// Schedule of a scheduled script, or null for import-triggered scripts.
    pub schedule: Option<ScriptSchedulePayload>,
    /// Whether a scheduled script also runs once when the host starts.
    pub run_on_startup: bool,
    /// Short description of the schedule, or null when the script has none.
    pub schedule_description: Option<String>,
    /// Creation time in UTC.
    pub created_at: DateTime<Utc>,
    /// Last update time in UTC.
    pub updated_at: DateTime<Utc>,
}

#[derive(SimpleObject, Clone)]
/// Identifier of a deleted post-processing script.
pub struct DeletePostProcessingScriptPayload {
    /// Deleted script ID.
    pub id: ID,
}

#[derive(SimpleObject, Clone)]
/// Result of one post-processing script execution.
pub struct PostProcessingScriptRunPayload {
    /// Script run ID.
    pub id: ID,
    /// ID of the configured script that produced this run.
    pub script_id: ID,
    /// Script name at execution time.
    pub script_name: String,
    /// Associated title ID, or null when not title-specific.
    pub title_id: Option<ID>,
    /// Associated title name, or null when not title-specific.
    pub title_name: Option<String>,
    /// Media facet processed, or null when not facet-specific.
    pub facet: Option<MediaFacetValue>,
    /// File path processed, or null when no file path applied.
    pub file_path: Option<String>,
    /// Run status.
    pub status: String,
    /// Process exit code, or null when the process did not exit normally.
    pub exit_code: Option<i32>,
    /// Tail of standard output, when captured.
    pub stdout_tail: Option<String>,
    /// Tail of standard error, when captured.
    pub stderr_tail: Option<String>,
    /// Runtime in milliseconds.
    pub duration_ms: Option<i32>,
    /// Start time in UTC.
    pub started_at: DateTime<Utc>,
    /// Completion time in UTC, or null while the run is active.
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(InputObject)]
/// Values required to create a post-processing script.
pub struct CreatePostProcessingScriptInput {
    /// Script display name.
    pub name: String,
    /// Script description, or null for no description.
    pub description: Option<String>,
    /// Script source type.
    pub script_type: String,
    /// Script content, or null when supplied by the selected script type.
    pub script_content: Option<String>,
    /// Explicit acknowledgement that inline shell executes with application privileges.
    pub inline_shell_acknowledged: Option<bool>,
    /// Media facets to process, or null for the service default.
    pub applied_facets: Option<Vec<String>>,
    /// Execution mode, or null for the service default.
    pub execution_mode: Option<ExecutionModeValue>,
    /// Maximum runtime in seconds, or null for the service default.
    pub timeout_secs: Option<i32>,
    /// Relative execution priority, or null for the service default.
    pub priority: Option<i32>,
    /// Whether debug output is enabled, or null for the service default.
    pub debug: Option<bool>,
    /// Whether the script starts enabled, or null for true.
    pub enabled: Option<bool>,
    /// Language of inline script content, or null for `SHELL`.
    pub language: Option<ScriptLanguageValue>,
    /// What starts the script, or null for `POST_IMPORT`.
    pub trigger: Option<ScriptTriggerValue>,
    /// Schedule, required when the trigger is `SCHEDULE`.
    pub schedule: Option<ScriptScheduleInput>,
    /// Whether a scheduled script also runs once at host start, or null for false.
    pub run_on_startup: Option<bool>,
}

#[derive(InputObject)]
/// Values that may be changed on an existing post-processing script.
pub struct UpdatePostProcessingScriptInput {
    /// Script ID to update.
    pub id: ID,
    /// Replacement script name, or null to leave unchanged.
    pub name: Option<String>,
    /// Replacement description, or null to leave unchanged.
    pub description: Option<String>,
    /// Replacement script source type, or null to leave unchanged.
    pub script_type: Option<String>,
    /// Replacement script content, or null to leave unchanged.
    pub script_content: Option<String>,
    /// Explicit acknowledgement that inline shell executes with application privileges.
    pub inline_shell_acknowledged: Option<bool>,
    /// Replacement media facets, or null to leave unchanged.
    pub applied_facets: Option<Vec<String>>,
    /// Replacement execution mode, or null to leave unchanged.
    pub execution_mode: Option<ExecutionModeValue>,
    /// Replacement maximum runtime in seconds, or null to leave unchanged.
    pub timeout_secs: Option<i32>,
    /// Replacement execution priority, or null to leave unchanged.
    pub priority: Option<i32>,
    /// Replacement enabled state, or null to leave unchanged.
    pub enabled: Option<bool>,
    /// Replacement debug state, or null to leave unchanged.
    pub debug: Option<bool>,
    /// Replacement inline script language, or null to leave unchanged.
    pub language: Option<ScriptLanguageValue>,
    /// The trigger is fixed when a script is created. Omit this field, or
    /// pass the stored trigger; any other value is rejected with a validation
    /// error.
    pub trigger: Option<ScriptTriggerValue>,
    /// Replacement schedule, or null to leave unchanged.
    pub schedule: Option<ScriptScheduleInput>,
    /// Replacement startup-run setting, or null to leave unchanged.
    pub run_on_startup: Option<bool>,
}

// ── Script interpreters ────────────────────────────────────────────────────

#[derive(SimpleObject, Clone)]
/// Interpreters operator scripts are launched with.
pub struct ScriptInterpreterSettingsPayload {
    /// Python interpreter path or command, or null to use `python3`.
    pub python: Option<String>,
    /// PowerShell interpreter path or command, or null to use `pwsh`.
    pub powershell: Option<String>,
    /// Batch interpreter path or command, or null to use `COMSPEC` or `cmd.exe`.
    pub batch: Option<String>,
    /// Go toolchain path or command, or null to use `go`.
    pub go: Option<String>,
}

#[derive(InputObject, Clone)]
/// Interpreter pin changes for operator scripts. An omitted field keeps the
/// current pin; null or an empty string clears it.
pub struct ScriptInterpreterSettingsInput {
    /// Python interpreter path or command, or null to use `python3`.
    pub python: MaybeUndefined<String>,
    /// PowerShell interpreter path or command, or null to use `pwsh`.
    pub powershell: MaybeUndefined<String>,
    /// Batch interpreter path or command, or null to use `COMSPEC` or `cmd.exe`.
    pub batch: MaybeUndefined<String>,
    /// Go toolchain path or command, or null to use `go`.
    pub go: MaybeUndefined<String>,
}

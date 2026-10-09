use super::{ExecutionModeValue, MediaFacetValue};
use async_graphql::{ID, InputObject, MaybeUndefined, SimpleObject};
use chrono::{DateTime, Utc};

// ── Post-Processing Scripts ────────────────────────────────────────────────

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
